use crate::error::AppError;
use crate::net::ProxyConfig;
use crate::services::settings::{self as settings_service, ProxySettingsView};
use crate::settings::{self as settings_file, AppSettings, Theme};
use crate::state::AppState;
use crate::store::secrets::secrets_path;
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};
use tauri::State;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok(settings.clone())
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<AppSettings, AppError> {
    let updated = settings_service::set_theme(theme)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(updated)
}

/// 设置全局启用的 agent 范围；只影响 UI 与分发入口，不触碰已部署文件。
#[tauri::command]
pub fn set_enabled_agents(
    state: State<'_, AppState>,
    agent_ids: Vec<String>,
) -> Result<AppSettings, AppError> {
    let updated = settings_service::set_enabled_agents(agent_ids)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(updated)
}

/// 返回 true 表示设置已写入文件；store root 生效需要重启应用。
#[tauri::command]
pub fn set_store_root(
    state: State<'_, AppState>,
    store_root: std::path::PathBuf,
) -> Result<bool, AppError> {
    let updated = settings_service::set_store_root(store_root)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated;
    Ok(true)
}

#[tauri::command]
pub fn set_close_to_tray(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSettings, AppError> {
    let updated =
        settings_service::set_close_to_tray_at(&settings_file::settings_path()?, enabled)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(updated)
}

#[tauri::command]
pub fn get_proxy_settings(state: State<'_, AppState>) -> Result<ProxySettingsView, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    let secrets = state
        .secrets
        .lock()
        .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
    Ok(settings_service::proxy_view(&settings, &secrets))
}

/// `password` 为 None 表示沿用已保存的密码；空串表示清除密码。
#[tauri::command]
pub fn set_proxy_settings(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: Option<String>,
) -> Result<ProxySettingsView, AppError> {
    let (updated, secrets) = settings_service::set_proxy_at(
        &settings_file::settings_path()?,
        &secrets_path(state.store.root()),
        url,
        username,
        password,
    )?;
    let mut current = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *current = updated;
    let mut current_secrets = state
        .secrets
        .lock()
        .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
    *current_secrets = secrets;
    Ok(settings_service::proxy_view(&current, &current_secrets))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    pub success: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

fn test_proxy(config: &ProxyConfig) -> Result<ProxyTestResult, AppError> {
    config.validate()?;
    let client = config.apply_blocking(
        reqwest::blocking::Client::builder()
            .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(12)),
    )?;
    let client = client
        .build()
        .map_err(|error| AppError::Network(format!("创建测试客户端失败：{error}")))?;

    let start = Instant::now();
    let mut last_error = None;
    // GitHub 是 Skills / 应用更新的真实目标；两个入口任意一个返回响应就说明代理可用。
    for target in ["https://api.github.com", "https://github.com"] {
        match client.head(target).send() {
            Ok(_response) => {
                return Ok(ProxyTestResult {
                    success: true,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: None,
                });
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    Ok(ProxyTestResult {
        success: false,
        latency_ms: start.elapsed().as_millis() as u64,
        error: last_error,
    })
}

#[tauri::command]
pub async fn test_proxy_settings(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: Option<String>,
) -> Result<ProxyTestResult, AppError> {
    // 只有用户名没变时才复用已保存密码；改了用户名还沿用旧密码只会得到误导性的失败。
    let saved_password = if password.is_none() {
        let settings = state
            .settings
            .lock()
            .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
        let same_user = !username.trim().is_empty()
            && settings.proxy.username.as_deref() == Some(username.trim());
        if same_user {
            state
                .secrets
                .lock()
                .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?
                .get(&settings_service::proxy_secret_ref())
                .map(str::to_string)
        } else {
            None
        }
    } else {
        None
    };
    let config = ProxyConfig::new(
        Some(url),
        (!username.trim().is_empty()).then_some(username),
        password.or(saved_password),
    );
    tauri::async_runtime::spawn_blocking(move || test_proxy(&config))
        .await
        .map_err(|error| AppError::InvalidInput(format!("代理测试任务失败：{error}")))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedProxy {
    pub url: String,
    pub proxy_type: String,
    pub port: u16,
}

/// 常见本地代理端口；mixed 端口同时给出 http / socks5 两种地址。
#[tauri::command]
pub async fn scan_local_proxies() -> Vec<DetectedProxy> {
    tauri::async_runtime::spawn_blocking(|| {
        const PORTS: &[(u16, &str, bool)] = &[
            (7890, "http", true),
            (7891, "socks5", false),
            (1080, "socks5", false),
            (8080, "http", false),
            (8888, "http", false),
            (3128, "http", false),
            (10808, "socks5", false),
            (10809, "http", false),
        ];
        let mut found = Vec::new();
        for &(port, proxy_type, mixed) in PORTS {
            let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
            if TcpStream::connect_timeout(&addr.into(), Duration::from_millis(120)).is_err() {
                continue;
            }
            found.push(DetectedProxy {
                url: format!("{proxy_type}://127.0.0.1:{port}"),
                proxy_type: proxy_type.to_string(),
                port,
            });
            if mixed {
                let other = if proxy_type == "http" {
                    "socks5"
                } else {
                    "http"
                };
                found.push(DetectedProxy {
                    url: format!("{other}://127.0.0.1:{port}"),
                    proxy_type: other.to_string(),
                    port,
                });
            }
        }
        found
    })
    .await
    .unwrap_or_default()
}
