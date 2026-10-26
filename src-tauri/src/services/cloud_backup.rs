//! 云备份的远端状态、连接测试与运行状态文件。
//!
//! 网络请求统一持有 `cloud_ops` 串行锁；请求期间不持有 settings / secrets 锁。

use crate::error::AppError;
use crate::net::webdav::{RemoteObject, WebDavClient, WebDavConfig, MAX_CLOUD_BYTES};
use crate::net::ProxyConfig;
use crate::services::backup::{self, SnapshotKind};
use crate::services::settings::cloud_secret_ref;
use crate::settings::CloudBackupSettings;
use crate::state::AppState;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRestoreReport {
    pub safety_backup: String,
    pub restored_files: u64,
    pub removed_files: u64,
    pub downloaded_snapshot: String,
    pub remote_size_bytes: u64,
    pub remote_modified_at: Option<i64>,
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

struct CloudConnection {
    cloud: CloudBackupSettings,
    password: String,
}

fn read_connection(state: &AppState) -> Result<CloudConnection, AppError> {
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
        secrets
            .get(&cloud_secret_ref())
            .map(str::to_string)
            .unwrap_or_default()
    };
    if cloud.base_url.trim().is_empty()
        || cloud.username.trim().is_empty()
        || password.trim().is_empty()
    {
        return Err(AppError::InvalidInput(
            "请先配置完整的 WebDAV 云备份".into(),
        ));
    }
    Ok(CloudConnection { cloud, password })
}

fn client_for(
    connection: &CloudConnection,
    proxy: &ProxyConfig,
) -> Result<WebDavClient, AppError> {
    WebDavClient::new(
        WebDavConfig {
            base_url: connection.cloud.base_url.clone(),
            username: connection.cloud.username.clone(),
            password: connection.password.clone(),
            remote_dir: connection.cloud.remote_dir.clone(),
        },
        Some(proxy),
    )
}

fn record_failure(state: &AppState, error: &AppError) {
    let mut runtime = load_runtime_status(&state.store);
    runtime.last_error = Some(error.to_string());
    if let Err(save_error) = save_runtime_status(&state.store, &runtime) {
        eprintln!("保存云备份状态失败：{save_error}");
    }
}

fn status_from_remote(connection: &CloudConnection, remote: RemoteObject) -> CloudStatus {
    CloudStatus {
        configured: true,
        has_password: true,
        auto_upload: connection.cloud.auto_upload,
        remote: Some(RemoteStatus {
            exists: remote.exists,
            size_bytes: remote.size_bytes,
            modified_at: remote.modified_at,
            etag: remote.etag,
        }),
        remote_error: None,
        last_upload_at: None,
        last_error: None,
    }
}

/// 手动上传：在 `maintenance` 写锁下打包当前真源，释放锁后再联网。
pub fn upload_now(state: &AppState) -> Result<CloudStatus, AppError> {
    let connection = read_connection(state)?;
    let proxy = state.proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;

    let staging = state.store.staging_dir();
    fs::create_dir_all(&staging)?;
    let archive = staging.join(format!(
        "cloud-{}.zip",
        chrono::Utc::now().timestamp_millis()
    ));
    let build = {
        let _maintenance = state.enter_maintenance()?;
        backup::write_archive_to(&state.store, &archive, SnapshotKind::Manual)
    };
    let result = match build {
        Ok(_) => upload_archive_locked(state, &connection, &proxy, &archive),
        Err(error) => {
            record_failure(state, &error);
            let _ = fs::remove_file(&archive);
            Err(error)
        }
    };
    let _ = fs::remove_file(&archive);
    result
}

/// 自动上传入口：直接上传已有快照，调用方无需持有 `cloud_ops`。
pub fn upload_archive(state: &AppState, archive: &Path) -> Result<CloudStatus, AppError> {
    let connection = read_connection(state)?;
    let proxy = state.proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;
    upload_archive_locked(state, &connection, &proxy, archive)
}

fn upload_archive_locked(
    state: &AppState,
    connection: &CloudConnection,
    proxy: &ProxyConfig,
    archive: &Path,
) -> Result<CloudStatus, AppError> {
    let result = (|| -> Result<RemoteObject, AppError> {
        backup::validate_archive_for_restore(archive)?;
        let client = client_for(connection, proxy)?;
        client.put(archive)
    })();
    match result {
        Ok(remote) => {
            let mut runtime = load_runtime_status(&state.store);
            runtime.last_upload_at = Some(chrono::Utc::now().timestamp_millis());
            runtime.last_error = None;
            if let Err(error) = save_runtime_status(&state.store, &runtime) {
                eprintln!("保存云备份状态失败：{error}");
            }
            let mut status = status_from_remote(connection, remote);
            status.last_upload_at = runtime.last_upload_at;
            status.last_error = runtime.last_error;
            Ok(status)
        }
        Err(error) => {
            record_failure(state, &error);
            Err(error)
        }
    }
}

fn unique_restore_name(store: &Store) -> String {
    let base = format!(
        "cloud_restore_{}",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let dir = backup::snapshots_dir(store);
    let mut candidate = base.clone();
    let mut suffix = 1;
    while dir.join(format!("{candidate}.zip")).exists() {
        suffix += 1;
        candidate = format!("{base}_{suffix}");
    }
    candidate
}

/// 下载云端最新包，走与本地快照相同的安全恢复流水线；不做自动保留清理。
pub fn restore_latest(state: &AppState) -> Result<CloudRestoreReport, AppError> {
    let connection = read_connection(state)?;
    let proxy = state.proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;
    let client = client_for(&connection, &proxy)?;

    let remote = client.head()?;
    if !remote.exists {
        return Err(AppError::Cloud("云端还没有备份，请先上传".into()));
    }
    if remote
        .size_bytes
        .is_some_and(|size| size > MAX_CLOUD_BYTES)
    {
        return Err(AppError::Cloud(format!(
            "云端备份超过下载上限（{MAX_CLOUD_BYTES} 字节）"
        )));
    }

    let staging = state.store.staging_dir();
    fs::create_dir_all(&staging)?;
    let temp = staging.join(format!(
        "cloud-{}.zip",
        chrono::Utc::now().timestamp_millis()
    ));
    let downloaded = match client.get_to_file(&temp, MAX_CLOUD_BYTES) {
        Ok(downloaded) => downloaded,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    };
    if let Err(error) = backup::validate_archive_for_restore(&temp) {
        let _ = fs::remove_file(&temp);
        return Err(AppError::Cloud(format!("云端备份无效：{error}")));
    }

    let name = unique_restore_name(&state.store);
    let target = backup::snapshots_dir(&state.store).join(format!("{name}.zip"));
    fs::create_dir_all(backup::snapshots_dir(&state.store))?;

    let restored = {
        let _maintenance = state.enter_maintenance()?;
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::Cloud("索引锁失败".into()))?;
        if let Err(error) = fs::rename(&temp, &target) {
            let _ = fs::remove_file(&temp);
            return Err(AppError::Cloud(format!("保存云端备份失败：{error}")));
        }
        let result = backup::restore(&state.store, &mut index, &name);
        state.notify_git("restore: cloud");
        result
    };
    let _ = fs::remove_file(&temp);

    match restored {
        Ok(report) => Ok(CloudRestoreReport {
            safety_backup: report.safety_backup,
            restored_files: report.restored_files,
            removed_files: report.removed_files,
            downloaded_snapshot: name,
            remote_size_bytes: downloaded
                .size_bytes
                .or(remote.size_bytes)
                .unwrap_or(0),
            remote_modified_at: remote.modified_at,
        }),
        Err(error) => Err(error),
    }
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

    fn test_state_with_asset(settings: CloudBackupSettings, password: Option<&str>) -> AppState {
        let state = test_state(settings, password);
        state
            .store
            .write(
                crate::domain::AssetType::Mcp,
                "pw",
                br#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
            )
            .unwrap();
        state
    }

    fn build_named_snapshot(state: &AppState, name: &str) -> std::path::PathBuf {
        backup::create(&state.store, Some(name), SnapshotKind::Manual).unwrap();
        backup::snapshots_dir(&state.store).join(format!("{name}.zip"))
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

    #[test]
    fn upload_now_writes_remote_and_status() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        let status = upload_now(&state).unwrap();
        assert!(server
            .object("/dav/ccswitchx/ccswitchx-backup.zip")
            .is_some());
        assert!(status.remote.as_ref().unwrap().exists);
        assert!(status.last_upload_at.is_some());
        assert_eq!(load_runtime_status(&state.store).last_error, None);
    }

    #[test]
    fn upload_rejects_empty_store() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"existing".to_vec());
        let state = test_state(config_for(&server), Some("pw"));
        let error = upload_now(&state).unwrap_err().to_string();
        assert!(error.contains("没有可备份的资产"));
        assert_eq!(
            server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(),
            b"existing"
        );
    }

    #[test]
    fn upload_failure_records_last_error() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("wrong"));
        assert!(upload_now(&state).is_err());
        let runtime = load_runtime_status(&state.store);
        assert!(runtime.last_error.is_some());
        assert!(runtime.last_upload_at.is_none());
    }

    #[test]
    fn upload_now_cleans_temp_archive() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        upload_now(&state).unwrap();
        let staging = state.store.root().join("state/staging");
        let leftovers = std::fs::read_dir(&staging)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("cloud-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn restore_latest_restores_assets_and_creates_safety_snapshot() {
        let server = FakeWebDav::start("me", "pw");
        let source = test_state_with_asset(config_for(&server), Some("pw"));
        let snapshot = build_named_snapshot(&source, "cloud-source");
        server.put_object(
            "/dav/ccswitchx/ccswitchx-backup.zip",
            std::fs::read(snapshot).unwrap(),
        );

        let target = test_state(config_for(&server), Some("pw"));
        std::fs::write(
            target.store.root().join("local/agents/sentinel.json"),
            "keep",
        )
        .unwrap();
        let report = restore_latest(&target).unwrap();

        assert!(target
            .store
            .asset_dir(crate::domain::AssetType::Mcp)
            .join("pw.json")
            .is_file());
        assert!(!report.safety_backup.is_empty());
        assert!(report.downloaded_snapshot.starts_with("cloud_restore_"));
        assert_eq!(
            std::fs::read_to_string(target.store.root().join("local/agents/sentinel.json")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn restore_missing_remote_is_friendly_error() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state(config_for(&server), Some("pw"));
        let error = restore_latest(&state).unwrap_err().to_string();
        assert!(error.contains("云端还没有备份"));
    }

    #[test]
    fn restore_rejects_foreign_archive() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"not a zip".to_vec());
        let state = test_state(config_for(&server), Some("pw"));
        assert!(restore_latest(&state).is_err());
        assert!(!state.store.root().join("agents").join("claude.json").exists());
    }

    #[test]
    fn restore_rejects_oversize_remote_without_downloading() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"x".to_vec());
        server.set_head_size_override(Some(MAX_CLOUD_BYTES + 1));
        let state = test_state(config_for(&server), Some("pw"));
        let before = server.request_count("GET");
        assert!(restore_latest(&state).is_err());
        assert_eq!(server.request_count("GET"), before);
    }
}
