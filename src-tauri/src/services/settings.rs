use crate::domain::agent::builtin_agents;
use crate::domain::secret::SecretRef;
use crate::error::AppError;
use crate::net::{parse_github_input, webdav, ProxyConfig};
use crate::settings::{self, AppSettings, BackupSettings, SkillRepo, Theme};
use crate::store::secrets::Secrets;
use serde::Serialize;
use std::collections::HashSet;

/// 代理密码与 MCP 等资产用同一套本地密钥文件分开存放。
pub const PROXY_SECRET_SCOPE: &str = "proxy";
pub const PROXY_SECRET_KEY: &str = "password";
pub const CLOUD_SECRET_SCOPE: &str = "cloud.webdav";
pub const CLOUD_SECRET_KEY: &str = "password";
pub const MAX_SKILL_REPOS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettingsView {
    pub url: Option<String>,
    pub username: Option<String>,
    pub has_password: bool,
    /// 实际生效的开关；没填地址时恒为 false。
    pub enabled: bool,
    /// WebDAV 云备份是否也用这个代理。
    pub webdav_uses_proxy: bool,
}

pub fn proxy_secret_ref() -> SecretRef {
    SecretRef {
        scope: PROXY_SECRET_SCOPE.into(),
        key: PROXY_SECRET_KEY.into(),
    }
}

pub fn proxy_view(settings: &AppSettings, secrets: &Secrets) -> ProxySettingsView {
    let url = settings
        .proxy
        .url
        .clone()
        .filter(|value| !value.trim().is_empty());
    let username = settings
        .proxy
        .username
        .clone()
        .filter(|value| !value.trim().is_empty());
    let has_password = url.is_some()
        && username.is_some()
        && secrets
            .get(&proxy_secret_ref())
            .is_some_and(|value| !value.is_empty());
    ProxySettingsView {
        url: url.clone(),
        username,
        has_password,
        enabled: url.is_some() && settings.proxy.enabled,
        webdav_uses_proxy: settings.proxy.webdav_uses_proxy,
    }
}

/// 密码语义：None = 保留旧密码，Some("") = 清除，Some(v) = 覆盖。
/// `enabled = false` 保留地址与凭据但按直连处理；清空地址则连同凭据一起清除。
pub fn set_proxy_at(
    config_path: &std::path::Path,
    secrets_path: &std::path::Path,
    url: String,
    username: String,
    password: Option<String>,
    enabled: bool,
    webdav_uses_proxy: bool,
) -> Result<(AppSettings, Secrets), AppError> {
    let mut current = settings::load_from(config_path)?;
    let mut secrets = Secrets::load(secrets_path)?;
    let url = url.trim();
    let username = username.trim();
    // 与地址存不存在无关：清空地址时保留用户对 WebDAV 的选择，重新填地址后不用再勾一次。
    current.proxy.webdav_uses_proxy = webdav_uses_proxy;

    if url.is_empty() {
        // 清空地址就是回到直连，认证信息一并清掉，避免留下无法使用的残留。
        current.proxy.url = None;
        current.proxy.username = None;
        current.proxy.enabled = false;
        secrets.remove(&proxy_secret_ref());
    } else {
        // 密码必须进 secrets 文件；地址里带 userinfo 会被原样写进 settings.json。
        if let Ok(parsed) = reqwest::Url::parse(url) {
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(AppError::InvalidInput(
                    "请把代理用户名和密码填在下方输入框，不要写进代理地址".into(),
                ));
            }
        }
        let username = (!username.is_empty()).then(|| username.to_string());
        let next_password = if username.is_none() {
            match password {
                Some(value) if !value.is_empty() => {
                    return Err(AppError::InvalidInput(
                        "填写代理密码前，请先填写代理用户名".into(),
                    ));
                }
                _ => {
                    // 没有用户名时认证信息没有意义，清掉旧密码避免留下无法使用的残留。
                    secrets.remove(&proxy_secret_ref());
                    None
                }
            }
        } else {
            match password {
                Some(value) if value.is_empty() => {
                    secrets.remove(&proxy_secret_ref());
                    None
                }
                Some(value) => Some(value),
                None => secrets.get(&proxy_secret_ref()).map(str::to_string),
            }
        };
        let config = ProxyConfig::new(
            Some(url.to_string()),
            username.clone(),
            next_password.clone(),
            enabled,
        );
        config.validate()?;
        current.proxy.url = Some(url.to_string());
        current.proxy.username = username;
        current.proxy.enabled = enabled;
        if let Some(value) = next_password {
            secrets.insert(&proxy_secret_ref(), &value);
        }
    }

    secrets.save(secrets_path)?;
    settings::save_to(config_path, &current)?;
    Ok((current, secrets))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSettingsView {
    /// 通道是否启用（参与「远端备份方式」单选）。
    pub enabled: bool,
    /// 变化后是否自动上传；`enabled` 为假时恒为假。
    pub auto_upload: bool,
    /// 启动或变化检测后是否自动下载；`enabled` 为假时恒为假。
    pub auto_download: bool,
    /// 上传防抖窗口（小时），1–24。
    pub upload_delay_hours: u32,
    pub base_url: String,
    pub username: String,
    pub remote_dir: String,
    pub has_password: bool,
}

pub fn cloud_secret_ref() -> SecretRef {
    SecretRef {
        scope: CLOUD_SECRET_SCOPE.into(),
        key: CLOUD_SECRET_KEY.into(),
    }
}

pub fn cloud_view(settings: &AppSettings, secrets: &Secrets) -> CloudSettingsView {
    CloudSettingsView {
        enabled: settings.cloud.enabled,
        auto_upload: settings.cloud.auto_upload,
        auto_download: settings.cloud.auto_download,
        upload_delay_hours: settings.cloud.upload_delay_hours,
        base_url: settings.cloud.base_url.clone(),
        username: settings.cloud.username.clone(),
        remote_dir: settings.cloud.remote_dir.clone(),
        has_password: secrets
            .get(&cloud_secret_ref())
            .is_some_and(|value| !value.is_empty()),
    }
}

/// 密码语义：None = 保留旧密码，Some("") = 清除，Some(v) = 覆盖。
/// 「通道启用」与「自动上传 / 自动下载」分开存：两者都必须先启用通道，关闭通道会一并停掉。
/// 返回的第三项表示连接参数（地址 / 用户名 / 目录 / 最终密码）是否变化，
/// 调用方据此作废旧远端的同步基线。
pub fn set_cloud_settings_at(
    config_path: &std::path::Path,
    secrets_path: &std::path::Path,
    base_url: String,
    username: String,
    password: Option<String>,
    remote_dir: String,
    enabled: bool,
    auto_upload: bool,
    auto_download: bool,
    upload_delay_hours: u32,
) -> Result<(AppSettings, Secrets, bool), AppError> {
    if !(1..=24).contains(&upload_delay_hours) {
        return Err(AppError::InvalidInput("上传延迟需在 1–24 小时之间".into()));
    }
    let mut current = settings::load_from(config_path)?;
    let mut secrets = Secrets::load(secrets_path)?;
    // 连接参数以保存前的值为基线：任何一项变化都要让旧远端的同步状态作废。
    let previous_cloud = current.cloud.clone();
    let previous_password = secrets.get(&cloud_secret_ref()).map(str::to_string);
    let base_url = webdav::normalize_base_url(&base_url)?;
    let username = username.trim().to_string();
    let remote_dir = webdav::validate_remote_dir(&remote_dir)?;
    let auto_upload = enabled && auto_upload;
    let auto_download = enabled && auto_download;

    if base_url.is_empty() {
        current.cloud.base_url.clear();
        current.cloud.username.clear();
        current.cloud.remote_dir = remote_dir;
        current.cloud.enabled = false;
        current.cloud.auto_upload = false;
        current.cloud.auto_download = false;
        secrets.remove(&cloud_secret_ref());
    } else {
        if username.is_empty() {
            return Err(AppError::InvalidInput("请填写 WebDAV 用户名".into()));
        }
        let next_password = match password {
            Some(value) if value.is_empty() => {
                secrets.remove(&cloud_secret_ref());
                None
            }
            Some(value) => Some(value),
            None => secrets.get(&cloud_secret_ref()).map(str::to_string),
        };
        let Some(next_password) = next_password else {
            return Err(AppError::InvalidInput("请填写 WebDAV 密码".into()));
        };
        current.cloud.base_url = base_url;
        current.cloud.username = username;
        current.cloud.remote_dir = remote_dir;
        current.cloud.enabled = enabled;
        current.cloud.auto_upload = auto_upload;
        current.cloud.auto_download = auto_download;
        secrets.insert(&cloud_secret_ref(), &next_password);
    }
    current.cloud.upload_delay_hours = upload_delay_hours;
    if current.cloud.enabled {
        // 远端备份只能选一种：启用 WebDAV 通道时停用 Git 自动同步。
        current.backup.git.enabled = false;
    }

    secrets.save(secrets_path)?;
    settings::save_to(config_path, &current)?;
    let next_password = secrets.get(&cloud_secret_ref()).map(str::to_string);
    let connection_changed = previous_cloud.base_url != current.cloud.base_url
        || previous_cloud.username != current.cloud.username
        || previous_cloud.remote_dir != current.cloud.remote_dir
        || previous_password != next_password;
    Ok((current, secrets, connection_changed))
}

pub fn set_close_to_tray_at(
    config_path: &std::path::Path,
    enabled: bool,
) -> Result<AppSettings, AppError> {
    let mut current = settings::load_from(config_path)?;
    current.close_to_tray = enabled;
    settings::save_to(config_path, &current)?;
    Ok(current)
}

/// 校验并规范化仓库列表：分支空串按 main，owner/name 必须是合法 GitHub 坐标。
pub fn validate_skill_repos(repos: Vec<SkillRepo>) -> Result<Vec<SkillRepo>, AppError> {
    if repos.len() > MAX_SKILL_REPOS {
        return Err(AppError::InvalidInput(format!(
            "最多只能配置 {MAX_SKILL_REPOS} 个仓库"
        )));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(repos.len());
    for repo in repos {
        let branch = repo.branch.trim();
        let branch = if branch.is_empty() { "main" } else { branch };
        let parsed = parse_github_input(&repo.slug(), Some(branch.to_string()))?;
        let key = (parsed.owner.to_lowercase(), parsed.name.to_lowercase());
        if !seen.insert(key) {
            return Err(AppError::InvalidInput(format!(
                "仓库重复：{}",
                parsed.slug()
            )));
        }
        out.push(SkillRepo {
            owner: parsed.owner,
            name: parsed.name,
            branch: parsed.branch.unwrap_or_else(|| "main".into()),
            enabled: repo.enabled,
        });
    }
    Ok(out)
}

pub fn set_skill_repos(repos: Vec<SkillRepo>) -> Result<AppSettings, AppError> {
    set_skill_repos_at(&settings::settings_path()?, repos)
}

pub fn set_skill_repos_at(
    config_path: &std::path::Path,
    repos: Vec<SkillRepo>,
) -> Result<AppSettings, AppError> {
    let normalized = validate_skill_repos(repos)?;
    let mut current = settings::load_from(config_path)?;
    current.skill_repos = Some(normalized);
    settings::save_to(config_path, &current)?;
    Ok(current)
}

pub fn get() -> Result<AppSettings, AppError> {
    Ok(settings::load()?)
}

/// 备份策略与 Git 偏好。间隔 0 表示关闭自动快照；保留份数只影响 snapshots。
pub fn set_backup_settings(backup: BackupSettings) -> Result<AppSettings, AppError> {
    set_backup_settings_at(&settings::settings_path()?, backup)
}

pub fn set_backup_settings_at(
    config_path: &std::path::Path,
    mut backup: BackupSettings,
) -> Result<AppSettings, AppError> {
    if backup.interval_hours > 720 {
        return Err(AppError::InvalidInput("自动备份间隔不能超过 720 小时".into()));
    }
    if !(1..=100).contains(&backup.retain_count) {
        return Err(AppError::InvalidInput("保留份数必须在 1 到 100 之间".into()));
    }
    backup.git.branch = backup.git.branch.trim().to_string();
    settings::validate_git_branch(&backup.git.branch).map_err(AppError::InvalidInput)?;
    backup.git.remote = backup
        .git
        .remote
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(remote) = &backup.git.remote {
        settings::validate_git_remote(remote).map_err(AppError::InvalidInput)?;
    }
    // 没有远端就无法验证；开启状态只会误导，直接纠正为停用，避免阻塞其他设置保存。
    if backup.git.enabled && backup.git.remote.is_none() {
        backup.git.enabled = false;
    }
    let mut current = settings::load_from(config_path)?;
    current.backup = backup;
    if current.backup.git.enabled {
        // 远端备份只能选一种：开启 Git 自动同步时停用整条 WebDAV 通道。
        current.cloud.enabled = false;
        current.cloud.auto_upload = false;
        current.cloud.auto_download = false;
    }
    settings::save_to(config_path, &current)?;
    Ok(current)
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
    let unique = unique_known_agents(agent_ids)?;
    let mut current = settings::load_from(config_path)?;
    current.enabled_agents = Some(unique);
    settings::save_to(config_path, &current)?;
    Ok(current)
}

pub fn set_agent_order(agent_ids: Vec<String>) -> Result<AppSettings, AppError> {
    set_agent_order_at(&settings::settings_path()?, agent_ids)
}

/// 空数组等于「恢复注册表顺序」，存 None，不留一个无意义的空顺序。
pub fn set_agent_order_at(
    config_path: &std::path::Path,
    agent_ids: Vec<String>,
) -> Result<AppSettings, AppError> {
    let unique = unique_known_agents(agent_ids)?;
    let mut current = settings::load_from(config_path)?;
    current.agent_order = if unique.is_empty() { None } else { Some(unique) };
    settings::save_to(config_path, &current)?;
    Ok(current)
}

/// 只接受内置 agent，去重并保持传入顺序。
fn unique_known_agents(agent_ids: Vec<String>) -> Result<Vec<String>, AppError> {
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
    Ok(unique)
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
    use crate::net::webdav::{normalize_base_url, remote_url, validate_remote_dir};
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
    fn set_agent_order_validates_and_deduplicates() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let updated =
            set_agent_order_at(&config, vec!["pi".into(), "claude".into(), "pi".into()]).unwrap();
        assert_eq!(
            updated.agent_order,
            Some(vec!["pi".to_string(), "claude".to_string()])
        );
        assert!(set_agent_order_at(&config, vec!["nope".into()]).is_err());

        let cleared = set_agent_order_at(&config, Vec::new()).unwrap();
        assert_eq!(cleared.agent_order, None, "空数组等于恢复注册表顺序");
        assert_eq!(
            cleared.enabled_agents, None,
            "改顺序不该动启用范围；None 仍是「全部启用」"
        );
    }

    #[test]
    fn backup_settings_are_validated_and_persisted() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let backup = BackupSettings {
            interval_hours: 6,
            retain_count: 5,
            git: crate::settings::GitBackupSettings {
                enabled: true,
                auto_commit: true,
                auto_pull: false,
                remote: Some("  https://example.com/assets.git  ".into()),
                branch: " backup/x ".into(),
            },
        };
        let updated = set_backup_settings_at(&config, backup).unwrap();
        assert_eq!(updated.backup.interval_hours, 6);
        assert_eq!(updated.backup.retain_count, 5);
        assert_eq!(
            updated.backup.git.remote.as_deref(),
            Some("https://example.com/assets.git")
        );
        assert_eq!(updated.backup.git.branch, "backup/x");

        let invalid = BackupSettings {
            retain_count: 0,
            ..updated.backup.clone()
        };
        assert!(set_backup_settings_at(&config, invalid).is_err());
        let invalid = BackupSettings {
            interval_hours: 999,
            ..updated.backup.clone()
        };
        assert!(set_backup_settings_at(&config, invalid).is_err());
    }

    #[test]
    fn git_and_webdav_auto_backup_are_mutually_exclusive() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        let (with_cloud, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            Some("pw".into()),
            "ccswitchx".into(),
            true,
            true,
            true,
            1,
        )
        .unwrap();
        assert!(with_cloud.cloud.enabled);
        assert!(with_cloud.cloud.auto_upload);
        assert!(with_cloud.cloud.auto_download);

        let with_git = set_backup_settings_at(
            &config,
            BackupSettings {
                git: crate::settings::GitBackupSettings {
                    enabled: true,
                    remote: Some("https://example.com/assets.git".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        assert!(with_git.backup.git.enabled);
        assert!(
            !with_git.cloud.enabled
                && !with_git.cloud.auto_upload
                && !with_git.cloud.auto_download,
            "启用 Git 自动同步时应停用整条 WebDAV 通道"
        );

        let (with_cloud_again, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            None,
            "ccswitchx".into(),
            true,
            true,
            true,
            1,
        )
        .unwrap();
        assert!(with_cloud_again.cloud.auto_upload);
        assert!(
            !with_cloud_again.backup.git.enabled,
            "启用 WebDAV 通道时应停用 Git 自动同步"
        );
    }

    #[test]
    fn cloud_auto_upload_requires_enabled_channel() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        let (saved, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            Some("pw".into()),
            "ccswitchx".into(),
            false,
            true,
            false,
            1,
        )
        .unwrap();
        assert!(!saved.cloud.enabled, "通道未启用时不能打开自动上传");
        assert!(!saved.cloud.auto_upload);

        // 只启用通道、不开自动上传 = 手动上传模式。
        let (manual, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            None,
            "ccswitchx".into(),
            true,
            false,
            false,
            1,
        )
        .unwrap();
        assert!(manual.cloud.enabled);
        assert!(!manual.cloud.auto_upload);
    }

    #[test]
    fn cloud_auto_download_requires_enabled_channel() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        let (saved, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            Some("pw".into()),
            "ccswitchx".into(),
            false,
            false,
            true,
            1,
        )
        .unwrap();
        assert!(!saved.cloud.enabled, "通道未启用时不能打开自动下载");
        assert!(!saved.cloud.auto_download);

        let (enabled, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            None,
            "ccswitchx".into(),
            true,
            false,
            true,
            1,
        )
        .unwrap();
        assert!(enabled.cloud.enabled);
        assert!(enabled.cloud.auto_download);
    }

    #[test]
    fn cloud_upload_delay_must_be_1_to_24() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        for bad in [0u32, 25] {
            let error = set_cloud_settings_at(
                &config,
                &secrets_path,
                "https://dav.example.com/dav".into(),
                "me".into(),
                Some("pw".into()),
                "ccswitchx".into(),
                true,
                false,
                false,
                bad,
            )
            .unwrap_err();
            assert!(
                matches!(&error, AppError::InvalidInput(message) if message == "上传延迟需在 1–24 小时之间"),
                "{bad} 应被拒绝：{error}"
            );
        }

        let (saved, _, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            Some("pw".into()),
            "ccswitchx".into(),
            true,
            false,
            false,
            24,
        )
        .unwrap();
        assert_eq!(saved.cloud.upload_delay_hours, 24);
    }

    #[test]
    fn cloud_connection_changed_flag_tracks_endpoint_and_password() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");
        let save = |base_url: &str, username: &str, password: Option<&str>, remote_dir: &str| {
            set_cloud_settings_at(
                &config,
                &secrets_path,
                base_url.into(),
                username.into(),
                password.map(str::to_string),
                remote_dir.into(),
                true,
                false,
                false,
                1,
            )
            .map(|(_, _, changed)| changed)
            .unwrap()
        };

        assert!(
            save("https://dav.example.com/dav", "me", Some("pw"), "ccswitchx"),
            "首次保存连接参数应视为变化"
        );
        assert!(!save(
            "https://dav.example.com/dav",
            "me",
            Some("pw"),
            "ccswitchx"
        ));
        assert!(
            save("https://dav.example.com/dav", "me", Some("pw"), "backups"),
            "改 remote_dir 应视为变化"
        );
        assert!(
            save("https://dav.example.com/dav", "you", Some("pw"), "backups"),
            "改用户名应视为变化"
        );
        assert!(
            save(
                "https://dav2.example.com/dav",
                "you",
                Some("pw"),
                "backups"
            ),
            "改地址应视为变化"
        );
        assert!(
            save(
                "https://dav2.example.com/dav",
                "you",
                Some("new-pw"),
                "backups"
            ),
            "改密码应视为变化"
        );
        assert!(
            !save("https://dav2.example.com/dav", "you", None, "backups"),
            "密码传 None 表示沿用，不算变化"
        );
    }

    #[test]
    fn git_backup_without_remote_is_forced_off() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let backup = BackupSettings {
            git: crate::settings::GitBackupSettings {
                enabled: true,
                remote: None,
                ..Default::default()
            },
            ..Default::default()
        };
        let updated = set_backup_settings_at(&config, backup).unwrap();
        assert!(
            !updated.backup.git.enabled,
            "没有远端就无法验证，开启状态应被纠正"
        );
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

    #[test]
    fn close_to_tray_is_persisted() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let updated = set_close_to_tray_at(&config, false).unwrap();
        assert!(!updated.close_to_tray);
        assert!(!settings::load_from(&config).unwrap().close_to_tray);
    }

    #[test]
    fn proxy_password_lives_in_secrets_and_can_be_cleared() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        let (updated, secrets) = set_proxy_at(
            &config,
            &secrets_path,
            " http://127.0.0.1:7890 ".into(),
            " me ".into(),
            Some("p@ss".into()),
            true,
            true,
        )
        .unwrap();
        assert_eq!(updated.proxy.url.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(updated.proxy.username.as_deref(), Some("me"));
        assert!(updated.proxy.enabled);
        let saved = Secrets::load(&secrets_path).unwrap();
        assert_eq!(saved.get(&proxy_secret_ref()), Some("p@ss"));
        assert_eq!(
            proxy_view(&updated, &saved),
            ProxySettingsView {
                url: Some("http://127.0.0.1:7890".into()),
                username: Some("me".into()),
                has_password: true,
                enabled: true,
                webdav_uses_proxy: true,
            }
        );
        assert_eq!(secrets.get(&proxy_secret_ref()), Some("p@ss"));

        // None = 保留密码；空串 = 清除。
        let (kept, kept_secrets) = set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7891".into(),
            "me".into(),
            None,
            true,
            true,
        )
        .unwrap();
        assert_eq!(kept_secrets.get(&proxy_secret_ref()), Some("p@ss"));
        assert!(proxy_view(&kept, &kept_secrets).has_password);

        let (cleared, cleared_secrets) = set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7891".into(),
            "me".into(),
            Some(String::new()),
            true,
            true,
        )
        .unwrap();
        assert!(!proxy_view(&cleared, &cleared_secrets).has_password);

        let (empty, empty_secrets) = set_proxy_at(
            &config,
            &secrets_path,
            String::new(),
            String::new(),
            None,
            true,
            true,
        )
        .unwrap();
        assert!(empty.proxy.url.is_none());
        assert!(empty.proxy.username.is_none());
        assert!(!empty.proxy.enabled);
        assert!(!proxy_view(&empty, &empty_secrets).has_password);
    }

    #[test]
    fn webdav_proxy_switch_persists_independently_of_the_proxy_toggle() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        let (updated, _) = set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7890".into(),
            "me".into(),
            Some("p@ss".into()),
            true,
            false,
        )
        .unwrap();
        assert!(updated.proxy.enabled, "全局代理仍开着");
        assert!(!updated.proxy.webdav_uses_proxy, "WebDAV 可以单独不走代理");

        let (restored, _) = set_proxy_at(
            &config,
            &secrets_path,
            String::new(),
            String::new(),
            None,
            true,
            true,
        )
        .unwrap();
        assert!(restored.proxy.webdav_uses_proxy, "清空地址也保留这次选择");
    }

    #[test]
    fn disabling_proxy_keeps_address_and_credentials() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7890".into(),
            "me".into(),
            Some("p@ss".into()),
            true,
            true,
        )
        .unwrap();

        let (disabled, secrets) = set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7890".into(),
            "me".into(),
            None,
            false,
            true,
        )
        .unwrap();
        assert!(!disabled.proxy.enabled);
        assert_eq!(disabled.proxy.url.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(disabled.proxy.username.as_deref(), Some("me"));
        assert_eq!(secrets.get(&proxy_secret_ref()), Some("p@ss"));
        let view = proxy_view(&disabled, &secrets);
        assert!(!view.enabled);
        assert!(view.has_password);

        // 重新启用时地址与密码都还在，不需要重填。
        let (enabled, _) = set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7890".into(),
            "me".into(),
            None,
            true,
            true,
        )
        .unwrap();
        assert!(enabled.proxy.enabled);
        assert_eq!(enabled.proxy.url.as_deref(), Some("http://127.0.0.1:7890"));
    }

    #[test]
    fn proxy_input_is_validated_before_writing() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");
        assert!(set_proxy_at(
            &config,
            &secrets_path,
            "ftp://127.0.0.1:21".into(),
            String::new(),
            None,
            true,
            true
        )
        .is_err());
        assert!(
            set_proxy_at(
                &config,
                &secrets_path,
                "http://user:pw@127.0.0.1:7890".into(),
                String::new(),
                None,
                true,
                true
            )
            .is_err(),
            "地址里的凭据不能写进设置文件"
        );
        assert!(!config.exists(), "校验失败不能留下设置文件");
    }

    #[test]
    fn proxy_save_survives_a_partial_settings_file() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");
        // 外部工具改坏设置文件（只留下 theme）时，网络面板的「保存」不能整体失效。
        std::fs::write(&config, br#"{"theme":"dark"}"#).unwrap();

        let (updated, _) = set_proxy_at(
            &config,
            &secrets_path,
            "http://127.0.0.1:7890".into(),
            String::new(),
            None,
            true,
            true,
        )
        .unwrap();
        assert_eq!(updated.theme, Theme::Dark);
        assert_eq!(updated.proxy.url.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(settings::load_from(&config)
            .unwrap()
            .store_root
            .ends_with(".ccswitchx"));
    }

    #[test]
    fn skill_repos_are_normalized_and_deduplicated() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let updated = set_skill_repos_at(
            &config,
            vec![
                SkillRepo {
                    owner: "acme".into(),
                    name: "skills".into(),
                    branch: "  ".into(),
                    enabled: true,
                },
                SkillRepo {
                    owner: "https://github.com/Other/repo".into(),
                    name: "unused".into(),
                    branch: "dev".into(),
                    enabled: false,
                },
            ],
        );
        // 第二个输入按 owner/name 拼接后不合法，必须整体拒绝。
        assert!(updated.is_err());

        let updated = set_skill_repos_at(
            &config,
            vec![SkillRepo {
                owner: "acme".into(),
                name: "skills".into(),
                branch: "  ".into(),
                enabled: true,
            }],
        )
        .unwrap();
        assert_eq!(
            updated.skill_repos.as_deref(),
            Some(
                [SkillRepo {
                    owner: "acme".into(),
                    name: "skills".into(),
                    branch: "main".into(),
                    enabled: true,
                }]
                .as_slice()
            )
        );
        assert!(validate_skill_repos(vec![
            SkillRepo {
                owner: "acme".into(),
                name: "skills".into(),
                branch: "main".into(),
                enabled: true,
            },
            SkillRepo {
                owner: "ACME".into(),
                name: "Skills".into(),
                branch: "dev".into(),
                enabled: true,
            },
        ])
        .is_err());
    }

    #[test]
    fn cloud_password_semantics() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let secrets_path = dir.path().join("secrets.local.json");

        let (saved, secrets, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            Some("app-password".into()),
            "ccswitchx".into(),
            true,
            true,
            false,
            1,
        )
        .unwrap();
        assert!(saved.cloud.enabled);
        assert!(saved.cloud.auto_upload);
        assert_eq!(secrets.get(&cloud_secret_ref()), Some("app-password"));
        assert!(!std::fs::read_to_string(&config)
            .unwrap()
            .contains("app-password"));
        assert!(cloud_view(&saved, &secrets).has_password);

        let (kept, kept_secrets, _) = set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            None,
            "ccswitchx".into(),
            true,
            false,
            false,
            1,
        )
        .unwrap();
        assert_eq!(kept_secrets.get(&cloud_secret_ref()), Some("app-password"));
        assert!(kept.cloud.enabled);
        assert!(!kept.cloud.auto_upload);

        let cleared = set_cloud_settings_at(
            &config,
            &secrets_path,
            String::new(),
            String::new(),
            None,
            "ccswitchx".into(),
            true,
            true,
            true,
            1,
        )
        .unwrap();
        assert!(cleared.1.get(&cloud_secret_ref()).is_none());
        assert!(!cleared.0.cloud.enabled);
        assert!(!cleared.0.cloud.auto_upload);
        assert!(!cleared.0.cloud.auto_download);

        assert!(set_cloud_settings_at(
            &config,
            &secrets_path,
            "https://dav.example.com/dav".into(),
            "me".into(),
            Some(String::new()),
            "ccswitchx".into(),
            false,
            false,
            false,
            1,
        )
        .is_err());
    }

    #[test]
    fn cloud_url_and_remote_dir_validation() {
        assert_eq!(
            normalize_base_url(" https://dav.example.com/dav/ ").unwrap(),
            "https://dav.example.com/dav"
        );
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8099/dav").unwrap(),
            "http://127.0.0.1:8099/dav"
        );
        for bad in [
            "ftp://example.com/dav",
            "https://user:pass@example.com/dav",
            "https://example.com/dav?x=1",
        ] {
            assert!(normalize_base_url(bad).is_err(), "{bad}");
        }
        assert_eq!(validate_remote_dir("").unwrap(), "ccswitchx");
        assert_eq!(
            validate_remote_dir("backups/ccswitchx").unwrap(),
            "backups/ccswitchx"
        );
        for bad in ["../x", "/abs", "a\\b", "a//b", "a/../b", "a b"] {
            assert!(validate_remote_dir(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn remote_url_joins_base_path() {
        let url =
            remote_url("https://dav.example.com/dav", "ccswitchx", "ccswitchx-backup.zip").unwrap();
        assert_eq!(
            url.as_str(),
            "https://dav.example.com/dav/ccswitchx/ccswitchx-backup.zip"
        );
    }
}
