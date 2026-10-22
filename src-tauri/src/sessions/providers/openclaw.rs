//! OpenClaw 会话解析器：Pi 同构 JSONL，按文件顺序全取，`sessions.json` 提供显示名。

use super::pi;
use crate::domain::session::{ParsedMessage, ParsedSession, SessionError, SessionWarning};
use crate::sessions::walk_files;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn parse_openclaw(path: &Path) -> Result<ParsedSession, SessionError> {
    let sessions_dir = path.parent().ok_or_else(|| {
        SessionError::Unsupported(format!("OpenClaw 会话文件没有所在目录：{}", path.display()))
    })?;
    let display_names = load_display_names(sessions_dir)?;

    let reader = BufReader::new(File::open(path)?);
    let mut header: Option<Value> = None;
    let mut messages: Vec<ParsedMessage> = Vec::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    let mut warnings = Vec::new();
    let mut updated_at: Option<i64> = None;

    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                warnings.push(SessionWarning {
                    line: line_number + 1,
                    message: format!("JSON 解析失败：{error}"),
                });
                continue;
            }
        };
        if header.is_none() {
            if value.get("type").and_then(Value::as_str) != Some("session") {
                return Err(SessionError::Unsupported(
                    "OpenClaw 会话缺少 session 头".to_string(),
                ));
            }
            updated_at = pi::entry_timestamp(&value);
            header = Some(value);
            continue;
        }
        if let Some(timestamp) = pi::entry_timestamp(&value) {
            updated_at = Some(updated_at.map_or(timestamp, |current| current.max(timestamp)));
        }
        pi::map_entry(&value, &mut messages, &mut tool_names);
    }

    let header = header
        .ok_or_else(|| SessionError::Unsupported("OpenClaw 会话缺少 session 头".to_string()))?;
    let created_at = pi::entry_timestamp(&header);
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
    let project_path = header
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    let display_name = display_names.get(&session_id).map(String::as_str);
    let title = pi::choose_title(
        display_name,
        &messages,
        project_path.as_deref(),
        &session_id,
    );

    Ok(ParsedSession {
        session_id,
        title,
        project_path,
        created_at,
        updated_at: updated_at.unwrap_or_else(|| pi::file_mtime_millis(path)),
        messages,
        warnings,
    })
}

pub fn parse_openclaw_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    Ok(vec![parse_openclaw(path)?])
}

pub fn discover_openclaw(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    walk_files(root, &|path| {
        let is_jsonl = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));
        if !is_jsonl {
            return false;
        }
        let mut in_agents = false;
        let mut in_sessions = false;
        for part in path.components() {
            let name = part.as_os_str();
            if name.eq_ignore_ascii_case("agents") {
                in_agents = true;
            } else if name.eq_ignore_ascii_case("sessions") {
                in_sessions = true;
            }
        }
        in_agents && in_sessions
    })
}

fn load_display_names(sessions_dir: &Path) -> Result<HashMap<String, String>, SessionError> {
    let index_path = sessions_dir.join("sessions.json");
    let text = std::fs::read_to_string(&index_path).map_err(|error| {
        SessionError::Unsupported(format!(
            "OpenClaw 缺少可读的 sessions.json（{}）：{error}",
            index_path.display()
        ))
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        SessionError::Unsupported(format!("OpenClaw sessions.json 解析失败：{error}"))
    })?;
    let mut names = HashMap::new();
    if let Some(entries) = value.as_object() {
        for entry in entries.values() {
            if let (Some(id), Some(name)) = (
                entry.get("sessionId").and_then(Value::as_str),
                entry.get("displayName").and_then(Value::as_str),
            ) {
                if !name.trim().is_empty() {
                    names.insert(id.to_string(), name.to_string());
                }
            }
        }
    }
    Ok(names)
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

    #[test]
    fn openclaw_reuses_pi_message_mapping() {
        let parsed =
            parse_openclaw(&fixture("openclaw/agents/main/sessions/session-abc.jsonl")).unwrap();
        assert_eq!(parsed.session_id, "oc1");
        assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/oc")));
        assert!(parsed
            .messages
            .iter()
            .any(|message| message.content == "另一条分支也要显示"));
        assert!(parsed.messages.iter().any(|message| {
            message.kind == SessionMessageKind::ToolCall
                && message.tool_name.as_deref() == Some("read")
        }));
        assert!(parsed.messages.iter().any(|message| {
            message.kind == SessionMessageKind::ToolResult
                && message.tool_name.as_deref() == Some("read")
                && message.content == "文件内容"
        }));
        assert!(parsed.messages.iter().any(|message| {
            message.kind == SessionMessageKind::ToolCall
                && message.tool_name.as_deref() == Some("bash")
        }));
    }

    #[test]
    fn openclaw_display_name_wins_over_first_message() {
        let parsed =
            parse_openclaw(&fixture("openclaw/agents/main/sessions/session-abc.jsonl")).unwrap();
        assert_eq!(parsed.title, "OpenClaw 显示名");

        let fallback =
            parse_openclaw(&fixture("openclaw/agents/other/sessions/session-def.jsonl")).unwrap();
        assert_eq!(fallback.title, "第二个会话");
    }

    #[test]
    fn openclaw_missing_sessions_index_is_an_explainable_error() {
        let error = parse_openclaw(&fixture("openclaw/agents/lone/sessions/session-lone.jsonl"))
            .expect_err("缺少 sessions.json 必须报错");
        assert!(
            matches!(&error, SessionError::Unsupported(message) if message.contains("sessions.json")),
            "{error:?}"
        );
    }

    #[test]
    fn openclaw_discovery_finds_agent_session_files() {
        let files = discover_openclaw(&fixture("openclaw"), true);
        assert_eq!(files.len(), 3, "{files:?}");
        assert!(files
            .iter()
            .all(|path| { path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") }));
        assert!(files.iter().any(|path| path.ends_with("session-abc.jsonl")));
    }
}
