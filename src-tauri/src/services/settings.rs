use crate::domain::agent::builtin_agents;
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

pub fn set_enabled_agents(agent_ids: Vec<String>) -> Result<AppSettings, AppError> {
    set_enabled_agents_at(&settings::settings_path()?, agent_ids)
}

/// 只接受内置 agent，并去重；空数组是合法值（全部禁用）。
pub fn set_enabled_agents_at(
    config_path: &std::path::Path,
    agent_ids: Vec<String>,
) -> Result<AppSettings, AppError> {
    let known: Vec<String> = builtin_agents().into_iter().map(|agent| agent.id).collect();
    let mut unique = Vec::new();
    for id in agent_ids {
        if !known.contains(&id) {
            return Err(AppError::InvalidInput(format!("未知的 agent：{id}")));
        }
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    let mut current = settings::load_from(config_path)?;
    current.enabled_agents = Some(unique);
    settings::save_to(config_path, &current)?;
    Ok(current)
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
    fn set_enabled_agents_validates_and_deduplicates() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let updated =
            set_enabled_agents_at(&config, vec!["claude".into(), "claude".into()]).unwrap();
        assert_eq!(updated.enabled_agents, Some(vec!["claude".to_string()]));
        assert!(set_enabled_agents_at(&config, vec!["nope".into()]).is_err());
        let empty = set_enabled_agents_at(&config, Vec::new()).unwrap();
        assert_eq!(empty.enabled_agents, Some(Vec::new()), "空数组表示全部禁用");
    }

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
