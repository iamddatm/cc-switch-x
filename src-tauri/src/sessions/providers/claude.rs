//! Claude Code 会话解析器；Qoder CN 的 JSONL 与它同构，复用同一份 `ClaudeFamily`。
//!
//! 真实样本（本机 291 个主会话）里首行通常是 `mode`/`ai-title` 之类的控制行，
//! 标题在重复出现的 `ai-title` 里；消息信封才带 `cwd`/`sessionId`/`timestamp`。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
    SessionWarning,
};
use crate::sessions::walk_files;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 512 * 1024;
const TITLE_MAX_CHARS: usize = 80;
const IGNORED_TITLE_PREFIXES: [&str; 4] = [
    "<local-command-caveat>",
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<task-notification>",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectFallback {
    None,
    WorkspaceDirectories,
}

/// Claude 家族的可配置部分：标题字段与项目目录回退。
pub struct ClaudeFamily {
    /// 按优先级排列的 `(记录 type, 标题字段)`；重复出现取最后一次。
    pub title_types: &'static [(&'static str, &'static str)],
    pub project_fallback: ProjectFallback,
}

pub const CLAUDE_FAMILY: ClaudeFamily = ClaudeFamily {
    title_types: &[("ai-title", "aiTitle"), ("custom-title", "customTitle")],
    project_fallback: ProjectFallback::None,
};

pub fn parse_claude(path: &Path) -> Result<ParsedSession, SessionError> {
    parse_with_family(path, &CLAUDE_FAMILY)
}

pub fn parse_claude_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    Ok(vec![parse_claude(path)?])
}

pub fn discover_claude(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    walk_files(root, &|path| {
        let is_jsonl = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
        if !is_jsonl {
            return false;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if name == "journal.jsonl" || name.starts_with("agent-") {
            return false;
        }
        !path
            .components()
            .any(|part| part.as_os_str().eq_ignore_ascii_case("subagents"))
    })
}

pub fn resume_claude(summary: &SessionSummary) -> Option<String> {
    Some(format!("claude --resume {}", summary.session_id))
}

struct Builder {
    session_id: Option<String>,
    project_path: Option<PathBuf>,
    fallback_project: Option<PathBuf>,
    created_at: Option<i64>,
    updated_at: Option<i64>,
    title_candidates: HashMap<&'static str, String>,
    tool_names: HashMap<String, String>,
    messages: Vec<ParsedMessage>,
    warnings: Vec<SessionWarning>,
}

impl Builder {
    fn new() -> Self {
        Self {
            session_id: None,
            project_path: None,
            fallback_project: None,
            created_at: None,
            updated_at: None,
            title_candidates: HashMap::new(),
            tool_names: HashMap::new(),
            messages: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn warning(&mut self, line: usize, message: impl Into<String>) {
        self.warnings.push(SessionWarning {
            line,
            message: message.into(),
        });
    }

    fn note_envelope(&mut self, value: &Value) {
        if self.session_id.is_none() {
            self.session_id = value
                .get("sessionId")
                .or_else(|| value.get("session_id"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if self.project_path.is_none() {
            self.project_path = value.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        }
        if let Some(timestamp) = value.get("timestamp").and_then(parse_timestamp) {
            self.created_at.get_or_insert(timestamp);
            self.updated_at = Some(timestamp);
        }
    }

    fn push(
        &mut self,
        role: SessionRole,
        kind: SessionMessageKind,
        content: String,
        tool_name: Option<String>,
    ) {
        if content.is_empty() {
            return;
        }
        self.messages.push(ParsedMessage {
            role,
            kind,
            content,
            tool_name,
        });
    }
}

fn parse_with_family(path: &Path, family: &ClaudeFamily) -> Result<ParsedSession, SessionError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_no = 0usize;
    let mut builder = Builder::new();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_no += 1;
        if line.len() > MAX_LINE_BYTES {
            builder.warning(line_no, "单行超过 8 MiB，已跳过");
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                builder.warning(line_no, format!("JSON 解析失败：{error}"));
                continue;
            }
        };
        if value.get("isSidechain").and_then(Value::as_bool) == Some(true)
            || value.get("isMeta").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }

        let record_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if record_type == "workspace-directories"
            && family.project_fallback == ProjectFallback::WorkspaceDirectories
            && builder.fallback_project.is_none()
        {
            builder.fallback_project = value
                .get("directories")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str)
                .map(PathBuf::from);
            continue;
        }
        let title_hit = family
            .title_types
            .iter()
            .find(|(record, _)| *record == record_type)
            .map(|(record, field)| (*record, *field));
        if let Some((record, field)) = title_hit {
            if let Some(title) = value
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                builder.title_candidates.insert(record, title.to_string());
            }
            continue;
        }

        match record_type {
            "user" | "assistant" => {
                builder.note_envelope(&value);
                if let Some(message) = value.get("message") {
                    let envelope_role = match record_type {
                        "assistant" => SessionRole::Assistant,
                        _ => SessionRole::User,
                    };
                    process_content(message.get("content"), envelope_role, &mut builder, line_no);
                }
            }
            "system" => {
                if let Some(text) = value
                    .get("content")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    builder.push(
                        SessionRole::System,
                        SessionMessageKind::Event,
                        text.to_string(),
                        None,
                    );
                }
            }
            _ => {}
        }
    }

    let session_id = builder
        .session_id
        .clone()
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .ok_or(SessionError::MissingId)?;

    let project_path = builder
        .project_path
        .clone()
        .or(builder.fallback_project.clone());
    let title = choose_title(family, &builder, project_path.as_deref(), &session_id);
    let updated_at = builder
        .updated_at
        .unwrap_or_else(|| file_mtime_millis(path));

    Ok(ParsedSession {
        session_id,
        title,
        project_path,
        created_at: builder.created_at,
        updated_at,
        messages: builder.messages,
        warnings: builder.warnings,
    })
}

fn process_content(
    content: Option<&Value>,
    envelope_role: SessionRole,
    builder: &mut Builder,
    line: usize,
) {
    match content {
        Some(Value::String(text)) => {
            /* 文本正文允许保留前导空白，但纯空白没有阅读价值 */
            if !text.trim().is_empty() {
                builder.push(envelope_role, SessionMessageKind::Text, text.clone(), None);
            }
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                process_block(block, envelope_role, builder, line);
            }
        }
        _ => {}
    }
}

fn process_block(block: &Value, envelope_role: SessionRole, builder: &mut Builder, line: usize) {
    let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
    match block_type {
        "text" => {
            if let Some(text) = block
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                builder.push(
                    envelope_role,
                    SessionMessageKind::Text,
                    text.to_string(),
                    None,
                );
            }
        }
        "thinking" => {
            if let Some(text) = block
                .get("thinking")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                builder.push(
                    envelope_role,
                    SessionMessageKind::Thinking,
                    text.to_string(),
                    None,
                );
            }
        }
        "redacted_thinking" => {
            builder.push(
                envelope_role,
                SessionMessageKind::Thinking,
                "[已加密的思考]".to_string(),
                None,
            );
        }
        "tool_use" | "server_tool_use" => {
            let name = block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            if let Some(id) = block.get("id").and_then(Value::as_str) {
                builder.tool_names.insert(id.to_string(), name.clone());
            }
            let input = block.get("input").cloned().unwrap_or(Value::Null);
            let content = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
            builder.push(
                SessionRole::Assistant,
                SessionMessageKind::ToolCall,
                content,
                Some(name),
            );
        }
        "tool_result" => {
            let call_id = block.get("tool_use_id").and_then(Value::as_str);
            let tool_name = call_id.and_then(|id| builder.tool_names.get(id)).cloned();
            let mut text = flatten_content(block.get("content"));
            if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                text = format!("[error] {text}");
            }
            builder.push(
                SessionRole::Tool,
                SessionMessageKind::ToolResult,
                text,
                tool_name,
            );
            for image in images_in_content(block.get("content")) {
                builder.push(SessionRole::Tool, SessionMessageKind::Image, image, None);
            }
        }
        "image" => {
            let content = image_content(block.get("source"));
            builder.push(envelope_role, SessionMessageKind::Image, content, None);
        }
        other => {
            let content = serde_json::to_string(block).unwrap_or_default();
            builder.warning(line, format!("未知内容块类型：{other}"));
            builder.push(envelope_role, SessionMessageKind::Text, content, None);
        }
    }
}

fn flatten_content(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Null) | None => String::new(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn images_in_content(content: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(blocks)) = content else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("image"))
        .map(|block| image_content(block.get("source")))
        .collect()
}

fn image_content(source: Option<&Value>) -> String {
    let media_type = source
        .and_then(|source| source.get("media_type"))
        .and_then(Value::as_str)
        .unwrap_or("未知类型");
    let Some(source) = source else {
        return format!("[图片：{media_type}，缺少来源]");
    };
    if source.get("type").and_then(Value::as_str) != Some("base64") {
        return format!("[图片：{media_type}，来源不是内联 base64]");
    }
    let Some(data) = source.get("data").and_then(Value::as_str) else {
        return format!("[图片：{media_type}，缺少内联数据]");
    };
    if data.len() > MAX_IMAGE_BYTES {
        return format!(
            "[图片：{media_type}，{} 字节，超过 512 KiB 未内联]",
            data.len()
        );
    }
    format!("data:{media_type};base64,{data}")
}

fn choose_title(
    family: &ClaudeFamily,
    builder: &Builder,
    project_path: Option<&Path>,
    session_id: &str,
) -> String {
    for (record, _) in family.title_types {
        if let Some(title) = builder.title_candidates.get(record) {
            if !title.is_empty() {
                return truncate_chars(title, TITLE_MAX_CHARS);
            }
        }
    }
    if let Some(first) = builder.messages.iter().find(|message| {
        message.role == SessionRole::User
            && message.kind == SessionMessageKind::Text
            && !IGNORED_TITLE_PREFIXES
                .iter()
                .any(|prefix| message.content.trim_start().starts_with(prefix))
    }) {
        let title = first.content.trim();
        if !title.is_empty() {
            return truncate_chars(title, TITLE_MAX_CHARS);
        }
    }
    if let Some(name) = project_path
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
    {
        return truncate_chars(&name, TITLE_MAX_CHARS);
    }
    truncate_chars(session_id, TITLE_MAX_CHARS)
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn file_mtime_millis(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::SessionMessageKind::*;
    use std::path::Path;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    fn fixture_dir(name: &str) -> PathBuf {
        fixture(name)
    }

    #[test]
    fn claude_extracts_text_thinking_calls_results_and_images() {
        let parsed = parse_claude(&fixture("claude.jsonl")).unwrap();
        assert_eq!(parsed.session_id, "s1");
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
        assert_eq!(parsed.title, "修复登录");
        let kinds: Vec<_> = parsed.messages.iter().map(|m| m.kind).collect();
        assert_eq!(
            kinds,
            vec![Text, Thinking, Text, ToolCall, ToolResult, Image]
        );
        assert_eq!(parsed.messages[3].tool_name.as_deref(), Some("Read"));
        assert_eq!(parsed.messages[4].tool_name.as_deref(), Some("Read"));
        assert!(parsed.messages[5]
            .content
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn claude_takes_the_last_ai_title_and_ignores_control_lines() {
        let parsed = parse_claude(&fixture("claude-with-control-lines.jsonl")).unwrap();
        assert_eq!(parsed.title, "最后的标题");
        assert!(parsed.warnings.is_empty(), "控制行不能被当成坏行");
    }

    #[test]
    fn a_bad_claude_line_is_reported_and_the_rest_survives() {
        let parsed = parse_claude(&fixture("claude-with-broken-line.jsonl")).unwrap();
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.warnings[0].line, 4);
        assert_eq!(parsed.messages.len(), 6, "坏行前后的消息都要在");
    }

    #[test]
    fn sidechains_are_dropped_without_warnings() {
        let parsed = parse_claude(&fixture("claude-with-sidechain.jsonl")).unwrap();
        assert!(parsed.warnings.is_empty());
        assert!(!parsed
            .messages
            .iter()
            .any(|m| m.content.contains("子代理内部")));
    }

    #[test]
    fn claude_discovery_skips_subagent_and_journal_files() {
        let files = discover_claude(&fixture_dir("claude-projects"), false);
        assert!(files.iter().any(|p| p.ends_with("s1.jsonl")));
        assert!(!files
            .iter()
            .any(|p| p.to_string_lossy().contains("/subagents/")));
        assert!(!files.iter().any(|p| p.ends_with("journal.jsonl")));
        assert!(!files.iter().any(|p| p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("agent-")));
    }

    #[test]
    fn an_oversized_inline_image_becomes_a_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge-image.jsonl");
        let data = "A".repeat(512 * 1024 + 1);
        let line = serde_json::json!({
            "type": "user",
            "sessionId": "s1",
            "cwd": "D:/repo",
            "timestamp": "2026-10-20T10:00:00Z",
            "message": {
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": data }
                }]
            }
        });
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let parsed = parse_claude(&path).unwrap();
        let image = parsed.messages.iter().find(|m| m.kind == Image).unwrap();
        assert!(image.content.starts_with("[图片："), "{}", image.content);
        assert!(!image.content.contains("base64"));
    }
}
