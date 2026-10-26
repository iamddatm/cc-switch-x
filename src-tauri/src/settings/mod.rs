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
    /// 备份策略与 Git 备份；旧设置文件缺少该字段时整套取默认值。
    #[serde(default)]
    pub backup: BackupSettings,
    /// WebDAV 云备份；密码单独放在 secrets.local.json。
    #[serde(default)]
    pub cloud: CloudBackupSettings,
}

/// 备份策略：间隔 0 表示关闭自动快照；保留份数只作用于 `backups/snapshots`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    #[serde(default = "default_backup_interval")]
    pub interval_hours: u32,
    #[serde(default = "default_backup_retain")]
    pub retain_count: u32,
    #[serde(default)]
    pub git: GitBackupSettings,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            interval_hours: default_backup_interval(),
            retain_count: default_backup_retain(),
            git: GitBackupSettings::default(),
        }
    }
}

/// Git 备份：enabled 只在 git init 成功后置真；关闭只停自动同步，不删仓库。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBackupSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_commit: bool,
    #[serde(default = "default_true")]
    pub auto_pull: bool,
    /// 远端地址只接受 https/ssh/git@；凭据交给系统 git。
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
}

impl Default for GitBackupSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_commit: true,
            auto_pull: true,
            remote: None,
            branch: default_branch(),
        }
    }
}

/// WebDAV 云备份：远端只保留一份最新状态包；密码单独放在 secrets.local.json。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudBackupSettings {
    #[serde(default)]
    pub auto_upload: bool,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default = "default_cloud_remote_dir")]
    pub remote_dir: String,
}

impl Default for CloudBackupSettings {
    fn default() -> Self {
        Self {
            auto_upload: false,
            base_url: String::new(),
            username: String::new(),
            remote_dir: default_cloud_remote_dir(),
        }
    }
}

fn default_cloud_remote_dir() -> String {
    "ccswitchx".to_string()
}

fn default_backup_interval() -> u32 {
    24
}

fn default_backup_retain() -> u32 {
    10
}

/// Git 分支名：允许常见的 `feature/x`，但拒绝会改变 CLI 语义或以 ref 语法歧义结尾的名字。
pub fn validate_git_branch(branch: &str) -> Result<(), String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("分支名不能为空".into());
    }
    if branch.len() > 200 {
        return Err("分支名过长".into());
    }
    if branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.ends_with(".lock")
        || branch == "HEAD"
        || branch.contains("..")
        || branch.contains("//")
        || branch.contains("@{")
        || branch.contains('\\')
    {
        return Err(format!("非法分支名：{branch}"));
    }
    if !branch
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '/' | '-'))
    {
        return Err(format!("分支名只能包含字母、数字、`.`、`_`、`/`、`-`：{branch}"));
    }
    Ok(())
}

/// 远端地址：凭据交给系统 git，因此这里只拒绝明显会把凭据写进配置/命令的写法。
pub fn validate_git_remote(remote: &str) -> Result<(), String> {
    let remote = remote.trim();
    if remote.is_empty() {
        return Err("远端地址不能为空".into());
    }
    if remote.starts_with('-') || remote.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err("远端地址包含非法字符".into());
    }
    let lower = remote.to_ascii_lowercase();
    if lower.starts_with("https://") {
        let rest = &remote[remote.find("://").map(|i| i + 3).unwrap_or(0)..];
        let host_part = rest.split('/').next().unwrap_or_default();
        if host_part.contains('@') {
            return Err("不要把用户名或令牌写进 https 地址，凭据请交给系统 git 管理".into());
        }
        return Ok(());
    }
    if lower.starts_with("http://") {
        return Err("远端必须使用 https://，不接受明文 http".into());
    }
    if lower.starts_with("ssh://") || lower.starts_with("file://") {
        return Ok(());
    }
    // scp 风格的 `git@host:path`
    if remote.starts_with("git@") && remote.contains(':') {
        return Ok(());
    }
    Err("远端地址只支持 https://、ssh://、file:// 或 git@host:path".into())
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
            backup: BackupSettings::default(),
            cloud: CloudBackupSettings::default(),
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
            backup: BackupSettings {
                interval_hours: 12,
                retain_count: 5,
                git: GitBackupSettings {
                    enabled: true,
                    auto_commit: false,
                    auto_pull: false,
                    remote: Some("git@github.com:me/assets.git".into()),
                    branch: "backup".into(),
                },
            },
            cloud: CloudBackupSettings {
                auto_upload: true,
                base_url: "https://dav.example.com/dav".into(),
                username: "me".into(),
                remote_dir: "backups/ccswitchx".into(),
            },
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
        assert_eq!(loaded.backup, BackupSettings::default());
        assert_eq!(loaded.backup.interval_hours, 24);
        assert_eq!(loaded.backup.retain_count, 10);
        assert!(!loaded.backup.git.enabled);
    }

    #[test]
    fn partial_backup_settings_get_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let value = serde_json::json!({
            "storeRoot": dir.path().join("data"),
            "theme": "system",
            "backup": {
                "intervalHours": 6,
                "git": { "enabled": true }
            }
        });
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.backup.interval_hours, 6);
        assert_eq!(loaded.backup.retain_count, 10);
        assert!(loaded.backup.git.enabled);
        assert!(loaded.backup.git.auto_commit);
        assert!(loaded.backup.git.auto_pull);
        assert_eq!(loaded.backup.git.branch, "main");
        assert!(loaded.backup.git.remote.is_none());
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

    #[test]
    fn git_branch_and_remote_validation() {
        for ok in ["main", "feature/backup", "v1.2.3", "my_branch"] {
            assert!(validate_git_branch(ok).is_ok(), "{ok} 应当合法");
        }
        for bad in ["", "-x", "a..b", "a//b", "a/", "a.lock", "HEAD", "a b", "refs@{1}"] {
            assert!(validate_git_branch(bad).is_err(), "{bad} 应当被拒绝");
        }

        for ok in [
            "https://github.com/me/assets.git",
            "ssh://git@example.com/me/assets.git",
            "git@github.com:me/assets.git",
            "file:///tmp/assets.git",
        ] {
            assert!(validate_git_remote(ok).is_ok(), "{ok} 应当合法");
        }
        for bad in [
            "",
            "-oProxyCommand=calc",
            "http://example.com/repo.git",
            "https://user:token@example.com/repo.git",
            "ftp://example.com/repo.git",
            "git@host:path with space",
        ] {
            assert!(validate_git_remote(bad).is_err(), "{bad} 应当被拒绝");
        }
    }

    #[test]
    fn old_settings_files_get_default_cloud_settings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let old = serde_json::json!({
            "storeRoot": dir.path().join("data"),
            "theme": "light",
            "enabledAgents": null
        });
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

        let loaded = load_from(&path).unwrap();
        assert!(!loaded.cloud.auto_upload);
        assert_eq!(loaded.cloud.base_url, "");
        assert_eq!(loaded.cloud.username, "");
        assert_eq!(loaded.cloud.remote_dir, "ccswitchx");
    }

    #[test]
    fn cloud_settings_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = AppSettings {
            cloud: CloudBackupSettings {
                auto_upload: true,
                base_url: "https://dav.example.com/dav".into(),
                username: "me".into(),
                remote_dir: "backups/ccswitchx".into(),
            },
            ..AppSettings::default()
        };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path).unwrap(), settings);
    }
}
