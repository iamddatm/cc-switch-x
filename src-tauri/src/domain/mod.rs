pub mod agent;
pub mod asset;
pub mod enablement;
pub mod frontmatter;
pub mod hook;
pub mod mcp;
pub mod prompt;
pub mod project;
pub mod rule;
pub mod secret;
pub mod session;
pub mod skill;
pub mod skill_meta;
pub mod subagent;

/// 列表行需要的、不落索引的可选元数据。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetDetails {
    pub name: Option<String>,
    pub description: Option<String>,
    pub transport: Option<String>,
}

pub use agent::{is_known_agent, AssetKind, AssetSupport, CapabilityStatus, SupportMap};
pub use asset::{AssetState, AssetSummary, AssetType, DriftKind};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("id 不能为空")]
    EmptyId,
    #[error("name 不能为空")]
    EmptyName,
    #[error("description 不能为空")]
    EmptyDescription,
    #[error("正文（提示词/指令）不能为空")]
    EmptyInstructions,
    #[error("命令不能为空")]
    EmptyCommand,
    #[error("未知的 hook 事件名：{0}")]
    UnknownHookEvent(String),
    #[error("MCP stdio 必须提供 command")]
    McpCommandRequired,
    #[error("MCP 非 stdio 必须提供 url")]
    McpUrlRequired,
    #[error("MCP id 与文件名不一致：{0} != {1}")]
    IdMismatch(String, String),
    #[error("frontmatter 未闭合")]
    UnclosedFrontmatter,
    #[error("YAML 解析失败：{0}")]
    Yaml(String),
    #[error("JSON 解析失败：{0}")]
    Json(String),
    #[error("文本内容不是合法 UTF-8：{0}")]
    Utf8(String),
}

/// 真源是文本文件，非 UTF-8 必须显式报错，不能当成空内容放过。
fn as_text(bytes: &[u8]) -> Result<&str, DomainError> {
    std::str::from_utf8(bytes).map_err(|e| DomainError::Utf8(e.to_string()))
}

pub fn parse_and_validate(
    asset_type: AssetType,
    id: &str,
    bytes: &[u8],
) -> Result<(), DomainError> {
    if id.trim().is_empty() {
        return Err(DomainError::EmptyId);
    }
    match asset_type {
        AssetType::Mcp => {
            let server: mcp::McpServer =
                serde_json::from_slice(bytes).map_err(|e| DomainError::Json(e.to_string()))?;
            if server.id != id {
                return Err(DomainError::IdMismatch(server.id, id.to_string()));
            }
            server.validate()
        }
        AssetType::Prompt => {
            prompt::parse_prompt(id, as_text(bytes)?)?;
            Ok(())
        }
        AssetType::Rule => {
            rule::parse_rule(id, as_text(bytes)?)?;
            Ok(())
        }
        AssetType::Subagent => {
            subagent::parse_subagent(as_text(bytes)?)?;
            Ok(())
        }
        AssetType::Hook => {
            hook::parse_hook(as_text(bytes)?)?;
            Ok(())
        }
        // 命令是逐字保留的 markdown：frontmatter 可有可无，不重排、不补字段，
        // 只要求是 UTF-8 且不是空白文件。
        AssetType::Command => {
            let text = as_text(bytes)?;
            if text.trim().is_empty() {
                return Err(DomainError::EmptyCommand);
            }
            Ok(())
        }
        AssetType::Skill => Ok(()),
    }
}

/// 从真源内容提取列表展示字段。解析失败只返回空元数据，不能影响资产本身。
pub fn describe(asset_type: AssetType, id: &str, bytes: &[u8]) -> AssetDetails {
    let Ok(text) = as_text(bytes) else {
        return AssetDetails::default();
    };
    match asset_type {
        AssetType::Skill => {
            let meta = skill::parse_metadata(text);
            AssetDetails {
                name: meta.name,
                description: meta.description,
                transport: None,
            }
        }
        AssetType::Mcp => {
            let Ok(server) = serde_json::from_slice::<mcp::McpServer>(bytes) else {
                return AssetDetails::default();
            };
            let transport = match server.transport {
                mcp::Transport::Stdio => "stdio",
                mcp::Transport::Http => "http",
                mcp::Transport::Sse => "sse",
                mcp::Transport::Ws => "ws",
            }
            .to_string();
            AssetDetails {
                name: Some(server.name.clone()),
                description: Some(mcp_summary(&server)),
                transport: Some(transport),
            }
        }
        AssetType::Prompt => match prompt::parse_prompt(id, text) {
            Ok(prompt) => AssetDetails {
                name: prompt.meta.title.clone(),
                description: prompt
                    .meta
                    .description
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| first_line(&prompt.body)),
                transport: None,
            },
            Err(_) => AssetDetails::default(),
        },
        AssetType::Rule => match rule::parse_rule(id, text) {
            Ok(rule) => AssetDetails {
                name: Some(rule.meta.name),
                description: non_empty(rule.meta.description),
                transport: None,
            },
            Err(_) => AssetDetails::default(),
        },
        AssetType::Subagent => match subagent::parse_subagent(text) {
            Ok(agent) => AssetDetails {
                name: Some(agent.name),
                description: non_empty(agent.description),
                transport: None,
            },
            Err(_) => AssetDetails::default(),
        },
        AssetType::Hook => match hook::parse_hook(text) {
            Ok(hook) => AssetDetails {
                name: Some(hook.name),
                description: non_empty(hook.event),
                transport: None,
            },
            Err(_) => AssetDetails::default(),
        },
        // 命令按文件名调用，名字就是 id；副标题取 frontmatter 的 description，
        // 没有就退到正文第一行（与提示词同一口径）。
        AssetType::Command => AssetDetails {
            name: None,
            description: command_description(text).or_else(|| {
                frontmatter::split_frontmatter(text)
                    .ok()
                    .and_then(|parts| first_line(&parts.body))
            }),
            transport: None,
        },
    }
}

fn command_description(text: &str) -> Option<String> {
    let parts = frontmatter::split_frontmatter(text).ok()?;
    let yaml = parts.yaml?;
    let value: serde_yaml::Value = serde_yaml::from_str(&yaml).ok()?;
    let description = value.get("description")?.as_str()?.trim().to_string();
    non_empty(description)
}

fn mcp_summary(server: &mcp::McpServer) -> String {
    let description = server.description.trim();
    if !description.is_empty() {
        return description.to_string();
    }
    match server.transport {
        mcp::Transport::Stdio => {
            let mut parts = Vec::new();
            if let Some(command) = server.command.as_deref() {
                parts.push(command.to_string());
            }
            parts.extend(server.args.iter().cloned());
            parts.join(" ")
        }
        _ => server.url.clone().unwrap_or_default(),
    }
}

fn first_line(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn display_name(asset_type: AssetType, id: &str, bytes: &[u8]) -> Result<String, DomainError> {
    match asset_type {
        AssetType::Mcp => {
            let server: mcp::McpServer =
                serde_json::from_slice(bytes).map_err(|e| DomainError::Json(e.to_string()))?;
            Ok(server.name)
        }
        AssetType::Prompt => Ok(prompt::parse_prompt(id, as_text(bytes)?)?
            .meta
            .title
            .unwrap_or_else(|| id.to_string())),
        AssetType::Rule => Ok(rule::parse_rule(id, as_text(bytes)?)?.meta.name),
        AssetType::Subagent => Ok(subagent::parse_subagent(as_text(bytes)?)?.name),
        AssetType::Hook => Ok(hook::parse_hook(as_text(bytes)?)?.name),
        // 命令的调用名来自文件名，不随 frontmatter 变化。
        AssetType::Command => Ok(id.to_string()),
        AssetType::Skill => Ok(id.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_type_dirs_are_stable() {
        assert_eq!(AssetType::Mcp.dir_name(), "mcp");
        assert_eq!(AssetType::Skill.dir_name(), "skills");
        assert_eq!(AssetType::Prompt.file_extension(), Some("md"));
    }

    #[test]
    fn parses_mcp_and_rejects_missing_command() {
        let ok =
            br#"{"id":"pw","name":"playwright","transport":"stdio","command":"npx","args":[]}"#;
        assert!(parse_and_validate(AssetType::Mcp, "pw", ok).is_ok());

        let bad = br#"{"id":"pw","name":"playwright","transport":"stdio"}"#;
        assert_eq!(
            parse_and_validate(AssetType::Mcp, "pw", bad),
            Err(DomainError::McpCommandRequired)
        );
    }

    #[test]
    fn parses_prompt_with_bom_and_crlf() {
        let raw = "\u{feff}---\r\ntitle: 团队提示词\r\n---\r\n\r\n正文\r\n";
        let prompt = prompt::parse_prompt("team", raw).unwrap();
        assert_eq!(prompt.meta.title.as_deref(), Some("团队提示词"));
        assert_eq!(prompt.body, "正文\n");
    }

    #[test]
    fn a_command_is_verbatim_markdown_and_may_omit_frontmatter() {
        // 没有 frontmatter 的命令也合法；名字沿用文件名
        assert!(parse_and_validate(AssetType::Command, "review", "# 审查\n".as_bytes()).is_ok());
        assert_eq!(
            display_name(AssetType::Command, "review", "# 审查\n".as_bytes()).unwrap(),
            "review"
        );
        // 空白命令拒绝
        assert_eq!(
            parse_and_validate(AssetType::Command, "empty", b"  \n"),
            Err(DomainError::EmptyCommand)
        );
    }

    #[test]
    fn command_description_comes_from_frontmatter_then_the_first_body_line() {
        let details = describe(
            AssetType::Command,
            "review",
            "---\ndescription: 审查并合并\n---\n\n正文\n".as_bytes(),
        );
        assert_eq!(details.description.as_deref(), Some("审查并合并"));

        let details = describe(
            AssetType::Command,
            "review",
            "\n\n没有 frontmatter 的第一行\n".as_bytes(),
        );
        assert_eq!(
            details.description.as_deref(),
            Some("没有 frontmatter 的第一行")
        );
    }

    #[test]
    fn rejects_mcp_id_mismatch_and_non_utf8_text() {
        let bytes = br#"{"id":"other","name":"x","transport":"stdio","command":"npx"}"#;
        assert_eq!(
            parse_and_validate(AssetType::Mcp, "pw", bytes),
            Err(DomainError::IdMismatch("other".into(), "pw".into()))
        );

        let gbk: Vec<u8> = vec![0xbd, 0xf1, 0xce, 0xef]; // “金文”的 GBK 字节，非 UTF-8
        assert!(matches!(
            parse_and_validate(AssetType::Prompt, "p", &gbk),
            Err(DomainError::Utf8(_))
        ));
    }
}
