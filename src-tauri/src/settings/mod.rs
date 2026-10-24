use crate::store::atomic::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 本机应用设置：只存路径与偏好，不含资产内容，也不参与任何同步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub store_root: PathBuf,
    pub theme: Theme,
    /// 全局启用的 agent；None = 全部启用（兼容旧设置文件）。
    #[serde(default)]
    pub enabled_agents: Option<Vec<String>>,
    /// 关闭主窗口时隐藏到托盘，而不是退出应用。
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// 全局出站代理；密码不进设置文件，单独放在 secrets.local.json。
    #[serde(default)]
    pub proxy: ProxySettings,
    /// Skill 发现用的仓库；None = 内置默认仓库。
    #[serde(default)]
    pub skill_repos: Option<Vec<SkillRepo>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettings {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

/// Skill 发现仓库。owner/name 必须能被解析成 GitHub 坐标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepo {
    pub owner: String,
    pub name: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl SkillRepo {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_true() -> bool {
    true
}

/// 原版 cc-switch 内置的四个仓库；用户改过仓库列表后不再使用这份。
pub fn default_skill_repos() -> Vec<SkillRepo> {
    [
        ("anthropics", "skills", "main"),
        ("ComposioHQ", "awesome-claude-skills", "master"),
        ("cexll", "myclaude", "master"),
        ("JimLiu", "baoyu-skills", "main"),
    ]
    .into_iter()
    .map(|(owner, name, branch)| SkillRepo {
        owner: owner.to_string(),
        name: name.to_string(),
        branch: branch.to_string(),
        enabled: true,
    })
    .collect()
}

pub fn effective_skill_repos(settings: &AppSettings) -> Vec<SkillRepo> {
    settings
        .skill_repos
        .clone()
        .unwrap_or_else(default_skill_repos)
}

/// 未设置过范围时视为全部启用；显式空数组表示全部禁用。
pub fn is_agent_enabled(settings: &AppSettings, agent_id: &str) -> bool {
    settings
        .enabled_agents
        .as_ref()
        .is_none_or(|ids| ids.iter().any(|id| id == agent_id))
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
            enabled_agents: None,
            close_to_tray: true,
            proxy: ProxySettings::default(),
            skill_repos: None,
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
            enabled_agents: Some(vec!["claude".into()]),
            close_to_tray: false,
            proxy: ProxySettings {
                url: Some("http://127.0.0.1:7890".into()),
                username: Some("me".into()),
            },
            skill_repos: Some(vec![SkillRepo {
                owner: "acme".into(),
                name: "skills".into(),
                branch: "dev".into(),
                enabled: false,
            }]),
        };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path).unwrap(), settings);
    }

    #[test]
    fn unset_scope_means_every_agent_is_enabled() {
        let settings = AppSettings::default();
        assert!(is_agent_enabled(&settings, "claude"));
        assert!(is_agent_enabled(&settings, "pi"));
    }

    #[test]
    fn explicit_scope_limits_the_visible_agents() {
        let settings = AppSettings {
            enabled_agents: Some(vec!["claude".into()]),
            ..AppSettings::default()
        };
        assert!(is_agent_enabled(&settings, "claude"));
        assert!(!is_agent_enabled(&settings, "pi"));
    }

    #[test]
    fn old_settings_files_get_safe_defaults_for_new_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let old = serde_json::json!({
            "storeRoot": dir.path().join("data"),
            "theme": "light",
            "enabledAgents": null
        });
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

        let loaded = load_from(&path).unwrap();
        assert!(loaded.close_to_tray, "旧设置应默认关闭到托盘");
        assert_eq!(loaded.proxy, ProxySettings::default());
        assert!(loaded.skill_repos.is_none());
        assert_eq!(effective_skill_repos(&loaded).len(), 4);
    }

    #[test]
    fn explicit_empty_repo_list_is_not_replaced_by_defaults() {
        let settings = AppSettings {
            skill_repos: Some(Vec::new()),
            ..AppSettings::default()
        };
        assert!(effective_skill_repos(&settings).is_empty());
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_from(&path).is_err());
    }
}
