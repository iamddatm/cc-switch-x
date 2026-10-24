use crate::domain::skill_meta::{read_meta, write_meta, SkillMeta, SkillSourceKind};
use crate::domain::{describe, display_name, parse_and_validate, AssetSummary, AssetType};
use crate::error::AppError;
use crate::index::{sha256_hex, Index};
use crate::store::{copy_dir, Store};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct AssetService;

impl AssetService {
    /// 列表先查索引，再读磁盘补齐描述/来源/mtime。索引仍是可随时重建的派生缓存，
    /// 不为了几个展示字段改表结构。
    pub fn list(
        store: &Store,
        index: &Index,
        asset_type: Option<AssetType>,
    ) -> Result<Vec<AssetSummary>, AppError> {
        let mut rows = index.list(asset_type)?;
        for row in &mut rows {
            enrich(store, row);
        }
        Ok(rows)
    }

    /// 读资产的文本。skill 在真源里是目录，读的是它下面的 `SKILL.md`。
    pub fn read(store: &Store, asset_type: AssetType, id: &str) -> Result<String, AppError> {
        let path = asset_file_path(store, asset_type, id)?;
        let bytes = std::fs::read(&path)?;
        String::from_utf8(bytes)
            .map_err(|_| AppError::InvalidInput(format!("{id} 不是 UTF-8 文件")))
    }

    /// 先校验再落盘：校验失败时真源文件保持原样。
    pub fn write(
        store: &Store,
        index: &mut Index,
        asset_type: AssetType,
        id: &str,
        content: &str,
    ) -> Result<AssetSummary, AppError> {
        let bytes = content.as_bytes();
        parse_and_validate(asset_type, id, bytes)?;
        store.backup(asset_type, id)?;
        store.write(asset_type, id, bytes)?;
        let name = display_name(asset_type, id, bytes)?;
        let mut summary = AssetSummary {
            asset_type,
            id: id.to_string(),
            name,
            updated_at: chrono::Utc::now().timestamp_millis(),
            hash: sha256_hex(bytes),
            description: None,
            source: None,
            mtime: None,
            transport: None,
            size: None,
            origin: None,
        };
        enrich(store, &mut summary);
        index.upsert(&summary)?;
        Ok(summary)
    }

    pub fn delete(
        store: &Store,
        index: &mut Index,
        asset_type: AssetType,
        id: &str,
    ) -> Result<bool, AppError> {
        let deleted = store.delete(asset_type, id)?;
        if deleted {
            index.remove(asset_type, id)?;
        }
        Ok(deleted)
    }

    /// 从本地目录导入一个 skill：目录名就是资产 ID，必须含 `SKILL.md`。
    pub fn import_skill_dir(
        store: &Store,
        index: &mut Index,
        source: &Path,
    ) -> Result<AssetSummary, AppError> {
        if !source.is_dir() {
            return Err(AppError::InvalidInput("选择的不是目录".into()));
        }
        if !source.join("SKILL.md").is_file() {
            return Err(AppError::InvalidInput(
                "选择的目录里没有 SKILL.md，不能作为 Skill 导入".into(),
            ));
        }
        let id = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| AppError::InvalidInput("无法读取目录名作为 Skill ID".into()))?
            .to_string();
        let target = store.entry_path(AssetType::Skill, &id)?;
        if target.exists() {
            return Err(AppError::InvalidInput(format!(
                "真源里已有同名 skill：{id}"
            )));
        }
        copy_dir(source, &target)?;
        let now = chrono::Utc::now().timestamp_millis();
        write_meta(
            &target,
            &SkillMeta {
                source_kind: Some(SkillSourceKind::Local),
                source_url: Some(source.display().to_string()),
                installed_at: Some(now),
                updated_at: Some(now),
                ..Default::default()
            },
        )?;
        let bytes = std::fs::read(target.join("SKILL.md"))?;
        let mut summary = AssetSummary {
            asset_type: AssetType::Skill,
            id: id.clone(),
            name: id,
            updated_at: now,
            hash: sha256_hex(&bytes),
            description: None,
            source: None,
            mtime: None,
            transport: None,
            size: None,
            origin: None,
        };
        enrich(store, &mut summary);
        index.upsert(&summary)?;
        Ok(summary)
    }
}

fn asset_file_path(
    store: &Store,
    asset_type: AssetType,
    id: &str,
) -> Result<PathBuf, crate::store::StoreError> {
    let path = store.entry_path(asset_type, id)?;
    Ok(if asset_type == AssetType::Skill {
        path.join("SKILL.md")
    } else {
        path
    })
}

fn enrich(store: &Store, summary: &mut AssetSummary) {
    let Ok(path) = asset_file_path(store, summary.asset_type, &summary.id) else {
        return;
    };
    if let Ok(metadata) = std::fs::metadata(&path) {
        summary.size = Some(metadata.len() as i64);
        summary.mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as i64);
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let details = describe(summary.asset_type, &summary.id, &bytes);
    if let Some(name) = details.name {
        if !name.trim().is_empty() {
            summary.name = name;
        }
    }
    summary.description = details.description;
    summary.transport = details.transport;
    if summary.asset_type == AssetType::Skill {
        let meta = store
            .entry_path(AssetType::Skill, &summary.id)
            .ok()
            .and_then(|dir| read_meta(&dir));
        summary.source = meta.as_ref().and_then(SkillMeta::repo_slug);
        summary.origin = meta.as_ref().map(SkillMeta::origin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::index::Index;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, Store, Index) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        (dir, store, index)
    }

    #[test]
    fn write_then_list_returns_summary() {
        let (_dir, store, mut index) = fixture();
        let summary = AssetService::write(
            &store,
            &mut index,
            AssetType::Mcp,
            "pw",
            r#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx","args":["-y","pw"]}"#,
        )
        .unwrap();
        assert_eq!(summary.name, "Playwright");
        assert_eq!(summary.transport.as_deref(), Some("stdio"));
        assert_eq!(summary.description.as_deref(), Some("npx -y pw"));
        let rows = AssetService::list(&store, &index, Some(AssetType::Mcp)).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].mtime.is_some());
    }

    #[test]
    fn skill_listing_uses_frontmatter_and_source_metadata() {
        let (dir, store, mut index) = fixture();
        let skill_dir = store.asset_dir(AssetType::Skill).join("lark");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Lark Wiki\ndescription: 查知识库\n---\n\n正文\n",
        )
        .unwrap();
        std::fs::write(
            skill_dir.join(".meta.json"),
            r#"{"repoOwner":"acme","repoName":"skills"}"#,
        )
        .unwrap();
        index.rebuild(&store).unwrap();

        let rows = AssetService::list(&store, &index, Some(AssetType::Skill)).unwrap();
        assert_eq!(rows[0].name, "Lark Wiki");
        assert_eq!(rows[0].description.as_deref(), Some("查知识库"));
        assert_eq!(rows[0].source.as_deref(), Some("acme/skills"));
        assert!(rows[0].mtime.is_some());
        let _ = dir;
    }

    #[test]
    fn import_skill_dir_copies_the_whole_directory() {
        let (dir, store, mut index) = fixture();
        let source = dir.path().join("incoming").join("my-skill");
        std::fs::create_dir_all(source.join("references")).unwrap();
        std::fs::write(source.join("SKILL.md"), "---\nname: My Skill\n---\n正文\n").unwrap();
        std::fs::write(source.join("references/note.md"), "补充\n").unwrap();

        let summary = AssetService::import_skill_dir(&store, &mut index, &source).unwrap();
        assert_eq!(summary.id, "my-skill");
        assert_eq!(summary.name, "My Skill");
        assert!(store
            .asset_dir(AssetType::Skill)
            .join("my-skill/references/note.md")
            .is_file());
    }

    #[test]
    fn invalid_content_is_rejected_before_write() {
        let (_dir, store, mut index) = fixture();
        let result = AssetService::write(&store, &mut index, AssetType::Mcp, "pw", "{}");
        assert!(result.is_err());
        assert!(store.list_ids(AssetType::Mcp).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_file_and_index_row() {
        let (_dir, store, mut index) = fixture();
        AssetService::write(
            &store,
            &mut index,
            AssetType::Mcp,
            "pw",
            r#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
        )
        .unwrap();
        assert!(AssetService::delete(&store, &mut index, AssetType::Mcp, "pw").unwrap());
        assert!(AssetService::list(&store, &index, Some(AssetType::Mcp))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn update_backs_up_previous_content_before_overwriting() {
        let (_dir, store, mut index) = fixture();
        let v1 = r#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#;
        let v2 = r#"{"id":"pw","name":"Playwright v2","transport":"stdio","command":"npx"}"#;
        AssetService::write(&store, &mut index, AssetType::Mcp, "pw", v1).unwrap();
        AssetService::write(&store, &mut index, AssetType::Mcp, "pw", v2).unwrap();

        assert_eq!(
            String::from_utf8(store.read(AssetType::Mcp, "pw").unwrap()).unwrap(),
            v2
        );
        let backups = collect_files(&store.root().join("backups"));
        assert_eq!(backups.len(), 1, "改一次应当留下一份旧内容备份");
        assert_eq!(
            String::from_utf8(std::fs::read(&backups[0]).unwrap()).unwrap(),
            v1
        );
        let rows = AssetService::list(&store, &index, Some(AssetType::Mcp)).unwrap();
        assert_eq!(rows[0].name, "Playwright v2");
    }

    #[test]
    fn reading_a_skill_returns_its_skill_md() {
        let (_dir, store, _index) = fixture();
        let dir = store.asset_dir(AssetType::Skill).join("demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "# demo\n").unwrap();
        assert_eq!(
            AssetService::read(&store, AssetType::Skill, "demo").unwrap(),
            "# demo\n"
        );
    }

    fn collect_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    out.extend(collect_files(&path));
                } else {
                    out.push(path);
                }
            }
        }
        out
    }
}
