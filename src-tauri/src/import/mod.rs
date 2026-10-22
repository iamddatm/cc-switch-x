pub mod mcp;
pub mod prompts;
pub mod skills;
pub mod source;

use crate::domain::enablement::AgentEnablement;
use crate::domain::is_known_agent;
use crate::domain::AssetType;
use crate::store::Store;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
pub use source::{connect, ImportDb, ImportError, ImportSource, SNAPSHOT_FORMAT};
use std::collections::BTreeSet;
use std::path::Path;

/// 导入前的预览：数量、被忽略的启用列、缺失的 skill 目录、与现有真源的冲突项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub mcp_count: i64,
    pub prompts_count: i64,
    pub skills_count: i64,
    pub ignored_columns: Vec<String>,
    pub missing_skill_dirs: Vec<String>,
    /// 形如 `mcp/pw`、`skill/demo`：真源里已有同 ID 条目，导入时按冲突策略处理。
    pub conflicts: Vec<String>,
}

pub fn preview(
    store: &Store,
    source: &ImportSource,
    skills_root: &Path,
) -> Result<ImportPreview, ImportError> {
    let db = connect(source)?;
    let conn = db.conn();

    let mut ignored: BTreeSet<String> = BTreeSet::new();
    for table in ["mcp_servers", "skills"] {
        for app in enabled_apps(conn, table)? {
            if !is_known_agent(&app) {
                ignored.insert(app);
            }
        }
    }

    let mut missing_skill_dirs = Vec::new();
    let mut stmt = conn.prepare("SELECT directory FROM skills ORDER BY id")?;
    for directory in stmt.query_map([], |row| row.get::<_, String>(0))?.flatten() {
        if !skills_root.join(&directory).join("SKILL.md").is_file() {
            missing_skill_dirs.push(directory);
        }
    }

    let mut conflicts = Vec::new();
    let existing_mcp: BTreeSet<String> = store.list_ids(AssetType::Mcp)?.into_iter().collect();
    let existing_skills: BTreeSet<String> = store.list_ids(AssetType::Skill)?.into_iter().collect();
    for id in list_column(conn, "mcp_servers", "id")? {
        if existing_mcp.contains(&id) {
            conflicts.push(format!("mcp/{id}"));
        }
    }
    for id in list_column(conn, "skills", "id")? {
        if existing_skills.contains(&id) {
            conflicts.push(format!("skill/{id}"));
        }
    }

    Ok(ImportPreview {
        mcp_count: count(conn, "mcp_servers")?,
        prompts_count: count(conn, "prompts")?,
        skills_count: count(conn, "skills")?,
        ignored_columns: ignored.into_iter().collect(),
        missing_skill_dirs,
        conflicts,
    })
}

fn count(conn: &Connection, table: &str) -> Result<i64, ImportError> {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })
    .map_err(|error| match error {
        rusqlite::Error::SqliteFailure(_, Some(ref message))
            if message.contains("no such table") =>
        {
            ImportError::MissingTable(table.to_string())
        }
        other => ImportError::Sql(other),
    })
}

/// 表里 `enabled_<app>` 列对应的 app 名；cc-switch 加新 agent 时我们只会忽略并报告。
fn enabled_apps(conn: &Connection, table: &str) -> Result<Vec<String>, ImportError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .collect::<Vec<_>>();
    Ok(columns
        .into_iter()
        .filter_map(|name| name.strip_prefix("enabled_").map(|app| app.to_string()))
        .collect())
}

fn list_column(conn: &Connection, table: &str, column: &str) -> Result<Vec<String>, ImportError> {
    let mut stmt = conn.prepare(&format!("SELECT {column} FROM {table} ORDER BY {column}"))?;
    let mut out = Vec::new();
    for value in stmt
        .query_map([], |row| row.get::<_, Option<String>>(0))?
        .flatten()
    {
        if let Some(value) = value {
            out.push(value);
        }
    }
    Ok(out)
}

/// 把外部 ID 变成能直接当文件/目录名的 slug：只留 `[A-Za-z0-9._-]`，其余换成 `-`。
/// cc-switch 的 skill id 形如 `owner/repo/skill`、prompt 名称可能是纯中文，都不能原样落盘。
pub fn slug(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= 128 {
        return Some(trimmed);
    }
    // 超长时保留前缀 + 原文哈希，既满足 store 的 128 上限又保持稳定唯一
    let digest = crate::index::sha256_hex(raw.as_bytes());
    let mut prefix = trimmed[..100].to_string();
    while prefix.ends_with('-') {
        prefix.pop();
    }
    Some(format!("{prefix}-{}", &digest[..12]))
}

pub fn safe_asset_id(raw: &str) -> String {
    slug(raw).unwrap_or_else(|| "asset".to_string())
}

/// 读—改—写某个 agent 的启用矩阵：只动调用方指定的部分，别的一律保留。
pub(crate) fn update_matrix<F>(store: &Store, agent_id: &str, change: F) -> Result<(), ImportError>
where
    F: FnOnce(&mut AgentEnablement),
{
    let path = store.agent_config_path(agent_id);
    let mut matrix = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        AgentEnablement::default()
    };
    change(&mut matrix);
    crate::store::atomic::write_atomic(&path, &serde_json::to_vec_pretty(&matrix)?)?;
    Ok(())
}

/// 导入完成后写 `state/import.json`（spec 3.1 第 7 条）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRecord {
    pub source: String,
    pub finished_at: i64,
    pub mcp_imported: usize,
    pub prompts_imported: usize,
    pub skills_imported: usize,
    pub failed: Vec<String>,
    pub missing_skill_dirs: Vec<String>,
    pub ignored_columns: Vec<String>,
}

pub fn record_import(store: &Store, record: &ImportRecord) -> Result<(), ImportError> {
    let path = store.root().join("state").join("import.json");
    crate::store::atomic::write_atomic(&path, &serde_json::to_vec_pretty(record)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::source::seed_fake_ccswitch;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn setup() -> (
        tempfile::TempDir,
        Store,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let db = dir.path().join("cc-switch.db");
        seed_fake_ccswitch(&db);
        let skills_root = dir.path().join("skills");
        std::fs::create_dir_all(skills_root.join("demo")).unwrap();
        std::fs::write(skills_root.join("demo/SKILL.md"), "# demo\n").unwrap();
        (dir, store, db, skills_root)
    }

    #[test]
    fn preview_counts_match_the_source_db() {
        let (_dir, store, db, skills_root) = setup();
        let preview = preview(&store, &source_of(&db), &skills_root).unwrap();
        assert_eq!(preview.mcp_count, 1);
        assert_eq!(preview.prompts_count, 1);
        assert_eq!(preview.skills_count, 1);
        assert!(preview.missing_skill_dirs.is_empty());
        assert!(preview.conflicts.is_empty(), "空真源不该有冲突");
    }

    #[test]
    fn unknown_enabled_columns_are_reported_not_fatal() {
        let (_dir, store, db, skills_root) = setup();
        let preview = preview(&store, &source_of(&db), &skills_root).unwrap();
        assert!(
            preview.ignored_columns.contains(&"quantum".to_string()),
            "未知 enabled_* 列要报告：{:?}",
            preview.ignored_columns
        );
        // 我们注册表里没有的 cc-switch app（opencode/hermes/mcode/pi/grokbuild）同样只报告不报错
        assert!(preview.ignored_columns.contains(&"opencode".to_string()));
        assert_eq!(preview.mcp_count, 1, "报告未知列不影响其余导入");
    }

    #[test]
    fn missing_skill_directories_are_listed() {
        let (_dir, store, db, skills_root) = setup();
        Connection::open(&db)
            .unwrap()
            .execute(
                "INSERT INTO skills (id,name,directory,enabled_claude) VALUES ('acme/gone/gone','gone','gone',1)",
                [],
            )
            .unwrap();
        let preview = preview(&store, &source_of(&db), &skills_root).unwrap();
        assert_eq!(preview.missing_skill_dirs, vec!["gone".to_string()]);
        assert_eq!(preview.skills_count, 2);
    }

    #[test]
    fn conflicts_list_assets_already_in_the_store() {
        let (_dir, store, db, skills_root) = setup();
        store
            .write(crate::domain::AssetType::Mcp, "pw", b"{}")
            .unwrap();
        let preview = preview(&store, &source_of(&db), &skills_root).unwrap();
        assert_eq!(preview.conflicts, vec!["mcp/pw".to_string()]);
    }

    #[test]
    fn safe_asset_id_handles_slashes_chinese_and_overlong_names() {
        assert_eq!(safe_asset_id("acme/demo/skill"), "acme-demo-skill");
        assert_eq!(safe_asset_id("local:demo"), "local-demo");
        assert_eq!(safe_asset_id("团队提示词"), "asset");
        let long = safe_asset_id(&"x".repeat(400));
        assert!(long.len() <= 128, "{}", long.len());
        assert_eq!(
            long,
            safe_asset_id(&"x".repeat(400)),
            "同一个输入必须得到同一个 ID"
        );
    }

    #[test]
    fn import_record_is_written_under_state() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        record_import(
            &store,
            &ImportRecord {
                source: "installed-db".into(),
                finished_at: 1,
                mcp_imported: 5,
                prompts_imported: 3,
                skills_imported: 81,
                failed: vec!["bad".into()],
                missing_skill_dirs: vec!["gone".into()],
                ignored_columns: vec!["quantum".into()],
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("store/state/import.json")).unwrap();
        assert!(text.contains("\"mcpImported\": 5"), "{text}");
        assert!(text.contains("\"ignoredColumns\""), "{text}");
    }

    fn source_of(db: &std::path::Path) -> ImportSource {
        ImportSource::InstalledDb(db.to_path_buf())
    }
}
