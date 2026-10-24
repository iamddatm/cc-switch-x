use crate::domain::agent::builtin_agents;
use crate::domain::secret::SecretRef;
use crate::error::AppError;
use crate::net::{parse_github_input, ProxyConfig};
use crate::settings::{self, AppSettings, SkillRepo, Theme};
use crate::store::secrets::Secrets;
use serde::Serialize;
use std::collections::HashSet;

/// 代理密码与 MCP 等资产用同一套本地密钥文件分开存放。
pub const PROXY_SECRET_SCOPE: &str = "proxy";
pub const PROXY_SECRET_KEY: &str = "password";
pub const MAX_SKILL_REPOS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettingsView {
    pub url: Option<String>,
    pub username: Option<String>,
    pub has_password: bool,
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
        url,
        username,
        has_password,
    }
}

/// 密码语义：None = 保留旧密码，Some("") = 清除，Some(v) = 覆盖。
pub fn set_proxy_at(
    config_path: &std::path::Path,
    secrets_path: &std::path::Path,
    url: String,
    username: String,
    password: Option<String>,
) -> Result<(AppSettings, Secrets), AppError> {
    let mut current = settings::load_from(config_path)?;
    let mut secrets = Secrets::load(secrets_path)?;
    let url = url.trim();
    let username = username.trim();

    if url.is_empty() {
        // 清空地址就是回到直连，认证信息一并清掉，避免留下无法使用的残留。
        current.proxy.url = None;
        current.proxy.username = None;
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
        );
        config.validate()?;
        current.proxy.url = Some(url.to_string());
        current.proxy.username = username;
        if let Some(value) = next_password {
            secrets.insert(&proxy_secret_ref(), &value);
        }
    }

    secrets.save(secrets_path)?;
    settings::save_to(config_path, &current)?;
    Ok((current, secrets))
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
        )
        .unwrap();
        assert_eq!(updated.proxy.url.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(updated.proxy.username.as_deref(), Some("me"));
        let saved = Secrets::load(&secrets_path).unwrap();
        assert_eq!(saved.get(&proxy_secret_ref()), Some("p@ss"));
        assert_eq!(
            proxy_view(&updated, &saved),
            ProxySettingsView {
                url: Some("http://127.0.0.1:7890".into()),
                username: Some("me".into()),
                has_password: true,
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
        )
        .unwrap();
        assert!(!proxy_view(&cleared, &cleared_secrets).has_password);

        let (empty, empty_secrets) =
            set_proxy_at(&config, &secrets_path, String::new(), String::new(), None).unwrap();
        assert!(empty.proxy.url.is_none());
        assert!(empty.proxy.username.is_none());
        assert!(!proxy_view(&empty, &empty_secrets).has_password);
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
            None
        )
        .is_err());
        assert!(
            set_proxy_at(
                &config,
                &secrets_path,
                "http://user:pw@127.0.0.1:7890".into(),
                String::new(),
                None
            )
            .is_err(),
            "地址里的凭据不能写进设置文件"
        );
        assert!(!config.exists(), "校验失败不能留下设置文件");
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
}
