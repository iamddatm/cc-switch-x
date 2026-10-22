//! 会话 provider 注册表与文件发现。
//!
//! 每个工具一个 provider：发现规则、解析函数、恢复命令都记在注册表里，
//! 扫描器只认这一份表，不为「文件 provider / 数据库 provider」写两套循环。

use crate::adapters::paths::expand_root;
use crate::domain::session::{ParsedSession, SessionError, SessionSummary};
use std::path::{Path, PathBuf};

pub mod providers;

pub type ParseFn = fn(&Path) -> Result<Vec<ParsedSession>, SessionError>;
pub type ResumeFn = fn(&SessionSummary) -> Option<String>;

pub struct SessionProvider {
    pub agent_id: &'static str,
    pub display_name: &'static str,
    /// 声明式默认根，`~` 由 `expand_root` 展开；provider 自己的 `discover`
    /// 负责环境变量等特殊位置。
    pub roots: &'static [&'static str],
    /// 只有拿到真实样本并核对过的 provider 才能是 true。
    pub verified: bool,
    /// `override_active=true` 时只许看传进来的 root，不许读环境变量或真实家目录。
    pub discover: fn(&Path, bool) -> Vec<PathBuf>,
    pub parse: Option<ParseFn>,
    pub resume: Option<ResumeFn>,
}

/// 通用发现：递归收集满足 `keep` 的普通文件。符号链接不跟随，避免目录环。
pub(crate) fn walk_files(root: &Path, keep: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && keep(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

static PROVIDERS: &[SessionProvider] = &[
    SessionProvider {
        agent_id: "claude",
        display_name: "Claude Code",
        roots: &["~/.claude/projects"],
        verified: false,
        discover: providers::claude::discover_claude,
        parse: Some(providers::claude::parse_claude_all),
        resume: Some(providers::claude::resume_claude),
    },
    SessionProvider {
        agent_id: "codex",
        display_name: "Codex",
        roots: &["~/.codex/sessions", "~/.codex/archived_sessions"],
        verified: false,
        discover: providers::codex::discover_codex,
        parse: Some(providers::codex::parse_codex_all),
        resume: Some(providers::codex::resume_codex),
    },
    SessionProvider {
        agent_id: "gemini",
        display_name: "Gemini CLI",
        roots: &["~/.gemini/tmp"],
        verified: false,
        discover: providers::gemini::discover_gemini,
        parse: Some(providers::gemini::parse_gemini_all),
        resume: Some(providers::gemini::resume_gemini),
    },
    SessionProvider {
        agent_id: "grokbuild",
        display_name: "Grok Build",
        roots: &["~/.grok/sessions", "~/.grok/archived_sessions"],
        verified: false,
        discover: providers::grokbuild::discover_grokbuild,
        parse: Some(providers::grokbuild::parse_grokbuild_all),
        resume: Some(providers::grokbuild::resume_grokbuild),
    },
    SessionProvider {
        agent_id: "hermes",
        display_name: "Hermes",
        roots: &["~/.hermes", "~/AppData/Local/hermes"],
        verified: false,
        discover: providers::hermes::discover_hermes,
        parse: Some(providers::hermes::parse_hermes_all),
        resume: None,
    },
    SessionProvider {
        agent_id: "mcode",
        display_name: "MiniMax Code",
        roots: &["~/.minimax"],
        verified: false,
        discover: providers::mcode::discover_mcode,
        parse: Some(providers::mcode::parse_mcode_all),
        resume: Some(providers::mcode::resume_mcode),
    },
    SessionProvider {
        agent_id: "openclaw",
        display_name: "OpenClaw",
        roots: &["~/.openclaw"],
        verified: false,
        discover: providers::openclaw::discover_openclaw,
        parse: Some(providers::openclaw::parse_openclaw_all),
        resume: None,
    },
    SessionProvider {
        agent_id: "opencode",
        display_name: "OpenCode",
        roots: &["~/.local/share/opencode"],
        verified: false,
        discover: providers::opencode::discover_opencode,
        parse: Some(providers::opencode::parse_opencode_all),
        resume: Some(providers::opencode::resume_opencode),
    },
    SessionProvider {
        agent_id: "pi",
        display_name: "Pi Coding Agent",
        roots: &["~/.pi/agent/sessions"],
        verified: false,
        discover: providers::pi::discover_pi,
        parse: Some(providers::pi::parse_pi_all),
        resume: Some(providers::pi::resume_pi),
    },
    SessionProvider {
        agent_id: "agy",
        display_name: "Agy (Antigravity)",
        roots: &[
            "~/.gemini/antigravity-cli/brain",
            "~/.gemini/antigravity/brain",
        ],
        verified: false,
        discover: providers::agy::discover_agy,
        parse: Some(providers::agy::parse_agy_all),
        resume: Some(providers::agy::resume_agy),
    },
    SessionProvider {
        agent_id: "qoder-cn",
        display_name: "Qoder CN",
        roots: &["~/.qoder-cn/projects"],
        verified: false,
        discover: providers::qoder_cn::discover_qoder_cn,
        parse: Some(providers::qoder_cn::parse_qoder_cn_all),
        resume: Some(providers::qoder_cn::resume_qoder_cn),
    },
];

pub fn providers() -> &'static [SessionProvider] {
    PROVIDERS
}

pub fn provider(agent_id: &str) -> Option<&'static SessionProvider> {
    providers().iter().find(|item| item.agent_id == agent_id)
}

/// 按注册表的根发现候选文件；展开与去重都在这里，provider 只回答「这个根下有哪些文件」。
pub fn discover_provider(
    provider: &SessionProvider,
    home: &Path,
    base_override: Option<&Path>,
) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in provider.roots {
        let expanded = expand_root(root, base_override, home);
        files.extend((provider.discover)(&expanded, base_override.is_some()));
    }
    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::SessionMessageKind;

    #[test]
    fn provider_registry_covers_every_promised_agent() {
        let ids: Vec<_> = providers().iter().map(|p| p.agent_id).collect();
        for expected in [
            "claude",
            "codex",
            "gemini",
            "grokbuild",
            "hermes",
            "mcode",
            "openclaw",
            "opencode",
            "pi",
            "agy",
            "qoder-cn",
        ] {
            assert!(ids.contains(&expected), "缺少 {expected}：{ids:?}");
        }
        assert!(
            !ids.contains(&"qoder"),
            "国际版会话位置未确认（C2），不在承诺范围"
        );
    }

    #[test]
    fn base_override_replaces_the_home_anchor_for_every_root() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let sandbox = dir.path().join("sandbox");
        // 先放一个真实文件，否则「全都在 sandbox 下」对空集合是空转。
        std::fs::create_dir_all(sandbox.join(".claude/projects/p1")).unwrap();
        std::fs::write(sandbox.join(".claude/projects/p1/s1.jsonl"), "").unwrap();

        let claude = provider("claude").unwrap();
        let files = discover_provider(claude, &home, Some(&sandbox));
        assert!(!files.is_empty(), "覆盖目录里的会话文件必须被发现");
        assert!(files.iter().all(|p| p.starts_with(&sandbox)), "{files:?}");
        assert!(
            files.iter().all(|p| !p.starts_with(&home)),
            "覆盖后不能再去真实家目录"
        );
    }

    #[test]
    fn roles_and_kinds_roundtrip_through_db_strings() {
        for kind in SessionMessageKind::ALL {
            assert_eq!(SessionMessageKind::from_db(kind.as_db()).unwrap(), kind);
        }
        assert!(SessionMessageKind::from_db("nope").is_err());
    }
}
