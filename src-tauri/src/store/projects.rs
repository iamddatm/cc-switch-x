use crate::domain::project::{strip_verbatim, ProjectRegistry};
use crate::error::AppError;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use std::path::PathBuf;

pub fn registry_path(store: &Store) -> PathBuf {
    store.root().join("local").join("projects.json")
}

/// 注册表缺失按空表处理；存在但解析不了必须报错 —— 静默当空表会忘掉所有项目。
/// 读入时顺手把旧版本写下的 `\\?\` 逐字路径还原成可读形式，否则同一项目会被当成两条。
pub fn load(store: &Store) -> Result<ProjectRegistry, AppError> {
    let path = registry_path(store);
    if !path.exists() {
        return Ok(ProjectRegistry::default());
    }
    let mut registry: ProjectRegistry = serde_json::from_slice(&std::fs::read(path)?)?;
    for entry in &mut registry.projects {
        entry.path = strip_verbatim(&entry.path);
    }
    Ok(registry)
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

    /// 旧版本把 canonicalize 的逐字路径直接写进了注册表，读入必须还原，
    /// 否则 UI 显示 `\\?\D:\...`，且同一项目会被再添加一次。
    #[test]
    fn legacy_verbatim_paths_are_normalized_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let path = registry_path(&store);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"version":1,"projects":[{"id":"app-1a2b3c4d5e6f","path":"\\\\?\\D:\\repos\\app","name":"app","addedAt":"2026-10-26T00:00:00Z","lastOpenedAt":"2026-10-26T00:00:00Z","autoSync":true}]}"#,
        )
        .unwrap();

        let expected = if cfg!(windows) {
            PathBuf::from(r"D:\repos\app")
        } else {
            PathBuf::from(r"\\?\D:\repos\app")
        };
        assert_eq!(load(&store).unwrap().projects[0].path, expected);
    }
}
