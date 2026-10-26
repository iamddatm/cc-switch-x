use crate::domain::project::ProjectRegistry;
use crate::error::AppError;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use std::path::PathBuf;

pub fn registry_path(store: &Store) -> PathBuf {
    store.root().join("local").join("projects.json")
}

/// 注册表缺失按空表处理；存在但解析不了必须报错 —— 静默当空表会忘掉所有项目。
pub fn load(store: &Store) -> Result<ProjectRegistry, AppError> {
    let path = registry_path(store);
    if !path.exists() {
        return Ok(ProjectRegistry::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub fn save(store: &Store, registry: &ProjectRegistry) -> Result<(), AppError> {
    let path = registry_path(store);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&path, &serde_json::to_vec_pretty(registry)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::ProjectEntry;

    #[test]
    fn missing_registry_is_empty_but_corrupt_registry_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        assert!(load(&store).unwrap().projects.is_empty());

        let path = registry_path(&store);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ 这不是 JSON").unwrap();
        assert!(matches!(load(&store).unwrap_err(), AppError::Json(_)));
    }

    #[test]
    fn registry_roundtrips_with_camel_case_keys() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let registry = ProjectRegistry {
            version: 1,
            projects: vec![ProjectEntry {
                id: "app-1a2b3c4d5e6f".into(),
                path: dir.path().join("app"),
                name: "app".into(),
                added_at: "2026-10-26T00:00:00Z".into(),
                last_opened_at: "2026-10-26T00:00:00Z".into(),
                auto_sync: true,
            }],
        };
        save(&store, &registry).unwrap();
        assert_eq!(load(&store).unwrap(), registry);
        let text = std::fs::read_to_string(registry_path(&store)).unwrap();
        assert!(text.contains("\"lastOpenedAt\""));
        assert!(text.contains("\"autoSync\""));
    }
}
