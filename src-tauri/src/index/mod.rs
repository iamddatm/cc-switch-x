mod schema;

use crate::domain::{display_name, AssetState, AssetSummary, AssetType};
use crate::store::Store;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("数据库错误：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("存储错误：{0}")]
    Store(#[from] crate::store::StoreError),
    #[error("资产解析失败：{0}")]
    Domain(#[from] crate::domain::DomainError),
}

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(schema::SCHEMA)?;
        Ok(Self { conn })
    }

    /// 全量重建：清空 assets 后按真源文件重新写入，因此删掉/改名的资产不会留下旧行。
    pub fn rebuild(&mut self, store: &Store) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM assets", [])?;
        for asset_type in AssetType::ALL {
            for id in store.list_ids(asset_type)? {
                let bytes = match asset_type {
                    AssetType::Skill => {
                        let Ok(dir) = store.entry_path(asset_type, &id) else {
                            continue;
                        };
                        std::fs::read(dir.join("SKILL.md")).unwrap_or_default()
                    }
                    _ => match store.read(asset_type, &id) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    },
                };
                let name = match display_name(asset_type, &id, &bytes) {
                    Ok(name) => name,
                    Err(_) => continue,
                };
                let hash = sha256_hex(&bytes);
                tx.execute(
                    "INSERT INTO assets (asset_type,id,name,updated_at,hash) VALUES (?1,?2,?3,?4,?5)",
                    params![asset_type_key(asset_type), id, name, now_millis(), hash],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list(&self, asset_type: Option<AssetType>) -> Result<Vec<AssetSummary>, IndexError> {
        let mut rows = Vec::new();
        match asset_type {
            Some(t) => {
                let mut stmt = self.conn.prepare(
                    "SELECT asset_type,id,name,updated_at,hash FROM assets WHERE asset_type=?1 ORDER BY name",
                )?;
                let iter = stmt.query_map([asset_type_key(t)], row_to_summary)?;
                for row in iter {
                    rows.push(row?);
                }
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT asset_type,id,name,updated_at,hash FROM assets ORDER BY asset_type,name",
                )?;
                let iter = stmt.query_map([], row_to_summary)?;
                for row in iter {
                    rows.push(row?);
                }
            }
        }
        Ok(rows)
    }

    pub fn search(&self, query: &str) -> Result<Vec<AssetSummary>, IndexError> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT asset_type,id,name,updated_at,hash FROM assets
             WHERE name LIKE ?1 OR id LIKE ?1 ORDER BY name",
        )?;
        let iter = stmt.query_map([pattern], row_to_summary)?;
        let mut rows = Vec::new();
        for row in iter {
            rows.push(row?);
        }
        Ok(rows)
    }

    pub fn upsert(&mut self, summary: &AssetSummary) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO assets (asset_type,id,name,updated_at,hash) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(asset_type,id) DO UPDATE SET name=?3,updated_at=?4,hash=?5",
            params![
                asset_type_key(summary.asset_type),
                summary.id,
                summary.name,
                summary.updated_at,
                summary.hash
            ],
        )?;
        Ok(())
    }

    pub fn remove(&mut self, asset_type: AssetType, id: &str) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM assets WHERE asset_type=?1 AND id=?2",
            params![asset_type_key(asset_type), id],
        )?;
        Ok(())
    }

    pub fn set_agent_state(&mut self, row: &AgentStateRow) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO agent_state (agent_id,asset_type,asset_id,state,last_error,applied_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(agent_id,asset_type,asset_id)
             DO UPDATE SET state=?4,last_error=?5,applied_at=?6",
            params![
                row.agent_id,
                asset_type_key(row.asset_type),
                row.asset_id,
                row.state.as_str(),
                row.last_error,
                row.applied_at
            ],
        )?;
        Ok(())
    }

    pub fn agent_states(&self) -> Result<Vec<AgentStateRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_id,asset_type,asset_id,state,last_error,applied_at FROM agent_state ORDER BY agent_id,asset_type,asset_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let type_key: String = row.get(1)?;
            let state_key: String = row.get(3)?;
            let state = AssetState::from_db(&state_key).ok_or_else(|| {
                rusqlite::Error::InvalidColumnName(format!("未知同步状态：{state_key}"))
            })?;
            Ok(AgentStateRow {
                agent_id: row.get(0)?,
                asset_type: asset_type_from_key(&type_key)?,
                asset_id: row.get(2)?,
                state,
                last_error: row.get(4)?,
                applied_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn record_run(
        &mut self,
        dry_run: bool,
        plan_json: &str,
        result: &str,
        backup_dir: &Path,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO runs (started_at,dry_run,plan_json,result,backup_dir,reverted) VALUES (?1,?2,?3,?4,?5,0)",
            params![
                now_millis(),
                dry_run as i64,
                plan_json,
                result,
                backup_dir.to_string_lossy()
            ],
        )?;
        Ok(())
    }

    /// 最近一次还没被回滚的执行记录。
    pub fn last_unreverted_run(&self) -> Result<Option<RunRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,backup_dir,result FROM runs WHERE reverted=0 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(RunRow {
                id: row.get(0)?,
                backup_dir: PathBuf::from(row.get::<_, String>(1)?),
                result: row.get(2)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    pub fn mark_run_reverted(&mut self, run_id: i64) -> Result<(), IndexError> {
        self.conn
            .execute("UPDATE runs SET reverted=1 WHERE id=?1", [run_id])?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStateRow {
    pub agent_id: String,
    pub asset_type: AssetType,
    pub asset_id: String,
    pub state: AssetState,
    pub last_error: Option<String>,
    pub applied_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: i64,
    pub backup_dir: PathBuf,
    pub result: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn asset_type_key(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Mcp => "mcp",
        AssetType::Skill => "skill",
        AssetType::Prompt => "prompt",
        AssetType::Rule => "rule",
        AssetType::Subagent => "subagent",
        AssetType::Hook => "hook",
    }
}

fn asset_type_from_key(key: &str) -> Result<AssetType, rusqlite::Error> {
    match key {
        "mcp" => Ok(AssetType::Mcp),
        "skill" => Ok(AssetType::Skill),
        "prompt" => Ok(AssetType::Prompt),
        "rule" => Ok(AssetType::Rule),
        "subagent" => Ok(AssetType::Subagent),
        "hook" => Ok(AssetType::Hook),
        other => Err(rusqlite::Error::InvalidColumnName(format!(
            "索引里出现未知资产类型：{other}"
        ))),
    }
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetSummary> {
    Ok(AssetSummary {
        asset_type: asset_type_from_key(&row.get::<_, String>(0)?)?,
        id: row.get(1)?,
        name: row.get(2)?,
        updated_at: row.get(3)?,
        hash: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::store::Store;
    use tempfile::tempdir;

    #[test]
    fn rebuild_indexes_files_and_removes_deleted_rows() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        store
            .write(
                AssetType::Mcp,
                "pw",
                br#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
            )
            .unwrap();
        store
            .write(
                AssetType::Mcp,
                "ctx",
                br#"{"id":"ctx","name":"Context7","transport":"http","url":"https://x"}"#,
            )
            .unwrap();

        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        let all = index.list(Some(AssetType::Mcp)).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|a| a.name == "Playwright"));

        store.delete(AssetType::Mcp, "pw").unwrap();
        index.rebuild(&store).unwrap();
        let all = index.list(Some(AssetType::Mcp)).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "ctx");
    }

    #[test]
    fn search_matches_name_and_id() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        store
            .write(
                AssetType::Mcp,
                "pw",
                br#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
            )
            .unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        assert_eq!(index.search("play").unwrap().len(), 1);
        assert_eq!(index.search("pw").unwrap().len(), 1);
    }

    #[test]
    fn rebuild_indexes_skill_directories() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let skill_dir = store.asset_dir(AssetType::Skill).join("lark-wiki");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Lark Wiki\n").unwrap();

        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        let skills = index.list(Some(AssetType::Skill)).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "lark-wiki");
    }
}
