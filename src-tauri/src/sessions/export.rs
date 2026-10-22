//! 会话导出（Markdown）与恢复命令生成。

use crate::domain::session::{
    SessionDetail, SessionMessage, SessionMessageKind, SessionRole, SessionSummary,
};
use crate::sessions::{provider, SessionProvider};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeInfo {
    pub agent_id: String,
    pub session_id: String,
    pub cwd: Option<String>,
    pub command: String,
}

pub fn render_markdown(detail: &SessionDetail) -> String {
    let agent_name = provider(&detail.session.agent_id)
        .map(|provider| provider.display_name)
        .unwrap_or(&detail.session.agent_id);
    let project = detail
        .session
        .project_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or("(未知)");
    let mut markdown = String::new();
    markdown.push_str(&format!("# {}\n\n", detail.session.title));
    markdown.push_str(&format!(
        "- Agent: {agent_name}\n- Session: {}\n- Project: {project}\n- Updated: {}\n- Source: {}\n\n",
        detail.session.session_id,
        format_timestamp(detail.session.updated_at),
        detail.session.source_path,
    ));
    for (index, message) in detail.messages.iter().enumerate() {
        markdown.push_str(&format!(
            "## {}. {}{}\n\n",
            index + 1,
            role_label(message.role),
            kind_suffix(message),
        ));
        let content = if message.kind == SessionMessageKind::Image {
            image_placeholder(&message.content)
        } else {
            message.content.clone()
        };
        markdown.push_str(&content);
        markdown.push_str("\n\n");
    }
    markdown
}

pub fn resume_info(provider: &SessionProvider, summary: &SessionSummary) -> Option<ResumeInfo> {
    let resume = provider.resume?;
    let base = resume(summary)?;
    let cwd = summary
        .project_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(str::to_string);
    if let Some(path) = &cwd {
        if path.contains('"') || path.contains('\r') || path.contains('\n') || path.contains('\0') {
            return None;
        }
    }
    let command = match &cwd {
        Some(path) => format!("cd \"{path}\" && {base}"),
        None => base,
    };
    Some(ResumeInfo {
        agent_id: provider.agent_id.to_string(),
        session_id: summary.session_id.clone(),
        cwd,
        command,
    })
}

fn role_label(role: SessionRole) -> &'static str {
    match role {
        SessionRole::User => "User",
        SessionRole::Assistant => "Assistant",
        SessionRole::Tool => "Tool",
        SessionRole::System => "System",
    }
}

fn kind_suffix(message: &SessionMessage) -> String {
    let label = match message.kind {
        SessionMessageKind::Text => return String::new(),
        SessionMessageKind::Thinking => "Thinking".to_string(),
        SessionMessageKind::ToolCall => "Tool Call".to_string(),
        SessionMessageKind::ToolResult => "Tool Result".to_string(),
        SessionMessageKind::Image => "Image".to_string(),
        SessionMessageKind::Event => "Event".to_string(),
    };
    match (&message.kind, message.tool_name.as_deref()) {
        (SessionMessageKind::ToolCall | SessionMessageKind::ToolResult, Some(name)) => {
            format!(" · {label}: {name}")
        }
        _ => format!(" · {label}"),
    }
}

fn image_placeholder(content: &str) -> String {
    if let Some(rest) = content.strip_prefix("data:") {
        let media_type = rest.split(';').next().unwrap_or("未知类型");
        return format!("[图片：{media_type}]");
    }
    content.to_string()
}

fn format_timestamp(timestamp: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| timestamp.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::{SessionMessage, SessionMessageKind, SessionRole};
    use crate::sessions::provider;

    fn summary(project_path: Option<&str>) -> SessionSummary {
        SessionSummary {
            agent_id: "claude".into(),
            session_id: "s1".into(),
            title: "标题".into(),
            project_path: project_path.map(str::to_string),
            source_path: "D:/x/s1.jsonl".into(),
            created_at: None,
            updated_at: 1_700_000_000_000,
            message_count: 4,
            parse_error: None,
        }
    }

    fn message(
        seq: u32,
        role: SessionRole,
        kind: SessionMessageKind,
        content: &str,
        tool_name: Option<&str>,
    ) -> SessionMessage {
        SessionMessage {
            seq,
            role,
            kind,
            content: content.to_string(),
            tool_name: tool_name.map(str::to_string),
        }
    }

    fn detail(messages: Vec<SessionMessage>) -> SessionDetail {
        SessionDetail {
            session: summary(Some("D:/x")),
            messages,
            total: 0,
        }
    }

    #[test]
    fn markdown_export_keeps_order_and_roles() {
        let detail = detail(vec![
            message(0, SessionRole::User, SessionMessageKind::Text, "问题", None),
            message(
                1,
                SessionRole::Assistant,
                SessionMessageKind::Thinking,
                "推理",
                None,
            ),
            message(
                2,
                SessionRole::Assistant,
                SessionMessageKind::ToolCall,
                "{\"path\":\"a.rs\"}",
                Some("Read"),
            ),
            message(
                3,
                SessionRole::Tool,
                SessionMessageKind::ToolResult,
                "文件内容",
                Some("Read"),
            ),
        ]);

        let markdown = render_markdown(&detail);

        assert!(markdown.starts_with("# 标题\n"), "{markdown}");
        assert!(markdown.contains("- Agent: Claude Code"), "{markdown}");
        assert!(markdown.contains("- Session: s1"), "{markdown}");
        assert!(markdown.contains("- Project: D:/x"), "{markdown}");
        assert!(markdown.contains("- Source: D:/x/s1.jsonl"), "{markdown}");
        let headings = [
            "## 1. User",
            "## 2. Assistant · Thinking",
            "## 3. Assistant · Tool Call: Read",
            "## 4. Tool · Tool Result: Read",
        ];
        let mut last = 0;
        for heading in headings {
            let index = markdown
                .find(heading)
                .unwrap_or_else(|| panic!("missing heading {heading}\n{markdown}"));
            assert!(index >= last, "heading order: {markdown}");
            last = index;
        }
        assert!(markdown.contains("问题"));
        assert!(markdown.contains("推理"));
        assert!(markdown.contains("文件内容"));
    }

    #[test]
    fn markdown_export_never_inlines_image_base64() {
        let detail = detail(vec![message(
            0,
            SessionRole::User,
            SessionMessageKind::Image,
            "data:image/png;base64,QUJD",
            None,
        )]);

        let markdown = render_markdown(&detail);

        assert!(markdown.contains("[图片：image/png]"), "{markdown}");
        assert!(!markdown.contains("QUJD"), "{markdown}");
        assert!(!markdown.contains("base64"), "{markdown}");
    }

    #[test]
    fn resume_command_includes_the_project_directory_only_when_known() {
        let claude = provider("claude").unwrap();
        assert_eq!(
            resume_info(claude, &summary(Some("D:/x"))).unwrap().command,
            "cd \"D:/x\" && claude --resume s1"
        );
        assert_eq!(
            resume_info(claude, &summary(None)).unwrap().command,
            "claude --resume s1"
        );
    }

    #[test]
    fn a_project_path_that_could_break_out_of_the_command_is_refused() {
        let claude = provider("claude").unwrap();
        assert!(resume_info(claude, &summary(Some("D:/x\" && rm -rf /"))).is_none());
    }
}
