//! MiniMax Code 会话解析器：只读 `runtime-state.sqlite`。

use crate::domain::session::{
    ParsedMessage, ParsedSession, SessionError, SessionMessageKind, SessionRole, SessionSummary,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TITLE_MAX_CHARS: usize = 80;

pub fn parse_mcode_all(database: &Path) -> Result<Vec<ParsedSession>, SessionError> {
    let conn = open_read_only(database)?;
    if !table_exists(&conn, "local_runtime_sessions") {
        return Err(SessionError::Unsupported(
            "MiniMax Code 数据库缺少 local_runtime_sessions 表".to_string(),
        ));
    }
    let mut stmt = conn
        .prepare(
            "SELECT session_id, COALESCE(title, ''), COALESCE(workspace_dir, ''), \
             COALESCE(created_at_ms, 0), COALESCE(updated_at_ms, 0) \
             FROM local_runtime_sessions \
             WHERE visibility <> 'hidden' AND archived = 0 AND parent_session_id IS NULL \
             AND session_kind NOT IN ('peek', 'channel', 'cron') \
             ORDER BY updated_at_ms DESC",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(db_error)?;

    let mut sessions = Vec::new();
    for row in rows {
        let (session_id, title, directory, created, updated) = row.map_err(db_error)?;
        let messages = read_messages(&conn, &session_id)?;
        let project_path = (!directory.trim().is_empty()).then(|| PathBuf::from(&directory));
        let title = choose_title(&title, project_path.as_deref(), &session_id);
        sessions.push(ParsedSession {
            session_id,
            title,
            project_path,
            created_at: (created > 0).then_some(created),
            updated_at: if updated > 0 {
                updated
            } else if created > 0 {
                created
            } else {
                file_mtime_millis(database)
            },
            messages,
            warnings: Vec::new(),
        });
    }
    Ok(sessions)
}

pub fn discover_mcode(root: &Path, override_active: bool) -> Vec<PathBuf> {
    let data_dir = if override_active {
        root.to_path_buf()
    } else {
        std::env::var_os("MINIMAX_DATA_DIR")
            .filter(|value| !value.is_empty())
            .or_else(|| std::env::var_os("MAVIS_DATA_DIR").filter(|value| !value.is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| root.to_path_buf())
    };
    let database = data_dir.join("v2/sqlite/runtime-state.sqlite");
    if database.is_file() {
        vec![database]
    } else {
        Vec::new()
    }
}

pub fn resume_mcode(summary: &SessionSummary) -> Option<String> {
    if !summary
        .session_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return None;
    }
    let data_dir = data_dir_from_source(&summary.source_path)
        .or_else(|| {
            std::env::var_os("MINIMAX_DATA_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::var_os("MAVIS_DATA_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })?;
    let directory = data_dir.to_string_lossy();
    #[cfg(windows)]
    let command = format!(
        "$env:MINIMAX_DATA_DIR='{}'; mcode --session {}",
        directory.replace('\'', "''"),
        summary.session_id
    );
    #[cfg(not(windows))]
    let command = format!(
        "env MINIMAX_DATA_DIR={} mcode --session {}",
        shell_escape(&directory),
        summary.session_id
    );
    Some(command)
}

fn data_dir_from_source(source: &str) -> Option<PathBuf> {
    Path::new(source)
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

#[cfg(not(windows))]
fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn read_messages(conn: &Connection, session_id: &str) -> Result<Vec<ParsedMessage>, SessionError> {
    let migrated = table_exists(conn, "local_runtime_message_row_migrations")
        && conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_runtime_message_row_migrations \
                 WHERE session_id = ?1)",
                [session_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false);
    let mut messages = Vec::new();
    if migrated {
        let mut stmt = conn
            .prepare(
                "SELECT role, data_json FROM local_runtime_message_rows \
                 WHERE session_id = ?1 ORDER BY id ASC",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        for row in rows {
            let (role, data) = row.map_err(db_error)?;
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                push_message(&mut messages, &role, &value);
            }
        }
    } else if table_exists(conn, "local_runtime_messages") {
        let mut stmt = conn
            .prepare(
                "SELECT display_messages_json FROM local_runtime_messages \
                 WHERE session_id = ?1",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map([session_id], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        for row in rows {
            let data = row.map_err(db_error)?;
            let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            for item in items {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("");
                push_message(&mut messages, role, &item);
            }
        }
    }
    Ok(messages)
}

fn push_message(messages: &mut Vec<ParsedMessage>, role: &str, value: &Value) {
    if !matches!(role, "user" | "assistant") {
        return;
    }
    let text = extract_text(value.get("msg_content"));
    if text.trim().is_empty() {
        return;
    }
    messages.push(ParsedMessage {
        role: if role == "user" {
            SessionRole::User
        } else {
            SessionRole::Assistant
        },
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

fn db_error(error: rusqlite::Error) -> SessionError {
    SessionError::Unsupported(format!("MiniMax Code 数据库读取失败：{error}"))
}

fn file_mtime_millis(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::{SessionMessageKind, SessionRole};
    use rusqlite::Connection;

    fn schema(conn: &Connection) {
        conn.execute_batch(
            "
            CREATE TABLE local_runtime_sessions (
                session_id TEXT PRIMARY KEY,
                title TEXT,
                workspace_dir TEXT,
                created_at_ms INTEGER,
                updated_at_ms INTEGER,
                visibility TEXT,
                parent_session_id TEXT,
                session_kind TEXT,
                archived INTEGER
            );
            CREATE TABLE local_runtime_messages (
                session_id TEXT,
                display_messages_json TEXT
            );
            CREATE TABLE local_runtime_message_row_migrations (
                session_id TEXT
            );
            CREATE TABLE local_runtime_message_rows (
                id INTEGER,
                session_id TEXT,
                role TEXT,
                data_json TEXT,
                created_at_ms INTEGER
            );
            ",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO local_runtime_sessions VALUES
             ('mvs_public','Project','D:/work',100,200,'visible',NULL,'conversation',0),
             ('mvs_child','Child','D:/work',100,200,'visible','mvs_public','task',0),
             ('mvs_hidden','Hidden','D:/work',100,200,'hidden',NULL,'conversation',0),
             ('mvs_archived','Archived','D:/work',100,200,'visible',NULL,'conversation',1),
             ('mvs_peek','Peek','D:/work',100,200,'visible',NULL,'peek',0);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO local_runtime_messages VALUES ('mvs_public', ?1)",
            [
                r#"[{"role":"user","msg_content":"Legacy question","timestamp":"100"},
                 {"role":"assistant","msg_content":"Legacy answer","created_at":200},
                 {"role":"tool","msg_content":"Hidden tool output"}]"#,
            ],
        )
        .unwrap();
    }

    #[test]
    fn mcode_filters_hidden_archived_and_child_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("v2/sqlite/runtime-state.sqlite");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let conn = Connection::open(&database).unwrap();
        schema(&conn);
        drop(conn);

        let sessions = parse_mcode_all(&database).unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "{:?}",
            sessions.iter().map(|s| &s.session_id).collect::<Vec<_>>()
        );
        assert_eq!(sessions[0].session_id, "mvs_public");
        assert_eq!(sessions[0].title, "Project");
        assert_eq!(
            sessions[0].project_path.as_deref(),
            Some(Path::new("D:/work"))
        );
        let contents: Vec<_> = sessions[0]
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(contents, vec!["Legacy question", "Legacy answer"]);

        assert_eq!(discover_mcode(dir.path(), true), vec![database]);
    }

    #[test]
    fn mcode_prefers_migrated_rows_over_legacy_blob() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("runtime-state.sqlite");
        let conn = Connection::open(&database).unwrap();
        schema(&conn);
        conn.execute_batch(
            "INSERT INTO local_runtime_message_row_migrations VALUES ('mvs_public');
             INSERT INTO local_runtime_message_rows VALUES
             (1,'mvs_public','user','{\"msg_content\":\"Migrated question\"}',100),
             (2,'mvs_public','assistant','{\"msg_content\":\"Migrated answer\"}',200);",
        )
        .unwrap();

        let sessions = parse_mcode_all(&database).unwrap();
        let contents: Vec<_> = sessions[0]
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(contents, vec!["Migrated question", "Migrated answer"]);

        conn.execute("DELETE FROM local_runtime_message_row_migrations", [])
            .unwrap();
        drop(conn);
        let sessions = parse_mcode_all(&database).unwrap();
        let contents: Vec<_> = sessions[0]
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(contents, vec!["Legacy question", "Legacy answer"]);
    }

    #[test]
    fn mcode_resume_command_quotes_the_data_dir() {
        let summary = SessionSummary {
            agent_id: "mcode".into(),
            session_id: "mvs_public".into(),
            title: "t".into(),
            project_path: None,
            source_path: "C:/data/MiniMax Code's/v2/sqlite/runtime-state.sqlite".into(),
            created_at: None,
            updated_at: 0,
            message_count: 0,
            parse_error: None,
            preview: None,
        };
        #[cfg(windows)]
        assert_eq!(
            resume_mcode(&summary).as_deref(),
            Some("$env:MINIMAX_DATA_DIR='C:/data/MiniMax Code''s'; mcode --session mvs_public")
        );
        #[cfg(not(windows))]
        assert_eq!(
            resume_mcode(&summary).as_deref(),
            Some("env MINIMAX_DATA_DIR='C:/data/MiniMax Code'\\''s' mcode --session mvs_public")
        );

        let mut unsafe_id = summary;
        unsafe_id.session_id = "mvs public; rm".into();
        assert_eq!(resume_mcode(&unsafe_id), None);
    }

    #[test]
    fn mcode_migrated_rows_keep_roles_and_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("runtime-state.sqlite");
        let conn = Connection::open(&database).unwrap();
        schema(&conn);
        conn.execute_batch(
            "INSERT INTO local_runtime_message_row_migrations VALUES ('mvs_public');
             INSERT INTO local_runtime_message_rows VALUES
             (1,'mvs_public','user','{\"msg_content\":\"问题\"}',100),
             (2,'mvs_public','assistant','{\"msg_content\":\"回答\"}',200);",
        )
        .unwrap();
        drop(conn);

        let sessions = parse_mcode_all(&database).unwrap();
        assert_eq!(sessions[0].messages[0].role, SessionRole::User);
        assert_eq!(sessions[0].messages[1].role, SessionRole::Assistant);
        assert!(sessions[0]
            .messages
            .iter()
            .all(|message| message.kind == SessionMessageKind::Text));
    }
}
