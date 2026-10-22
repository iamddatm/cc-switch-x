//! Agy（Antigravity）会话解析器：transcript JSONL + `conversation_summaries.db`。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
    SessionWarning,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const TITLE_MAX_CHARS: usize = 80;

pub fn parse_agy_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let is_database = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("db"));
    if is_database {
        return parse_summary_db(path);
    }
    Ok(vec![parse_transcript(path)?])
}

pub fn discover_agy(root: &Path, _override_active: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let canonical_root = root.canonicalize().ok();
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut dirs: Vec<_> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .collect();
        dirs.sort_by_key(|entry| entry.file_name());
        for entry in dirs {
            if !entry.file_name().to_str().is_some_and(valid_session_id) {
                continue;
            }
            let transcript = entry
                .path()
                .join(".system_generated/logs/transcript_full.jsonl");
            if !transcript.is_file() {
                continue;
            }
            let contained = transcript
                .canonicalize()
                .ok()
                .zip(canonical_root.as_ref())
                .is_some_and(|(path, root)| path.starts_with(root));
            if contained {
                files.push(transcript);
            }
        }
    }
    if let Some(base) = root.parent() {
        let database = base.join("conversation_summaries.db");
        if database.is_file() {
            files.push(database);
        }
    }
    files
}

pub fn resume_agy(summary: &SessionSummary) -> Option<String> {
    Some(format!("agy --conversation={}", summary.session_id))
}

fn parse_transcript(path: &Path) -> Result<ParsedSession, SessionError> {
    let session_id = transcript_session_id(path).ok_or(SessionError::MissingId)?;
    let (records, warnings) = read_records(path)?;
    let mut messages = Vec::new();
    let mut created_at: Option<i64> = None;
    let mut transcript_updated: Option<i64> = None;

    for value in &records {
        if let Some(timestamp) = value.get("created_at").and_then(parse_timestamp) {
            created_at = Some(created_at.map_or(timestamp, |current| current.min(timestamp)));
            transcript_updated =
                Some(transcript_updated.map_or(timestamp, |current| current.max(timestamp)));
        }
        let role = match value.get("type").and_then(Value::as_str) {
            Some("USER_INPUT") => SessionRole::User,
            Some("PLANNER_RESPONSE") => SessionRole::Assistant,
            _ if value.get("source").and_then(Value::as_str) == Some("MODEL") => {
                SessionRole::Assistant
            }
            _ => SessionRole::System,
        };
        let raw = value.get("content").and_then(Value::as_str).unwrap_or("");
        let content = if role == SessionRole::User {
            user_request(raw)
        } else {
            raw
        };
        let content = if content.trim().is_empty() {
            serde_json::to_string_pretty(value).unwrap_or_default()
        } else {
            content.to_string()
        };
        messages.push(ParsedMessage {
            role,
            kind: SessionMessageKind::Text,
            content,
            tool_name: None,
        });
    }

    let (summary_title, summary_project, summary_updated) = summary_metadata(path, &session_id);
    let title = summary_title
        .filter(|title| !title.trim().is_empty())
        .map(|title| truncate_chars(title.trim(), TITLE_MAX_CHARS))
        .or_else(|| first_user_text(&messages))
        .unwrap_or_else(|| truncate_chars(&session_id, TITLE_MAX_CHARS));
    let updated_at = match (transcript_updated, summary_updated) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
    .unwrap_or_else(|| file_mtime_millis(path));

    Ok(ParsedSession {
        session_id,
        title,
        project_path: summary_project,
        created_at,
        updated_at,
        messages,
        warnings,
    })
}

fn read_records(path: &Path) -> Result<(Vec<Value>, Vec<SessionWarning>), SessionError> {
    let mut records = Vec::new();
    let mut index_by_step: HashMap<u64, usize> = HashMap::new();
    let mut warnings = Vec::new();
    let reader = BufReader::new(File::open(path)?);
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
        match value.get("step_index").and_then(Value::as_u64) {
            Some(step) => {
                if let Some(&position) = index_by_step.get(&step) {
                    records[position] = value;
                } else {
                    index_by_step.insert(step, records.len());
                    records.push(value);
                }
            }
            None => records.push(value),
        }
    }
    Ok((records, warnings))
}

fn parse_summary_db(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_error)?;
    let mut stmt = conn
        .prepare(
            "SELECT conversation_id, COALESCE(title, ''), COALESCE(workspace_uris, ''), \
             COALESCE(last_modified_time, '') FROM conversation_summaries",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(db_error)?;

    let mut sessions = Vec::new();
    for row in rows {
        let (id, title, uris, modified) = row.map_err(db_error)?;
        if !valid_session_id(&id) {
            continue;
        }
        let project_path = workspace_project(&uris);
        let title = if title.trim().is_empty() {
            project_path
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| id.clone())
        } else {
            title
        };
        sessions.push(ParsedSession {
            session_id: id,
            title: truncate_chars(title.trim(), TITLE_MAX_CHARS),
            project_path,
            created_at: None,
            updated_at: parse_summary_timestamp(&modified).unwrap_or(0),
            messages: Vec::new(),
            warnings: Vec::new(),
        });
    }
    Ok(sessions)
}

fn summary_metadata(
    transcript: &Path,
    session_id: &str,
) -> (Option<String>, Option<PathBuf>, Option<i64>) {
    let Some(id_dir) = transcript
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    else {
        return (None, None, None);
    };
    let Some(database) = id_dir
        .parent()
        .and_then(Path::parent)
        .map(|base| base.join("conversation_summaries.db"))
    else {
        return (None, None, None);
    };
    let Ok(conn) = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return (None, None, None);
    };
    let row = conn
        .query_row(
            "SELECT title, workspace_uris, last_modified_time \
             FROM conversation_summaries WHERE conversation_id = ?1",
            [session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                    row.get::<_, String>(2).unwrap_or_default(),
                ))
            },
        )
        .ok();
    match row {
        Some((title, uris, modified)) => (
            Some(title),
            workspace_project(&uris),
            parse_summary_timestamp(&modified),
        ),
        None => (None, None, None),
    }
}

fn transcript_session_id(path: &Path) -> Option<String> {
    path.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|id| valid_session_id(id))
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn user_request(content: &str) -> &str {
    content
        .split_once("<USER_REQUEST>")
        .and_then(|(_, rest)| rest.split_once("</USER_REQUEST>"))
        .map(|(inner, _)| inner.trim())
        .filter(|inner| !inner.is_empty())
        .unwrap_or(content)
}

fn workspace_project(uris: &str) -> Option<PathBuf> {
    let uris: Vec<String> = serde_json::from_str(uris).ok()?;
    uris.iter().find_map(|uri| file_uri_to_path(uri))
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    let path = if cfg!(windows) {
        decoded
            .strip_prefix('/')
            .filter(|text| text.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(&decoded)
    } else {
        &decoded
    };
    Some(PathBuf::from(path))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    parse_summary_timestamp(text)
}

fn parse_summary_timestamp(text: &str) -> Option<i64> {
    if text.trim().is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|time| time.and_utc().timestamp_millis())
        })
}

fn first_user_text(messages: &[ParsedMessage]) -> Option<String> {
    messages
        .iter()
        .find(|message| {
            message.role == SessionRole::User && message.kind == SessionMessageKind::Text
        })
        .map(|message| truncate_chars(message.content.trim(), TITLE_MAX_CHARS))
        .filter(|title| !title.is_empty())
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

fn db_error(error: rusqlite::Error) -> SessionError {
    SessionError::Unsupported(format!("Agy 摘要库读取失败：{error}"))
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
    use crate::domain::session::{SessionMessageKind, SessionRole};
    use rusqlite::Connection;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    fn file_uri(path: &Path) -> String {
        let text = path.to_string_lossy().replace('\\', "/");
        format!("file:///{}", text.trim_start_matches('/'))
    }

    fn summary_db(path: &Path, conversations: &[(&str, &str, &str)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries(
                conversation_id TEXT,
                title TEXT,
                workspace_uris TEXT,
                preview TEXT,
                last_modified_time TEXT
            );",
        )
        .unwrap();
        for (id, title, uris) in conversations {
            conn.execute(
                "INSERT INTO conversation_summaries VALUES (?1, ?2, ?3, '', '2026-10-13T17:26:12Z')",
                rusqlite::params![id, title, uris],
            )
            .unwrap();
        }
    }

    #[test]
    fn agy_maps_user_input_and_planner_response_and_dedupes_steps() {
        let parsed = parse_agy_all(&fixture(
            "agy/brain/abc-123/.system_generated/logs/transcript_full.jsonl",
        ))
        .unwrap();
        assert_eq!(parsed.len(), 1);
        let messages = &parsed[0].messages;
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert_eq!(messages[0].role, SessionRole::User);
        assert_eq!(messages[0].content, "修复构建");
        assert_eq!(messages[1].role, SessionRole::Assistant);
        assert_eq!(messages[1].content, "done");
        assert_eq!(messages[2].role, SessionRole::Assistant);
        assert_eq!(messages[2].content, "model step");
        assert_eq!(parsed[0].title, "修复构建");
    }

    #[test]
    fn agy_unknown_step_is_preserved_as_text_not_dropped() {
        let parsed = parse_agy_all(&fixture(
            "agy/brain/unknown-1/.system_generated/logs/transcript_full.jsonl",
        ))
        .unwrap();
        let message = &parsed[0].messages[0];
        assert_eq!(message.role, SessionRole::System);
        assert_eq!(message.kind, SessionMessageKind::Text);
        assert!(message.content.contains("\"tool\""), "{}", message.content);
        assert!(message.content.contains("\"read\""), "{}", message.content);
    }

    #[test]
    fn agy_metadata_only_session_has_no_messages_but_is_listed() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project space");
        let db = dir.path().join("antigravity-cli/conversation_summaries.db");
        let uris = serde_json::to_string(&[file_uri(&project)]).unwrap();
        summary_db(&db, &[("abc-123", "Test title", &uris)]);

        let sessions = parse_agy_all(&db).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "abc-123");
        assert_eq!(sessions[0].title, "Test title");
        assert_eq!(sessions[0].project_path.as_deref(), Some(project.as_path()));
        assert!(sessions[0].messages.is_empty());
        assert!(sessions[0].updated_at > 0);
    }

    #[test]
    fn agy_discovery_lists_transcripts_before_the_summary_db() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("antigravity-cli/brain");
        let logs = root.join("abc-123/.system_generated/logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("transcript_full.jsonl"), "").unwrap();
        std::fs::write(
            dir.path().join("antigravity-cli/conversation_summaries.db"),
            "",
        )
        .unwrap();

        let files = discover_agy(&root, true);
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files[0].ends_with("transcript_full.jsonl"));
        assert!(files[1].ends_with("conversation_summaries.db"));
    }

    #[test]
    fn agy_transcript_uses_summary_metadata_when_available() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("antigravity-cli/brain");
        let logs = root.join("abc-123/.system_generated/logs");
        std::fs::create_dir_all(&logs).unwrap();
        let transcript = logs.join("transcript_full.jsonl");
        std::fs::write(
            &transcript,
            r#"{"step_index":0,"type":"USER_INPUT","content":"原始提问","created_at":"2026-10-13T17:26:12Z"}"#,
        )
        .unwrap();
        let project = dir.path().join("repo");
        let uris = serde_json::to_string(&[file_uri(&project)]).unwrap();
        summary_db(
            &dir.path().join("antigravity-cli/conversation_summaries.db"),
            &[("abc-123", "摘要标题", &uris)],
        );

        let parsed = parse_agy_all(&transcript).unwrap();
        assert_eq!(parsed[0].title, "摘要标题");
        assert_eq!(parsed[0].project_path.as_deref(), Some(project.as_path()));
    }

    #[test]
    fn agy_resume_uses_the_conversation_id() {
        let summary = SessionSummary {
            agent_id: "agy".into(),
            session_id: "abc-123".into(),
            title: "t".into(),
            project_path: None,
            source_path: String::new(),
            created_at: None,
            updated_at: 0,
            message_count: 0,
            parse_error: None,
        };
        assert_eq!(
            resume_agy(&summary).as_deref(),
            Some("agy --conversation=abc-123")
        );
    }
}
