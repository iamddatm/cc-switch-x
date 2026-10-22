//! OpenCode 会话解析器：优先 `opencode.db`（V1/V2 表），无数据库时读 `storage/session` JSON。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const TITLE_MAX_CHARS: usize = 80;

pub fn parse_opencode_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let is_database = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("db"));
    if is_database {
        return parse_sqlite(path);
    }
    Ok(vec![parse_session_file(path)?])
}

pub fn discover_opencode(root: &Path, override_active: bool) -> Vec<PathBuf> {
    let data_root = if override_active {
        root.to_path_buf()
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(|value| PathBuf::from(value).join("opencode"))
            .unwrap_or_else(|| root.to_path_buf())
    };
    let database = data_root.join("opencode.db");
    if database.is_file() {
        return vec![database];
    }
    let mut files = Vec::new();
    collect_json_files(&data_root.join("storage/session"), &mut files);
    files.sort();
    files
}

pub fn resume_opencode(summary: &SessionSummary) -> Option<String> {
    Some(format!("opencode -s {}", summary.session_id))
}

fn parse_sqlite(path: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db_error)?;
    let has_v1 = table_exists(&conn, "session");
    let has_v2 = table_exists(&conn, "session_v2") && table_exists(&conn, "session_message");
    if !has_v1 && !has_v2 {
        return Err(SessionError::Unsupported(
            "OpenCode 数据库没有 session/session_v2 表".to_string(),
        ));
    }

    let fallback_updated = file_mtime_millis(path);
    let mut sessions = Vec::new();
    let mut v2_ids = HashSet::new();
    if has_v2 {
        for row in read_session_rows(&conn, "session_v2")? {
            v2_ids.insert(row.id.clone());
            let messages = read_v2_messages(&conn, &row.id)?;
            sessions.push(build_session(row, messages, fallback_updated));
        }
    }
    if has_v1 {
        for row in read_session_rows(&conn, "session")? {
            if v2_ids.contains(&row.id) {
                continue;
            }
            let messages = read_v1_messages(&conn, &row.id)?;
            sessions.push(build_session(row, messages, fallback_updated));
        }
    }
    Ok(sessions)
}

struct SessionRow {
    id: String,
    title: String,
    directory: String,
    created: i64,
    updated: i64,
}

fn read_session_rows(conn: &Connection, table: &str) -> Result<Vec<SessionRow>, SessionError> {
    let sql = format!(
        "SELECT id, COALESCE(title, ''), COALESCE(directory, ''), \
         COALESCE(time_created, 0), COALESCE(time_updated, 0) \
         FROM {table} ORDER BY time_updated DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                directory: row.get(2)?,
                created: row.get(3)?,
                updated: row.get(4)?,
            })
        })
        .map_err(db_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
}

fn read_v1_messages(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ParsedMessage>, SessionError> {
    let mut parts: HashMap<String, Vec<Value>> = HashMap::new();
    let mut part_stmt = conn
        .prepare(
            "SELECT message_id, data FROM part WHERE session_id = ?1 \
             ORDER BY time_created ASC, id ASC",
        )
        .map_err(db_error)?;
    let part_rows = part_stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?;
    for row in part_rows {
        let (message_id, data) = row.map_err(db_error)?;
        if let Ok(part) = serde_json::from_str::<Value>(&data) {
            parts.entry(message_id).or_default().push(part);
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, data FROM message WHERE session_id = ?1 \
             ORDER BY time_created ASC, id ASC",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?;

    let mut messages = Vec::new();
    for row in rows {
        let (message_id, data) = row.map_err(db_error)?;
        let Ok(info) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let role = info
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if let Some(parts) = parts.remove(&message_id) {
            for part in &parts {
                push_part_messages(&mut messages, role, part);
            }
        }
    }
    Ok(messages)
}

fn read_v2_messages(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ParsedMessage>, SessionError> {
    let mut stmt = conn
        .prepare(
            "SELECT type, data FROM session_message WHERE session_id = ?1 \
             ORDER BY seq ASC, rowid ASC",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(db_error)?;

    let mut messages = Vec::new();
    for row in rows {
        let (message_type, data) = row.map_err(db_error)?;
        if !matches!(message_type.as_str(), "user" | "assistant" | "system") {
            continue;
        }
        let Ok(info) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        match (
            message_type.as_str(),
            info.get("text").and_then(Value::as_str),
            info.get("content"),
        ) {
            ("user" | "system", Some(text), _) => {
                push_text_part(&mut messages, &message_type, text);
            }
            (_, _, Some(Value::Array(items))) if message_type != "system" => {
                for item in items {
                    push_part_messages(&mut messages, &message_type, item);
                }
            }
            (_, _, Some(Value::String(text))) if message_type == "user" => {
                push_text_part(&mut messages, &message_type, text);
            }
            (_, Some(text), _) => push_text_part(&mut messages, &message_type, text),
            _ => {}
        }
    }
    Ok(messages)
}

fn push_text_part(messages: &mut Vec<ParsedMessage>, role: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    messages.push(ParsedMessage {
        role: role_from_str(role),
        kind: SessionMessageKind::Text,
        content: text.to_string(),
        tool_name: None,
    });
}

fn push_part_messages(messages: &mut Vec<ParsedMessage>, role: &str, part: &Value) {
    if let Some(text) = part.as_str() {
        push_text_part(messages, role, text);
        return;
    }
    match part.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => {
            if part.get("synthetic").and_then(Value::as_bool) == Some(true)
                || part.get("ignored").and_then(Value::as_bool) == Some(true)
            {
                return;
            }
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                push_text_part(messages, role, text);
            }
        }
        "reasoning" => {
            if let Some(text) = part
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                messages.push(ParsedMessage {
                    role: role_from_str(role),
                    kind: SessionMessageKind::Thinking,
                    content: text.to_string(),
                    tool_name: None,
                });
            }
        }
        "tool" => push_tool_messages(messages, part),
        "agent" => {
            let content = part
                .get("name")
                .and_then(Value::as_str)
                .map(|name| format!("@{name}"))
                .unwrap_or_else(|| serde_json::to_string(part).unwrap_or_default());
            messages.push(ParsedMessage {
                role: SessionRole::System,
                kind: SessionMessageKind::Event,
                content,
                tool_name: None,
            });
        }
        "patch" => {
            let content = match part.get("files").and_then(Value::as_array) {
                Some(files) if !files.is_empty() => format!(
                    "patch: {}",
                    files
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                _ => serde_json::to_string(part).unwrap_or_default(),
            };
            messages.push(ParsedMessage {
                role: SessionRole::System,
                kind: SessionMessageKind::Event,
                content,
                tool_name: None,
            });
        }
        "file" => {
            let content = part
                .get("filename")
                .or_else(|| part.get("url"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| serde_json::to_string(part).unwrap_or_default());
            messages.push(ParsedMessage {
                role: role_from_str(role),
                kind: SessionMessageKind::Event,
                content,
                tool_name: None,
            });
        }
        _ => {}
    }
}

fn push_tool_messages(messages: &mut Vec<ParsedMessage>, part: &Value) {
    let name = part
        .get("tool")
        .or_else(|| part.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let state = part.get("state");
    let input = state
        .and_then(|state| state.get("input"))
        .or_else(|| part.get("input"))
        .cloned()
        .unwrap_or(Value::Null);
    let content = if input.is_null() {
        "{}".to_string()
    } else {
        serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string())
    };
    messages.push(ParsedMessage {
        role: SessionRole::Assistant,
        kind: SessionMessageKind::ToolCall,
        content,
        tool_name: Some(name.clone()),
    });

    let status = state
        .and_then(|state| state.get("status"))
        .or_else(|| part.get("status"))
        .and_then(Value::as_str);
    let output = state
        .and_then(|state| state.get("output"))
        .or_else(|| part.get("output"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let error = state
        .and_then(|state| state.get("error"))
        .or_else(|| part.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let result = match status {
        Some("error") if !error.is_empty() => format!("[error] {error}"),
        Some("pending" | "running") => output.to_string(),
        _ if !output.is_empty() => output.to_string(),
        _ if !error.is_empty() => format!("[error] {error}"),
        _ => String::new(),
    };
    if !result.is_empty() {
        messages.push(ParsedMessage {
            role: SessionRole::Tool,
            kind: SessionMessageKind::ToolResult,
            content: result,
            tool_name: Some(name),
        });
    }
}

fn build_session(
    row: SessionRow,
    messages: Vec<ParsedMessage>,
    fallback_updated: i64,
) -> ParsedSession {
    let project_path = (!row.directory.trim().is_empty()).then(|| PathBuf::from(&row.directory));
    let title = choose_title(&row.title, project_path.as_deref(), &row.id);
    ParsedSession {
        session_id: row.id,
        title,
        project_path,
        created_at: (row.created > 0).then_some(row.created),
        updated_at: if row.updated > 0 {
            row.updated
        } else if row.created > 0 {
            row.created
        } else {
            fallback_updated
        },
        messages,
        warnings: Vec::new(),
    }
}

fn parse_session_file(path: &Path) -> Result<ParsedSession, SessionError> {
    let data = std::fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&data)?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(SessionError::MissingId)?
        .to_string();
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let directory = value
        .get("directory")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let created = value.pointer("/time/created").and_then(value_to_millis);
    let updated = value.pointer("/time/updated").and_then(value_to_millis);
    let storage = storage_root(path).ok_or_else(|| {
        SessionError::Unsupported(format!(
            "OpenCode 会话文件不在 storage 目录下：{}",
            path.display()
        ))
    })?;
    let messages = read_file_messages(&storage, &id)?;
    let row = SessionRow {
        id,
        title,
        directory,
        created: created.unwrap_or(0),
        updated: updated.or(created).unwrap_or(0),
    };
    Ok(build_session(row, messages, file_mtime_millis(path)))
}

fn read_file_messages(
    storage: &Path,
    session_id: &str,
) -> Result<Vec<ParsedMessage>, SessionError> {
    let mut message_files = Vec::new();
    collect_json_files(
        &storage.join("message").join(session_id),
        &mut message_files,
    );
    message_files.sort();

    let mut entries: Vec<(i64, String, String, Vec<PathBuf>)> = Vec::new();
    for message_path in message_files {
        let Ok(data) = std::fs::read_to_string(&message_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let Some(message_id) = value.get("id").and_then(Value::as_str) else {
            continue;
        };
        let role = value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let created = value
            .pointer("/time/created")
            .and_then(value_to_millis)
            .unwrap_or(0);
        let mut part_files = Vec::new();
        collect_json_files(&storage.join("part").join(message_id), &mut part_files);
        part_files.sort();
        entries.push((created, message_id.to_string(), role, part_files));
    }
    entries.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));

    let mut messages = Vec::new();
    for (_, _, role, part_files) in entries {
        for part_path in part_files {
            let Ok(data) = std::fs::read_to_string(&part_path) else {
                continue;
            };
            let Ok(part) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            push_part_messages(&mut messages, &role, &part);
        }
    }
    Ok(messages)
}

fn storage_root(session_file: &Path) -> Option<PathBuf> {
    session_file
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "storage"))
        .map(Path::to_path_buf)
}

fn collect_json_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            collect_json_files(&path, files);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

fn db_error(error: rusqlite::Error) -> SessionError {
    SessionError::Unsupported(format!("OpenCode 数据库读取失败：{error}"))
}

fn role_from_str(role: &str) -> SessionRole {
    match role {
        "user" => SessionRole::User,
        "assistant" => SessionRole::Assistant,
        _ => SessionRole::System,
    }
}

fn choose_title(title: &str, project_path: Option<&Path>, session_id: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return truncate_chars(title, TITLE_MAX_CHARS);
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

    fn v1_schema(conn: &Connection) {
        conn.execute_batch(
            "
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                directory TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            ",
        )
        .unwrap();
    }

    fn v2_schema(conn: &Connection) {
        conn.execute_batch(
            "
            CREATE TABLE session_v2 (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                directory TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );
            CREATE TABLE session_message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                type TEXT NOT NULL,
                seq INTEGER NOT NULL,
                data TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );
            ",
        )
        .unwrap();
    }

    #[test]
    fn opencode_sqlite_v1_reads_session_message_parts_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        v1_schema(&conn);
        conn.execute(
            "INSERT INTO session VALUES ('ses_1', '标题', 'D:/repo', 1000, 3000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('msg_1', 'ses_1', 1000, '{\"role\":\"user\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('msg_2', 'ses_1', 2000, '{\"role\":\"assistant\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('prt_1', 'ses_1', 'msg_1', 1000, '{\"type\":\"text\",\"text\":\"你好\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('prt_2', 'ses_1', 'msg_2', 2000, '{\"type\":\"tool\",\"callID\":\"c1\",\"tool\":\"bash\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"echo hi\"},\"output\":\"file.txt\"}}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('prt_3', 'ses_1', 'msg_2', 2001, '{\"type\":\"text\",\"text\":\"完成\"}')",
            [],
        )
        .unwrap();
        drop(conn);

        let sessions = parse_opencode_all(&db).unwrap();
        assert_eq!(sessions.len(), 1);
        let messages = &sessions[0].messages;
        assert_eq!(messages[0].role, SessionRole::User);
        assert_eq!(messages[0].content, "你好");
        assert_eq!(messages[1].kind, SessionMessageKind::ToolCall);
        assert_eq!(messages[1].tool_name.as_deref(), Some("bash"));
        assert!(messages[1].content.contains("echo hi"));
        assert_eq!(messages[2].kind, SessionMessageKind::ToolResult);
        assert_eq!(messages[2].tool_name.as_deref(), Some("bash"));
        assert_eq!(messages[2].content, "file.txt");
        assert_eq!(messages[3].content, "完成");
    }

    #[test]
    fn opencode_v2_sorts_by_seq_not_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        v2_schema(&conn);
        conn.execute(
            "INSERT INTO session_v2 VALUES ('ses_1', 'V2', 'D:/repo', 1000, 3000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message VALUES ('msg_1', 'ses_1', 'user', 1, '{\"text\":\"First\"}', 5000, 5000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message VALUES ('msg_2', 'ses_1', 'assistant', 2, '{\"content\":[{\"type\":\"text\",\"text\":\"Second\"}]}', 1000, 1000)",
            [],
        )
        .unwrap();
        drop(conn);

        let sessions = parse_opencode_all(&db).unwrap();
        let contents: Vec<_> = sessions[0]
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(contents, vec!["First", "Second"]);
    }

    #[test]
    fn opencode_prefers_sqlite_over_file_storage() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        v1_schema(&conn);
        conn.execute(
            "INSERT INTO session VALUES ('ses_1', '数据库标题', 'D:/repo', 1000, 3000)",
            [],
        )
        .unwrap();
        drop(conn);

        let session_file = dir.path().join("storage/session/proj/ses_1.json");
        std::fs::create_dir_all(session_file.parent().unwrap()).unwrap();
        std::fs::write(
            &session_file,
            r#"{"id":"ses_1","title":"文件标题","directory":"D:/repo","time":{"created":1000,"updated":3000}}"#,
        )
        .unwrap();

        let files = discover_opencode(dir.path(), true);
        assert_eq!(files, vec![db.clone()]);
        let sessions = parse_opencode_all(&files[0]).unwrap();
        assert_eq!(sessions[0].title, "数据库标题");
    }

    #[test]
    #[allow(deprecated)]
    fn opencode_discovery_uses_xdg_data_home_unless_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let xdg = dir.path().join("xdg");
        let db = xdg.join("opencode/opencode.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        std::fs::write(&db, "").unwrap();

        let previous = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", &xdg);
        let files = discover_opencode(Path::new("D:/unused"), false);
        if let Some(previous) = previous {
            std::env::set_var("XDG_DATA_HOME", previous);
        } else {
            std::env::remove_var("XDG_DATA_HOME");
        }
        assert_eq!(files, vec![db]);
    }

    #[test]
    fn opencode_resume_uses_the_session_id() {
        let summary = SessionSummary {
            agent_id: "opencode".into(),
            session_id: "ses_1".into(),
            title: "t".into(),
            project_path: None,
            source_path: "D:/opencode.db".into(),
            created_at: None,
            updated_at: 0,
            message_count: 0,
            parse_error: None,
        };
        assert_eq!(
            resume_opencode(&summary).as_deref(),
            Some("opencode -s ses_1")
        );
    }
}
