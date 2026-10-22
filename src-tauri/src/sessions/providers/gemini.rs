//! Gemini CLI 会话解析器。本机没有样本，格式来自 cc-switch 对 Gemini CLI
//! `loadConversationRecord` 的代码级还原：`.jsonl` 是增量日志，必须按顺序回放。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const TITLE_MAX_CHARS: usize = 80;

pub fn parse_gemini(path: &Path) -> Result<ParsedSession, SessionError> {
    let document = load_document(path)?;
    map_document(&document, path)
}

pub fn parse_gemini_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    Ok(vec![parse_gemini(path)?])
}

pub fn discover_gemini(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return out;
    };
    for project in projects.flatten() {
        if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let chats = project.path().join("chats");
        let Ok(entries) = std::fs::read_dir(&chats) else {
            continue;
        };
        let mut jsonl_stems = HashSet::new();
        let mut json_files = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("session-") {
                continue;
            }
            if name.ends_with(".jsonl") {
                jsonl_stems.insert(name.trim_end_matches(".jsonl").to_string());
                out.push(entry.path());
            } else if name.ends_with(".json") {
                json_files.push((name.trim_end_matches(".json").to_string(), entry.path()));
            }
        }
        for (stem, path) in json_files {
            if !jsonl_stems.contains(&stem) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

pub fn resume_gemini(summary: &SessionSummary) -> Option<String> {
    Some(format!("gemini --resume {}", summary.session_id))
}

fn load_document(path: &Path) -> Result<Value, SessionError> {
    let text = std::fs::read_to_string(path)?;
    let is_jsonl = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
    if is_jsonl {
        return replay_jsonl(&text);
    }
    let document: Value = serde_json::from_str(&text)?;
    if !document.is_object() {
        return Err(SessionError::Unsupported(
            "Gemini 会话不是 JSON 对象".to_string(),
        ));
    }
    Ok(document)
}

/// 按 Gemini CLI 的 `loadConversationRecord` 回放增量日志。
fn replay_jsonl(text: &str) -> Result<Value, SessionError> {
    let mut metadata = Map::new();
    let mut log = MessageLog::default();
    let mut seen_record = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(Value::Object(mut record)) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        seen_record = true;
        if let Some(id) = record.get("$rewindTo").and_then(Value::as_str) {
            log.rewind_to(id);
        } else if record.get("id").is_some_and(Value::is_string) {
            log.upsert(Value::Object(record));
        } else if let Some(Value::Object(mut set)) = record.remove("$set") {
            if let Some(Value::Array(messages)) = set.remove("messages") {
                log.reset(messages);
            }
            metadata.extend(set);
        } else if record.get("sessionId").is_some_and(Value::is_string) {
            if let Some(Value::Array(messages)) = record.remove("messages") {
                for message in messages {
                    log.upsert(message);
                }
            }
            metadata.extend(record);
        }
    }

    if !seen_record {
        return Err(SessionError::Unsupported("Gemini 会话文件为空".to_string()));
    }
    metadata.insert("messages".to_string(), Value::Array(log.messages));
    Ok(Value::Object(metadata))
}

#[derive(Default)]
struct MessageLog {
    messages: Vec<Value>,
    index: HashMap<String, usize>,
}

impl MessageLog {
    fn upsert(&mut self, message: Value) {
        let Some(id) = message
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        match self.index.get(&id) {
            Some(&position) => self.messages[position] = message,
            None => {
                self.index.insert(id, self.messages.len());
                self.messages.push(message);
            }
        }
    }

    fn rewind_to(&mut self, id: &str) {
        let keep = self.index.get(id).copied().unwrap_or(0);
        self.messages.truncate(keep);
        self.index.retain(|_, position| *position < keep);
    }

    fn reset(&mut self, messages: Vec<Value>) {
        self.messages.clear();
        self.index.clear();
        for message in messages {
            self.upsert(message);
        }
    }
}

fn map_document(document: &Value, path: &Path) -> Result<ParsedSession, SessionError> {
    let session_id = document
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .ok_or(SessionError::MissingId)?;

    let mut messages = Vec::new();
    if let Some(Value::Array(items)) = document.get("messages") {
        for item in items {
            map_message(item, &mut messages);
        }
    }

    let project_path = project_root(path);
    let title = choose_title(&messages, project_path.as_deref(), &session_id);
    let created_at = document.get("startTime").and_then(parse_timestamp);
    let updated_at = document
        .get("lastUpdated")
        .and_then(parse_timestamp)
        .unwrap_or_else(|| file_mtime_millis(path));

    Ok(ParsedSession {
        session_id,
        title,
        project_path,
        created_at,
        updated_at,
        messages,
        warnings: Vec::new(),
    })
}

fn map_message(message: &Value, out: &mut Vec<ParsedMessage>) {
    let message_type = message.get("type").and_then(Value::as_str).unwrap_or("");
    match message_type {
        "user" => {
            let text = content_text(message.get("content"));
            if !text.trim().is_empty() {
                out.push(ParsedMessage {
                    role: SessionRole::User,
                    kind: SessionMessageKind::Text,
                    content: text,
                    tool_name: None,
                });
            }
        }
        "gemini" => {
            if let Some(Value::Array(thoughts)) = message.get("thoughts") {
                let mut parts = Vec::new();
                for thought in thoughts {
                    let subject = thought.get("subject").and_then(Value::as_str).unwrap_or("");
                    let description = thought
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let text = [subject, description]
                        .into_iter()
                        .filter(|part| !part.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
                if !parts.is_empty() {
                    out.push(ParsedMessage {
                        role: SessionRole::Assistant,
                        kind: SessionMessageKind::Thinking,
                        content: parts.join("\n"),
                        tool_name: None,
                    });
                }
            }
            let text = content_text(message.get("content"));
            if !text.trim().is_empty() {
                out.push(ParsedMessage {
                    role: SessionRole::Assistant,
                    kind: SessionMessageKind::Text,
                    content: text,
                    tool_name: None,
                });
            }
            if let Some(Value::Array(calls)) = message.get("toolCalls") {
                for call in calls {
                    let name = call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let args = call.get("args").cloned().unwrap_or(Value::Null);
                    out.push(ParsedMessage {
                        role: SessionRole::Assistant,
                        kind: SessionMessageKind::ToolCall,
                        content: serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string()),
                        tool_name: Some(name.clone()),
                    });
                    let mut result = tool_result_text(call);
                    if !result.is_empty() {
                        let prefix = match call.get("status").and_then(Value::as_str) {
                            Some("error") => "[error] ",
                            Some("cancelled") | Some("canceled") => "[interrupted] ",
                            _ => "",
                        };
                        result = format!("{prefix}{result}");
                        out.push(ParsedMessage {
                            role: SessionRole::Tool,
                            kind: SessionMessageKind::ToolResult,
                            content: result,
                            tool_name: Some(name),
                        });
                    }
                }
            }
        }
        "info" | "error" => {
            let text = value_to_text(message.get("content"));
            if !text.trim().is_empty() {
                out.push(ParsedMessage {
                    role: SessionRole::System,
                    kind: SessionMessageKind::Event,
                    content: text,
                    tool_name: None,
                });
            }
        }
        _ => {}
    }
}

fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(object)) => object
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn tool_result_text(call: &Value) -> String {
    if let Some(display) = call.get("resultDisplay") {
        let text = value_to_text(Some(display));
        if !text.is_empty() {
            return text;
        }
    }
    let Some(Value::Array(items)) = call.get("result") else {
        return String::new();
    };
    let mut parts = Vec::new();
    for item in items {
        let response = item
            .get("functionResponse")
            .and_then(|response| response.get("response"));
        let Some(response) = response else {
            continue;
        };
        for key in ["output", "error"] {
            if let Some(text) = response.get(key).and_then(Value::as_str) {
                parts.push(text.to_string());
            }
        }
    }
    parts.join("\n")
}

fn value_to_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn project_root(path: &Path) -> Option<PathBuf> {
    let chats = path.parent()?;
    if !chats
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("chats")
    {
        return None;
    }
    let project = chats.parent()?;
    std::fs::read_to_string(project.join(".project_root"))
        .ok()
        .map(|text| PathBuf::from(text.trim()))
        .filter(|path| !path.as_os_str().is_empty())
}

fn choose_title(messages: &[ParsedMessage], project: Option<&Path>, session_id: &str) -> String {
    if let Some(first) = messages.iter().find(|message| {
        message.role == SessionRole::User
            && message.kind == SessionMessageKind::Text
            && !is_ignored_user_text(&message.content)
    }) {
        return truncate_chars(first.content.trim(), TITLE_MAX_CHARS);
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

fn is_ignored_user_text(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.starts_with('?')
        || trimmed.starts_with("<session_context>")
        || trimmed.starts_with("<hook_context>")
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
    use std::path::Path;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    #[test]
    fn gemini_jsonl_replay_applies_set_upsert_and_rewind() {
        let parsed = parse_gemini(&fixture("gemini.jsonl")).unwrap();
        let texts: Vec<_> = parsed
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(
            texts,
            vec!["第一问", "第一答", "第二问"],
            "rewind 之后的消息不能回来"
        );
    }

    #[test]
    fn gemini_old_json_and_jsonl_documents_agree() {
        let old = parse_gemini(&fixture("gemini.json")).unwrap();
        let new = parse_gemini(&fixture("gemini.jsonl")).unwrap();
        assert_eq!(old.messages, new.messages);
        assert_eq!(new.session_id, "g1");
    }

    #[test]
    fn gemini_reads_the_project_root_file() {
        let parsed = parse_gemini(&fixture("gemini/demo/chats/session-1.jsonl")).unwrap();
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
    }

    #[test]
    fn set_messages_replaces_the_whole_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-1.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"g1\"}\n",
                "{\"id\":\"m1\",\"type\":\"user\",\"content\":\"旧\"}\n",
                "{\"$set\":{\"messages\":[{\"id\":\"m2\",\"type\":\"user\",\"content\":\"新\"}]}}\n"
            ),
        )
        .unwrap();
        let parsed = parse_gemini(&path).unwrap();
        let texts: Vec<_> = parsed
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(texts, vec!["新"]);
    }

    #[test]
    fn gemini_discovery_hides_json_when_jsonl_exists() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("demo/chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(chats.join("session-1.json"), "{}").unwrap();
        std::fs::write(chats.join("session-1.jsonl"), "").unwrap();
        std::fs::write(chats.join("session-2.json"), "{}").unwrap();
        let files = discover_gemini(dir.path(), false);
        assert!(files.iter().any(|p| p.ends_with("session-1.jsonl")));
        assert!(!files.iter().any(|p| p.ends_with("session-1.json")));
        assert!(files.iter().any(|p| p.ends_with("session-2.json")));
    }

    #[test]
    fn gemini_maps_thoughts_and_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-1.json");
        let doc = serde_json::json!({
            "sessionId": "g1",
            "messages": [{
                "type": "gemini",
                "thoughts": [{"subject": "标题", "description": "想一想"}],
                "content": "回答",
                "toolCalls": [{
                    "name": "read_file",
                    "args": {"path": "a.rs"},
                    "resultDisplay": "文件内容",
                    "status": "success"
                }]
            }]
        });
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let parsed = parse_gemini(&path).unwrap();
        assert!(parsed.messages.iter().any(|m| m.kind
            == crate::domain::session::SessionMessageKind::Thinking
            && m.content.contains("想一想")));
        assert!(parsed.messages.iter().any(|m| m.kind
            == crate::domain::session::SessionMessageKind::ToolCall
            && m.tool_name.as_deref() == Some("read_file")));
        assert!(parsed.messages.iter().any(|m| m.kind
            == crate::domain::session::SessionMessageKind::ToolResult
            && m.content == "文件内容"));
    }
}
