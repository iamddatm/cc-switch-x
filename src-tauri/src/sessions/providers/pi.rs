//! Pi Coding Agent 会话解析器。本机 51 个真实样本全部是 `version=3`：
//! 首行 `type=session` 是头，后续是带 `id/parentId` 的树形 entry，只输出活动分支。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
    SessionWarning,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 500_000;
const MAX_IMAGE_BYTES: usize = 512 * 1024;
const TITLE_MAX_CHARS: usize = 80;

pub fn parse_pi(path: &Path) -> Result<ParsedSession, SessionError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(SessionError::Unsupported(format!(
            "Pi 会话超过 128 MiB：{} 字节",
            metadata.len()
        )));
    }

    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    let mut line_no = 0usize;
    let mut header: Option<Value> = None;
    let mut warnings = Vec::new();
    let mut entries: Vec<Value> = Vec::new();
    let mut parents: HashMap<String, (Option<String>, usize)> = HashMap::new();
    let mut latest_id: Option<String> = None;
    let mut legacy_previous: Option<String> = None;
    let mut session_info_name: Option<String> = None;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_no += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(SessionWarning {
                    line: line_no,
                    message: format!("JSON 解析失败：{error}"),
                });
                continue;
            }
        };
        if header.is_none() {
            if value.get("type").and_then(Value::as_str) != Some("session") {
                return Err(SessionError::Unsupported(
                    "Pi 会话缺少 session 头".to_string(),
                ));
            }
            header = Some(value);
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(SessionError::Unsupported(format!(
                "Pi 会话超过 {MAX_ENTRIES} 条 entry"
            )));
        }
        if value.get("type").and_then(Value::as_str) == Some("session_info") {
            if let Some(name) = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                session_info_name = Some(name.to_string());
            }
        }
        let version = header
            .as_ref()
            .and_then(|header| header.get("version"))
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| (version < 2).then(|| format!("legacy-{}", entries.len() + 1)));
        let Some(id) = id else {
            continue;
        };
        let parent = value
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| (version < 2).then(|| legacy_previous.clone()).flatten());
        entries.push(value);
        parents.insert(id.clone(), (parent, entries.len() - 1));
        latest_id = Some(id.clone());
        legacy_previous = Some(id);
    }

    let header =
        header.ok_or_else(|| SessionError::Unsupported("Pi 会话缺少 session 头".to_string()))?;
    let session_id = header
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .ok_or(SessionError::MissingId)?;
    let project_path = header.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    let created_at = header.get("timestamp").and_then(parse_timestamp);
    let mut updated_at = created_at;

    let active = active_branch(&parents, latest_id.as_deref());
    let mut messages = Vec::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if !active.contains(&index) {
            continue;
        }
        if let Some(timestamp) = entry_timestamp(entry) {
            updated_at = Some(updated_at.map_or(timestamp, |current| current.max(timestamp)));
        }
        map_entry(entry, &mut messages, &mut tool_names);
    }

    let title = choose_title(
        session_info_name.as_deref(),
        &messages,
        project_path.as_deref(),
        &session_id,
    );
    Ok(ParsedSession {
        session_id,
        title,
        project_path,
        created_at,
        updated_at: updated_at.unwrap_or_else(|| file_mtime_millis(path)),
        messages,
        warnings,
    })
}

pub fn parse_pi_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    Ok(vec![parse_pi(path)?])
}

pub fn discover_pi(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_jsonl(root, &mut out);
    if let Ok(children) = std::fs::read_dir(root) {
        for child in children.flatten() {
            if child.file_type().is_ok_and(|kind| kind.is_dir()) {
                collect_jsonl(&child.path(), &mut out);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        {
            out.push(path);
        }
    }
}

pub fn resume_pi(summary: &SessionSummary) -> Option<String> {
    let path = &summary.source_path;
    if path.contains('"')
        || path.contains('\n')
        || path.contains('\r')
        || path.contains('$')
        || path.contains('`')
        || path.contains('%')
        || path.contains('!')
    {
        return None;
    }
    Some(format!("pi --session \"{path}\""))
}

fn active_branch(
    parents: &HashMap<String, (Option<String>, usize)>,
    latest: Option<&str>,
) -> HashSet<usize> {
    let mut active = HashSet::new();
    let mut visited = HashSet::new();
    let mut current = latest.map(str::to_string);
    while let Some(id) = current {
        if !visited.insert(id.clone()) {
            break;
        }
        let Some((parent, index)) = parents.get(&id) else {
            break;
        };
        active.insert(*index);
        current = parent.clone();
    }
    active
}

pub(crate) fn map_entry(
    entry: &Value,
    messages: &mut Vec<ParsedMessage>,
    tool_names: &mut HashMap<String, String>,
) {
    if entry.get("type").and_then(Value::as_str) != Some("message") {
        return;
    }
    let Some(message) = entry.get("message") else {
        return;
    };
    let role = message.get("role").and_then(Value::as_str).unwrap_or("");
    match role {
        "user" => map_user_content(message.get("content"), messages),
        "assistant" => map_assistant_content(message.get("content"), messages, tool_names),
        "toolResult" => map_tool_result(message, messages, tool_names),
        "bashExecution" => map_bash_execution(message, messages),
        "branchSummary" | "compactionSummary" => {
            let content = message
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| serde_json::to_string(message).unwrap_or_default());
            push(
                messages,
                SessionRole::System,
                SessionMessageKind::Event,
                content,
                None,
            );
        }
        _ => {}
    }
}

fn map_user_content(content: Option<&Value>, messages: &mut Vec<ParsedMessage>) {
    match content {
        Some(Value::String(text)) => {
            if !text.trim().is_empty() {
                push(
                    messages,
                    SessionRole::User,
                    SessionMessageKind::Text,
                    text.clone(),
                    None,
                );
            }
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        if let Some(text) = block
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                        {
                            push(
                                messages,
                                SessionRole::User,
                                SessionMessageKind::Text,
                                text.to_string(),
                                None,
                            );
                        }
                    }
                    "image" => push(
                        messages,
                        SessionRole::User,
                        SessionMessageKind::Image,
                        image_content(block),
                        None,
                    ),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn map_assistant_content(
    content: Option<&Value>,
    messages: &mut Vec<ParsedMessage>,
    tool_names: &mut HashMap<String, String>,
) {
    match content {
        Some(Value::String(text)) => {
            if !text.trim().is_empty() {
                push(
                    messages,
                    SessionRole::Assistant,
                    SessionMessageKind::Text,
                    text.clone(),
                    None,
                );
            }
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "thinking" => {
                        if let Some(text) = block
                            .get("thinking")
                            .and_then(Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                        {
                            push(
                                messages,
                                SessionRole::Assistant,
                                SessionMessageKind::Thinking,
                                text.to_string(),
                                None,
                            );
                        }
                    }
                    "text" => {
                        if let Some(text) = block
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                        {
                            push(
                                messages,
                                SessionRole::Assistant,
                                SessionMessageKind::Text,
                                text.to_string(),
                                None,
                            );
                        }
                    }
                    "toolCall" => {
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        if let Some(id) = block.get("id").and_then(Value::as_str) {
                            tool_names.insert(id.to_string(), name.clone());
                        }
                        let arguments = block.get("arguments").cloned().unwrap_or(Value::Null);
                        push(
                            messages,
                            SessionRole::Assistant,
                            SessionMessageKind::ToolCall,
                            serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".to_string()),
                            Some(name),
                        );
                    }
                    "image" => push(
                        messages,
                        SessionRole::Assistant,
                        SessionMessageKind::Image,
                        image_content(block),
                        None,
                    ),
                    other => push(
                        messages,
                        SessionRole::Assistant,
                        SessionMessageKind::Text,
                        serde_json::to_string(block).unwrap_or_default(),
                        Some(other.to_string()),
                    ),
                }
            }
        }
        _ => {}
    }
}

fn map_tool_result(
    message: &Value,
    messages: &mut Vec<ParsedMessage>,
    tool_names: &HashMap<String, String>,
) {
    let tool_name = message
        .get("toolCallId")
        .and_then(Value::as_str)
        .and_then(|id| tool_names.get(id))
        .cloned();
    let is_error = message.get("isError").and_then(Value::as_bool) == Some(true);
    let mut first = true;
    match message.get("content") {
        Some(Value::String(text)) => {
            let content = if is_error {
                format!("[error] {text}")
            } else {
                text.clone()
            };
            push(
                messages,
                SessionRole::Tool,
                SessionMessageKind::ToolResult,
                content,
                tool_name.clone(),
            );
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        if let Some(text) = block
                            .get("text")
                            .and_then(Value::as_str)
                            .filter(|text| !text.trim().is_empty())
                        {
                            let content = if is_error && first {
                                format!("[error] {text}")
                            } else {
                                text.to_string()
                            };
                            first = false;
                            push(
                                messages,
                                SessionRole::Tool,
                                SessionMessageKind::ToolResult,
                                content,
                                tool_name.clone(),
                            );
                        }
                    }
                    "image" => push(
                        messages,
                        SessionRole::Tool,
                        SessionMessageKind::Image,
                        image_content(block),
                        None,
                    ),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn map_bash_execution(message: &Value, messages: &mut Vec<ParsedMessage>) {
    let command = message
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !command.is_empty() {
        push(
            messages,
            SessionRole::Assistant,
            SessionMessageKind::ToolCall,
            command,
            Some("bash".to_string()),
        );
    }
    let mut output = message
        .get("output")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if message.get("cancelled").and_then(Value::as_bool) == Some(true) {
        output = format!("[cancelled] {output}");
    } else if let Some(code) = message.get("exitCode").and_then(Value::as_i64) {
        if code != 0 {
            output = format!("[exit {code}] {output}");
        }
    }
    if !output.trim().is_empty() {
        push(
            messages,
            SessionRole::Tool,
            SessionMessageKind::ToolResult,
            output,
            Some("bash".to_string()),
        );
    }
}

fn image_content(block: &Value) -> String {
    let media_type = block
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("未知类型");
    let Some(data) = block.get("data").and_then(Value::as_str) else {
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

fn push(
    messages: &mut Vec<ParsedMessage>,
    role: SessionRole,
    kind: SessionMessageKind,
    content: String,
    tool_name: Option<String>,
) {
    if content.is_empty() {
        return;
    }
    messages.push(ParsedMessage {
        role,
        kind,
        content,
        tool_name,
    });
}

pub(crate) fn entry_timestamp(entry: &Value) -> Option<i64> {
    entry
        .get("timestamp")
        .and_then(parse_timestamp)
        .or_else(|| {
            entry
                .get("message")
                .and_then(|message| message.get("timestamp"))
                .and_then(parse_timestamp)
        })
}

pub(crate) fn choose_title(
    explicit: Option<&str>,
    messages: &[ParsedMessage],
    project: Option<&Path>,
    session_id: &str,
) -> String {
    if let Some(title) = explicit.filter(|title| !title.trim().is_empty()) {
        return truncate_chars(title.trim(), TITLE_MAX_CHARS);
    }
    if let Some(first) = messages.iter().find(|message| {
        message.role == SessionRole::User && message.kind == SessionMessageKind::Text
    }) {
        let title = first.content.trim();
        if !title.is_empty() {
            return truncate_chars(title, TITLE_MAX_CHARS);
        }
    }
    if let Some(name) = project
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
    if let Some(text) = value.as_str() {
        return chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|time| time.timestamp_millis());
    }
    let number = value.as_i64()?;
    Some(if number > 10_000_000_000 {
        number
    } else {
        number * 1000
    })
}

pub(crate) fn file_mtime_millis(path: &Path) -> i64 {
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

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    #[test]
    fn pi_follows_only_the_active_branch() {
        let parsed = parse_pi(&fixture("pi.jsonl")).unwrap();
        assert!(!parsed
            .messages
            .iter()
            .any(|m| m.content.contains("被放弃的分支")));
        assert!(parsed
            .messages
            .iter()
            .any(|m| m.content.contains("活动分支")));
    }

    #[test]
    fn pi_maps_tool_calls_results_and_bash_execution() {
        let parsed = parse_pi(&fixture("pi.jsonl")).unwrap();
        assert!(parsed
            .messages
            .iter()
            .any(|m| m.kind == SessionMessageKind::ToolCall
                && m.tool_name.as_deref() == Some("read")));
        assert!(parsed
            .messages
            .iter()
            .any(|m| m.kind == SessionMessageKind::ToolResult
                && m.tool_name.as_deref() == Some("read")));
        assert!(parsed
            .messages
            .iter()
            .any(|m| m.kind == SessionMessageKind::ToolCall
                && m.tool_name.as_deref() == Some("bash")));
    }

    #[test]
    fn pi_without_a_session_header_is_a_named_error() {
        assert!(matches!(
            parse_pi(&fixture("pi-no-header.jsonl")),
            Err(SessionError::Unsupported(_))
        ));
    }

    #[test]
    fn pi_resume_uses_the_source_path_and_refuses_unsafe_paths() {
        let mut summary = SessionSummary {
            agent_id: "pi".into(),
            session_id: "p1".into(),
            title: "t".into(),
            project_path: None,
            source_path: "D:/sessions/p1.jsonl".into(),
            created_at: None,
            updated_at: 0,
            message_count: 0,
            parse_error: None,
        };
        assert_eq!(
            resume_pi(&summary).as_deref(),
            Some("pi --session \"D:/sessions/p1.jsonl\"")
        );
        summary.source_path = "D:/bad\"path.jsonl".into();
        assert_eq!(resume_pi(&summary), None);
    }
}
