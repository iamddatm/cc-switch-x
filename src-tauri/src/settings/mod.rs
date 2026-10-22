use crate::store::atomic::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 本机应用设置：只存路径与偏好，不含资产内容，也不参与任何同步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub store_root: PathBuf,
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    System,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            store_root: default_store_root(),
            theme: Theme::System,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

pub fn default_store_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ccswitchx")
}

pub fn settings_path() -> Result<PathBuf, SettingsError> {
    let base = dirs::config_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir"))?;
    Ok(base.join("cc-switch-x").join("settings.json"))
}

pub fn load() -> Result<AppSettings, SettingsError> {
    load_from(&settings_path()?)
}

pub fn load_from(path: &Path) -> Result<AppSettings, SettingsError> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn save(settings: &AppSettings) -> Result<(), SettingsError> {
    save_to(&settings_path()?, settings)
}

pub fn save_to(path: &Path, settings: &AppSettings) -> Result<(), SettingsError> {
    let bytes = serde_json::to_vec_pretty(settings)?;
    write_atomic(path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let settings = load_from(&dir.path().join("settings.json")).unwrap();
        assert_eq!(settings.theme, Theme::System);
        assert!(settings.store_root.ends_with(".ccswitchx"));
    }

    #[test]
    fn roundtrip_preserves_values() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let settings = AppSettings {
            store_root: dir.path().join("data"),
            theme: Theme::Dark,
        };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path).unwrap(), settings);
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_from(&path).is_err());
    }
}
