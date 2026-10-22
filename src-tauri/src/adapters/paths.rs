use std::path::{Path, PathBuf};

/// 把注册表里声明式路径（一律以 `~/` 开头）展开成真实路径。
///
/// `base_override` 覆盖的是 `~` 这个锚点，因此该 agent 的**所有**落点会一起重定位，
/// 包括 `~/.claude.json` 这种与配置根同级的文件。真机验证与非常规安装位置都靠它，
/// 用来保证「绝不拿用户真实配置当试验田」。
pub fn expand_root(declared: &str, base_override: Option<&Path>, home: &Path) -> PathBuf {
    let rest = declared.strip_prefix("~/").unwrap_or(declared);
    base_override.unwrap_or(home).join(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{builtin_agents, AgentDescriptor, McpTarget};
    use crate::domain::is_known_agent;
    use tempfile::tempdir;

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap_or_else(|| panic!("注册表里没有 {id}"))
    }

    #[test]
    fn claude_mcp_lives_in_claude_json_not_settings() {
        let claude = agent("claude");
        match &claude.paths.mcp {
            McpTarget::JsonRoot { file, key } => {
                assert_eq!(file, "~/.claude.json");
                assert_eq!(key, "mcpServers");
            }
            other => panic!("Claude 的 MCP 落点被改成了 {other:?}"),
        }
    }

    #[test]
    fn unmeasured_restart_semantics_stay_none_not_false() {
        // 只有 Qoder CN 有 requiresRestart 的实测依据；其余没有依据就不能写成「不用重启」
        assert_eq!(agent("claude").mcp_needs_restart, None);
        assert_eq!(agent("codex").mcp_needs_restart, None);
        assert_eq!(agent("qoder-cn").mcp_needs_restart, Some(true));
    }

    #[test]
    fn codex_mcp_is_a_toml_table() {
        match &agent("codex").paths.mcp {
            McpTarget::TomlTable { file, table } => {
                assert_eq!(file, "~/.codex/config.toml");
                assert_eq!(table, "mcp_servers");
            }
            other => panic!("Codex 的 MCP 落点被改成了 {other:?}"),
        }
    }

    #[test]
    fn qoder_ide_target_is_marked_unverified() {
        assert!(
            !agent("qoder").verified,
            "A2b 未实测：IDE 究竟读 mcp.json 还是 settings.json 没确认，不得当成已验证能力"
        );
    }

    #[test]
    fn agents_without_markdown_rules_declare_none() {
        // ~/.codex/rules/default.rules 是命令执行策略 DSL，不是 markdown 规则
        assert_eq!(agent("codex").paths.rules_dir, None);
        assert_eq!(agent("gemini").paths.rules_dir, None);
    }

    #[test]
    fn every_declared_path_is_home_anchored() {
        for descriptor in builtin_agents() {
            let declared = [
                descriptor.paths.mcp_file().to_string(),
                descriptor.paths.skills_dir.clone(),
                descriptor.paths.prompt_file.clone(),
            ];
            for path in declared {
                assert!(
                    path.starts_with("~/"),
                    "{} 的落点 {path} 没有以 ~/ 开头",
                    descriptor.id
                );
            }
        }
    }

    #[test]
    fn pi_declares_only_the_asset_types_it_actually_has() {
        let pi = agent("pi");
        assert!(pi.supports.mcp && pi.supports.skills && pi.supports.prompts);
        assert!(!pi.supports.rules && !pi.supports.subagents && !pi.supports.hooks);
        match &pi.paths.mcp {
            McpTarget::JsonRoot { file, key } => {
                assert_eq!(file, "~/.pi/agent/mcp.json");
                assert_eq!(key, "mcpServers");
            }
            other => panic!("pi 的 MCP 落点被改成了 {other:?}"),
        }
        assert_eq!(pi.paths.prompt_file, "~/.pi/agent/AGENTS.md");
        assert!(
            is_known_agent("pi"),
            "cc-switch 的 enabled_pi 列与 pi 的 prompt 行必须认得，否则导入会丢条目"
        );
    }

    #[test]
    fn expand_root_replaces_the_home_anchor() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("real-home");
        assert_eq!(
            expand_root("~/.claude.json", None, &home),
            home.join(".claude.json")
        );

        let sandbox = dir.path().join("sandbox");
        assert_eq!(
            expand_root("~/.claude.json", Some(&sandbox), &home),
            sandbox.join(".claude.json"),
            "真机验证靠 base 覆盖把写入引到临时目录"
        );
    }
}
