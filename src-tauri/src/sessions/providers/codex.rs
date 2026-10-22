//! Codex 会话解析器：`sessions/YYYY/MM/DD/rollout-*.jsonl`（另支持 `.jsonl.zst`）。
//!
//! 真实样本里 `session_meta` 会在长会话中途重复出现，只有第一份是身份来源；
//! `response_item` 是消息的 canonical 源，`event_msg` 里的正文副本必须忽略，否则双计。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
    SessionWarning,
};
use crate::sessions::walk_files;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 512 * 1024;
const TITLE_MAX_CHARS: usize = 80;
const IGNORED_TITLE_PREFIXES: [&str; 3] = [
    "# AGENTS.md instructions",
    "<environment_context>",
    "<user_instructions>",
];

pub fn parse_codex(path: &Path) -> Result<ParsedSession, SessionError> {
    parse_inner(path).map(|(session, _)| session)
}

pub fn parse_codex_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let (session, skipped) = parse_inner(path)?;
    if skipped {
        Ok(Vec::new())
    } else {
        Ok(vec![session])
    }
}

pub fn discover_codex(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    let mut files = walk_files(root, &|path| is_plain(path) || is_compressed(path));
    let plain: HashSet<PathBuf> = files
        .iter()
        .filter(|path| is_plain(path))
        .cloned()
        .collect();
    files.retain(|path| match plain_twin(path) {
        Some(twin) => !plain.contains(&twin),
        None => true,
    });
    files
}

pub fn resume_codex(summary: &SessionSummary) -> Option<String> {
    Some(format!("codex resume {}", summary.session_id))
}

fn is_plain(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
}

fn is_compressed(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
}

fn plain_twin(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let plain = name.strip_suffix(".zst")?;
    Some(path.with_file_name(plain))
}

struct Builder {
    session_id: Option<String>,
    project_path: Option<PathBuf>,
    created_at: Option<i64>,
    updated_at: Option<i64>,
    title: Option<String>,
    tool_names: HashMap<String, String>,
    messages: Vec<ParsedMessage>,
    warnings: Vec<SessionWarning>,
    meta_seen: bool,
    skipped: bool,
}

impl Builder {
    fn new() -> Self {
        Self {
            session_id: None,
            project_path: None,
            created_at: None,
            updated_at: None,
            title: None,
            tool_names: HashMap::new(),
            messages: Vec::new(),
            warnings: Vec::new(),
            meta_seen: false,
            skipped: false,
        }
    }

    fn warning(&mut self, line: usize, message: impl Into<String>) {
        self.warnings.push(SessionWarning {
            line,
            message: message.into(),
        });
    }

    fn note_time(&mut self, value: &Value, payload: Option<&Value>) {
        let timestamp = value
            .get("timestamp")
            .and_then(parse_timestamp)
            .or_else(|| {
                payload
                    .and_then(|payload| payload.get("timestamp"))
                    .and_then(parse_timestamp)
            });
        if let Some(timestamp) = timestamp {
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
        if content.is_empty() || self.skipped {
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

fn parse_inner(path: &Path) -> Result<(ParsedSession, bool), SessionError> {
    let compressed = is_compressed(path);
    let reader = open_reader(path)?;
    let mut reader = reader;
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
        let record_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = value.get("payload");
        builder.note_time(&value, payload);

        match record_type {
            "session_meta" => {
                if builder.meta_seen {
                    continue;
                }
                builder.meta_seen = true;
                let Some(payload) = payload else {
                    continue;
                };
                builder.session_id = payload
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| builder.session_id.clone());
                builder.project_path = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .or_else(|| builder.project_path.clone());
                if payload
                    .get("source")
                    .and_then(|source| source.get("subagent"))
                    .is_some()
                {
                    builder.skipped = true;
                }
            }
            "turn_context" => {
                if builder.project_path.is_none() {
                    builder.project_path = payload
                        .and_then(|payload| payload.get("cwd"))
                        .and_then(Value::as_str)
                        .map(PathBuf::from);
                }
            }
            "response_item" => {
                if let Some(payload) = payload {
                    process_response_item(payload, &mut builder, line_no);
                }
            }
            "event_msg" => {
                if let Some(payload) = payload {
                    process_event_msg(payload, &mut builder);
                }
            }
            _ => {}
        }
    }

    if builder.skipped {
        builder.messages.clear();
    }

    let session_id = builder
        .session_id
        .clone()
        .or_else(|| session_id_from_filename(path))
        .filter(|id| !id.is_empty())
        .ok_or(SessionError::MissingId)?;
    let title = choose_title(&builder, session_id.as_str());
    let updated_at = if compressed {
        file_mtime_millis(path)
    } else {
        builder
            .updated_at
            .unwrap_or_else(|| file_mtime_millis(path))
    };

    Ok((
        ParsedSession {
            session_id,
            title,
            project_path: builder.project_path,
            created_at: builder.created_at,
            updated_at,
            messages: builder.messages,
            warnings: builder.warnings,
        },
        builder.skipped,
    ))
}

fn open_reader(path: &Path) -> Result<Box<dyn BufRead>, SessionError> {
    let file = File::open(path)?;
    if is_compressed(path) {
        let decoder = zstd::stream::read::Decoder::new(file)?;
        Ok(Box::new(BufReader::new(decoder)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

fn process_response_item(payload: &Value, builder: &mut Builder, line: usize) {
    let item_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    match item_type {
        "message" => {
            let role = match payload.get("role").and_then(Value::as_str).unwrap_or("") {
                "user" => SessionRole::User,
                "assistant" => SessionRole::Assistant,
                _ => SessionRole::System,
            };
            process_message_content(payload.get("content"), role, builder, line);
        }
        "reasoning" => {
            let mut parts = Vec::new();
            collect_texts(payload.get("summary"), &mut parts);
            collect_texts(payload.get("content"), &mut parts);
            builder.push(
                SessionRole::Assistant,
                SessionMessageKind::Thinking,
                parts.join("\n"),
                None,
            );
        }
        "function_call" | "custom_tool_call" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            if let Some(call_id) = payload.get("call_id").and_then(Value::as_str) {
                builder.tool_names.insert(call_id.to_string(), name.clone());
            }
            let input = payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            builder.push(
                SessionRole::Assistant,
                SessionMessageKind::ToolCall,
                value_to_text(&input),
                Some(name),
            );
        }
        "local_shell_call" | "web_search_call" => {
            let name = if item_type == "local_shell_call" {
                "local_shell"
            } else {
                "web_search"
            };
            if let Some(call_id) = payload.get("call_id").and_then(Value::as_str) {
                builder
                    .tool_names
                    .insert(call_id.to_string(), name.to_string());
            }
            let detail = payload
                .get("action")
                .cloned()
                .unwrap_or_else(|| payload.clone());
            builder.push(
                SessionRole::Assistant,
                SessionMessageKind::ToolCall,
                value_to_text(&detail),
                Some(name.to_string()),
            );
        }
        "function_call_output" | "custom_tool_call_output" => {
            let call_id = payload.get("call_id").and_then(Value::as_str);
            let tool_name = call_id.and_then(|id| builder.tool_names.get(id)).cloned();
            let output = payload.get("output").cloned().unwrap_or(Value::Null);
            builder.push(
                SessionRole::Tool,
                SessionMessageKind::ToolResult,
                value_to_text(&output),
                tool_name,
            );
        }
        "compaction" => {}
        other => builder.warning(line, format!("未知 response_item：{other}")),
    }
}

fn process_event_msg(payload: &Value, builder: &mut Builder) {
    let event_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "thread_name_updated" => {
            if let Some(title) = payload
                .get("thread_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                builder.title = Some(title.to_string());
            }
        }
        "task_complete" | "turn_aborted" => {
            builder.push(
                SessionRole::System,
                SessionMessageKind::Event,
                event_type.to_string(),
                None,
            );
        }
        _ => {}
    }
}

fn process_message_content(
    content: Option<&Value>,
    role: SessionRole,
    builder: &mut Builder,
    line: usize,
) {
    match content {
        Some(Value::String(text)) => {
            if !text.trim().is_empty() {
                builder.push(role, SessionMessageKind::Text, text.clone(), None);
            }
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
                match block_type {
                    "input_text" | "output_text" => {
                        if let Some(text) = block
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                        {
                            builder.push(role, SessionMessageKind::Text, text.to_string(), None);
                        }
                    }
                    "input_image" => {
                        let content = image_content(block);
                        builder.push(role, SessionMessageKind::Image, content, None);
                    }
                    other => builder.warning(line, format!("未知内容块类型：{other}")),
                }
            }
        }
        _ => {}
    }
}

fn image_content(block: &Value) -> String {
    let detail = block
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("未知类型");
    let Some(url) = block.get("image_url").and_then(Value::as_str) else {
        return format!("[图片：{detail}，缺少 image_url]");
    };
    if !url.starts_with("data:") {
        return format!("[图片：{detail}，来源不是内联 data URL]");
    }
    if url.len() > MAX_IMAGE_BYTES {
        return format!("[图片：{detail}，{} 字节，超过 512 KiB 未内联]", url.len());
    }
    url.to_string()
}

fn collect_texts(value: Option<&Value>, out: &mut Vec<String>) {
    let Some(Value::Array(items)) = value else {
        return;
    };
    for item in items {
        if let Some(text) = item
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        {
            out.push(text.to_string());
        }
    }
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn choose_title(builder: &Builder, session_id: &str) -> String {
    if let Some(title) = builder.title.as_deref() {
        return truncate_chars(title, TITLE_MAX_CHARS);
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
    if let Some(name) = builder
        .project_path
        .as_deref()
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

fn session_id_from_filename(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let name = name.strip_suffix(".zst").unwrap_or(name);
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let tail: String = stem
        .chars()
        .rev()
        .take(36)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if tail.len() == 36 && tail.matches('-').count() == 4 {
        Some(tail)
    } else {
        Some(stem.to_string())
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
    use crate::domain::session::SessionRole;
    use std::io::Write;
    use std::path::Path;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    #[test]
    fn codex_maps_session_meta_messages_reasoning_and_tools() {
        let parsed = parse_codex(&fixture("codex.jsonl")).unwrap();
        assert_eq!(parsed.session_id, "c1");
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
        assert_eq!(parsed.title, "修复构建");
        let kinds: Vec<_> = parsed.messages.iter().map(|m| m.kind).collect();
        assert_eq!(kinds, vec![Text, Thinking, ToolCall, ToolResult, Event]);
        assert_eq!(parsed.messages[2].tool_name.as_deref(), Some("exec"));
        assert_eq!(parsed.messages[3].tool_name.as_deref(), Some("exec"));
    }

    #[test]
    fn codex_uses_thread_name_and_ignores_repeated_session_meta() {
        let parsed = parse_codex(&fixture("codex-repeated-meta.jsonl")).unwrap();
        assert_eq!(parsed.session_id, "c1", "中途重复的 session_meta 不能改 id");
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
        assert_eq!(parsed.title, "最后的线程名");
    }

    #[test]
    fn event_msg_duplicates_are_not_counted_twice() {
        let parsed = parse_codex(&fixture("codex-event-duplicates.jsonl")).unwrap();
        assert_eq!(
            parsed
                .messages
                .iter()
                .filter(|m| m.role == SessionRole::User)
                .count(),
            1
        );
        assert_eq!(
            parsed
                .messages
                .iter()
                .filter(|m| m.role == SessionRole::Assistant)
                .count(),
            1
        );
    }

    #[test]
    fn a_subagent_session_file_is_skipped_entirely() {
        let parsed = parse_codex(&fixture("codex-subagent.jsonl")).unwrap();
        assert!(parsed.messages.is_empty(), "子代理会话不进入列表");
    }

    #[test]
    fn a_zstd_compressed_session_reads_the_same_as_the_plain_one() {
        let plain = parse_codex(&fixture("codex.jsonl")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let compressed_path = dir.path().join("codex.jsonl.zst");
        let source = std::fs::read(fixture("codex.jsonl")).unwrap();
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        encoder.write_all(&source).unwrap();
        std::fs::write(&compressed_path, encoder.finish().unwrap()).unwrap();

        let compressed = parse_codex(&compressed_path).unwrap();
        assert_eq!(plain.session_id, compressed.session_id);
        assert_eq!(plain.project_path, compressed.project_path);
        assert_eq!(plain.title, compressed.title);
        assert_eq!(plain.messages, compressed.messages);
    }

    #[test]
    fn unknown_codex_records_are_counted_not_fatal() {
        let parsed = parse_codex(&fixture("codex-with-unknown-type.jsonl")).unwrap();
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.contains("未知 response_item")));
        assert!(!parsed.messages.is_empty());
    }

    #[test]
    fn codex_discovery_prefers_the_plain_file_over_the_compressed_twin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("a.jsonl.zst"), "").unwrap();
        std::fs::write(dir.path().join("b.jsonl.zst"), "").unwrap();
        let files = discover_codex(dir.path(), false);
        assert!(files.iter().any(|p| p.ends_with("a.jsonl")));
        assert!(!files.iter().any(|p| p.ends_with("a.jsonl.zst")));
        assert!(files.iter().any(|p| p.ends_with("b.jsonl.zst")));
    }
}
