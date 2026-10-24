use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 导入源：本机装好的 cc-switch 库，或已解包的云快照目录。
/// 云快照的下载与解包属传输层（Plan 5），这里只认解包后的 `manifest.json + db.sql`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSource {
    InstalledDb(PathBuf),
    SnapshotDir(PathBuf),
}

/// cc-switch 云快照的格式标识（spec 7.2）。
pub const SNAPSHOT_FORMAT: &str = "cc-switch-webdav-sync";

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("数据库错误：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("快照清单的 format 不是 {SNAPSHOT_FORMAT}：{0}")]
    BadManifest(String),
    #[error("导入源缺少文件：{0}")]
    MissingFile(String),
    #[error("导入源缺少表：{0}")]
    MissingTable(String),
    #[error("真源读取失败：{0}")]
    Store(#[from] crate::store::StoreError),
    #[error("写入失败：{0}")]
    Secrets(#[from] crate::store::secrets::SecretsError),
    #[error("暂不支持的 MCP type：{0}")]
    UnsupportedTransport(String),
    #[error("索引写入失败：{0}")]
    Index(#[from] crate::index::IndexError),
    #[error("元数据写入失败：{0}")]
    Meta(#[from] crate::domain::skill_meta::SkillMetaError),
}

/// 打开的导入源。SnapshotDir 要把 `db.sql` 还原进临时库，临时目录必须活得比连接久，
/// 所以一起装进这个结构里，由它负责清理。
pub struct ImportDb {
    conn: Connection,
    _temp: Option<tempfile::TempDir>,
}

impl ImportDb {
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

pub fn connect(source: &ImportSource) -> Result<ImportDb, ImportError> {
    match source {
        // 只给 READ_ONLY，**故意不给 SQLITE_OPEN_URI**：那样文件名按字面解释，
        // 路径里的 `?`/`#`/`%` 不会被当成查询串或转义序列，也不需要自己拼 `?mode=ro`。
        ImportSource::InstalledDb(path) => {
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            // cc-switch 可能正在运行：等一小会儿，等不到就报错，绝不去碰它的写锁
            conn.busy_timeout(Duration::from_millis(500))?;
            Ok(ImportDb { conn, _temp: None })
        }
        ImportSource::SnapshotDir(dir) => open_snapshot(dir),
    }
}

fn open_snapshot(dir: &Path) -> Result<ImportDb, ImportError> {
    let manifest_path = dir.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(ImportError::MissingFile("manifest.json".to_string()));
    }
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    let format = manifest
        .get("format")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    if format != SNAPSHOT_FORMAT {
        return Err(ImportError::BadManifest(format));
    }

    let sql_path = dir.join("db.sql");
    if !sql_path.is_file() {
        return Err(ImportError::MissingFile("db.sql".to_string()));
    }
    let temp = tempfile::tempdir()?;
    let conn = Connection::open(temp.path().join("import.db"))?;
    conn.execute_batch(&std::fs::read_to_string(&sql_path)?)?;
    Ok(ImportDb {
        conn,
        _temp: Some(temp),
    })
}

/// 列名照本机实测的 cc-switch v4.0.16 建；多出来的 `enabled_quantum` 用来验证未知列处理。
/// 同一份 SQL 既建临时库（InstalledDb），也当云快照里的 `db.sql`（SnapshotDir）。
#[cfg(test)]
pub(crate) fn fake_ccswitch_sql() -> &'static str {
    r#"
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY, name TEXT, server_config TEXT, description TEXT,
        homepage TEXT, docs TEXT, tags TEXT,
        enabled_claude INTEGER, enabled_codex INTEGER, enabled_gemini INTEGER,
        enabled_opencode INTEGER, enabled_hermes INTEGER, enabled_grokbuild INTEGER,
        enabled_mcode INTEGER, enabled_pi INTEGER, enabled_quantum INTEGER
    );
    CREATE TABLE prompts (
        id TEXT, app_type TEXT, name TEXT, content TEXT, description TEXT,
        enabled INTEGER, created_at INTEGER, updated_at INTEGER,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        id TEXT PRIMARY KEY, name TEXT, description TEXT, directory TEXT,
        repo_owner TEXT, repo_name TEXT, repo_branch TEXT, readme_url TEXT,
        enabled_claude INTEGER, enabled_codex INTEGER, enabled_gemini INTEGER,
        enabled_opencode INTEGER, enabled_hermes INTEGER, installed_at INTEGER,
        content_hash TEXT, updated_at INTEGER, enabled_grokbuild INTEGER,
        enabled_mcode INTEGER
    );
    INSERT INTO mcp_servers (id,name,server_config,enabled_claude,enabled_codex,enabled_quantum)
        VALUES ('pw','playwright','{"type":"stdio","command":"npx","args":[],"env":{}}',1,0,1);
    INSERT INTO prompts (id,app_type,name,content,enabled,updated_at)
        VALUES ('p1','claude','团队提示词','正文',1,100);
    INSERT INTO skills (id,name,directory,repo_owner,repo_name,repo_branch,enabled_claude)
        VALUES ('acme/demo/skill','demo','demo','acme','demo','main',1);
    "#
}

#[cfg(test)]
pub(crate) fn seed_fake_ccswitch(path: &Path) {
    // 同样不给 URI 位，否则带 `#`/`?`/`%` 的测试路径连库都建不出来
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .unwrap();
    conn.execute_batch(fake_ccswitch_sql()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn opens_installed_db_read_only() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("cc-switch.db");
        seed_fake_ccswitch(&db);
        let before = std::fs::read(&db).unwrap();

        let opened = connect(&ImportSource::InstalledDb(db.clone())).unwrap();
        let count: i64 = opened
            .conn()
            .query_row("SELECT COUNT(*) FROM mcp_servers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        drop(opened);

        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "导入绝不能写 cc-switch 的库"
        );
    }

    #[test]
    fn writing_to_the_source_db_is_rejected() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("cc-switch.db");
        seed_fake_ccswitch(&db);
        let opened = connect(&ImportSource::InstalledDb(db)).unwrap();
        assert!(
            opened
                .conn()
                .execute("DELETE FROM mcp_servers", [])
                .is_err(),
            "只读连接上的写操作必须失败"
        );
    }

    #[test]
    fn awkward_paths_still_open() {
        let dir = tempdir().unwrap();
        // 空格与中文用户名是 Windows 上真实会遇到的；`?`/`#` 属 SQLite URI 语义，
        // 打不开就如实报错，不假装支持。
        let db = dir.path().join("张三 has space").join("cc-switch.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        seed_fake_ccswitch(&db);
        let opened = connect(&ImportSource::InstalledDb(db)).unwrap();
        let count: i64 = opened
            .conn()
            .query_row("SELECT COUNT(*) FROM prompts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn snapshot_dir_requires_manifest_format_and_db_sql() {
        let dir = tempdir().unwrap();
        assert!(connect(&ImportSource::SnapshotDir(dir.path().to_path_buf())).is_err());

        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"format":"cc-switch-webdav-sync"}"#,
        )
        .unwrap();
        assert!(
            connect(&ImportSource::SnapshotDir(dir.path().to_path_buf())).is_err(),
            "缺 db.sql 不能算有效快照"
        );

        std::fs::write(dir.path().join("db.sql"), "NOT SQL AT ALL").unwrap();
        assert!(connect(&ImportSource::SnapshotDir(dir.path().to_path_buf())).is_err());
    }

    #[test]
    fn snapshot_dir_materializes_db_sql() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), r#"{"format":"x"}"#).unwrap();
        assert!(
            connect(&ImportSource::SnapshotDir(dir.path().to_path_buf())).is_err(),
            "格式标识不对必须拒绝，不能瞎猜"
        );

        std::fs::write(
            dir.path().join("manifest.json"),
            r#"{"format":"cc-switch-webdav-sync"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("db.sql"), fake_ccswitch_sql()).unwrap();
        let opened = connect(&ImportSource::SnapshotDir(dir.path().to_path_buf())).unwrap();
        let count: i64 = opened
            .conn()
            .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
