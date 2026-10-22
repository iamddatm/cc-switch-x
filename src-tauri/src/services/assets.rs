use crate::domain::{display_name, parse_and_validate, AssetSummary, AssetType};
use crate::error::AppError;
use crate::index::{sha256_hex, Index};
use crate::store::Store;

pub struct AssetService;

impl AssetService {
    pub fn list(
        _store: &Store,
        index: &Index,
        asset_type: Option<AssetType>,
    ) -> Result<Vec<AssetSummary>, AppError> {
        Ok(index.list(asset_type)?)
    }

    pub fn read(store: &Store, asset_type: AssetType, id: &str) -> Result<String, AppError> {
        let bytes = store.read(asset_type, id)?;
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
        let summary = AssetSummary {
            asset_type,
            id: id.to_string(),
            name,
            updated_at: chrono::Utc::now().timestamp_millis(),
            hash: sha256_hex(bytes),
        };
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::index::Index;
    use crate::store::Store;
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
            r#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
        )
        .unwrap();
        assert_eq!(summary.name, "Playwright");
        assert_eq!(
            AssetService::list(&store, &index, Some(AssetType::Mcp))
                .unwrap()
                .len(),
            1
        );
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
