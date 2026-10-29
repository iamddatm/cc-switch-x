//! 云备份的远端状态、连接测试与运行状态文件。
//!
//! 网络请求统一持有 `cloud_ops` 串行锁；请求期间不持有 settings / secrets 锁。

use crate::error::AppError;
use crate::net::webdav::{
    RemoteObject, WebDavClient, WebDavConfig, BACKUP_OBJECT, MAX_CLOUD_BYTES,
};
use crate::net::ProxyConfig;
use crate::services::backup::{self, SnapshotKind};
use crate::services::settings::cloud_secret_ref;
use crate::settings::CloudBackupSettings;
use crate::state::AppState;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// 云端 manifest 的格式标识与协议版本；与快照包内部的 `manifest.json` 不是一回事：
/// 包内那份列文件，这份只描述包本身，供换机时不上传/不下载就先看清来源。
const CLOUD_MANIFEST_FORMAT: &str = "ccswitchx-cloud";
const CLOUD_MANIFEST_SCHEMA: u32 = 1;

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

/// 云端 manifest 的前端视图：只带展示需要的字段，哈希留在后端校验用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudManifestInfo {
    pub app_version: String,
    pub device_name: String,
    /// 上传时间，毫秒时间戳。
    pub created_at: i64,
    /// 包内文件数；manifest 没写时为 0。
    pub files: u64,
    pub schema: u32,
    /// format 与 schema 与本机一致时才为真；不兼容的包不能恢复。
    pub compatible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudArchive {
    name: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudManifest {
    format: String,
    schema: u32,
    app_version: String,
    device_name: String,
    created_at: i64,
    #[serde(default)]
    files: u64,
    archive: CloudArchive,
}

impl CloudManifest {
    fn compatible(&self) -> bool {
        self.format == CLOUD_MANIFEST_FORMAT && self.schema <= CLOUD_MANIFEST_SCHEMA
    }

    /// 恢复前的兼容闸：格式、协议版本、目标文件名任一对不上都不动本地数据。
    fn ensure_restorable(&self) -> Result<(), AppError> {
        if self.format != CLOUD_MANIFEST_FORMAT {
            return Err(AppError::Cloud(format!(
                "云端 manifest 不是本应用写的（format: {}）",
                self.format
            )));
        }
        if self.schema > CLOUD_MANIFEST_SCHEMA {
            return Err(AppError::Cloud(format!(
                "云端备份由更新版本的 CC Switch X 上传（manifest schema {}，本机最高 {CLOUD_MANIFEST_SCHEMA}），请先升级应用",
                self.schema
            )));
        }
        if self.archive.name != BACKUP_OBJECT {
            return Err(AppError::Cloud(format!(
                "云端 manifest 描述的包是 {}，不是 {BACKUP_OBJECT}",
                self.archive.name
            )));
        }
        Ok(())
    }

    fn info(&self) -> CloudManifestInfo {
        CloudManifestInfo {
            app_version: self.app_version.clone(),
            device_name: self.device_name.clone(),
            created_at: self.created_at,
            files: self.files,
            schema: self.schema,
            compatible: self.compatible(),
        }
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<CloudManifest, AppError> {
    serde_json::from_slice(bytes)
        .map_err(|error| AppError::Cloud(format!("云端 manifest 无法解析：{error}")))
}

/// 上传设备名：只用于让人分清是哪台机器传的包，取不到就写 Unknown Device。
fn device_name() -> String {
    ["CC_SWITCH_X_DEVICE_NAME", "COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .map(|value| value.chars().take(64).collect())
        .unwrap_or_else(|| "Unknown Device".to_string())
}

/// 分块算哈希：备份包可以到 512 MB，不能整个读进内存。
fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    pub configured: bool,
    pub has_password: bool,
    pub auto_upload: bool,
    pub remote: Option<RemoteStatus>,
    pub remote_error: Option<String>,
    /// 远端 manifest 视图；旧版上传的裸包没有 manifest，这里为 None。
    pub manifest: Option<CloudManifestInfo>,
    /// manifest 读到了但解析失败的原因；不影响远端包本身的可用性判断。
    pub manifest_error: Option<String>,
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
            manifest: None,
            manifest_error: None,
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
    // manifest 只影响展示与兼容提示：读不到就当旧版包，不能连累远端状态。
    // 只有确认远端有包才多读一次，网络不通时不把等待时间翻倍。
    let (manifest, manifest_error) = if remote.as_ref().is_some_and(|item| item.exists) {
        match client.get_manifest() {
            Ok(Some(bytes)) => match parse_manifest(&bytes) {
                Ok(manifest) => (Some(manifest.info()), None),
                Err(error) => (None, Some(error.to_string())),
            },
            Ok(None) => (None, None),
            Err(error) => (None, Some(error.to_string())),
        }
    } else {
        (None, None)
    };
    Ok(CloudStatus {
        configured,
        has_password,
        auto_upload: cloud.auto_upload,
        remote,
        remote_error,
        manifest,
        manifest_error,
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

fn status_from_remote(
    connection: &CloudConnection,
    remote: RemoteObject,
    manifest: Option<CloudManifestInfo>,
) -> CloudStatus {
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
        manifest,
        manifest_error: None,
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
    let result = (|| -> Result<(RemoteObject, CloudManifest), AppError> {
        let summary = backup::validate_archive_for_restore(archive)?;
        let size_bytes = fs::metadata(archive)?.len();
        let sha256 = sha256_file(archive)?;
        let client = client_for(connection, proxy)?;
        let remote = client.put(archive)?;

        // 包先落地、manifest 最后写；manifest 写失败就整次算失败，避免界面谎报成功。
        let manifest = CloudManifest {
            format: CLOUD_MANIFEST_FORMAT.into(),
            schema: CLOUD_MANIFEST_SCHEMA,
            app_version: env!("CARGO_PKG_VERSION").into(),
            device_name: device_name(),
            created_at: chrono::Utc::now().timestamp_millis(),
            files: summary.files,
            archive: CloudArchive {
                name: BACKUP_OBJECT.into(),
                size_bytes,
                sha256,
            },
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        client.put_manifest(&bytes)?;
        Ok((remote, manifest))
    })();
    match result {
        Ok((remote, manifest)) => {
            let mut runtime = load_runtime_status(&state.store);
            runtime.last_upload_at = Some(chrono::Utc::now().timestamp_millis());
            runtime.last_error = None;
            if let Err(error) = save_runtime_status(&state.store, &runtime) {
                eprintln!("保存云备份状态失败：{error}");
            }
            let mut status = status_from_remote(connection, remote, Some(manifest.info()));
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

    // 下载前先校 manifest：格式/版本不兼容就停手，不浪费一次整包下载。
    let manifest = match client.get_manifest()? {
        Some(bytes) => {
            let manifest = parse_manifest(&bytes)?;
            manifest.ensure_restorable()?;
            Some(manifest)
        }
        None => None,
    };

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
    if let Some(manifest) = &manifest {
        // manifest 最后写：哈希对不上说明包和描述不是同一次上传的产物（中途断网或外部替换）。
        let digest = sha256_file(&temp)?;
        if !digest.eq_ignore_ascii_case(&manifest.archive.sha256) {
            let _ = fs::remove_file(&temp);
            return Err(AppError::Cloud(
                "云端备份与 manifest 记录的哈希不一致：上次上传可能中断，请在源设备重新上传".into(),
            ));
        }
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

    #[test]
    fn upload_writes_manifest_describing_the_archive() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        let status = upload_now(&state).unwrap();

        let zip = server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap();
        let manifest_bytes = server.object("/dav/ccswitchx/manifest.json").unwrap();
        let manifest: CloudManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest.format, CLOUD_MANIFEST_FORMAT);
        assert_eq!(manifest.schema, CLOUD_MANIFEST_SCHEMA);
        assert_eq!(manifest.app_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(manifest.archive.name, BACKUP_OBJECT);
        assert_eq!(manifest.archive.size_bytes, zip.len() as u64);
        assert_eq!(manifest.archive.sha256, crate::index::sha256_hex(&zip));
        assert!(manifest.files >= 1);

        let info = status.manifest.expect("上传后应带回 manifest 视图");
        assert!(info.compatible);
        assert_eq!(info.files, manifest.files);
    }

    #[test]
    fn status_reports_manifest_of_uploaded_backup() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        upload_now(&state).unwrap();
        let status = status(&state).unwrap();
        let manifest = status.manifest.expect("远端应有 manifest");
        assert!(manifest.compatible);
        assert!(!manifest.device_name.is_empty());
        assert!(manifest.created_at > 0);
    }

    #[test]
    fn status_treats_missing_manifest_as_legacy_not_error() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"12345".to_vec());
        let state = test_state(config_for(&server), Some("pw"));
        let status = status(&state).unwrap();
        assert!(status.manifest.is_none());
        assert!(status.manifest_error.is_none());
        assert!(status.remote_error.is_none());
    }

    #[test]
    fn upload_fails_when_manifest_cannot_be_written() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        // 第一次 PUT 是备份包，第二次才是 manifest；manifest 写不进去时整次上传算失败。
        server.set_fail_put_after(1);
        let error = upload_now(&state).unwrap_err().to_string();
        assert!(error.contains("PUT"));
        assert!(load_runtime_status(&state.store).last_error.is_some());
        assert!(server.object("/dav/ccswitchx/manifest.json").is_none());
    }

    #[test]
    fn restore_rejects_archive_that_does_not_match_manifest() {
        let server = FakeWebDav::start("me", "pw");
        let source = test_state_with_asset(config_for(&server), Some("pw"));
        let snapshot = build_named_snapshot(&source, "cloud-source");
        upload_archive(&source, &snapshot).unwrap();

        // 用另一份包覆盖远端 zip，manifest 仍描述上传时那份。
        let other = test_state(config_for(&server), Some("pw"));
        other
            .store
            .write(
                crate::domain::AssetType::Mcp,
                "other",
                br#"{"id":"other","name":"Other","transport":"stdio","command":"x"}"#,
            )
            .unwrap();
        let other_snapshot = build_named_snapshot(&other, "cloud-other");
        server.put_object(
            "/dav/ccswitchx/ccswitchx-backup.zip",
            std::fs::read(other_snapshot).unwrap(),
        );

        let target = test_state(config_for(&server), Some("pw"));
        let error = restore_latest(&target).unwrap_err().to_string();
        assert!(error.contains("哈希不一致"));
        assert!(!target
            .store
            .root()
            .join("agents")
            .join("claude.json")
            .exists());
    }

    #[test]
    fn restore_rejects_foreign_and_newer_manifests() {
        for (manifest, expected) in [
            (
                r#"{"format":"other-app","schema":1,"appVersion":"1.0.0","deviceName":"x","createdAt":1,"files":1,"archive":{"name":"ccswitchx-backup.zip","sizeBytes":1,"sha256":"00"}}"#,
                "不是本应用写的",
            ),
            (
                r#"{"format":"ccswitchx-cloud","schema":99,"appVersion":"9.9.9","deviceName":"x","createdAt":1,"files":1,"archive":{"name":"ccswitchx-backup.zip","sizeBytes":1,"sha256":"00"}}"#,
                "更新版本",
            ),
        ] {
            let server = FakeWebDav::start("me", "pw");
            let source = test_state_with_asset(config_for(&server), Some("pw"));
            let snapshot = build_named_snapshot(&source, "cloud-source");
            server.put_object(
                "/dav/ccswitchx/ccswitchx-backup.zip",
                std::fs::read(snapshot).unwrap(),
            );
            server.put_object("/dav/ccswitchx/manifest.json", manifest.as_bytes().to_vec());

            let target = test_state(config_for(&server), Some("pw"));
            let error = restore_latest(&target).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
}
