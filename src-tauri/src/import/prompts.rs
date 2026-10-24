use crate::domain::prompt::{render_prompt, Prompt, PromptMetadata};
use crate::domain::{is_known_agent, AssetSummary, AssetType};
use crate::import::slug;
use crate::import::source::ImportError;
use crate::import::update_matrix;
use crate::index::{sha256_hex, Index};
use crate::store::Store;
use rusqlite::Connection;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptsReport {
    pub imported: usize,
    pub unchanged: usize,
    pub failed: Vec<String>,
    /// 某个 agent 同时有多条启用行：只能挑一条，其余如实报告。
    pub ambiguous_active: Vec<String>,
}

struct PromptRow {
    id: String,
    app_type: String,
    name: String,
    content: String,
    description: Option<String>,
    enabled: bool,
    updated_at: i64,
}

/// 真源文件名：`<app>-<名字 slug>`；名字是纯中文等无 ascii 字符时退回用行 ID。
pub fn prompt_id(app_type: &str, name: &str, row_id: &str) -> String {
    let slug = slug(name)
        .or_else(|| slug(row_id))
        .unwrap_or_else(|| "prompt".to_string());
    format!("{app_type}-{slug}")
}

fn read_rows(conn: &Connection) -> Result<Vec<PromptRow>, ImportError> {
    let mut stmt = conn.prepare(
        "SELECT id, app_type, name, content, description, enabled, updated_at FROM prompts ORDER BY id, app_type",
    )?;
    let mut out = Vec::new();
    for row in stmt
        .query_map([], |row| {
            Ok(PromptRow {
                id: row.get(0)?,
                app_type: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                content: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                description: row.get(4)?,
                enabled: row.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                updated_at: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            })
        })?
        .flatten()
    {
        out.push(row);
    }
    Ok(out)
}

pub fn run(
    store: &Store,
    index: &mut Index,
    conn: &Connection,
) -> Result<PromptsReport, ImportError> {
    let mut report = PromptsReport::default();
    let mut best: BTreeMap<String, (i64, String)> = BTreeMap::new();
    let mut enabled_count: BTreeMap<String, usize> = BTreeMap::new();

    for row in read_rows(conn)? {
        if !is_known_agent(&row.app_type) {
            report.failed.push(row.id);
            continue;
        }
        let id = prompt_id(&row.app_type, &row.name, &row.id);
        let text = render_prompt(&Prompt {
            id: id.clone(),
            meta: PromptMetadata {
                title: Some(row.name.clone()),
                description: row.description.clone(),
            },
            body: row.content.clone(),
        });
        let bytes = text.as_bytes();
        if store
            .read(AssetType::Prompt, &id)
            .map(|existing| existing == bytes)
            .unwrap_or(false)
        {
            report.unchanged += 1;
        } else {
            if store.read(AssetType::Prompt, &id).is_ok() {
                store.backup(AssetType::Prompt, &id)?;
            }
            store.write(AssetType::Prompt, &id, bytes)?;
            report.imported += 1;
        }
        index.upsert(&AssetSummary {
            asset_type: AssetType::Prompt,
            id: id.clone(),
            name: row.name.clone(),
            updated_at: chrono::Utc::now().timestamp_millis(),
            hash: sha256_hex(bytes),
            description: None,
            source: None,
            mtime: None,
            transport: None,
            size: None,
            origin: None,
        })?;

        if row.enabled {
            *enabled_count.entry(row.app_type.clone()).or_default() += 1;
            let replace = match best.get(&row.app_type) {
                Some((newest, _)) => row.updated_at > *newest,
                None => true,
            };
            if replace {
                best.insert(row.app_type.clone(), (row.updated_at, id));
            }
        }
    }

    for (app, (_, id)) in best {
        update_matrix(store, &app, move |matrix| {
            matrix.active_prompt = Some(id);
        })?;
    }
    report.ambiguous_active = enabled_count
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(app, _)| app)
        .collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enablement::AgentEnablement;
    use crate::domain::prompt::parse_prompt;
    use crate::import::source::fake_ccswitch_sql;
    use crate::index::Index;
    use crate::store::Store;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn seeded(rows: &[(&str, &str, &str, &str, i64, i64)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(fake_ccswitch_sql()).unwrap();
        conn.execute("DELETE FROM prompts", []).unwrap();
        for (id, app, name, content, enabled, updated_at) in rows {
            conn.execute(
                "INSERT INTO prompts (id,app_type,name,content,enabled,updated_at) VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![id, app, name, content, enabled, updated_at],
            )
            .unwrap();
        }
        conn
    }

    fn active_prompt(store: &Store, agent_id: &str) -> Option<String> {
        let bytes = std::fs::read(store.agent_config_path(agent_id)).ok()?;
        serde_json::from_slice::<AgentEnablement>(&bytes)
            .ok()?
            .active_prompt
    }

    #[test]
    fn rows_become_markdown_files_with_frontmatter() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let conn = seeded(&[("p1", "claude", "team", "正文内容", 1, 100)]);

        let report = run(&store, &mut index, &conn).unwrap();
        assert_eq!(report.imported, 1);

        let ids = store.list_ids(AssetType::Prompt).unwrap();
        assert_eq!(ids, vec!["claude-team".to_string()]);
        let text =
            String::from_utf8(store.read(AssetType::Prompt, "claude-team").unwrap()).unwrap();
        let parsed = parse_prompt("claude-team", &text).unwrap();
        assert_eq!(parsed.meta.title.as_deref(), Some("team"));
        assert_eq!(parsed.body, "正文内容");
    }

    #[test]
    fn the_newest_enabled_row_wins_and_ambiguity_is_reported() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let conn = seeded(&[
            ("p1", "claude", "old", "旧", 1, 100),
            ("p2", "claude", "new", "新", 1, 200),
            ("p3", "codex", "off", "没启用", 0, 50),
        ]);

        let report = run(&store, &mut index, &conn).unwrap();
        assert_eq!(report.imported, 3, "未启用的行也是资产库里的条目");
        assert_eq!(report.ambiguous_active, vec!["claude".to_string()]);
        assert_eq!(
            active_prompt(&store, "claude").as_deref(),
            Some("claude-new"),
            "同时只能有一个激活项，取 updated_at 最新的"
        );
        assert_eq!(
            active_prompt(&store, "codex"),
            None,
            "没有启用行就不该有激活项"
        );
    }

    #[test]
    fn non_ascii_names_get_a_safe_and_stable_id() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let conn = seeded(&[("auto-imported-1", "claude", "团队提示词", "正文", 1, 100)]);

        run(&store, &mut index, &conn).unwrap();
        let ids = store.list_ids(AssetType::Prompt).unwrap();
        assert_eq!(ids.len(), 1);
        let id = &ids[0];
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "ID 会直接当文件名：{id}"
        );
        assert!(id.starts_with("claude-"), "{id}");

        // 再导入一次不能因为 ID 算得不稳定而多出一条
        let second = run(&store, &mut index, &conn).unwrap();
        assert_eq!(second.imported, 0);
        assert_eq!(second.unchanged, 1);
        assert_eq!(store.list_ids(AssetType::Prompt).unwrap(), vec![id.clone()]);
    }

    #[test]
    fn a_bad_row_fails_only_itself() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let conn = seeded(&[
            ("ok", "claude", "good", "正文", 1, 100),
            ("bad", "unknownapp", "x", "正文", 1, 100),
        ]);

        let report = run(&store, &mut index, &conn).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(
            report.failed,
            vec!["bad".to_string()],
            "未知 app_type 不能瞎猜落点"
        );
    }
}
