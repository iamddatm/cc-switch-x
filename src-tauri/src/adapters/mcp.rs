use crate::adapters::paths::expand_root;
use crate::domain::agent::{AgentDescriptor, AgentPaths, McpTarget};
use crate::domain::mcp::{McpServer, Transport};
use crate::error::AppError;
use crate::services::apply::{Action, PlanItem};
use crate::store::secrets::Secrets;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table};

/// 一条 MCP 在某个 agent 里的落点内容。
/// JSON 侧带 `type`；Codex 的 TOML 侧只写它认得的键（`command/args/env` 或 `url/http_headers`）。
#[derive(Debug, Clone)]
struct Rendered {
    transport: &'static str,
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    url: Option<String>,
    headers: BTreeMap<String, String>,
}

fn target_path(paths: &AgentPaths, base_override: Option<&Path>, home: &Path) -> PathBuf {
    expand_root(paths.mcp_file(), base_override, home)
}

fn location(paths: &AgentPaths) -> String {
    match &paths.mcp {
        McpTarget::JsonRoot { key, .. } => key.clone(),
        McpTarget::TomlTable { table, .. } => table.clone(),
    }
}

/// 把 canonical 里的 `${secret:…}` 解成明文。解不了就直接失败——
/// 绝不能把空值或残缺引用写进 agent 配置。
fn render(server: &McpServer, secrets: &Secrets) -> Result<Rendered, AppError> {
    let resolve = |value: &str| -> Result<String, AppError> {
        secrets.resolve_text(value).map_err(|error| {
            AppError::InvalidInput(format!("解析密钥失败（{}）：{error}", server.id))
        })
    };
    let mut env = BTreeMap::new();
    for (key, value) in &server.env {
        env.insert(key.clone(), resolve(value)?);
    }
    let mut headers = BTreeMap::new();
    for (key, value) in &server.headers {
        headers.insert(key.clone(), resolve(value)?);
    }
    Ok(Rendered {
        transport: match server.transport {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
            Transport::Sse => "sse",
            Transport::Ws => "ws",
        },
        command: server.command.clone(),
        args: server.args.clone(),
        env,
        url: server.url.as_deref().map(resolve).transpose()?,
        headers,
    })
}

impl Rendered {
    fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("type".into(), Value::String(self.transport.to_string()));
        if let Some(command) = &self.command {
            map.insert("command".into(), Value::String(command.clone()));
        }
        if !self.args.is_empty() {
            map.insert(
                "args".into(),
                Value::Array(self.args.iter().cloned().map(Value::String).collect()),
            );
        }
        if !self.env.is_empty() {
            map.insert("env".into(), string_map(&self.env));
        }
        if let Some(url) = &self.url {
            map.insert("url".into(), Value::String(url.clone()));
        }
        if !self.headers.is_empty() {
            map.insert("headers".into(), string_map(&self.headers));
        }
        Value::Object(map)
    }

    /// Codex 的 TOML 形态：stdio 写 command/args（env 走子表），远程写 url + http_headers。
    fn to_toml_table(&self) -> Table {
        let mut table = Table::new();
        match self.command.as_deref() {
            Some(command) => {
                table.insert("command", toml_edit::value(command));
                if !self.args.is_empty() {
                    let mut array = Array::new();
                    for arg in &self.args {
                        array.push(arg.clone());
                    }
                    table.insert("args", toml_edit::Value::Array(array).into());
                }
            }
            None => {
                if let Some(url) = &self.url {
                    table.insert("url", toml_edit::value(url.as_str()));
                }
                if !self.headers.is_empty() {
                    let mut sub = Table::new();
                    for (key, value) in &self.headers {
                        sub.insert(key, toml_edit::value(value.as_str()));
                    }
                    table.insert("http_headers", Item::Table(sub));
                }
            }
        }
        if !self.env.is_empty() {
            let mut sub = Table::new();
            for (key, value) in &self.env {
                sub.insert(key, toml_edit::value(value.as_str()));
            }
            table.insert("env", Item::Table(sub));
        }
        table
    }
}

fn string_map(values: &BTreeMap<String, String>) -> Value {
    let mut map = Map::new();
    for (key, value) in values {
        map.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(map)
}

pub fn plan_enable(
    agent: &AgentDescriptor,
    server: &McpServer,
    secrets: &Secrets,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PlanItem, AppError> {
    let rendered = render(server, secrets)?;
    let path = target_path(&agent.paths, base_override, home);
    let current = read_target(&path)?;
    let contents = match &agent.paths.mcp {
        McpTarget::JsonRoot { key, .. } => {
            merge_json(&current, key, &server.id, Some(&rendered.to_json()))?
        }
        McpTarget::TomlTable { table, .. } => {
            merge_toml(&current, table, &server.id, Some(&rendered))?
        }
    };
    Ok(plan_item(agent, &server.id, path, contents, Action::Enable))
}

pub fn plan_disable(
    agent: &AgentDescriptor,
    server_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PlanItem, AppError> {
    let path = target_path(&agent.paths, base_override, home);
    let current = read_target(&path)?;
    let contents = match &agent.paths.mcp {
        McpTarget::JsonRoot { key, .. } => merge_json(&current, key, server_id, None)?,
        McpTarget::TomlTable { table, .. } => merge_toml(&current, table, server_id, None)?,
    };
    Ok(plan_item(agent, server_id, path, contents, Action::Disable))
}

fn plan_item(
    agent: &AgentDescriptor,
    server_id: &str,
    path: PathBuf,
    contents: Vec<u8>,
    action: Action,
) -> PlanItem {
    let marker = match action {
        Action::Disable => '-',
        _ => '+',
    };
    PlanItem {
        agent_id: agent.id.clone(),
        asset_type: crate::domain::AssetType::Mcp,
        asset_id: server_id.to_string(),
        action,
        target_path: path,
        contents,
        diff: vec![format!("{marker} {}.{}", location(&agent.paths), server_id)],
        // 没有实测依据时按「要重启」处理，不把未知报成安全
        needs_restart: agent.mcp_needs_restart.unwrap_or(true),
    }
}

fn read_target(path: &Path) -> Result<Vec<u8>, AppError> {
    if path.exists() {
        Ok(std::fs::read(path)?)
    } else {
        Ok(Vec::new())
    }
}

/// 键级合并 JSON：只动 `root_key` 下的那一个子键，其余键与顺序原样保留。
/// 解析不了的 JSON 直接失败，绝不覆盖一个我们看不懂的文件。
fn merge_json(
    current: &[u8],
    root_key: &str,
    id: &str,
    entry: Option<&Value>,
) -> Result<Vec<u8>, AppError> {
    let text = String::from_utf8_lossy(current).to_string();
    let mut doc: Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|error| AppError::InvalidInput(format!("目标 JSON 解析失败：{error}")))?
    };
    let root = doc
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidInput("目标 JSON 根节点不是对象".into()))?;
    let servers = root
        .entry(root_key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidInput(format!("{root_key} 不是对象")))?;
    match entry {
        Some(value) => {
            servers.insert(id.to_string(), value.clone());
        }
        None => {
            servers.remove(id);
        }
    }
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out.into_bytes())
}

/// 键级合并 TOML：用 toml_edit 只增删 `[<table>.<id>]`，注释、空行、别的表逐字节不动。
fn merge_toml(
    current: &[u8],
    table_name: &str,
    id: &str,
    entry: Option<&Rendered>,
) -> Result<Vec<u8>, AppError> {
    let text = String::from_utf8_lossy(current).to_string();
    let mut doc: DocumentMut = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse()
            .map_err(|error| AppError::InvalidInput(format!("目标 TOML 解析失败：{error}")))?
    };
    if !doc[table_name].is_table() {
        doc[table_name] = Item::Table(Table::new());
    }
    let table = doc
        .get_mut(table_name)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| AppError::InvalidInput(format!("{table_name} 不是表")))?;
    match entry {
        Some(rendered) => {
            table.insert(id, Item::Table(rendered.to_toml_table()));
        }
        None => {
            table.remove(id);
        }
    }
    Ok(doc.to_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::domain::mcp::McpServer;
    use crate::store::secrets::Secrets;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn agent(id: &str) -> crate::domain::agent::AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn server(id: &str) -> McpServer {
        McpServer {
            id: id.into(),
            name: id.into(),
            transport: crate::domain::mcp::Transport::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into(), "pkg".into()],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            description: String::new(),
            homepage: String::new(),
            docs: String::new(),
            tags: Vec::new(),
        }
    }

    fn seeded(dir: &std::path::Path, agent_id: &str, content: &str) -> std::path::PathBuf {
        let descriptor = agent(agent_id);
        let path = expand_root_for(&descriptor, dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    /// 测试里把「home」直接当成临时目录，落点就是 dir/<去掉 ~/ 的部分>
    fn expand_root_for(
        descriptor: &crate::domain::agent::AgentDescriptor,
        home: &std::path::Path,
    ) -> std::path::PathBuf {
        crate::adapters::paths::expand_root(descriptor.paths.mcp_file(), None, home)
    }

    #[test]
    fn json_target_keeps_foreign_keys_and_their_order() {
        let dir = tempdir().unwrap();
        let path = seeded(
            dir.path(),
            "claude",
            r#"{"permissions":{"defaultMode":"acceptEdits"},"mcpServers":{"theirs":{"command":"x"}},"statusLine":{"type":"command","command":"echo hi"},"env":{"FOO":"bar"}}"#,
        );

        let plan = plan_enable(
            &agent("claude"),
            &server("newone"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        assert_eq!(plan.target_path, path);
        let after: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        let keys: Vec<String> = after.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            vec!["permissions", "mcpServers", "statusLine", "env"],
            "未管辖键必须原样保留且顺序不变"
        );
        assert_eq!(after["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(after["statusLine"]["command"], "echo hi");
        assert_eq!(after["env"]["FOO"], "bar");
        assert_eq!(
            after["mcpServers"]["theirs"]["command"], "x",
            "别人的 MCP 不能被删"
        );
        assert_eq!(after["mcpServers"]["newone"]["command"], "npx");
        assert_eq!(after["mcpServers"]["newone"]["type"], "stdio");
        assert_eq!(
            after["mcpServers"]["newone"]["args"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn json_disable_removes_only_our_key() {
        let dir = tempdir().unwrap();
        let plan = plan_disable(&agent("claude"), "theirs", dir.path(), None);
        // 目标文件不存在时不应炸，而是当作空对象处理
        let plan = plan.unwrap();
        let after: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        assert!(after["mcpServers"].get("theirs").is_none());
    }

    #[test]
    fn toml_target_touches_only_its_own_table() {
        let dir = tempdir().unwrap();
        let original = "# 顶部注释\nmodel = \"gpt\"\n\n[features]\nhooks = true\n\n[mcp_servers.theirs]\ncommand = \"keep\"\nargs = [\"a\"]\n\n# 尾部注释\n";
        let path = seeded(dir.path(), "codex", original);

        let plan = plan_enable(
            &agent("codex"),
            &server("codegraph"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        let after = String::from_utf8(plan.contents.clone()).unwrap();
        let original_lines: Vec<&str> = original.lines().collect();
        let after_lines: Vec<&str> = after.lines().collect();

        // 原有每一行都必须按原顺序保留（注释与空行也算）
        let mut cursor = 0usize;
        for line in &original_lines {
            let found = after_lines[cursor..]
                .iter()
                .position(|candidate| candidate == line)
                .unwrap_or_else(|| panic!("原有行被改掉或删掉：{line:?}\n--- 写入后 ---\n{after}"));
            cursor += found + 1;
        }
        // 多出来的行必须只有我们那一段
        let extra: Vec<&&str> = after_lines
            .iter()
            .filter(|line| !original_lines.contains(line))
            .collect();
        assert_eq!(
            extra,
            vec![
                &"[mcp_servers.codegraph]",
                &"command = \"npx\"",
                &"args = [\"-y\", \"pkg\"]"
            ],
            "只能多出我们自己的表：\n{after}"
        );
        assert!(path.is_file());
    }

    #[test]
    fn dotted_server_id_is_quoted_in_toml() {
        let dir = tempdir().unwrap();
        seeded(dir.path(), "codex", "[mcp_servers]\n");
        let plan = plan_enable(
            &agent("codex"),
            &server("code.graph"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        let after = String::from_utf8(plan.contents).unwrap();
        assert!(after.contains("[mcp_servers.\"code.graph\"]"), "{after}");
    }

    #[test]
    fn remote_transport_writes_url_not_command() {
        let dir = tempdir().unwrap();
        let mut remote = server("tavily");
        remote.transport = crate::domain::mcp::Transport::Http;
        remote.command = None;
        remote.url = Some("https://api.tavily.com/mcp".into());

        let plan = plan_enable(
            &agent("gemini"),
            &remote,
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        assert_eq!(value["mcpServers"]["tavily"]["type"], "http");
        assert_eq!(
            value["mcpServers"]["tavily"]["url"],
            "https://api.tavily.com/mcp"
        );
        assert!(value["mcpServers"]["tavily"].get("command").is_none());
    }

    #[test]
    fn unresolved_secret_fails_the_plan() {
        let dir = tempdir().unwrap();
        let mut with_secret = server("pw");
        with_secret
            .env
            .insert("API_TOKEN".into(), "${secret:mcp.pw.API_TOKEN}".into());
        let error = plan_enable(
            &agent("claude"),
            &with_secret,
            &Secrets::default(),
            dir.path(),
            None,
        )
        .expect_err("引用解析不了就不能出计划，更不能写空值");
        assert!(error.to_string().contains("缺少密钥"), "{error}");
    }

    #[test]
    fn resolved_secret_is_written_as_plaintext_into_the_agent_config() {
        let dir = tempdir().unwrap();
        let mut with_secret = server("pw");
        with_secret
            .env
            .insert("API_TOKEN".into(), "${secret:mcp.pw.API_TOKEN}".into());
        let mut secrets = Secrets::default();
        secrets.insert(
            &crate::domain::secret::SecretRef {
                scope: "mcp.pw".into(),
                key: "API_TOKEN".into(),
            },
            "sk-live",
        );

        let plan = plan_enable(&agent("claude"), &with_secret, &secrets, dir.path(), None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        assert_eq!(value["mcpServers"]["pw"]["env"]["API_TOKEN"], "sk-live");
    }

    #[test]
    fn an_override_root_relocates_every_write() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("real-home");
        let sandbox = dir.path().join("sandbox");
        std::fs::create_dir_all(&sandbox).unwrap();
        seeded(&sandbox, "claude", "{\"mcpServers\":{}}");

        let plan = plan_enable(
            &agent("claude"),
            &server("pw"),
            &Secrets::default(),
            &home,
            Some(&sandbox),
        )
        .unwrap();
        assert_eq!(plan.target_path, sandbox.join(".claude.json"));
        assert!(!plan.target_path.starts_with(&home), "绝不能写到真实 home");
    }

    #[test]
    fn agents_that_need_restart_are_flagged_on_the_plan() {
        let dir = tempdir().unwrap();
        seeded(dir.path(), "qoder-cn", "{\"mcpServers\":{}}");
        let plan = plan_enable(
            &agent("qoder-cn"),
            &server("pw"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        assert!(plan.needs_restart);
        assert!(!plan.diff.is_empty());
    }
}
