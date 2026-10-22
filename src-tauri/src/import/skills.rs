use crate::domain::{is_known_agent, AssetSummary, AssetType};
use crate::import::safe_asset_id;
use crate::import::source::ImportError;
use crate::import::update_matrix;
use crate::index::{sha256_hex, Index};
use crate::store::{atomic::write_atomic, copy_dir, Store};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsReport {
    pub imported: usize,
    pub unchanged: usize,
    /// DB 有记录但 `skills/<directory>` 不存在：如实报告，不建空目录冒充导入成功。
    pub missing_dirs: Vec<String>,
    pub failed: Vec<String>,
}

struct SkillRow {
    id: String,
    name: String,
    directory: String,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    repo_branch: Option<String>,
    readme_url: Option<String>,
    content_hash: Option<String>,
    enabled: Vec<(String, bool)>,
}

/// 来源信息写在 skill 目录里的 `.meta.json`，不进 `agents/<agent>.json`（spec 4.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillMeta {
    repo_owner: Option<String>,
    repo_name: Option<String>,
    repo_branch: Option<String>,
    readme_url: Option<String>,
    source_directory: String,
    content_hash: Option<String>,
}

fn read_rows(conn: &Connection) -> Result<Vec<SkillRow>, ImportError> {
    let mut stmt = conn.prepare("PRAGMA table_info(skills)")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .collect::<Vec<_>>();
    let enabled_columns: Vec<String> = columns
        .iter()
        .filter(|name| name.starts_with("enabled_"))
        .cloned()
        .collect();

    let fixed = [
        "id",
        "name",
        "directory",
        "repo_owner",
        "repo_name",
        "repo_branch",
        "readme_url",
        "content_hash",
    ];
    let sql = format!(
        "SELECT {} FROM skills ORDER BY id",
        fixed
            .iter()
            .map(|name| name.to_string())
            .chain(enabled_columns.clone())
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut out = Vec::new();
    for row in stmt
        .query_map([], |row| {
            let mut enabled = Vec::new();
            for (offset, column) in enabled_columns.iter().enumerate() {
                let on: i64 = row
                    .get::<_, Option<i64>>(fixed.len() + offset)?
                    .unwrap_or(0);
                enabled.push((column["enabled_".len()..].to_string(), on != 0));
            }
            Ok(SkillRow {
                id: row.get(0)?,
                name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                directory: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                repo_owner: row.get(3)?,
                repo_name: row.get(4)?,
                repo_branch: row.get(5)?,
                readme_url: row.get(6)?,
                content_hash: row.get(7)?,
                enabled,
            })
        })?
        .flatten()
    {
        out.push(row);
    }
    Ok(out)
}

fn read_meta(target: &Path) -> Option<SkillMeta> {
    let text = std::fs::read_to_string(target.join(".meta.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn run(
    store: &Store,
    index: &mut Index,
    conn: &Connection,
    skills_root: &Path,
) -> Result<SkillsReport, ImportError> {
    let mut report = SkillsReport::default();

    for row in read_rows(conn)? {
        if row.directory.trim().is_empty() {
            report.failed.push(row.id);
            continue;
        }
        let source = skills_root.join(&row.directory);
        if !source.join("SKILL.md").is_file() {
            report.missing_dirs.push(row.directory);
            continue;
        }
        let id = safe_asset_id(&row.id);
        let target = store.entry_path(AssetType::Skill, &id)?;

        let already_there = target.join("SKILL.md").is_file()
            && read_meta(&target)
                .and_then(|meta| meta.content_hash)
                .as_deref()
                == row.content_hash.as_deref();
        if already_there {
            report.unchanged += 1;
        } else {
            copy_dir(&source, &target)?;
            let meta = SkillMeta {
                repo_owner: row.repo_owner.clone(),
                repo_name: row.repo_name.clone(),
                repo_branch: row.repo_branch.clone(),
                readme_url: row.readme_url.clone(),
                source_directory: row.directory.clone(),
                content_hash: row.content_hash.clone(),
            };
            write_atomic(
                &target.join(".meta.json"),
                &serde_json::to_vec_pretty(&meta)?,
            )?;
            report.imported += 1;
        }
        let skill_bytes = std::fs::read(target.join("SKILL.md"))?;
        index.upsert(&AssetSummary {
            asset_type: AssetType::Skill,
            id: id.clone(),
            name: if row.name.trim().is_empty() {
                id.clone()
            } else {
                row.name.clone()
            },
            updated_at: chrono::Utc::now().timestamp_millis(),
            hash: sha256_hex(&skill_bytes),
        })?;

        for (app, on) in &row.enabled {
            if !*on || !is_known_agent(app) {
                continue;
            }
            let id_for_matrix = id.clone();
            update_matrix(store, app, move |matrix| {
                matrix.enable(AssetType::Skill, &id_for_matrix);
            })?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enablement::AgentEnablement;
    use crate::import::source::fake_ccswitch_sql;
    use crate::index::Index;
    use crate::store::Store;
    use rusqlite::Connection;
    use tempfile::tempdir;

    /// cc-switch 的 skill id 形如 `owner/repo/skill` 或 `local:name`，都不能直接当目录名。
    fn seeded(rows: &[(&str, &str, &str, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(fake_ccswitch_sql()).unwrap();
        conn.execute("DELETE FROM skills", []).unwrap();
        for (id, name, directory, enabled_claude) in rows {
            conn.execute(
                "INSERT INTO skills (id,name,directory,repo_owner,repo_name,repo_branch,readme_url,installed_at,content_hash,enabled_claude)
                 VALUES (?1,?2,?3,'acme','demo','main','https://readme',1700000000,'hash',?4)",
                rusqlite::params![id, name, directory, enabled_claude],
            )
            .unwrap();
        }
        conn
    }

    fn source_skill(skills_root: &std::path::Path, directory: &str) {
        let dir = skills_root.join(directory);
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: d\n---\n\n正文\n",
        )
        .unwrap();
        std::fs::write(dir.join("references/note.md"), "补充\n").unwrap();
    }

    #[test]
    fn copying_does_not_move_the_source_and_keeps_the_provenance() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let skills_root = dir.path().join("cc-switch/skills");
        source_skill(&skills_root, "demo");
        let conn = seeded(&[("acme/demo/skill", "demo", "demo", 1)]);

        let report = run(&store, &mut index, &conn, &skills_root).unwrap();
        assert_eq!(report.imported, 1);

        let id = "acme-demo-skill";
        assert_eq!(
            store.list_ids(AssetType::Skill).unwrap(),
            vec![id.to_string()]
        );
        assert!(
            skills_root.join("demo/SKILL.md").exists(),
            "复制不移动：cc-switch 的目录必须还在"
        );
        let copied = store.asset_dir(AssetType::Skill).join(id);
        assert!(copied.join("SKILL.md").is_file());
        assert_eq!(
            std::fs::read_to_string(copied.join("references/note.md")).unwrap(),
            "补充\n",
            "子目录要整份带上"
        );

        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(copied.join(".meta.json")).unwrap())
                .unwrap();
        assert_eq!(meta["repoOwner"], "acme");
        assert_eq!(meta["repoBranch"], "main");
        assert_eq!(meta["sourceDirectory"], "demo");
    }

    #[test]
    fn a_missing_source_directory_is_reported_not_created() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let skills_root = dir.path().join("cc-switch/skills");
        std::fs::create_dir_all(&skills_root).unwrap();
        let conn = seeded(&[("acme/gone/skill", "gone", "gone", 1)]);

        let report = run(&store, &mut index, &conn, &skills_root).unwrap();
        assert_eq!(report.missing_dirs, vec!["gone".to_string()]);
        assert!(
            store.list_ids(AssetType::Skill).unwrap().is_empty(),
            "不能建空目录冒充导入成功"
        );
        assert!(!store
            .asset_dir(AssetType::Skill)
            .join("acme-gone-skill")
            .exists());
    }

    #[test]
    fn importing_twice_changes_nothing() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let skills_root = dir.path().join("cc-switch/skills");
        source_skill(&skills_root, "demo");
        let conn = seeded(&[("local:demo", "demo", "demo", 1)]);

        let first = run(&store, &mut index, &conn, &skills_root).unwrap();
        let second = run(&store, &mut index, &conn, &skills_root).unwrap();
        assert_eq!(first.imported, 1);
        assert_eq!(second.imported, 0);
        assert_eq!(second.unchanged, 1);
        assert_eq!(
            store.list_ids(AssetType::Skill).unwrap(),
            vec!["local-demo".to_string()]
        );

        let enablement: AgentEnablement =
            serde_json::from_slice(&std::fs::read(store.agent_config_path("claude")).unwrap())
                .unwrap();
        assert_eq!(
            enablement.enabled_for(AssetType::Skill),
            vec!["local-demo".to_string()],
            "两次导入不能撑出重复项"
        );
    }
}
