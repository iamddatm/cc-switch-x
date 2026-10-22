//! Grok Build 会话解析器：`summary.json` 提供元数据，同目录 `chat_history.jsonl` 是正文。

use super::pi;
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

pub fn parse_grokbuild(path: &Path) -> Result<ParsedSession, SessionError> {
    let text = std::fs::read_to_string(path)?;
    let summary: Value = serde_json::from_str(&text)?;
    let info = summary.get("info").ok_or_else(|| {
        SessionError::Unsupported("Grok Build summary.json 缺少 info".to_string())
    })?;
    let session_id = info
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            SessionError::Unsupported("Grok Build summary.json 缺少 info.id".to_string())
        })?
        .to_string();
    let project_path = info
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);

    let session_dir = path.parent().ok_or_else(|| {
        SessionError::Unsupported(format!(
            "Grok Build summary.json 没有所在目录：{}",
            path.display()
        ))
    })?;
    let chat_path = session_dir.join("chat_history.jsonl");
    if !chat_path.is_file() {
        return Err(SessionError::Unsupported(format!(
            "Grok Build 会话缺少 chat_history.jsonl：{}",
            chat_path.display()
        )));
    }
    let (messages, chat_created, chat_updated, warnings) = read_chat_history(&chat_path)?;

    let explicit_title = summary
        .get("generated_title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            summary
                .get("session_summary")
                .and_then(Value::as_str)
                .filter(|title| !title.trim().is_empty())
        });
    let title = pi::choose_title(
        explicit_title,
        &messages,
        project_path.as_deref(),
        &session_id,
    );
    let created_at = summary
        .get("created_at")
        .and_then(parse_timestamp)
        .or(chat_created);
    let summary_updated = summary
        .get("last_active_at")
        .or_else(|| summary.get("updated_at"))
        .and_then(parse_timestamp);
    let updated_at = match (summary_updated, chat_updated) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
    .unwrap_or_else(|| pi::file_mtime_millis(path));

    Ok(ParsedSession {
        session_id,
        title,
        project_path,
        created_at,
        updated_at,
        messages,
        warnings,
    })
}

pub fn parse_grokbuild_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    Ok(vec![parse_grokbuild(path)?])
}

pub fn discover_grokbuild(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    walk_files(root, &|path| {
        path.file_name().is_some_and(|name| name == "summary.json")
    })
}

pub fn resume_grokbuild(summary: &SessionSummary) -> Option<String> {
    Some(format!("grok --resume {}", summary.session_id))
}

type ChatHistory = (
    Vec<ParsedMessage>,
    Option<i64>,
    Option<i64>,
    Vec<SessionWarning>,
);

fn read_chat_history(path: &Path) -> Result<ChatHistory, SessionError> {
    let reader = BufReader::new(File::open(path)?);
    let mut messages = Vec::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut warnings = Vec::new();
    let mut created_at: Option<i64> = None;
    let mut updated_at: Option<i64> = None;

    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(SessionWarning {
                    line: line_number + 1,
                    message: format!("JSON 解析失败：{error}"),
                });
                continue;
            }
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "system" | "user" | "assistant" | "tool") {
            continue;
        }
        if let Some(timestamp) = value
            .get("timestamp")
            .or_else(|| value.get("ts"))
            .and_then(parse_timestamp)
        {
            created_at = Some(created_at.map_or(timestamp, |current| current.min(timestamp)));
            updated_at = Some(updated_at.map_or(timestamp, |current| current.max(timestamp)));
        }
        let text = extract_text(value.get("content"));
        match kind {
            "assistant" => {
                if !text.trim().is_empty() {
                    push_text(&mut messages, SessionRole::Assistant, text);
                }
                if let Some(calls) = value.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let function = call.get("function");
                        let name = function
                            .and_then(|function| function.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        if let Some(id) = call.get("id").and_then(Value::as_str) {
                            tool_names.insert(id.to_string(), name.clone());
                        }
                        let arguments = function
                            .and_then(|function| function.get("arguments"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        let content = match arguments {
                            Value::String(text) => text,
                            Value::Null => "{}".to_string(),
                            other => serde_json::to_string(&other).unwrap_or_default(),
                        };
                        messages.push(ParsedMessage {
                            role: SessionRole::Assistant,
                            kind: SessionMessageKind::ToolCall,
                            content,
                            tool_name: Some(name),
                        });
                    }
                }
            }
            "tool" => {
                let tool_name = value
                    .get("tool_call_id")
                    .or_else(|| value.get("call_id"))
                    .and_then(Value::as_str)
                    .and_then(|id| tool_names.get(id))
                    .cloned();
                if !text.trim().is_empty() {
                    messages.push(ParsedMessage {
                        role: SessionRole::Tool,
                        kind: SessionMessageKind::ToolResult,
                        content: text,
                        tool_name,
                    });
                }
            }
            "user" => push_text(&mut messages, SessionRole::User, text),
            "system" => push_text(&mut messages, SessionRole::System, text),
            _ => {}
        }
    }

    Ok((messages, created_at, updated_at, warnings))
}

fn push_text(messages: &mut Vec<ParsedMessage>, role: SessionRole, text: String) {
    if text.trim().is_empty() {
        return;
    }
    messages.push(ParsedMessage {
        role,
        kind: SessionMessageKind::Text,
        content: text,
        tool_name: None,
    });
}

fn extract_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
        None => String::new(),
    }
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(if number > 10_000_000_000 {
            number
        } else {
            number * 1000
        });
    }
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::{SessionMessageKind, SessionRole};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    #[test]
    fn grok_reads_summary_and_chat_history() {
        let parsed = parse_grokbuild(&fixture("grokbuild/sessions/p1/s1/summary.json")).unwrap();
        assert_eq!(parsed.session_id, "g1");
        assert_eq!(parsed.title, "Grok 标题");
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
        assert_eq!(parsed.messages.len(), 5);
        assert_eq!(parsed.messages[0].role, SessionRole::System);
        assert_eq!(parsed.messages[1].content, "帮我清理");
        assert_eq!(parsed.messages[2].content, "好的");
        assert_eq!(parsed.messages[3].kind, SessionMessageKind::ToolCall);
        assert_eq!(
            parsed.messages[3].tool_name.as_deref(),
            Some("run_terminal_cmd")
        );
        assert!(parsed.messages[3].content.contains("ls"));
        assert_eq!(parsed.messages[4].kind, SessionMessageKind::ToolResult);
        assert_eq!(parsed.messages[4].content, "a.rs");
        assert_eq!(
            parsed.messages[4].tool_name.as_deref(),
            Some("run_terminal_cmd")
        );
    }

    #[test]
    fn grok_title_falls_back_to_first_user_message() {
        let parsed = parse_grokbuild(&fixture("grokbuild/sessions/p2/s2/summary.json")).unwrap();
        assert_eq!(parsed.title, "修一下");
    }

    #[test]
    fn grok_missing_chat_history_is_an_explainable_error() {
        let error = parse_grokbuild(&fixture("grokbuild/sessions/p3/s3/summary.json"))
            .expect_err("缺少 chat_history.jsonl 必须报错");
        assert!(
            matches!(&error, SessionError::Unsupported(message) if message.contains("chat_history.jsonl")),
            "{error:?}"
        );
    }

    #[test]
    fn grok_discovery_finds_summary_files() {
        let files = discover_grokbuild(&fixture("grokbuild/sessions"), true);
        assert_eq!(files.len(), 3, "{files:?}");
        assert!(files.iter().all(|path| path.ends_with("summary.json")));
        assert!(files
            .iter()
            .any(|path| path.ends_with("p1/s1/summary.json")));
    }
}
