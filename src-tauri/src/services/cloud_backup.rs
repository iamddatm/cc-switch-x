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

/// 一次远端元数据观测：只记录对象本身，不涉及包内容。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedRemote {
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub etag: Option<String>,
    /// 观测时间，毫秒时间戳。
    pub checked_at: i64,
}

impl CachedRemote {
    pub fn from_remote(remote: &RemoteObject, checked_at: i64) -> Self {
        Self {
            exists: remote.exists,
            size_bytes: remote.size_bytes,
            modified_at: remote.modified_at,
            etag: remote.etag.clone(),
            checked_at,
        }
    }

    /// 与一次新探测结果比对；`checked_at` 不参与，它只说明这份缓存有多旧。
    fn matches_remote(&self, remote: &RemoteObject) -> bool {
        self.exists == remote.exists
            && self.size_bytes == remote.size_bytes
            && self.modified_at == remote.modified_at
            && self.etag == remote.etag
    }
}

/// 运行状态文件分两组字段，写入边界不同：
/// - `synced_*`：只在同步动作成功后写，自动任务据此判断「是否已同步」；
/// - `observed_*`：实时状态探测可写，只用于展示与省一次 manifest 请求。
///
/// 硬规则：实时状态探测绝不能写 `synced_*`，否则一次开面板就会让自动下载误以为云端已同步。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRuntimeStatus {
    // ── 同步基线 ──
    /// 云端那份包当前的内容指纹（见 `backup::content_fingerprint`）。
    /// 旧状态文件没这个字段按未知处理，退化成多传一次覆盖，不会丢数据。
    #[serde(default)]
    pub synced_fingerprint: Option<String>,
    /// 云端包的 sha256（来自 manifest 或下载结果）。
    #[serde(default)]
    pub synced_sha256: Option<String>,
    /// 同步成功时云端的元数据，供自动下载判断「云端是否变了」。
    #[serde(default)]
    pub synced_meta: Option<CachedRemote>,
    #[serde(default)]
    pub last_upload_at: Option<i64>,
    #[serde(default)]
    pub last_download_at: Option<i64>,
    /// 自动上传的最近错误。
    #[serde(default)]
    pub last_error: Option<String>,
    /// 自动下载的最近错误。
    #[serde(default)]
    pub last_download_error: Option<String>,
    // ── 观测缓存 ──
    #[serde(default)]
    pub observed_remote: Option<CachedRemote>,
    #[serde(default)]
    pub observed_manifest: Option<CloudManifestInfo>,
    #[serde(default)]
    pub observed_manifest_error: Option<String>,
    // ── 运行控制与统计 ──
    #[serde(default)]
    pub auth_paused_until: Option<i64>,
    /// 统计月份，如 `2026-11`；月份变化后上传量从 0 重新累加。
    #[serde(default)]
    pub upload_month: Option<String>,
    #[serde(default)]
    pub uploaded_bytes_month: u64,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub auto_download: bool,
    pub upload_delay_hours: u32,
    pub remote: Option<RemoteStatus>,
    pub remote_error: Option<String>,
    /// 远端 manifest 视图；旧版上传的裸包没有 manifest，这里为 None。
    pub manifest: Option<CloudManifestInfo>,
    /// manifest 读到了但解析失败的原因；不影响远端包本身的可用性判断。
    pub manifest_error: Option<String>,
    pub last_upload_at: Option<i64>,
    pub last_download_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_download_error: Option<String>,
    /// 本机本自然月累计上传字节；月份变了从头计。
    pub uploaded_bytes_month: u64,
    /// true 表示这份状态来自本地运行状态文件，没有联网探测。
    pub from_cache: bool,
}

impl From<&CachedRemote> for RemoteStatus {
    fn from(cached: &CachedRemote) -> Self {
        Self {
            exists: cached.exists,
            size_bytes: cached.size_bytes,
            modified_at: cached.modified_at,
            etag: cached.etag.clone(),
        }
    }
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

/// 连接参数变化后作废旧远端的所有同步基线与观测缓存。
pub fn reset_runtime_status(store: &Store) -> Result<(), AppError> {
    match fs::remove_file(status_path(store)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// 读设置与密钥但不联网，组装一份可用于「秒开面板」的状态。
pub fn status_cached(state: &AppState) -> Result<CloudStatus, AppError> {
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
    Ok(CloudStatus {
        configured,
        has_password,
        auto_upload: cloud.auto_upload,
        auto_download: cloud.auto_download,
        upload_delay_hours: cloud.upload_delay_hours,
        remote: if configured {
            runtime.observed_remote.as_ref().map(RemoteStatus::from)
        } else {
            None
        },
        remote_error: None,
        manifest: if configured {
            runtime.observed_manifest.clone()
        } else {
            None
        },
        manifest_error: if configured {
            runtime.observed_manifest_error.clone()
        } else {
            None
        },
        last_upload_at: runtime.last_upload_at,
        last_download_at: runtime.last_download_at,
        last_error: runtime.last_error,
        last_download_error: runtime.last_download_error,
        uploaded_bytes_month: runtime.uploaded_bytes_month,
        from_cache: true,
    })
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
            auto_download: cloud.auto_download,
            upload_delay_hours: cloud.upload_delay_hours,
            remote: None,
            remote_error: None,
            manifest: None,
            manifest_error: None,
            last_upload_at: runtime.last_upload_at,
            last_download_at: runtime.last_download_at,
            last_error: runtime.last_error,
            last_download_error: runtime.last_download_error,
            uploaded_bytes_month: runtime.uploaded_bytes_month,
            from_cache: false,
        });
    }

    let proxy = state.webdav_proxy_config()?;
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
        proxy.as_ref(),
    )?;
    let mut runtime = runtime;
    let remote = match client.head() {
        Ok(remote) => remote,
        Err(error) => {
            // 探测失败不是观测结果：保留上一次缓存，只把错误报给界面。
            return Ok(CloudStatus {
                configured,
                has_password,
                auto_upload: cloud.auto_upload,
                auto_download: cloud.auto_download,
                upload_delay_hours: cloud.upload_delay_hours,
                remote: None,
                remote_error: Some(error.to_string()),
                manifest: None,
                manifest_error: None,
                last_upload_at: runtime.last_upload_at,
                last_download_at: runtime.last_download_at,
                last_error: runtime.last_error,
                last_download_error: runtime.last_download_error,
                uploaded_bytes_month: runtime.uploaded_bytes_month,
                from_cache: false,
            });
        }
    };

    // manifest 只影响展示与兼容提示：读不到就当旧版包，不能连累远端状态。
    // 元数据与上次观测一致时直接复用缓存，省一次 GET；只有确认远端有包才需要 manifest。
    let metadata_unchanged = remote.exists
        && runtime
            .observed_remote
            .as_ref()
            .is_some_and(|cached| cached.matches_remote(&remote));
    let (manifest, manifest_error) = if !remote.exists {
        (None, None)
    } else if metadata_unchanged {
        (
            runtime.observed_manifest.clone(),
            runtime.observed_manifest_error.clone(),
        )
    } else {
        match client.get_manifest() {
            Ok(Some(bytes)) => match parse_manifest(&bytes) {
                Ok(manifest) => (Some(manifest.info()), None),
                Err(error) => (None, Some(error.to_string())),
            },
            Ok(None) => (None, None),
            Err(error) => (None, Some(error.to_string())),
        }
    };
    // 成功探测只刷新观测缓存，并解除认证暂停；同步基线一概不动。
    let cached_remote = CachedRemote::from_remote(&remote, chrono::Utc::now().timestamp_millis());
    let remote_status = RemoteStatus::from(&cached_remote);
    runtime.observed_remote = Some(cached_remote);
    runtime.observed_manifest = manifest.clone();
    runtime.observed_manifest_error = manifest_error.clone();
    runtime.auth_paused_until = None;
    if let Err(error) = save_runtime_status(&state.store, &runtime) {
        eprintln!("保存云备份状态失败：{error}");
    }
    Ok(CloudStatus {
        configured,
        has_password,
        auto_upload: cloud.auto_upload,
        auto_download: cloud.auto_download,
        upload_delay_hours: cloud.upload_delay_hours,
        remote: Some(remote_status),
        remote_error: None,
        manifest,
        manifest_error,
        last_upload_at: runtime.last_upload_at,
        last_download_at: runtime.last_download_at,
        last_error: runtime.last_error,
        last_download_error: runtime.last_download_error,
        uploaded_bytes_month: runtime.uploaded_bytes_month,
        from_cache: false,
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
    let proxy = state.webdav_proxy_config()?;
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
        proxy.as_ref(),
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
    proxy: Option<&ProxyConfig>,
) -> Result<WebDavClient, AppError> {
    WebDavClient::new(
        WebDavConfig {
            base_url: connection.cloud.base_url.clone(),
            username: connection.cloud.username.clone(),
            password: connection.password.clone(),
            remote_dir: connection.cloud.remote_dir.clone(),
        },
        proxy,
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
    runtime: &CloudRuntimeStatus,
    remote: RemoteObject,
    manifest: Option<CloudManifestInfo>,
) -> CloudStatus {
    CloudStatus {
        configured: true,
        has_password: true,
        auto_upload: connection.cloud.auto_upload,
        auto_download: connection.cloud.auto_download,
        upload_delay_hours: connection.cloud.upload_delay_hours,
        remote: Some(RemoteStatus {
            exists: remote.exists,
            size_bytes: remote.size_bytes,
            modified_at: remote.modified_at,
            etag: remote.etag,
        }),
        remote_error: None,
        manifest,
        manifest_error: None,
        last_upload_at: runtime.last_upload_at,
        last_download_at: runtime.last_download_at,
        last_error: runtime.last_error.clone(),
        last_download_error: runtime.last_download_error.clone(),
        uploaded_bytes_month: runtime.uploaded_bytes_month,
        from_cache: false,
    }
}

/// 手动上传：在 `maintenance` 写锁下打包当前真源，释放锁后再联网。
pub fn upload_now(state: &AppState) -> Result<CloudStatus, AppError> {
    upload_current(state, SnapshotKind::Manual)
}

/// 把「当前真源」打成临时包直接上传，不新建本地快照；调用方无需持有 `cloud_ops`。
/// 云端只有一份包，它代表上传那一刻的真源，和快照列表彼此独立。
pub fn upload_current(state: &AppState, kind: SnapshotKind) -> Result<CloudStatus, AppError> {
    let connection = read_connection(state)?;
    let proxy = state.webdav_proxy_config()?;
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
        backup::write_archive_to(&state.store, &archive, kind)
    };
    let result = match build {
        Ok(_) => upload_archive_locked(state, &connection, proxy.as_ref(), &archive),
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
    let proxy = state.webdav_proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;
    upload_archive_locked(state, &connection, proxy.as_ref(), archive)
}

fn upload_archive_locked(
    state: &AppState,
    connection: &CloudConnection,
    proxy: Option<&ProxyConfig>,
    archive: &Path,
) -> Result<CloudStatus, AppError> {
    let result = (|| -> Result<(RemoteObject, CloudManifest, String), AppError> {
        let summary = backup::validate_archive_for_restore(archive)?;
        let size_bytes = fs::metadata(archive)?.len();
        let sha256 = sha256_file(archive)?;
        let content_fingerprint = backup::fingerprint_of_archive(archive)?;
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
        Ok((remote, manifest, content_fingerprint))
    })();
    match result {
        Ok((remote, manifest, content_fingerprint)) => {
            let now = chrono::Utc::now().timestamp_millis();
            let mut runtime = load_runtime_status(&state.store);
            runtime.last_upload_at = Some(now);
            runtime.last_error = None;
            // 上传成功是「同步动作」：同步基线与观测缓存此刻一致，一起写。
            runtime.synced_fingerprint = Some(content_fingerprint);
            runtime.synced_sha256 = Some(manifest.archive.sha256.clone());
            let cached_remote = CachedRemote::from_remote(&remote, now);
            runtime.synced_meta = Some(cached_remote.clone());
            runtime.observed_remote = Some(cached_remote);
            runtime.observed_manifest = Some(manifest.info());
            runtime.observed_manifest_error = None;
            // 本月上传量按自然月累计；跨月第一笔从 0 开始。
            let month = chrono::Local::now().format("%Y-%m").to_string();
            if runtime.upload_month.as_deref() != Some(month.as_str()) {
                runtime.upload_month = Some(month);
                runtime.uploaded_bytes_month = 0;
            }
            runtime.uploaded_bytes_month = runtime
                .uploaded_bytes_month
                .saturating_add(manifest.archive.size_bytes);
            if let Err(error) = save_runtime_status(&state.store, &runtime) {
                eprintln!("保存云备份状态失败：{error}");
            }
            Ok(status_from_remote(
                connection,
                &runtime,
                remote,
                Some(manifest.info()),
            ))
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
///
/// `on_progress(已下载字节, 总字节)` 用于把下载进度交给 UI；总字节未知时传 0。
pub fn restore_latest(
    state: &AppState,
    on_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<CloudRestoreReport, AppError> {
    let connection = read_connection(state)?;
    let proxy = state.webdav_proxy_config()?;
    let _guard = state
        .cloud_ops
        .lock()
        .map_err(|_| AppError::Cloud("云备份任务锁失败".into()))?;
    let client = client_for(&connection, proxy.as_ref())?;

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
    let downloaded = match client.get_to_file(&temp, MAX_CLOUD_BYTES, on_progress) {
        Ok(downloaded) => downloaded,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    };
    let summary = match backup::validate_archive_for_restore(&temp) {
        Ok(summary) => summary,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(AppError::Cloud(format!("云端备份无效：{error}")));
        }
    };
    if let Some(manifest) = &manifest {
        // manifest 最后写：哈希、大小、文件数任一与包对不上，都说明包和描述不是同一次上传的
        // 产物（中途断网或外部替换）。哈希已随下载算出，这里不用再读一遍文件。
        let downloaded_bytes = downloaded.remote.size_bytes.unwrap_or(0);
        let size_mismatch = manifest.archive.size_bytes != downloaded_bytes;
        let files_mismatch = manifest.files != summary.files;
        let hash_mismatch = !downloaded
            .sha256
            .eq_ignore_ascii_case(&manifest.archive.sha256);
        if hash_mismatch || size_mismatch || files_mismatch {
            let reason = if size_mismatch {
                format!(
                    "大小对不上（包内 {downloaded_bytes} 字节，manifest 记录 {} 字节）",
                    manifest.archive.size_bytes
                )
            } else if files_mismatch {
                format!(
                    "文件数对不上（包内 {} 个，manifest 记录 {} 个）",
                    summary.files, manifest.files
                )
            } else {
                "内容哈希对不上".to_string()
            };
            let _ = fs::remove_file(&temp);
            return Err(AppError::Cloud(format!(
                "云端备份与 manifest 不一致：{reason}。上次上传可能中断，请在源设备重新上传"
            )));
        }
    }
    // 包还叫 temp 时先读内容指纹：恢复成功后要用它把云端状态标成「和本地一致」，
    // 免得换机恢复完立刻把同样的内容原样传回去。
    let content_fingerprint = match backup::fingerprint_of_archive(&temp) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(AppError::Cloud(format!("云端备份无效：{error}")));
        }
    };

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
        Ok(report) => {
            let now = chrono::Utc::now().timestamp_millis();
            let mut runtime = load_runtime_status(&state.store);
            // 恢复成功也等于一次同步：本地内容从此与云端一致。
            runtime.synced_fingerprint = Some(content_fingerprint);
            runtime.synced_sha256 = Some(downloaded.sha256.clone());
            let cached_remote = CachedRemote::from_remote(&remote, now);
            runtime.synced_meta = Some(cached_remote.clone());
            runtime.observed_remote = Some(cached_remote);
            runtime.observed_manifest = manifest.as_ref().map(CloudManifest::info);
            runtime.observed_manifest_error = None;
            if let Err(error) = save_runtime_status(&state.store, &runtime) {
                eprintln!("保存云备份状态失败：{error}");
            }
            Ok(CloudRestoreReport {
                safety_backup: report.safety_backup,
                restored_files: report.restored_files,
                removed_files: report.removed_files,
                downloaded_snapshot: name,
                remote_size_bytes: downloaded
                    .remote
                    .size_bytes
                    .or(remote.size_bytes)
                    .unwrap_or(0),
                remote_modified_at: remote.modified_at,
            })
        }
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
            enabled: true,
            auto_upload: false,
            auto_download: false,
            upload_delay_hours: 1,
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

    fn sample_cached_remote() -> CachedRemote {
        CachedRemote {
            exists: true,
            size_bytes: Some(12),
            modified_at: Some(34),
            etag: Some("etag".into()),
            checked_at: 56,
        }
    }

    #[test]
    fn runtime_status_roundtrip_includes_new_fields() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = CloudRuntimeStatus {
            synced_fingerprint: Some("f".repeat(64)),
            synced_sha256: Some("s".repeat(64)),
            synced_meta: Some(sample_cached_remote()),
            last_upload_at: Some(123),
            last_download_at: Some(456),
            last_error: Some("upload boom".into()),
            last_download_error: Some("download boom".into()),
            observed_remote: Some(CachedRemote {
                checked_at: 789,
                ..sample_cached_remote()
            }),
            observed_manifest: Some(CloudManifestInfo {
                app_version: "9.9.9".into(),
                device_name: "Box".into(),
                created_at: 111,
                files: 7,
                schema: 1,
                compatible: true,
            }),
            observed_manifest_error: Some("manifest boom".into()),
            auth_paused_until: Some(999),
            upload_month: Some("2026-11".into()),
            uploaded_bytes_month: 1024,
        };
        save_runtime_status(&store, &status).unwrap();
        assert_eq!(load_runtime_status(&store), status);
    }

    #[test]
    fn runtime_status_roundtrip_and_corrupt_file_is_default() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = CloudRuntimeStatus {
            last_upload_at: Some(123),
            last_error: Some("boom".into()),
            synced_fingerprint: Some("f".repeat(64)),
            ..CloudRuntimeStatus::default()
        };
        save_runtime_status(&store, &status).unwrap();
        assert_eq!(load_runtime_status(&store), status);

        std::fs::write(status_path(&store), "{bad json").unwrap();
        assert_eq!(load_runtime_status(&store), CloudRuntimeStatus::default());
    }

    #[test]
    fn reset_runtime_status_removes_file_and_ignores_missing() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        save_runtime_status(
            &store,
            &CloudRuntimeStatus {
                last_upload_at: Some(1),
                ..CloudRuntimeStatus::default()
            },
        )
        .unwrap();
        assert!(status_path(&store).exists());

        reset_runtime_status(&store).unwrap();
        assert!(!status_path(&store).exists());

        // 文件本来就不存在时也算成功。
        reset_runtime_status(&store).unwrap();
    }

    #[test]
    fn runtime_status_without_fingerprint_field_reads_none() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        std::fs::write(
            status_path(&store),
            r#"{"lastUploadAt":5,"lastError":null}"#,
        )
        .unwrap();
        let loaded = load_runtime_status(&store);
        assert_eq!(loaded.last_upload_at, Some(5));
        assert!(
            loaded.synced_fingerprint.is_none(),
            "旧版本状态文件没有该字段时按未知处理，退化成多传一次"
        );
    }

    #[test]
    fn upload_now_records_synced_fingerprint() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        upload_now(&state).unwrap();
        assert_eq!(
            load_runtime_status(&state.store)
                .synced_fingerprint
                .as_deref(),
            Some(backup::content_fingerprint(&state.store).unwrap().as_str())
        );
    }

    #[test]
    fn upload_failure_keeps_synced_fingerprint_unknown() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("wrong"));
        assert!(upload_now(&state).is_err());
        assert!(load_runtime_status(&state.store).synced_fingerprint.is_none());
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
    fn cached_status_does_not_probe_network() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state(config_for(&server), Some("pw"));
        save_runtime_status(
            &state.store,
            &CloudRuntimeStatus {
                observed_remote: Some(CachedRemote {
                    size_bytes: Some(42),
                    ..sample_cached_remote()
                }),
                observed_manifest: Some(CloudManifestInfo {
                    app_version: "1.0.0".into(),
                    device_name: "Box".into(),
                    created_at: 1,
                    files: 2,
                    schema: 1,
                    compatible: true,
                }),
                ..CloudRuntimeStatus::default()
            },
        )
        .unwrap();

        let status = status_cached(&state).unwrap();
        assert!(status.from_cache);
        assert_eq!(status.remote.as_ref().unwrap().size_bytes, Some(42));
        assert_eq!(status.manifest.as_ref().unwrap().files, 2);
        assert_eq!(server.request_count("PROPFIND"), 0);
        assert_eq!(server.request_count("GET"), 0);
    }

    #[test]
    fn live_status_never_overwrites_synced_baseline() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"12345".to_vec());
        let state = test_state(config_for(&server), Some("pw"));
        save_runtime_status(
            &state.store,
            &CloudRuntimeStatus {
                synced_fingerprint: Some("a".into()),
                synced_sha256: Some("b".into()),
                ..CloudRuntimeStatus::default()
            },
        )
        .unwrap();

        let status = status(&state).unwrap();
        assert!(!status.from_cache);

        let runtime = load_runtime_status(&state.store);
        assert_eq!(runtime.synced_fingerprint.as_deref(), Some("a"));
        assert_eq!(runtime.synced_sha256.as_deref(), Some("b"));
        assert!(
            runtime.synced_meta.is_none(),
            "实时探测不能伪造同步基线"
        );
        assert_eq!(
            runtime.observed_remote.as_ref().unwrap().size_bytes,
            Some(5),
            "实时探测应写入观测缓存"
        );
    }

    #[test]
    fn live_status_reuses_observed_manifest_when_metadata_matches() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        upload_now(&state).unwrap();
        let gets = server.request_count("GET");

        status(&state).unwrap();
        assert_eq!(
            server.request_count("GET"),
            gets,
            "云端元数据没变时不该重新 GET manifest"
        );
        assert!(load_runtime_status(&state.store).observed_manifest.is_some());
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
    fn upload_increments_monthly_bytes_and_resets_on_new_month() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state_with_asset(config_for(&server), Some("pw"));
        // 预置上个月的统计：新月份的第一笔上传应从 0 重新累加。
        let last_month = (chrono::Local::now() - chrono::Duration::days(32))
            .format("%Y-%m")
            .to_string();
        save_runtime_status(
            &state.store,
            &CloudRuntimeStatus {
                upload_month: Some(last_month.clone()),
                uploaded_bytes_month: 999,
                ..CloudRuntimeStatus::default()
            },
        )
        .unwrap();

        let status = upload_now(&state).unwrap();
        let upload_size = server
            .object("/dav/ccswitchx/ccswitchx-backup.zip")
            .unwrap()
            .len() as u64;
        let runtime = load_runtime_status(&state.store);
        let this_month = chrono::Local::now().format("%Y-%m").to_string();
        assert_eq!(runtime.upload_month.as_deref(), Some(this_month.as_str()));
        assert_ne!(runtime.upload_month.as_deref(), Some(last_month.as_str()));
        assert_eq!(runtime.uploaded_bytes_month, upload_size);
        assert_eq!(status.uploaded_bytes_month, upload_size);
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
        let report = restore_latest(&target, None).unwrap();

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
    fn restore_latest_records_synced_baseline() {
        let server = FakeWebDav::start("me", "pw");
        let source = test_state_with_asset(config_for(&server), Some("pw"));
        let snapshot = build_named_snapshot(&source, "cloud-source");
        let fingerprint = backup::fingerprint_of_archive(&snapshot).unwrap();
        let sha256 = crate::index::sha256_hex(&std::fs::read(&snapshot).unwrap());
        server.put_object(
            "/dav/ccswitchx/ccswitchx-backup.zip",
            std::fs::read(&snapshot).unwrap(),
        );

        let target = test_state(config_for(&server), Some("pw"));
        let report = restore_latest(&target, None).unwrap();

        // 恢复完本地内容就等于云端，基线要跟上，否则下一轮会把同样的内容再传回去。
        let runtime = load_runtime_status(&target.store);
        assert_eq!(runtime.synced_fingerprint.as_deref(), Some(fingerprint.as_str()));
        assert_eq!(runtime.synced_sha256.as_deref(), Some(sha256.as_str()));
        let synced_meta = runtime.synced_meta.expect("恢复成功应记录云端元数据");
        assert!(synced_meta.exists);
        assert_eq!(synced_meta.size_bytes, Some(report.remote_size_bytes));
        let observed = runtime.observed_remote.expect("恢复成功应写观测缓存");
        assert_eq!(observed, synced_meta);
        assert_eq!(
            backup::content_fingerprint(&target.store).unwrap(),
            fingerprint,
            "恢复后的真源内容应与云端包一致"
        );
        assert!(report.downloaded_snapshot.starts_with("cloud_restore_"));
    }

    #[test]
    fn restore_missing_remote_is_friendly_error() {
        let server = FakeWebDav::start("me", "pw");
        let state = test_state(config_for(&server), Some("pw"));
        let error = restore_latest(&state, None).unwrap_err().to_string();
        assert!(error.contains("云端还没有备份"));
    }

    #[test]
    fn restore_rejects_foreign_archive() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"not a zip".to_vec());
        let state = test_state(config_for(&server), Some("pw"));
        assert!(restore_latest(&state, None).is_err());
        assert!(!state.store.root().join("agents").join("claude.json").exists());
    }

    #[test]
    fn restore_rejects_oversize_remote_without_downloading() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"x".to_vec());
        server.set_head_size_override(Some(MAX_CLOUD_BYTES + 1));
        let state = test_state(config_for(&server), Some("pw"));
        let before = server.request_count("GET");
        assert!(restore_latest(&state, None).is_err());
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
        let error = restore_latest(&target, None).unwrap_err().to_string();
        assert!(error.contains("与 manifest 不一致"), "{error}");
        assert!(!target
            .store
            .root()
            .join("agents")
            .join("claude.json")
            .exists());
    }

    #[test]
    fn restore_rejects_manifest_size_mismatch() {
        let server = FakeWebDav::start("me", "pw");
        let source = test_state_with_asset(config_for(&server), Some("pw"));
        let snapshot = build_named_snapshot(&source, "cloud-source");
        upload_archive(&source, &snapshot).unwrap();

        // 篡改 manifest 描述的大小：包本身没变，但描述与包已不是同一次上传的产物。
        let mut manifest: CloudManifest = serde_json::from_slice(
            &server.object("/dav/ccswitchx/manifest.json").unwrap(),
        )
        .unwrap();
        manifest.archive.size_bytes += 1;
        server.put_object(
            "/dav/ccswitchx/manifest.json",
            serde_json::to_vec(&manifest).unwrap(),
        );

        let target = test_state(config_for(&server), Some("pw"));
        let error = restore_latest(&target, None).unwrap_err().to_string();
        assert!(error.contains("大小对不上"), "{error}");
        assert!(!target
            .store
            .root()
            .join("agents")
            .join("claude.json")
            .exists());
    }

    #[test]
    fn restore_rejects_manifest_file_count_mismatch() {
        let server = FakeWebDav::start("me", "pw");
        let source = test_state_with_asset(config_for(&server), Some("pw"));
        let snapshot = build_named_snapshot(&source, "cloud-source");
        upload_archive(&source, &snapshot).unwrap();

        let mut manifest: CloudManifest = serde_json::from_slice(
            &server.object("/dav/ccswitchx/manifest.json").unwrap(),
        )
        .unwrap();
        manifest.files += 1;
        server.put_object(
            "/dav/ccswitchx/manifest.json",
            serde_json::to_vec(&manifest).unwrap(),
        );

        let target = test_state(config_for(&server), Some("pw"));
        let error = restore_latest(&target, None).unwrap_err().to_string();
        assert!(error.contains("文件数对不上"), "{error}");
    }

    #[test]
    fn restore_reports_download_progress() {
        let server = FakeWebDav::start("me", "pw");
        let source = test_state_with_asset(config_for(&server), Some("pw"));
        let snapshot = build_named_snapshot(&source, "cloud-source");
        let size = std::fs::metadata(&snapshot).unwrap().len();
        server.put_object(
            "/dav/ccswitchx/ccswitchx-backup.zip",
            std::fs::read(&snapshot).unwrap(),
        );

        let target = test_state(config_for(&server), Some("pw"));
        let seen = std::sync::Mutex::new(Vec::new());
        restore_latest(
            &target,
            Some(&|received: u64, total: u64| {
                seen.lock().unwrap().push((received, total));
            }),
        )
        .unwrap();

        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.last().copied(), Some((size, size)));
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
            let error = restore_latest(&target, None).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
}
