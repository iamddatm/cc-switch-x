use crate::adapters::cli::CliChannel;
use std::path::{Path, PathBuf};

/// 一次落盘的「往哪儿写、能不能走 CLI」。所有适配器共用，避免每个入口都拖三个参数。
/// `base_override` 覆盖的是 `~` 这个锚点，所以该 agent 的全部落点会一起重定位 ——
/// 真机验证与非常规安装位置都靠它，用来保证「绝不拿用户真实配置当试验田」。
#[derive(Debug)]
pub struct WriteTarget<'a> {
    pub home: &'a Path,
    pub base_override: Option<&'a Path>,
    pub channel: CliChannel,
}

impl<'a> WriteTarget<'a> {
    pub fn new(home: &'a Path, base_override: Option<&'a Path>) -> Self {
        Self {
            home,
            base_override,
            channel: CliChannel::NotApplicable,
        }
    }

    pub fn with_channel(mut self, channel: CliChannel) -> Self {
        self.channel = channel;
        self
    }

    pub fn expand(&self, declared: &str) -> PathBuf {
        expand_root(declared, self.base_override, self.home)
    }
}

/// 把注册表里声明式路径（一律以 `~/` 开头）展开成真实路径。
pub fn expand_root(declared: &str, base_override: Option<&Path>, home: &Path) -> PathBuf {
    let rest = declared.strip_prefix("~/").unwrap_or(declared);
    base_override.unwrap_or(home).join(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{builtin_agents, AgentDescriptor, HooksTarget, McpTarget};
    use crate::domain::{is_known_agent, AssetKind};
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
            !agent("qoder").switchable(AssetKind::Mcp),
            "A2b 未实测：IDE 究竟读 mcp.json 还是 settings.json 没确认，不得当成已验证能力"
        );
        assert!(
            agent("qoder").supported(AssetKind::Mcp),
            "落点是存在的，只是没验证，UI 要显示「未实测」而不是「不支持」"
        );
    }

    #[test]
    fn agents_without_markdown_rules_declare_none() {
        // ~/.codex/rules/default.rules 是命令执行策略 DSL，不是 markdown 规则
        assert_eq!(agent("codex").paths.rules_dir, None);
        assert_eq!(agent("gemini").paths.rules_dir, None);
    }

    #[test]
    fn capability_table_and_declared_targets_never_disagree() {
        // 「支持」与「有落点」必须同进同退：前者开开关、后者给路径，
        // 任一边单独亮灯都会让面板对着不存在的目录写入。
        for descriptor in builtin_agents() {
            let rules_agree =
                descriptor.supported(AssetKind::Rule) == descriptor.paths.rules_dir.is_some();
            assert!(
                rules_agree,
                "{} 的 rules 能力与 rules_dir 不一致",
                descriptor.id
            );

            let agents_agree =
                descriptor.supported(AssetKind::Subagent) == descriptor.paths.agents_dir.is_some();
            assert!(agents_agree, "{} 的 subagents 落点不一致", descriptor.id);

            let has_hooks_target = matches!(descriptor.paths.hooks, HooksTarget::Json { .. });
            assert_eq!(
                descriptor.supported(AssetKind::Hook),
                has_hooks_target,
                "{} 的 hooks 能力与 hooks 落点不一致",
                descriptor.id
            );
        }
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
        assert!(pi.supported(AssetKind::Mcp));
        assert!(pi.switchable(AssetKind::Skill));
        assert!(pi.switchable(AssetKind::Prompt));
        for kind in [AssetKind::Rule, AssetKind::Subagent, AssetKind::Hook] {
            assert!(!pi.supported(kind), "pi 不该有 {kind:?} 落点");
        }
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
