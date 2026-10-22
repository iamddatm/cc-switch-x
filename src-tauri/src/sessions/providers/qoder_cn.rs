//! Qoder CN 会话解析器：主 JSONL 与 Claude 同构，复用 `ClaudeFamily`。

use crate::domain::session::{ParsedSession, SessionError, SessionSummary};
use crate::sessions::providers::claude::{parse_with_family, ClaudeFamily, ProjectFallback};
use crate::sessions::walk_files;
use std::path::{Path, PathBuf};

pub const QODER_CN_FAMILY: ClaudeFamily = ClaudeFamily {
    title_types: &[("ai-title", "aiTitle"), ("custom-title", "customTitle")],
    project_fallback: ProjectFallback::WorkspaceDirectories,
};

pub fn parse_qoder_cn(path: &Path) -> Result<ParsedSession, SessionError> {
    parse_with_family(path, &QODER_CN_FAMILY)
}

pub fn parse_qoder_cn_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    Ok(vec![parse_qoder_cn(path)?])
}

pub fn discover_qoder_cn(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    walk_files(root, &|path| {
        let is_jsonl = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
        is_jsonl
            && !path
                .components()
                .any(|part| part.as_os_str().eq_ignore_ascii_case("subagents"))
    })
}

pub fn resume_qoder_cn(summary: &SessionSummary) -> Option<String> {
    Some(format!("qoderclicn --resume {}", summary.session_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::SessionMessageKind;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    fn in_subagents(path: &Path) -> bool {
        path.components()
            .any(|part| part.as_os_str().eq_ignore_ascii_case("subagents"))
    }

    #[test]
    fn qoder_cn_uses_ai_title_and_workspace_directories() {
        let parsed = parse_qoder_cn(&fixture("qoder-cn.jsonl")).unwrap();
        assert_eq!(parsed.session_id, "q1");
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
        assert_eq!(parsed.title, "Qoder 会话标题");
        assert!(parsed
            .messages
            .iter()
            .any(|m| m.kind == SessionMessageKind::ToolCall));
    }

    #[test]
    fn qoder_cn_discovery_skips_subagent_files() {
        let files = discover_qoder_cn(&fixture("qoder-projects"), false);
        assert!(files.iter().any(|p| p.ends_with("main-q1.jsonl")));
        assert!(!files.iter().any(|p| in_subagents(p)), "{files:?}");
    }

    #[test]
    fn qoder_cn_resume_uses_the_session_id() {
        use crate::domain::session::SessionSummary;
        let summary = SessionSummary {
            agent_id: "qoder-cn".into(),
            session_id: "q1".into(),
            title: "t".into(),
            project_path: None,
            source_path: "D:/q1.jsonl".into(),
            created_at: None,
            updated_at: 0,
            message_count: 0,
            parse_error: None,
        };
        assert_eq!(
            resume_qoder_cn(&summary).as_deref(),
            Some("qoderclicn --resume q1")
        );
    }
}
