use crate::error::AppError;
use crate::settings::{self, AppSettings, Theme};

pub fn get() -> Result<AppSettings, AppError> {
    Ok(settings::load()?)
}

pub fn set_theme(theme: Theme) -> Result<AppSettings, AppError> {
    let mut current = settings::load()?;
    current.theme = theme;
    settings::save(&current)?;
    Ok(current)
}

pub fn set_store_root(store_root: std::path::PathBuf) -> Result<AppSettings, AppError> {
    set_store_root_at(&settings::settings_path()?, store_root)
}

/// 只改写设置文件；store/index 在启动时绑定，因此切换后需要重启才生效。
pub fn set_store_root_at(
    config_path: &std::path::Path,
    store_root: std::path::PathBuf,
) -> Result<AppSettings, AppError> {
    if store_root.as_os_str().is_empty() {
        return Err(AppError::InvalidInput("store root 不能为空".into()));
    }
    std::fs::create_dir_all(&store_root)
        .map_err(|e| AppError::InvalidInput(format!("无法创建目录：{e}")))?;
    let mut current = settings::load_from(config_path)?;
    current.store_root = store_root;
    settings::save_to(config_path, &current)?;
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn set_store_root_persists_and_points_to_new_root() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let new_root = dir.path().join("moved-store");
        let updated = set_store_root_at(&config, new_root.clone()).unwrap();
        assert_eq!(updated.store_root, new_root);
        assert_eq!(settings::load_from(&config).unwrap().store_root, new_root);
    }
}
