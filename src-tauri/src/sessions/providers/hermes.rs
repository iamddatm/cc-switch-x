//! Hermes 会话解析器：`state.db` 优先，`sessions/*.jsonl` 回退；WSL 路径不支持。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionWarning,
};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const TITLE_MAX_CHARS: usize = 80;
const CONTENT_JSON_PREFIX: &str = "\u{0}json:";

pub fn parse_hermes_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    if is_wsl_path(path) {
        return Err(SessionError::Unsupported(
            "Hermes WSL 会话暂不支持".to_string(),
        ));
    }
    let is_database = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("db"));
    if is_database {
        parse_sqlite(path)
    } else {
        Ok(vec![parse_jsonl(path)?])
    }
}

pub fn discover_hermes(root: &Path, override_active: bool) -> Vec<PathBuf> {
    let data_dir = hermes_data_dir(root, override_active);
    if is_wsl_path(&data_dir) {
        return Vec::new();
    }
    let mut files = Vec::new();
    let database = data_dir.join("state.db");
    if database.is_file() {
        files.push(database);
    }
    let Ok(entries) = std::fs::read_dir(data_dir.join("sessions")) else {
        return files;
    };
    let mut transcripts: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        })
        .collect();
    transcripts.sort();
    files.extend(transcripts);
    files
}

fn hermes_data_dir(root: &Path, override_active: bool) -> PathBuf {
    if override_active {
        return root.to_path_buf();
    }
    if let Some(home) = std::env::var_os("HERMES_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        return PathBuf::from(local).join("hermes");
    }
    root.to_path_buf()
}

fn is_wsl_path(path: &Path) -> bool {
    let text = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    text.starts_with("\\\\wsl$\\")
        || text.starts_with("\\\\wsl.localhost\\")
        || text.contains("\\wsl$\\")
        || text.contains("\\wsl.localhost\\")
}

fn parse_sqlite(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let conn = open_read_only(path)?;
    if !table_exists(&conn, "sessions") {
        return Err(SessionError::Unsupported(
            "Hermes 数据库缺少 sessions 表".to_string(),
        ));
    }
    let session_columns = table_columns(&conn, "sessions");
    let has = |name: &str| session_columns.iter().any(|column| column == name);
    if !has("id") {
        return Err(SessionError::Unsupported(
            "Hermes sessions 表缺少 id 列".to_string(),
        ));
    }
    let title_expr = if has("title") {
        "COALESCE(title, '')"
    } else {
        "''"
    };
    let cwd_expr = if has("cwd") { "cwd" } else { "NULL" };
    let directory_expr = if has("directory") {
        "directory"
    } else {
        "NULL"
    };
    let created_expr = if has("started_at") {
        "started_at"
    } else if has("created_at") {
        "created_at"
    } else {
        "NULL"
    };
    let updated_expr = if has("last_activity_at") {
        "last_activity_at"
    } else if has("ended_at") {
        "ended_at"
    } else if has("updated_at") {
        "updated_at"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, {title_expr}, {cwd_expr}, {directory_expr}, {created_expr}, {updated_expr} \
         FROM sessions ORDER BY rowid DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row_time(row, 4),
                row_time(row, 5),
            ))
        })
        .map_err(db_error)?;

    let mut sessions = Vec::new();
    for row in rows {
        let (session_id, title, cwd, directory, created, updated) = row.map_err(db_error)?;
        let messages = read_sqlite_messages(&conn, &session_id)?;
        let project_path = cwd
            .or(directory)
            .filter(|path| !path.trim().is_empty())
            .map(PathBuf::from);
        let title = choose_title(&title, &messages, project_path.as_deref(), &session_id);
        sessions.push(ParsedSession {
            session_id,
            title,
            project_path,
            created_at: created,
            updated_at: updated
                .or(created)
                .unwrap_or_else(|| file_mtime_millis(path)),
            messages,
            warnings: Vec::new(),
        });
    }
    Ok(sessions)
}

fn read_sqlite_messages(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ParsedMessage>, SessionError> {
    if !table_exists(conn, "messages") {
        return Ok(Vec::new());
    }
    let columns = table_columns(conn, "messages");
    let has = |name: &str| columns.iter().any(|column| column == name);
    let expr = |name: &str| {
        if has(name) {
            name.to_string()
        } else {
            format!("NULL AS {name}")
        }
    };
    let filter = match (has("active"), has("compacted")) {
        (true, true) => " AND (active = 1 OR compacted = 1)",
        (true, false) => " AND active = 1",
        _ => "",
    };
    let sql = format!(
        "SELECT {}, {}, {}, {}, {}, {} FROM messages \
         WHERE session_id = ?1{filter} ORDER BY id ASC",
        expr("role"),
        expr("content"),
        expr("tool_calls"),
        expr("tool_call_id"),
        expr("tool_name"),
        expr("reasoning"),
    );
    let mut stmt = conn.prepare(&sql).map_err(db_error)?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(db_error)?;

    let mut messages = Vec::new();
    let mut tool_names = HashMap::new();
    for row in rows {
        let (role, content, tool_calls, tool_call_id, tool_name, reasoning) =
            row.map_err(db_error)?;
        let role = role.unwrap_or_default();
        let text = decode_content(content.as_deref().unwrap_or(""));
        match role.as_str() {
            "tool" => {
                let name = tool_name.or_else(|| {
                    tool_call_id
                        .as_deref()
                        .and_then(|id| tool_names.get(id))
                        .cloned()
                });
                push_tool_result(&mut messages, &text, name);
            }
            "assistant" => {
                push_thinking(&mut messages, reasoning.as_deref());
                push_text(&mut messages, SessionRole::Assistant, &text);
                let calls = tool_calls
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
                push_tool_calls(&mut messages, &mut tool_names, calls.as_ref());
            }
            "user" => push_text(&mut messages, SessionRole::User, &text),
            "system" => push_text(&mut messages, SessionRole::System, &text),
            _ => push_text(&mut messages, SessionRole::System, &text),
        }
    }
    Ok(messages)
}

fn parse_jsonl(path: &Path) -> Result<ParsedSession, SessionError> {
    let reader = BufReader::new(File::open(path)?);
    let mut session_id: Option<String> = None;
    let mut title: Option<String> = None;
    let mut project: Option<String> = None;
    let mut messages = Vec::new();
    let mut tool_names = HashMap::new();
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
        let line_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        if line_type == "session" || line_type == "init" {
            if session_id.is_none() {
                session_id = value
                    .get("id")
                    .or_else(|| value.get("sessionId"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if title.is_none() {
                title = value
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if project.is_none() {
                project = value
                    .get("cwd")
                    .or_else(|| value.get("directory"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
        }

        let (message, timestamp) = if line_type == "message" {
            let message = value.get("message");
            (
                message,
                value
                    .get("timestamp")
                    .or_else(|| message.and_then(|message| message.get("ts"))),
            )
        } else if value.get("role").is_some() {
            (
                Some(&value),
                value.get("timestamp").or_else(|| value.get("ts")),
            )
        } else {
            (None, value.get("timestamp").or_else(|| value.get("ts")))
        };
        if let Some(timestamp) = timestamp.and_then(value_to_millis) {
            created_at = Some(created_at.map_or(timestamp, |current| current.min(timestamp)));
            updated_at = Some(updated_at.map_or(timestamp, |current| current.max(timestamp)));
        }
        if let Some(message) = message {
            push_jsonl_message(&mut messages, &mut tool_names, message);
        }
    }

    let session_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .ok_or(SessionError::MissingId)?;
    let project_path = project
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    let title = choose_title(
        title.as_deref().unwrap_or(""),
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

fn push_jsonl_message(
    messages: &mut Vec<ParsedMessage>,
    tool_names: &mut HashMap<String, String>,
    message: &Value,
) {
    let Some(role) = message.get("role").and_then(Value::as_str) else {
        return;
    };
    let text = decode_value(message.get("content"));
    match role {
        "tool" => {
            let name = message
                .get("tool_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    message
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .and_then(|id| tool_names.get(id))
                        .cloned()
                });
            push_tool_result(messages, &text, name);
        }
        "assistant" => {
            push_thinking(messages, message.get("reasoning").and_then(Value::as_str));
            push_text(messages, SessionRole::Assistant, &text);
            let calls = match message.get("tool_calls") {
                Some(Value::String(raw)) => serde_json::from_str::<Value>(raw).ok(),
                Some(value) => Some(value.clone()),
                None => None,
            };
            push_tool_calls(messages, tool_names, calls.as_ref());
        }
        "user" => push_text(messages, SessionRole::User, &text),
        _ => push_text(messages, SessionRole::System, &text),
    }
}

fn push_text(messages: &mut Vec<ParsedMessage>, role: SessionRole, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    messages.push(ParsedMessage {
        role,
        kind: SessionMessageKind::Text,
        content: text.to_string(),
        tool_name: None,
    });
}

fn push_thinking(messages: &mut Vec<ParsedMessage>, reasoning: Option<&str>) {
    let Some(reasoning) = reasoning.filter(|text| !text.trim().is_empty()) else {
        return;
    };
    messages.push(ParsedMessage {
        role: SessionRole::Assistant,
        kind: SessionMessageKind::Thinking,
        content: reasoning.to_string(),
        tool_name: None,
    });
}

fn push_tool_result(messages: &mut Vec<ParsedMessage>, text: &str, tool_name: Option<String>) {
    if text.trim().is_empty() {
        return;
    }
    messages.push(ParsedMessage {
        role: SessionRole::Tool,
        kind: SessionMessageKind::ToolResult,
        content: text.to_string(),
        tool_name,
    });
}

fn push_tool_calls(
    messages: &mut Vec<ParsedMessage>,
    tool_names: &mut HashMap<String, String>,
    calls: Option<&Value>,
) {
    let Some(calls) = calls.and_then(Value::as_array) else {
        return;
    };
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

fn decode_content(raw: &str) -> String {
    match raw.strip_prefix(CONTENT_JSON_PREFIX) {
        Some(json) => match serde_json::from_str::<Value>(json) {
            Ok(value) => extract_text(&value),
            Err(_) => raw.to_string(),
        },
        None => raw.to_string(),
    }
}

fn decode_value(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => decode_content(text),
        Some(value) => extract_text(value),
        None => String::new(),
    }
}

fn extract_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn choose_title(
    title: &str,
    messages: &[ParsedMessage],
    project_path: Option<&Path>,
    session_id: &str,
) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return truncate_chars(title, TITLE_MAX_CHARS);
    }
    if let Some(first) = messages.iter().find(|message| {
        message.role == SessionRole::User && message.kind == SessionMessageKind::Text
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

fn value_to_millis(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_f64() {
        return millis_from_number(number);
    }
    let text = value.as_str()?;
    parse_time_string(text).or_else(|| text.parse::<f64>().ok().and_then(millis_from_number))
}

fn row_time(row: &rusqlite::Row<'_>, index: usize) -> Option<i64> {
    match row.get_ref(index).ok()? {
        ValueRef::Integer(number) => millis_from_number(number as f64),
        ValueRef::Real(number) => millis_from_number(number),
        ValueRef::Text(bytes) => {
            let text = std::str::from_utf8(bytes).ok()?;
            parse_time_string(text)
                .or_else(|| text.parse::<f64>().ok().and_then(millis_from_number))
        }
        _ => None,
    }
}

fn millis_from_number(number: f64) -> Option<i64> {
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    Some(if number > 10_000_000_000.0 {
        number as i64
    } else {
        (number * 1000.0).round() as i64
    })
}

fn parse_time_string(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|time| time.and_utc().timestamp_millis())
        })
}

fn open_read_only(path: &Path) -> Result<Connection, SessionError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_error)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(db_error)?;
    Ok(conn)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

fn db_error(error: rusqlite::Error) -> SessionError {
    SessionError::Unsupported(format!("Hermes 数据库读取失败：{error}"))
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
    use std::time::Duration;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sessions")
            .join(name)
    }

    fn hermes_db(dir: &Path) -> (PathBuf, Connection) {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("state.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY, title TEXT, cwd TEXT,
                 started_at REAL, last_activity_at REAL
             );
             CREATE TABLE messages (
                 id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT,
                 role TEXT, content TEXT, tool_calls TEXT, tool_call_id TEXT,
                 tool_name TEXT, timestamp REAL,
                 active INTEGER NOT NULL DEFAULT 1,
                 compacted INTEGER NOT NULL DEFAULT 0,
                 reasoning TEXT
             );",
        )
        .unwrap();
        (path, conn)
    }

    fn insert_message(
        conn: &Connection,
        session: &str,
        role: &str,
        content: &str,
        timestamp: f64,
        active: i64,
        compacted: i64,
    ) {
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp, active, compacted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![session, role, content, timestamp, active, compacted],
        )
        .unwrap();
    }

    #[test]
    fn hermes_sqlite_is_preferred_over_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let hermes_dir = dir.path().join("hermes");
        let (database, conn) = hermes_db(&hermes_dir);
        conn.execute(
            "INSERT INTO sessions VALUES ('h1', 'SQLite 标题', 'D:/hrepo', 100.0, 200.0)",
            [],
        )
        .unwrap();
        insert_message(&conn, "h1", "user", "第一问", 101.0, 1, 0);
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, reasoning, timestamp)
             VALUES ('h1', 'assistant', '回答', ?1, '想', 102.0)",
            [r#"[{"id":"c1","function":{"name":"terminal","arguments":"{\"command\":\"ls\"}"}}]"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_call_id, timestamp)
             VALUES ('h1', 'tool', '结果', 'c1', 103.0)",
            [],
        )
        .unwrap();
        insert_message(&conn, "h1", "assistant", "被撤销", 104.0, 0, 0);
        insert_message(&conn, "h1", "assistant", "压缩保留", 105.0, 0, 1);
        drop(conn);

        let jsonl = hermes_dir.join("sessions/h1.jsonl");
        std::fs::create_dir_all(jsonl.parent().unwrap()).unwrap();
        std::fs::write(
            &jsonl,
            "{\"type\":\"session\",\"id\":\"h1\",\"title\":\"JSONL 标题\",\"cwd\":\"D:/hrepo\"}\n\
             {\"role\":\"user\",\"content\":\"JSONL 问题\"}\n",
        )
        .unwrap();

        let files = discover_hermes(&hermes_dir, true);
        assert_eq!(files, vec![database.clone(), jsonl.clone()]);

        let sqlite_sessions = parse_hermes_all(&database).unwrap();
        assert_eq!(sqlite_sessions[0].title, "SQLite 标题");
        let messages = &sqlite_sessions[0].messages;
        let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            contents,
            vec![
                "第一问",
                "想",
                "回答",
                "{\"command\":\"ls\"}",
                "结果",
                "压缩保留"
            ]
        );
        assert!(!contents.contains(&"被撤销"));
        assert!(messages.iter().any(|m| {
            m.kind == SessionMessageKind::ToolCall && m.tool_name.as_deref() == Some("terminal")
        }));
        assert!(messages.iter().any(|m| {
            m.kind == SessionMessageKind::ToolResult
                && m.tool_name.as_deref() == Some("terminal")
                && m.content == "结果"
        }));

        let jsonl_sessions = parse_hermes_all(&jsonl).unwrap();
        assert_eq!(jsonl_sessions[0].title, "JSONL 标题");
    }

    #[test]
    fn hermes_jsonl_supports_flat_and_nested_shapes() {
        let parsed = parse_hermes_all(&fixture("hermes/sessions/mixed.jsonl")).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].session_id, "h2");
        assert_eq!(parsed[0].title, "Hermes JSONL");
        assert_eq!(
            parsed[0].project_path.as_deref(),
            Some(Path::new("D:/hrepo2"))
        );
        let messages = &parsed[0].messages;
        assert_eq!(messages[0].role, SessionRole::User);
        assert_eq!(messages[0].content, "扁平问题");
        assert_eq!(messages[1].kind, SessionMessageKind::Thinking);
        assert_eq!(messages[1].content, "想一下");
        assert_eq!(messages[2].content, "嵌套回答");
        assert_eq!(messages[3].kind, SessionMessageKind::ToolCall);
        assert_eq!(messages[3].tool_name.as_deref(), Some("terminal"));
        assert_eq!(messages[4].kind, SessionMessageKind::ToolResult);
        assert_eq!(messages[4].tool_name.as_deref(), Some("terminal"));
        assert_eq!(messages[4].content, "结果");
    }

    #[test]
    fn hermes_wsl_paths_are_reported_not_parsed() {
        let wsl = Path::new(r"\\wsl$\Ubuntu\home\me\.hermes\state.db");
        assert!(is_wsl_path(wsl));
        assert!(is_wsl_path(Path::new(
            r"\\wsl.localhost\Ubuntu\home\me\.hermes\sessions\s1.jsonl"
        )));
        let error = parse_hermes_all(wsl).expect_err("WSL 路径必须拒绝");
        assert!(
            matches!(&error, SessionError::Unsupported(message) if message.contains("WSL")),
            "{error:?}"
        );

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("wsl$/Ubuntu/hermes");
        std::fs::create_dir_all(root.join("sessions")).unwrap();
        std::fs::write(root.join("sessions/s1.jsonl"), "{}").unwrap();
        assert!(discover_hermes(&root, true).is_empty());
    }

    #[test]
    fn hermes_sqlite_limits_reads_with_a_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let (database, conn) = hermes_db(dir.path());
        conn.busy_timeout(Duration::from_millis(500)).unwrap();
        conn.execute(
            "INSERT INTO sessions VALUES ('h1', 'T', 'D:/h', 1.0, 2.0)",
            [],
        )
        .unwrap();
        drop(conn);
        assert_eq!(parse_hermes_all(&database).unwrap().len(), 1);
    }
}
