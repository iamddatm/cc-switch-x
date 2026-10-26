//! 云备份的远端状态、连接测试与运行状态文件。
//!
//! 网络请求统一持有 `cloud_ops` 串行锁；请求期间不持有 settings / secrets 锁。

use crate::error::AppError;
use crate::net::webdav::{WebDavClient, WebDavConfig};
use crate::services::settings::cloud_secret_ref;
use crate::state::AppState;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRuntimeStatus {
    pub last_upload_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    pub configured: bool,
    pub has_password: bool,
    pub auto_upload: bool,
    pub remote: Option<RemoteStatus>,
    pub remote_error: Option<String>,
    pub last_upload_at: Option<i64>,
    pub last_error: Option<String>,
}

pub fn status_path(store: &Store) -> PathBuf {
    store.root().join("state").join("cloud-backup.json")
}

/// 状态文件只是运行记录：读不到或损坏都按「从未上传」处理，不影响主流程。
pub fn load_runtime_status(store: &Store) -> CloudRuntimeStatus {
    match std::fs::read(status_path(store)) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(status) => status,
            Err(error) => {
                eprintln!("读取云备份状态失败：{error}");
                CloudRuntimeStatus::default()
            }
        },
        Err(error) => {
            if error.kind() != ErrorKind::NotFound {
                eprintln!("读取云备份状态失败：{error}");
            }
            CloudRuntimeStatus::default()
        }
    }
}

pub fn save_runtime_status(store: &Store, status: &CloudRuntimeStatus) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(status)?;
    write_atomic(&status_path(store), &bytes)?;
    Ok(())
}

pub fn status(state: &AppState) -> Result<CloudStatus, AppError> {
    let cloud = {
        let settings = state
            .settings
            .lock()
            .map_err(|_| AppError::Cloud("读取云备份设置失败".into()))?;
        settings.cloud.clone()
    };
    let password = {
        let secrets = state
            .secrets
            .lock()
            .map_err(|_| AppError::Cloud("读取云备份密钥失败".into()))?;
        secrets.get(&cloud_secret_ref()).map(str::to_string)
    };
    let has_password = password
        .as_deref()
        .is_some_and(|value| !value.is_empty());
    let configured = !cloud.base_url.trim().is_empty()
        && !cloud.username.trim().is_empty()
        && has_password;
    let runtime = load_runtime_status(&state.store);
    if !configured {
        return Ok(CloudStatus {
            configured: false,
            has_password,
            auto_upload: cloud.auto_upload,
            remote: None,
            remote_error: None,
            last_upload_at: runtime.last_upload_at,
            last_error: runtime.last_error,
        });
    }

    let proxy = state.proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;
    let client = WebDavClient::new(
        WebDavConfig {
            base_url: cloud.base_url,
            username: cloud.username,
            password: password.unwrap_or_default(),
            remote_dir: cloud.remote_dir,
        },
        Some(&proxy),
    )?;
    let (remote, remote_error) = match client.head() {
        Ok(remote) => (
            Some(RemoteStatus {
                exists: remote.exists,
                size_bytes: remote.size_bytes,
                modified_at: remote.modified_at,
                etag: remote.etag,
            }),
            None,
        ),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(CloudStatus {
        configured,
        has_password,
        auto_upload: cloud.auto_upload,
        remote,
        remote_error,
        last_upload_at: runtime.last_upload_at,
        last_error: runtime.last_error,
    })
}

/// 用表单里的候选连接参数测一次远端；密码 `None` 时沿用已保存的密钥。
pub fn test_connection(
    state: &AppState,
    base_url: &str,
    username: &str,
    password: Option<&str>,
    remote_dir: &str,
) -> Result<u64, AppError> {
    let password = match password {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => {
            let secrets = state
                .secrets
                .lock()
                .map_err(|_| AppError::Cloud("读取云备份密钥失败".into()))?;
            secrets
                .get(&cloud_secret_ref())
                .map(str::to_string)
                .unwrap_or_default()
        }
    };
    if password.trim().is_empty() {
        return Err(AppError::InvalidInput("请填写 WebDAV 密码".into()));
    }
    let proxy = state.proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;
    let client = WebDavClient::new(
        WebDavConfig {
            base_url: base_url.trim().to_string(),
            username: username.trim().to_string(),
            password,
            remote_dir: remote_dir.trim().to_string(),
        },
        Some(&proxy),
    )?;
    let started = Instant::now();
    client.test()?;
    Ok(started.elapsed().as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::webdav::test_support::FakeWebDav;
    use crate::settings::{AppSettings, CloudBackupSettings};
    use std::sync::Mutex;
    use tempfile::{tempdir, TempDir};

    static TEMP_DIRS: Mutex<Vec<TempDir>> = Mutex::new(Vec::new());

    fn config_for(server: &FakeWebDav) -> CloudBackupSettings {
        CloudBackupSettings {
            auto_upload: false,
            base_url: server.base_url(),
            username: "me".into(),
            remote_dir: "ccswitchx".into(),
        }
    }

    fn test_state(settings: CloudBackupSettings, password: Option<&str>) -> AppState {
        let dir = tempdir().unwrap();
        let settings = AppSettings {
            store_root: dir.path().join("store"),
            cloud: settings,
            ..AppSettings::default()
        };
        let state = AppState::init(settings).unwrap();
        if let Some(password) = password {
            state
                .secrets
                .lock()
                .unwrap()
                .insert(&cloud_secret_ref(), password);
        }
        TEMP_DIRS.lock().unwrap().push(dir);
        state
    }

    #[test]
    fn runtime_status_roundtrip_and_corrupt_file_is_default() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = CloudRuntimeStatus {
            last_upload_at: Some(123),
            last_error: Some("boom".into()),
        };
        save_runtime_status(&store, &status).unwrap();
        assert_eq!(load_runtime_status(&store), status);

        std::fs::write(status_path(&store), "{bad json").unwrap();
        assert_eq!(load_runtime_status(&store), CloudRuntimeStatus::default());
    }

    #[test]
    fn status_without_config_does_not_probe_network() {
        let state = test_state(CloudBackupSettings::default(), None);
        let status = status(&state).unwrap();
        assert!(!status.configured);
        assert!(status.remote.is_none());
        assert!(status.remote_error.is_none());
    }

    #[test]
    fn status_reports_remote_object() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"12345".to_vec());
        let state = test_state(config_for(&server), Some("pw"));
        let status = status(&state).unwrap();
        assert!(status.configured);
        assert!(status.remote.as_ref().unwrap().exists);
        assert_eq!(status.remote.as_ref().unwrap().size_bytes, Some(5));
    }

    #[test]
    fn status_probe_error_is_reported_without_failing() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state(config_for(&server), Some("wrong"));
        let status = status(&state).unwrap();
        assert!(status.remote_error.is_some());
        assert!(status.remote.is_none());
    }

    #[test]
    fn test_connection_rejects_wrong_password() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state(config_for(&server), Some("pw"));
        let error = test_connection(&state, &server.base_url(), "me", Some("wrong"), "ccswitchx")
            .unwrap_err()
            .to_string();
        assert!(error.contains("认证失败"));
        assert!(!error.contains("wrong"));
    }

    #[test]
    fn test_connection_returns_latency() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state(config_for(&server), Some("pw"));
        let latency =
            test_connection(&state, &server.base_url(), "me", Some("pw"), "ccswitchx").unwrap();
        assert!(latency < 5_000);
    }
}
