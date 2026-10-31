//! 全量快照：把可移植真源打包成 zip，供本机回滚或跨机器恢复。
//!
//! 刻意不收录 `local/`、`state/`、`secrets.local.json`、`index.db`、`logs/`、
//! `backups/` 与 `.git/`：这些都带本机路径或密钥，换机恢复只会制造错配。
//!
//! 恢复是 allowlist overlay：只写 `assets/`、`agents/` 与 `.gitignore`，
//! 其余目录原样保留；失败时按恢复前安全快照回滚。

use crate::domain::agent::is_known_agent;
use crate::domain::AssetType;
use crate::error::AppError;
use crate::index::Index;
use crate::net::archive::{extract_archive, ArchiveLimits};
use crate::services::git_sync;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const SNAPSHOT_DIR: &str = "snapshots";
const MANIFEST_NAME: &str = "manifest.json";
const FORMAT: &str = "ccswitchx-backup";
const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotKind {
    Manual,
    Auto,
    PreRestore,
    PrePull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub name: String,
    pub kind: SnapshotKind,
    /// 毫秒时间戳。
    pub created_at: i64,
    pub size_bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSection {
    pub id: String,
    pub path: String,
    pub bytes: u64,
    pub items: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupUsage {
    pub sections: Vec<UsageSection>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBackupReport {
    /// 恢复前自动创建的安全快照名。
    pub safety_backup: String,
    pub restored_files: u64,
    pub removed_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveBuild {
    pub created_at: i64,
    pub files: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSummary {
    pub files: u64,
    pub total_bytes: u64,
    pub max_depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: String,
    schema: u32,
    created_at: i64,
    kind: SnapshotKind,
    app_version: String,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    path: String,
    size: u64,
    sha256: String,
}

impl ManifestFile {
    fn for_bytes(path: String, bytes: &[u8]) -> Self {
        Self {
            path,
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        }
    }
}

pub fn snapshots_dir(store: &Store) -> PathBuf {
    store.root().join("backups").join(SNAPSHOT_DIR)
}

/// 名称直接参与文件名；允许中文，但拒绝路径分隔符、控制字符与 Windows 保留名。
pub fn validate_name(name: &str) -> Result<(), AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Backup("快照名不能为空".into()));
    }
    if name.chars().count() > 80 {
        return Err(AppError::Backup("快照名不能超过 80 个字符".into()));
    }
    if name == "." || name == ".." || name.contains("..") {
        return Err(AppError::Backup("快照名不能包含 `..`".into()));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err(AppError::Backup("快照名不能以空格或点结尾".into()));
    }
    if name.to_ascii_lowercase().ends_with(".zip") {
        return Err(AppError::Backup("快照名不需要写 .zip 后缀".into()));
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return Err(AppError::Backup(
            "快照名不能包含 < > : \" / \\ | ? * 或控制字符".into(),
        ));
    }
    let upper = name.to_ascii_uppercase();
    let reserved = upper == "CON"
        || upper == "PRN"
        || upper == "AUX"
        || upper == "NUL"
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if reserved {
        return Err(AppError::Backup("快照名不能使用系统保留名".into()));
    }
    Ok(())
}

fn snapshot_path(store: &Store, name: &str) -> Result<PathBuf, AppError> {
    let name = name.trim();
    validate_name(name)?;
    Ok(snapshots_dir(store).join(format!("{name}.zip")))
}

fn default_name() -> String {
    chrono::Local::now().format("backup_%Y%m%d_%H%M%S").to_string()
}

/// 创建快照。调用方负责持有 maintenance 写锁，保证读到一致的真源。
pub fn create(
    store: &Store,
    name: Option<&str>,
    kind: SnapshotKind,
) -> Result<SnapshotInfo, AppError> {
    let dir = snapshots_dir(store);
    fs::create_dir_all(&dir)?;

    let (name, target) = match name {
        Some(name) => {
            validate_name(name)?;
            let target = snapshot_path(store, name)?;
            if target.exists() {
                return Err(AppError::Backup(format!("快照已存在：{name}")));
            }
            (name.trim().to_string(), target)
        }
        None => {
            let base = default_name();
            let mut candidate = base.clone();
            let mut suffix = 1;
            loop {
                let target = snapshot_path(store, &candidate)?;
                if !target.exists() {
                    break (candidate, target);
                }
                suffix += 1;
                candidate = format!("{base}_{suffix}");
            }
        }
    };

    let tmp = dir.join(format!(".tmp-{}-{}.zip", std::process::id(), chrono::Utc::now().timestamp_millis()));
    let build = match write_archive_to(store, &tmp, kind) {
        Ok(build) => build,
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&tmp);
        return Err(AppError::Backup(format!("发布快照失败：{error}")));
    }

    let info = SnapshotInfo {
        name,
        kind,
        created_at: build.created_at,
        size_bytes: build.size_bytes,
        files: build.files,
    };
    Ok(info)
}

/// 把当前可移植真源写成快照包；调用方负责临时文件与最终发布。
pub fn write_archive_to(
    store: &Store,
    target: &Path,
    kind: SnapshotKind,
) -> Result<ArchiveBuild, AppError> {
    let files = collect_files(store)?;
    let mut manifest_files = Vec::with_capacity(files.len());
    for (absolute, relative) in &files {
        let bytes = fs::read(absolute)?;
        manifest_files.push(ManifestFile::for_bytes(relative.clone(), &bytes));
    }
    let manifest = Manifest {
        format: FORMAT.into(),
        schema: SCHEMA,
        created_at: chrono::Utc::now().timestamp_millis(),
        kind,
        app_version: env!("CARGO_PKG_VERSION").into(),
        files: manifest_files,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    write_zip(target, &manifest_bytes, &files)?;
    Ok(ArchiveBuild {
        created_at: manifest.created_at,
        files: manifest.files.len() as u64,
        size_bytes: fs::metadata(target).map(|meta| meta.len()).unwrap_or(0),
    })
}

/// 内容指纹：只由「相对路径 + 文件 sha256」决定，不含打包时间与 app_version。
/// 排序后再算，保证内存里算的值和从旧快照 manifest 读出的值可直接比较，
/// 应用升级本身不会触发一次多余的备份与上传。
fn fingerprint_of_files(files: &[ManifestFile]) -> String {
    let mut ordered = files
        .iter()
        .map(|file| (file.path.as_str(), file.sha256.as_str()))
        .collect::<Vec<_>>();
    ordered.sort_unstable();
    let mut buffer = String::new();
    for (path, sha256) in ordered {
        buffer.push_str(path);
        buffer.push('\0');
        buffer.push_str(sha256);
        buffer.push('\0');
    }
    sha256_hex(buffer.as_bytes())
}

/// 当前可移植真源的内容指纹，与快照包内 manifest 的 `files` 同源。
pub fn content_fingerprint(store: &Store) -> Result<String, AppError> {
    let files = collect_files(store)?;
    let mut manifest_files = Vec::with_capacity(files.len());
    for (absolute, relative) in &files {
        let bytes = fs::read(absolute)?;
        manifest_files.push(ManifestFile::for_bytes(relative.clone(), &bytes));
    }
    Ok(fingerprint_of_files(&manifest_files))
}

/// 已有快照包的内容指纹；只读包内 manifest.json，不解压内容。
pub fn fingerprint_of_archive(path: &Path) -> Result<String, AppError> {
    read_manifest(path).map(|manifest| fingerprint_of_files(&manifest.files))
}

/// 上传前的本地预检：确保包能通过恢复端的同一条上限校验。
pub fn validate_archive_for_restore(path: &Path) -> Result<ArchiveSummary, AppError> {
    let manifest = read_manifest(path)?;
    if !manifest.files.iter().any(|file| file.path != ".gitignore") {
        return Err(AppError::Cloud(
            "本地没有可备份的资产，已拒绝上传".into(),
        ));
    }
    let limits = ArchiveLimits::default();
    let entries = manifest.files.len() + 1;
    if entries > limits.max_entries {
        return Err(AppError::Cloud(format!(
            "备份包超过可恢复上限：条目数 {entries} 超过 {}",
            limits.max_entries
        )));
    }
    let total_bytes = manifest
        .files
        .iter()
        .map(|file| file.size.max(limits.min_entry_bytes))
        .fold(limits.min_entry_bytes, u64::saturating_add);
    if total_bytes > limits.max_total_bytes {
        return Err(AppError::Cloud(format!(
            "备份包超过可恢复上限：解压后约 {total_bytes} 字节，上限 {} 字节",
            limits.max_total_bytes
        )));
    }
    let max_depth = manifest
        .files
        .iter()
        .map(|file| file.path.split('/').filter(|segment| !segment.is_empty()).count())
        .max()
        .unwrap_or(0);
    if max_depth > limits.max_depth {
        return Err(AppError::Cloud(format!(
            "备份包超过可恢复上限：路径深度 {max_depth} 超过 {}",
            limits.max_depth
        )));
    }
    Ok(ArchiveSummary {
        files: manifest.files.len() as u64,
        total_bytes,
        max_depth,
    })
}

fn write_zip(
    path: &Path,
    manifest_bytes: &[u8],
    files: &[(PathBuf, String)],
) -> Result<(), AppError> {
    let file = fs::File::create(path)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer
        .start_file(MANIFEST_NAME, options)
        .map_err(|error| AppError::Backup(format!("写快照清单失败：{error}")))?;
    writer.write_all(manifest_bytes)?;
    for (absolute, relative) in files {
        let mut input = fs::File::open(absolute)?;
        writer
            .start_file(relative.clone(), options)
            .map_err(|error| AppError::Backup(format!("写快照条目 {relative} 失败：{error}")))?;
        std::io::copy(&mut input, &mut writer)?;
    }
    writer
        .finish()
        .map_err(|error| AppError::Backup(format!("完成快照失败：{error}")))?;
    Ok(())
}

/// 只收集可移植真源；相对路径统一用 `/`，manifest 与恢复端使用同一套路径。
fn collect_files(store: &Store) -> Result<Vec<(PathBuf, String)>, AppError> {
    let mut files = Vec::new();
    for asset_type in AssetType::ALL {
        walk_files(
            &store.asset_dir(asset_type),
            store.root(),
            &mut files,
        )?;
    }
    let agents_dir = store.root().join("agents");
    let mut agent_files = Vec::new();
    walk_files(&agents_dir, store.root(), &mut agent_files)?;
    for (absolute, relative) in agent_files {
        let path = Path::new(&relative);
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && is_known_agent(stem)
        {
            files.push((absolute, relative));
        }
    }
    let gitignore = store.root().join(".gitignore");
    if gitignore.is_file() {
        files.push((gitignore, ".gitignore".into()));
    }
    files.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(files)
}

fn walk_files(
    dir: &Path,
    base: &Path,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<(), AppError> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    let mut entries = entries.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            // 不跟随链接，避免把 store 外部内容打进快照。
            continue;
        }
        if meta.is_dir() {
            walk_files(&entry.path(), base, out)?;
        } else if meta.is_file() {
            let absolute = entry.path();
            let relative = absolute
                .strip_prefix(base)
                .map_err(|_| AppError::Backup("快照条目不在 store 根目录下".into()))?;
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((absolute, relative));
        }
    }
    Ok(())
}

pub fn list(store: &Store) -> Result<Vec<SnapshotInfo>, AppError> {
    let dir = snapshots_dir(store);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut snapshots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("zip")
            || entry.file_name().to_string_lossy().starts_with(".tmp-")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let fallback = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let manifest = read_manifest(&path).ok();
        let (kind, created_at, files) = match manifest {
            Some(manifest) => (
                manifest.kind,
                manifest.created_at,
                manifest.files.len() as u64,
            ),
            None => (SnapshotKind::Manual, system_time_millis(fallback), 0),
        };
        snapshots.push(SnapshotInfo {
            name: stem.to_string(),
            kind,
            created_at,
            size_bytes: meta.len(),
            files,
        });
    }
    snapshots.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(snapshots)
}

fn read_manifest(path: &Path) -> Result<Manifest, AppError> {
    let file = fs::File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| AppError::Backup(format!("无法读取快照：{error}")))?;
    let mut entry = archive
        .by_name(MANIFEST_NAME)
        .map_err(|_| AppError::Backup("快照缺少 manifest.json".into()))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.format != FORMAT {
        return Err(AppError::Backup(format!(
            "不是 CC Switch X 快照：{}",
            manifest.format
        )));
    }
    if manifest.schema != SCHEMA {
        return Err(AppError::Backup(format!(
            "快照格式版本不支持：{}",
            manifest.schema
        )));
    }
    Ok(manifest)
}

pub fn rename(store: &Store, old: &str, new: &str) -> Result<SnapshotInfo, AppError> {
    let old = old.trim();
    let new = new.trim();
    let from = snapshot_path(store, old)?;
    let to = snapshot_path(store, new)?;
    if !from.is_file() {
        return Err(AppError::NotFound(format!("快照不存在：{old}")));
    }
    if to.exists() {
        return Err(AppError::Backup(format!("快照已存在：{new}")));
    }
    fs::rename(&from, &to)?;
    snapshot_info(store, new)
}

pub fn delete(store: &Store, name: &str) -> Result<(), AppError> {
    let path = snapshot_path(store, name)?;
    if !path.is_file() {
        return Err(AppError::NotFound(format!("快照不存在：{name}")));
    }
    fs::remove_file(&path)?;
    Ok(())
}

fn snapshot_info(store: &Store, name: &str) -> Result<SnapshotInfo, AppError> {
    let path = snapshot_path(store, name)?;
    let meta = fs::metadata(&path)?;
    let manifest = read_manifest(&path).ok();
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Ok(match manifest {
        Some(manifest) => SnapshotInfo {
            name: name.to_string(),
            kind: manifest.kind,
            created_at: manifest.created_at,
            size_bytes: meta.len(),
            files: manifest.files.len() as u64,
        },
        None => SnapshotInfo {
            name: name.to_string(),
            kind: SnapshotKind::Manual,
            created_at: system_time_millis(modified),
            size_bytes: meta.len(),
            files: 0,
        },
    })
}

/// 只保留最近 `keep` 份；目录名不参与判断，按修改时间排序，避免用户重命名后顺序错乱。
pub fn prune(store: &Store, keep: usize) -> Result<usize, AppError> {
    let keep = keep.max(1);
    let dir = snapshots_dir(store);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(0);
    };
    let mut files = entries
        .flatten()
        .filter(|entry| {
            entry.path().is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("zip")
                && !entry.file_name().to_string_lossy().starts_with(".tmp-")
        })
        .filter_map(|entry| {
            let modified = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, entry.path()))
        })
        .collect::<Vec<_>>();
    if files.len() <= keep {
        return Ok(0);
    }
    files.sort_by_key(|(modified, _)| *modified);
    let excess = files.len() - keep;
    let mut removed = 0;
    for (_, path) in files.into_iter().take(excess) {
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn usage(store: &Store) -> Result<BackupUsage, AppError> {
    let root = store.root();
    let backups = root.join("backups");
    let snapshots = snapshots_dir(store);
    let logs = root.join("logs");

    let mut sections = Vec::new();
    let (snapshot_bytes, snapshot_items) = measure_entries(&snapshots, |entry| {
        entry.path().is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("zip")
            && !entry.file_name().to_string_lossy().starts_with(".tmp-")
    });
    sections.push(UsageSection {
        id: "snapshots".into(),
        path: snapshots.to_string_lossy().into_owned(),
        bytes: snapshot_bytes,
        items: snapshot_items,
    });

    let (write_bytes, write_items) = measure_entries(&backups, |entry| {
        entry.file_name().to_string_lossy() != SNAPSHOT_DIR
    });
    sections.push(UsageSection {
        id: "writes".into(),
        path: backups.to_string_lossy().into_owned(),
        bytes: write_bytes,
        items: write_items,
    });

    let (log_bytes, log_items) = measure_entries(&logs, |_| true);
    sections.push(UsageSection {
        id: "logs".into(),
        path: logs.to_string_lossy().into_owned(),
        bytes: log_bytes,
        items: log_items,
    });

    let mut index_bytes = 0;
    let mut index_items = 0;
    for name in ["index.db", "index.db-wal", "index.db-shm"] {
        let path = root.join(name);
        if let Ok(meta) = fs::metadata(&path) {
            index_bytes += meta.len();
            index_items += 1;
        }
    }
    sections.push(UsageSection {
        id: "index".into(),
        path: root.join("index.db").to_string_lossy().into_owned(),
        bytes: index_bytes,
        items: index_items,
    });

    let total_bytes = sections.iter().map(|section| section.bytes).sum();
    Ok(BackupUsage {
        sections,
        total_bytes,
    })
}

fn measure_entries(
    dir: &Path,
    include: impl Fn(&fs::DirEntry) -> bool,
) -> (u64, u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut bytes = 0;
    let mut items = 0;
    for entry in entries.flatten() {
        if !include(&entry) {
            continue;
        }
        items += 1;
        bytes += walk_bytes(&entry.path());
    }
    (bytes, items)
}

fn walk_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        return entries.flatten().map(|entry| walk_bytes(&entry.path())).sum();
    }
    meta.len()
}

/// 清理整类备份。snapshots 只能逐个管理，index 与 store 不在这里删除。
pub fn delete_category(store: &Store, id: &str) -> Result<u64, AppError> {
    let root = store.root();
    let (dir, keep_snapshots) = match id {
        "writes" => (root.join("backups"), true),
        "logs" => (root.join("logs"), false),
        _ => {
            return Err(AppError::InvalidInput(format!(
                "该备份类别不支持整体清理：{id}"
            )))
        }
    };
    let mut freed = 0;
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(0);
    };
    for entry in entries.flatten() {
        if keep_snapshots && entry.file_name().to_string_lossy() == SNAPSHOT_DIR {
            continue;
        }
        freed += walk_bytes(&entry.path());
        remove_path(&entry.path())?;
    }
    Ok(freed)
}

pub fn reveal_location(store: &Store, id: &str) -> Result<PathBuf, AppError> {
    let path = match id {
        "snapshots" => snapshots_dir(store),
        "writes" => store.root().join("backups"),
        "logs" => store.root().join("logs"),
        "store" => store.root().to_path_buf(),
        _ => {
            return Err(AppError::InvalidInput(format!(
                "未知的备份位置：{id}"
            )))
        }
    };
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// 恢复快照。调用方负责持 maintenance 写锁与 index 锁。
/// 返回前会重建 assets 索引，并清掉不可信的 agent_state / drift。
pub fn restore(
    store: &Store,
    index: &mut Index,
    name: &str,
) -> Result<RestoreBackupReport, AppError> {
    let zip = snapshot_path(store, name)?;
    if !zip.is_file() {
        return Err(AppError::NotFound(format!("快照不存在：{name}")));
    }
    // 清理交给调用方：这里 prune 有可能把正在恢复的源快照删掉。
    let safety = create(store, None, SnapshotKind::PreRestore)?;
    let safety_zip = snapshot_path(store, &safety.name)?;

    match apply_snapshot(store, &zip) {
        Ok((restored_files, removed_files)) => {
            rebuild_after_restore(store, index)?;
            Ok(RestoreBackupReport {
                safety_backup: safety.name,
                restored_files,
                removed_files,
            })
        }
        Err(error) => {
            let rollback = apply_snapshot(store, &safety_zip);
            if let Err(index_error) = rebuild_after_restore(store, index) {
                return Err(AppError::Backup(format!(
                    "{error}；回滚后重建索引失败：{index_error}"
                )));
            }
            match rollback {
                Ok(_) => Err(AppError::Backup(format!("{error}；已按安全快照回滚"))),
                Err(rollback_error) => Err(AppError::Backup(format!(
                    "{error}；回滚也失败：{rollback_error}"
                ))),
            }
        }
    }
}

fn rebuild_after_restore(store: &Store, index: &mut Index) -> Result<(), AppError> {
    index.reset_agent_state_and_drift()?;
    index.rebuild(store)?;
    Ok(())
}

/// 解压 + 校验 + overlay，返回（写入文件数，删除条目数）。失败时调用方用安全快照重放。
fn apply_snapshot(store: &Store, zip: &Path) -> Result<(u64, u64), AppError> {
    let staging = store
        .staging_dir()
        .join(format!("restore-{}", chrono::Utc::now().timestamp_millis()));
    if staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    let result = (|| {
        extract_archive(zip, &staging, &ArchiveLimits::default())?;
        let manifest = read_manifest_from_dir(&staging)?;
        let plan = validate_manifest(store, &staging, &manifest)?;
        overlay(store, &staging, &plan)
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn read_manifest_from_dir(dir: &Path) -> Result<Manifest, AppError> {
    let path = dir.join(MANIFEST_NAME);
    let bytes = fs::read(&path)
        .map_err(|_| AppError::Backup("快照缺少 manifest.json".into()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.format != FORMAT {
        return Err(AppError::Backup(format!(
            "不是 CC Switch X 快照：{}",
            manifest.format
        )));
    }
    if manifest.schema != SCHEMA {
        return Err(AppError::Backup(format!(
            "快照格式版本不支持：{}",
            manifest.schema
        )));
    }
    Ok(manifest)
}

struct RestorePlan {
    /// 每类资产在快照里的条目：非 skill 是 id → 相对文件路径；skill 是 id → 目录相对路径。
    assets: HashMap<AssetType, BTreeMap<String, String>>,
    agents: BTreeMap<String, String>,
    gitignore: Option<String>,
    files: HashSet<String>,
}

fn validate_manifest(
    store: &Store,
    staging: &Path,
    manifest: &Manifest,
) -> Result<RestorePlan, AppError> {
    let mut plan = RestorePlan {
        assets: HashMap::new(),
        agents: BTreeMap::new(),
        gitignore: None,
        files: HashSet::new(),
    };
    for file in &manifest.files {
        let relative = Path::new(&file.path);
        if file.path.contains('\\') || relative.is_absolute() {
            return Err(AppError::Backup(format!(
                "快照路径非法：{}",
                file.path
            )));
        }
        if !plan.files.insert(file.path.clone()) {
            return Err(AppError::Backup(format!(
                "快照清单包含重复路径：{}",
                file.path
            )));
        }
        let absolute = staging.join(relative);
        let meta = fs::metadata(&absolute)
            .map_err(|_| AppError::Backup(format!("快照缺少文件：{}", file.path)))?;
        if !meta.is_file() || meta.len() != file.size {
            return Err(AppError::Backup(format!(
                "快照文件大小不一致：{}",
                file.path
            )));
        }
        let bytes = fs::read(&absolute)?;
        if sha256_hex(&bytes) != file.sha256 {
            return Err(AppError::Backup(format!(
                "快照文件校验失败：{}",
                file.path
            )));
        }
        classify(store, &file.path, &mut plan)?;
    }

    // 防止 zip 里夹带清单之外的文件（解压器本身只挡越界，不挡多余文件）。
    let mut actual = Vec::new();
    walk_relative(staging, staging, &mut actual)?;
    for path in actual {
        if path == MANIFEST_NAME {
            continue;
        }
        if !plan.files.contains(&path) {
            return Err(AppError::Backup(format!(
                "快照包含清单之外的文件：{path}"
            )));
        }
    }
    Ok(plan)
}

fn classify(store: &Store, path: &str, plan: &mut RestorePlan) -> Result<(), AppError> {
    if path == ".gitignore" {
        plan.gitignore = Some(path.to_string());
        return Ok(());
    }
    let components = Path::new(path)
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_string_lossy().to_string(),
            _ => String::new(),
        })
        .collect::<Vec<_>>();
    if components.iter().any(|component| component.is_empty()) {
        return Err(AppError::Backup(format!("快照路径非法：{path}")));
    }
    match components.first().map(String::as_str) {
        Some("assets") => {
            let dir = components
                .get(1)
                .ok_or_else(|| AppError::Backup(format!("快照路径非法：{path}")))?;
            let asset_type = AssetType::ALL
                .into_iter()
                .find(|kind| kind.dir_name() == dir)
                .ok_or_else(|| AppError::Backup(format!("未知资产目录：{dir}")))?;
            match asset_type {
                AssetType::Skill => {
                    let id = components
                        .get(2)
                        .ok_or_else(|| AppError::Backup(format!("Skill 路径缺少 ID：{path}")))?;
                    store
                        .entry_path(AssetType::Skill, id)
                        .map_err(|_| AppError::Backup(format!("非法 Skill ID：{id}")))?;
                    if components.len() < 4 {
                        return Err(AppError::Backup(format!(
                            "Skill 条目必须是 id 下的文件：{path}"
                        )));
                    }
                    plan.assets
                        .entry(AssetType::Skill)
                        .or_default()
                        .insert(id.clone(), format!("assets/skills/{id}"));
                }
                _ => {
                    if components.len() != 3 {
                        return Err(AppError::Backup(format!(
                            "资产路径层级非法：{path}"
                        )));
                    }
                    let file_name = &components[2];
                    let extension = asset_type
                        .file_extension()
                        .ok_or_else(|| AppError::Backup(format!("资产类型不支持文件：{dir}")))?;
                    let id = file_name
                        .strip_suffix(&format!(".{extension}"))
                        .ok_or_else(|| {
                            AppError::Backup(format!("资产扩展名不匹配：{path}"))
                        })?;
                    let expected = store
                        .entry_path(asset_type, id)
                        .map_err(|_| AppError::Backup(format!("非法资产 ID：{id}")))?;
                    if expected.file_name().and_then(|name| name.to_str())
                        != Some(file_name.as_str())
                    {
                        return Err(AppError::Backup(format!("资产文件名非法：{path}")));
                    }
                    plan.assets
                        .entry(asset_type)
                        .or_default()
                        .insert(id.to_string(), path.to_string());
                }
            }
        }
        Some("agents") => {
            if components.len() != 2 {
                return Err(AppError::Backup(format!("agent 路径非法：{path}")));
            }
            let file_name = &components[1];
            let id = file_name.strip_suffix(".json").ok_or_else(|| {
                AppError::Backup(format!("agent 配置必须是 .json：{path}"))
            })?;
            if !is_known_agent(id) {
                return Err(AppError::Backup(format!("未知 agent：{id}")));
            }
            plan.agents.insert(id.to_string(), path.to_string());
        }
        _ => {
            return Err(AppError::Backup(format!(
                "快照包含不允许的路径：{path}"
            )))
        }
    }
    Ok(())
}

fn walk_relative(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), AppError> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_relative(root, &entry.path(), out)?;
        } else if meta.is_file() {
            let absolute = entry.path();
            let relative = absolute
                .strip_prefix(root)
                .map_err(|_| AppError::Backup("快照文件不在暂存目录下".into()))?;
            out.push(
                relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    Ok(())
}

fn overlay(
    store: &Store,
    staging: &Path,
    plan: &RestorePlan,
) -> Result<(u64, u64), AppError> {
    let mut restored = 0;
    let mut removed = 0;

    // 非 skill：按 ID 覆盖/删除，保留真源目录里的其他文件时也要让快照成为唯一事实。
    for asset_type in AssetType::ALL {
        let expected = plan.assets.get(&asset_type);
        let current = store.list_ids(asset_type)?;
        for id in current {
            if expected.is_some_and(|map| map.contains_key(&id)) {
                continue;
            }
            if store.delete(asset_type, &id)? {
                removed += 1;
            }
        }
        if asset_type == AssetType::Skill {
            let Some(map) = expected else { continue };
            for (id, relative) in map {
                let target = store
                    .entry_path(AssetType::Skill, id)
                    .map_err(|_| AppError::Backup(format!("非法 Skill ID：{id}")))?;
                if target.exists() {
                    remove_path(&target)?;
                }
                crate::store::copy_dir(&staging.join(relative), &target)?;
                restored += count_files(&target);
            }
        } else {
            let Some(map) = expected else { continue };
            for (id, relative) in map {
                let bytes = fs::read(staging.join(relative))?;
                store.write(asset_type, id, &bytes)?;
                restored += 1;
            }
        }
    }

    let agents_dir = store.root().join("agents");
    fs::create_dir_all(&agents_dir)?;
    for entry in fs::read_dir(&agents_dir)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if is_known_agent(stem) && !plan.agents.contains_key(stem) {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    for (id, relative) in &plan.agents {
        let bytes = fs::read(staging.join(relative))?;
        write_atomic(&agents_dir.join(format!("{id}.json")), &bytes)?;
        restored += 1;
    }

    if let Some(gitignore) = &plan.gitignore {
        let bytes = fs::read(staging.join(gitignore))?;
        write_atomic(&store.root().join(".gitignore"), &bytes)?;
        // 清单里缺了必需行时补齐；用户自定义行保持不动。
        Store::init(store.root().to_path_buf())?;
        restored += 1;
    }
    Ok((restored, removed))
}

fn count_files(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                return 0;
            };
            if meta.is_dir() {
                count_files(&path)
            } else if meta.is_file() {
                1
            } else {
                0
            }
        })
        .sum()
}

fn remove_path(path: &Path) -> Result<(), AppError> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        // Windows 的目录 junction 只能用 remove_dir 删链接本身；两种都试，不跟随目标。
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))?;
    } else if meta.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn system_time_millis(time: SystemTime) -> i64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// 启动后立即检查一次，之后每小时检查。间隔为 0 时完全不创建自动快照。
pub fn spawn_periodic(app: tauri::AppHandle) {
    use tauri::Manager;
    let _ = std::thread::Builder::new()
        .name("cc-switch-x-backup".into())
        .spawn(move || loop {
            if let Some(state) = app.try_state::<crate::state::AppState>() {
                periodic_once(&state);
            }
            std::thread::sleep(std::time::Duration::from_secs(3600));
        });
}

pub(crate) fn periodic_once(state: &crate::state::AppState) {
    let (interval_hours, retain) = match state.settings.lock() {
        Ok(settings) => (
            settings.backup.interval_hours as i64,
            settings.backup.retain_count.max(1) as usize,
        ),
        Err(_) => return,
    };
    if interval_hours == 0 {
        return;
    }
    let latest = list(&state.store)
        .ok()
        .and_then(|items| items.first().map(|snapshot| snapshot.created_at))
        .unwrap_or(0);
    let due = latest + interval_hours * 3600 * 1000;
    if chrono::Utc::now().timestamp_millis() < due {
        return;
    }
    if let Err(error) = auto_snapshot_if_changed(state, retain) {
        eprintln!("自动备份失败：{error}");
    }
    if let Err(error) = auto_upload_if_needed(state) {
        eprintln!("自动上传云备份失败：{error}");
    }
}

/// 内容有变化才新建 Auto 快照并清理超额；与最新一份快照内容一致时返回 `None`，
/// 免得等内容把 `retain_count` 窗口里真正有内容的历史挤掉。
pub(crate) fn auto_snapshot_if_changed(
    state: &crate::state::AppState,
    retain: usize,
) -> Result<Option<SnapshotInfo>, AppError> {
    let current = content_fingerprint(&state.store)?;
    let newest = list(&state.store)?.into_iter().next();
    if let Some(newest) = newest {
        let archived = snapshot_path(&state.store, &newest.name)
            .and_then(|path| fingerprint_of_archive(&path));
        if archived
            .as_deref()
            .is_ok_and(|fingerprint| fingerprint == current.as_str())
        {
            return Ok(None);
        }
    }
    let _guard = state.enter_maintenance()?;
    let info = create(&state.store, None, SnapshotKind::Auto)?;
    let _ = prune(&state.store, retain.max(1));
    Ok(Some(info))
}

/// 云端只有一份包，它应当等于本地最新快照：内容有变、或云端还不是这份内容时才上传覆盖。
/// 基线独立于「是否新建了快照」，所以手动备份过的内容、以及上次上传失败的内容都能被补传。
pub(crate) fn auto_upload_if_needed(state: &crate::state::AppState) -> Result<(), AppError> {
    let (auto_upload, git) = match state.settings.lock() {
        Ok(settings) => (settings.cloud.auto_upload, settings.backup.git.clone()),
        Err(_) => return Ok(()),
    };
    if !auto_upload {
        return Ok(());
    }
    // 远端备份只能选一种：Git 自动同步生效时不再上传 WebDAV。
    // 历史设置可能同时开启两者；以「Git 已验证且已开启」为生效条件，避免两边都跑。
    let git_active = git.enabled
        && git_sync::is_remote_verified(&state.store, git.remote.as_deref());
    if git_active {
        return Ok(());
    }
    let Some(newest) = list(&state.store)?.into_iter().next() else {
        return Ok(());
    };
    let archive = snapshot_path(&state.store, &newest.name)?;
    let fingerprint = fingerprint_of_archive(&archive)?;
    let runtime = crate::services::cloud_backup::load_runtime_status(&state.store);
    if runtime.remote_fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return Ok(());
    }
    crate::services::cloud_backup::upload_archive(state, &archive).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::index::Index;
    use crate::net::webdav::test_support::FakeWebDav;
    use crate::services::cloud_backup::load_runtime_status;
    use crate::services::settings::cloud_secret_ref;
    use crate::settings::{AppSettings, CloudBackupSettings};
    use crate::state::AppState;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static CLOUD_TEST_DIRS: Mutex<Vec<tempfile::TempDir>> = Mutex::new(Vec::new());

    fn state_with_asset_and_cloud(
        auto_upload: bool,
        base_url: String,
        password: &str,
    ) -> AppState {
        let dir = tempdir().unwrap();
        let settings = AppSettings {
            store_root: dir.path().join("store"),
            cloud: CloudBackupSettings {
                auto_upload,
                base_url,
                username: "me".into(),
                remote_dir: "ccswitchx".into(),
            },
            ..AppSettings::default()
        };
        let state = AppState::init(settings).unwrap();
        state
            .secrets
            .lock()
            .unwrap()
            .insert(&cloud_secret_ref(), password);
        write_mcp(&state.store, "pw", "Playwright");
        CLOUD_TEST_DIRS.lock().unwrap().push(dir);
        state
    }

    fn fixture() -> (tempfile::TempDir, Store, Index) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&store.root().join("index.db")).unwrap();
        (dir, store, index)
    }

    fn write_mcp(store: &Store, id: &str, name: &str) {
        store
            .write(
                AssetType::Mcp,
                id,
                format!(
                    r#"{{"id":"{id}","name":"{name}","transport":"stdio","command":"npx"}}"#
                )
                .as_bytes(),
            )
            .unwrap();
    }

    fn write_manifest_only_zip(
        path: &Path,
        file_count: usize,
        file_size: u64,
        path_components: &[&str],
    ) {
        let files = (0..file_count)
            .map(|index| ManifestFile {
                path: format!("{}/{}", path_components.join("/"), index),
                size: file_size,
                sha256: "00".into(),
            })
            .collect();
        let manifest = Manifest {
            format: FORMAT.into(),
            schema: SCHEMA,
            created_at: 1,
            kind: SnapshotKind::Manual,
            app_version: "test".into(),
            files,
        };
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        let file = fs::File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file(MANIFEST_NAME, options).unwrap();
        writer.write_all(&bytes).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn snapshot_contains_only_portable_truth() {
        let (_dir, store, _index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        std::fs::create_dir_all(store.asset_dir(AssetType::Skill).join("demo")).unwrap();
        std::fs::write(
            store.asset_dir(AssetType::Skill).join("demo/SKILL.md"),
            "# Demo\n",
        )
        .unwrap();
        std::fs::write(store.root().join("agents/claude.json"), "{}").unwrap();
        std::fs::write(store.root().join("local/agents/claude.json"), "{}").unwrap();
        std::fs::create_dir_all(store.root().join("state/applied")).unwrap();
        std::fs::write(store.root().join("state/applied/claude.json"), "{}").unwrap();
        std::fs::write(store.root().join("secrets.local.json"), "{}").unwrap();
        std::fs::write(store.root().join("logs/app.log"), "log").unwrap();
        std::fs::create_dir_all(store.root().join("backups/123/mcp")).unwrap();
        std::fs::write(store.root().join("backups/123/mcp/old.json"), "{}").unwrap();

        let info = create(&store, Some("cross-machine"), SnapshotKind::Manual).unwrap();
        assert_eq!(info.name, "cross-machine");
        assert!(info.files >= 4);

        let file = std::fs::File::open(snapshot_path(&store, "cross-machine").unwrap()).unwrap();
        let mut archive = ZipArchive::new(file).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"manifest.json".to_string()));
        assert!(names.contains(&"assets/mcp/pw.json".to_string()));
        assert!(names.contains(&"assets/skills/demo/SKILL.md".to_string()));
        assert!(names.contains(&"agents/claude.json".to_string()));
        assert!(names.contains(&".gitignore".to_string()));
        for forbidden in [
            "secrets.local.json",
            "local/agents/claude.json",
            "state/applied/claude.json",
            "logs/app.log",
            "backups/123/mcp/old.json",
        ] {
            assert!(
                !names.iter().any(|name| name == forbidden),
                "{forbidden} 不应进入快照"
            );
        }
    }

    #[test]
    fn names_are_validated_and_renamed() {
        let (_dir, store, _index) = fixture();
        for bad in ["", "..", "a/b", "a\\b", "a:b", "CON", "name.", "name.zip"] {
            assert!(validate_name(bad).is_err(), "{bad} 应当被拒绝");
        }
        create(&store, Some("快照 1"), SnapshotKind::Manual).unwrap();
        assert!(create(&store, Some("快照 1"), SnapshotKind::Manual).is_err());
        let info = rename(&store, "快照 1", "快照 2").unwrap();
        assert_eq!(info.name, "快照 2");
        assert!(!snapshot_path(&store, "快照 1").unwrap().exists());
        delete(&store, "快照 2").unwrap();
        assert!(list(&store).unwrap().is_empty());
    }

    #[test]
    fn prune_only_touches_snapshots() {
        let (_dir, store, _index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        std::fs::create_dir_all(store.root().join("backups/old/mcp")).unwrap();
        std::fs::write(store.root().join("backups/old/mcp/x.json"), "{}").unwrap();
        std::fs::create_dir_all(store.root().join("backups/skills/1_demo")).unwrap();
        std::fs::write(store.root().join("backups/skills/1_demo/SKILL.md"), "x").unwrap();

        for index in 0..4 {
            let info = create(&store, None, SnapshotKind::Auto).unwrap();
            // 保证 mtime 可区分，避免同毫秒创建导致排序不稳定。
            std::thread::sleep(std::time::Duration::from_millis(5));
            let _ = index;
            let _ = info;
        }
        let removed = prune(&store, 2).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(list(&store).unwrap().len(), 2);
        assert!(store.root().join("backups/old/mcp/x.json").exists());
        assert!(store.root().join("backups/skills/1_demo/SKILL.md").exists());
    }

    #[test]
    fn restore_overlays_assets_and_agents_but_keeps_local_and_secrets() {
        let (_dir, store, mut index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        store
            .write(
                AssetType::Prompt,
                "style",
                b"---\nname: Style\n---\nold\n",
            )
            .unwrap();
        std::fs::write(store.root().join("agents/claude.json"), r#"{"activePrompt":"style"}"#)
            .unwrap();
        std::fs::write(
            store.root().join("local/agents/claude.json"),
            r#"{"baseOverride":"D:/local"}"#,
        )
        .unwrap();
        std::fs::write(store.root().join("secrets.local.json"), r#"{"k":"v"}"#).unwrap();
        index.rebuild(&store).unwrap();
        index
            .set_agent_state(&crate::index::AgentStateRow {
                agent_id: "claude".into(),
                asset_type: AssetType::Mcp,
                asset_id: "pw".into(),
                state: crate::domain::AssetState::Effective,
                last_error: None,
                applied_at: 1,
            })
            .unwrap();
        create(&store, Some("before"), SnapshotKind::Manual).unwrap();

        write_mcp(&store, "pw", "Changed");
        store
            .write(
                AssetType::Prompt,
                "style",
                b"---\nname: Style\n---\nnew\n",
            )
            .unwrap();
        store
            .write(
                AssetType::Rule,
                "temp",
                b"---\nname: Temp\n---\nx\n",
            )
            .unwrap();
        std::fs::write(store.root().join("agents/claude.json"), r#"{"activePrompt":"temp"}"#)
            .unwrap();

        let report = restore(&store, &mut index, "before").unwrap();
        prune(&store, 10).unwrap();
        assert!(report.safety_backup.starts_with("backup_"));
        assert_eq!(report.restored_files, 4, "mcp + prompt + agent 矩阵 + .gitignore");
        assert_eq!(report.removed_files, 1, "快照之外的 Rule 应当被删除");

        let mcp = store.read(AssetType::Mcp, "pw").unwrap();
        assert!(String::from_utf8_lossy(&mcp).contains("Playwright"));
        let prompt = store.read(AssetType::Prompt, "style").unwrap();
        assert!(String::from_utf8_lossy(&prompt).contains("old"));
        assert!(!store.entry_path(AssetType::Rule, "temp").unwrap().exists());
        assert_eq!(
            std::fs::read_to_string(store.root().join("agents/claude.json")).unwrap(),
            r#"{"activePrompt":"style"}"#
        );
        assert_eq!(
            std::fs::read_to_string(store.root().join("local/agents/claude.json")).unwrap(),
            r#"{"baseOverride":"D:/local"}"#,
            "本机覆盖不能被快照恢复覆盖"
        );
        assert_eq!(
            std::fs::read_to_string(store.root().join("secrets.local.json")).unwrap(),
            r#"{"k":"v"}"#,
            "密钥不能被快照恢复覆盖"
        );
        assert!(index.agent_states().unwrap().is_empty());
        assert!(list(&store)
            .unwrap()
            .iter()
            .any(|snapshot| snapshot.kind == SnapshotKind::PreRestore));
    }

    #[test]
    fn corrupt_manifest_and_unknown_paths_are_rejected() {
        let (_dir, store, _index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        let zip = store.root().join("bad.zip");
        let file = std::fs::File::create(&zip).unwrap();
        let mut writer = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let manifest = serde_json::json!({
            "format": FORMAT,
            "schema": SCHEMA,
            "createdAt": 1,
            "kind": "manual",
            "appVersion": "test",
            "files": [
                {"path": "local/agents/claude.json", "size": 2, "sha256": "00"}
            ]
        });
        writer.start_file("manifest.json", options).unwrap();
        writer
            .write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .unwrap();
        writer.start_file("local/agents/claude.json", options).unwrap();
        writer.write_all(b"{}").unwrap();
        writer.finish().unwrap();

        let error = apply_snapshot(&store, &zip).unwrap_err().to_string();
        assert!(error.contains("不允许的路径") || error.contains("校验失败"), "{error}");
    }

    #[test]
    fn validate_archive_rejects_empty_package() {
        let (_dir, store, _index) = fixture();
        let info = create(&store, Some("empty"), SnapshotKind::Manual).unwrap();
        let path = snapshots_dir(&store).join(format!("{}.zip", info.name));
        let error = validate_archive_for_restore(&path).unwrap_err().to_string();
        assert!(error.contains("没有可备份的资产"));
    }

    #[test]
    fn validate_archive_rejects_too_many_entries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("too-many.zip");
        write_manifest_only_zip(&path, 30_001, 1, &["a"]);
        assert!(validate_archive_for_restore(&path)
            .unwrap_err()
            .to_string()
            .contains("超过可恢复上限"));
    }

    #[test]
    fn validate_archive_rejects_oversize_charge() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oversize.zip");
        write_manifest_only_zip(&path, 1, 600 * 1024 * 1024, &["a"]);
        assert!(validate_archive_for_restore(&path)
            .unwrap_err()
            .to_string()
            .contains("超过可恢复上限"));
    }

    #[test]
    fn validate_archive_accepts_normal_package() {
        let (_dir, store, _index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        let info = create(&store, Some("ok"), SnapshotKind::Manual).unwrap();
        let path = snapshots_dir(&store).join(format!("{}.zip", info.name));
        let summary = validate_archive_for_restore(&path).unwrap();
        assert!(summary.files >= 2);
        assert!(summary.total_bytes > 0);
    }

    fn plain_state() -> AppState {
        let dir = tempdir().unwrap();
        let settings = AppSettings {
            store_root: dir.path().join("store"),
            ..AppSettings::default()
        };
        let state = AppState::init(settings).unwrap();
        write_mcp(&state.store, "pw", "Playwright");
        CLOUD_TEST_DIRS.lock().unwrap().push(dir);
        state
    }

    #[test]
    fn content_fingerprint_tracks_only_portable_truth() {
        let (_dir, store, _index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        let base = content_fingerprint(&store).unwrap();

        std::fs::create_dir_all(store.root().join("local/agents")).unwrap();
        std::fs::write(
            store.root().join("local/agents/claude.json"),
            "{\"a\":1}",
        )
        .unwrap();
        std::fs::create_dir_all(store.root().join("state/applied")).unwrap();
        std::fs::write(store.root().join("state/applied/claude.json"), "{}").unwrap();
        std::fs::write(store.root().join("secrets.local.json"), "{}").unwrap();
        std::fs::create_dir_all(store.root().join("logs")).unwrap();
        std::fs::write(store.root().join("logs/app.log"), "log").unwrap();
        assert_eq!(
            base,
            content_fingerprint(&store).unwrap(),
            "不进快照的内容变化不该改变指纹"
        );

        write_mcp(&store, "pw", "Playwright v2");
        assert_ne!(base, content_fingerprint(&store).unwrap(), "真源改动必须被指纹看到");
    }

    #[test]
    fn archive_fingerprint_matches_content_fingerprint() {
        let (_dir, store, _index) = fixture();
        write_mcp(&store, "pw", "Playwright");
        let content = content_fingerprint(&store).unwrap();

        create(&store, Some("first"), SnapshotKind::Manual).unwrap();
        create(&store, Some("second"), SnapshotKind::Auto).unwrap();
        let first = fingerprint_of_archive(&snapshot_path(&store, "first").unwrap()).unwrap();
        let second = fingerprint_of_archive(&snapshot_path(&store, "second").unwrap()).unwrap();
        assert_eq!(first, second, "同内容的两份快照指纹必须相同");
        assert_eq!(first, content, "包内算出的指纹要和内存里算的可直接比较");
    }

    #[test]
    fn auto_snapshot_creates_only_when_content_changes() {
        let state = plain_state();
        assert!(auto_snapshot_if_changed(&state, 5).unwrap().is_some());
        assert!(
            auto_snapshot_if_changed(&state, 5).unwrap().is_none(),
            "内容没变不该再建一份等内容快照"
        );
        assert_eq!(list(&state.store).unwrap().len(), 1);

        write_mcp(&state.store, "other", "Other");
        assert!(auto_snapshot_if_changed(&state, 5).unwrap().is_some());
        assert_eq!(list(&state.store).unwrap().len(), 2);
    }

    #[test]
    fn auto_upload_skips_when_remote_already_matches() {
        let server = FakeWebDav::start("me", "pw");
        let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
        auto_snapshot_if_changed(&state, 5).unwrap().unwrap();
        auto_upload_if_needed(&state).unwrap();
        let puts = server.request_count("PUT");
        assert!(puts > 0, "首轮没有云端副本，应当上传");

        auto_upload_if_needed(&state).unwrap();
        assert_eq!(
            server.request_count("PUT"),
            puts,
            "云端已经是同一份内容，不该再覆盖一次"
        );
    }

    #[test]
    fn auto_upload_retries_after_failure() {
        let server = FakeWebDav::start("me", "pw");
        server.set_fail_put(true);
        let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
        auto_snapshot_if_changed(&state, 5).unwrap().unwrap();
        assert!(auto_upload_if_needed(&state).is_err());
        assert!(load_runtime_status(&state.store).remote_fingerprint.is_none());

        server.set_fail_put(false);
        auto_upload_if_needed(&state).unwrap();
        assert!(server
            .object("/dav/ccswitchx/ccswitchx-backup.zip")
            .is_some());
        assert_eq!(
            load_runtime_status(&state.store)
                .remote_fingerprint
                .as_deref(),
            Some(content_fingerprint(&state.store).unwrap().as_str()),
            "补传成功后基线要落值，否则每轮都会重传"
        );
    }

    #[test]
    fn auto_upload_uploads_manual_snapshot_absent_from_cloud() {
        let server = FakeWebDav::start("me", "pw");
        let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
        create(&state.store, Some("manual"), SnapshotKind::Manual).unwrap();

        auto_upload_if_needed(&state).unwrap();

        assert!(
            server
                .object("/dav/ccswitchx/ccswitchx-backup.zip")
                .is_some(),
            "手动备份过的内容云端还没有，得补上去"
        );
        assert_eq!(
            list(&state.store).unwrap().len(),
            1,
            "最新快照内容就是当前内容，不该再造 Auto 快照"
        );
    }

    #[test]
    fn periodic_once_auto_uploads_when_enabled() {
        let server = FakeWebDav::start("me", "pw");
        let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
        state.settings.lock().unwrap().backup.interval_hours = 1;

        periodic_once(&state);

        assert_eq!(list(&state.store).unwrap().len(), 1);
        assert!(server
            .object("/dav/ccswitchx/ccswitchx-backup.zip")
            .is_some());
    }

    #[test]
    fn periodic_once_skips_upload_when_disabled() {
        let server = FakeWebDav::start("me", "pw");
        let state = state_with_asset_and_cloud(false, server.base_url(), "pw");
        state.settings.lock().unwrap().backup.interval_hours = 1;

        periodic_once(&state);

        assert!(server
            .object("/dav/ccswitchx/ccswitchx-backup.zip")
            .is_none());
    }

    #[test]
    fn periodic_once_skips_upload_when_git_backup_is_active() {
        if !crate::services::git_sync::is_available() {
            return;
        }
        let server = FakeWebDav::start("me", "pw");
        let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
        state.settings.lock().unwrap().backup.interval_hours = 1;

        let bare = state.store.root().parent().unwrap().join("remote.git");
        let output = std::process::Command::new("git")
            .args(["init", "--bare"])
            .arg(&bare)
            .output()
            .unwrap();
        assert!(output.status.success());
        let url = format!("file:///{}", bare.to_string_lossy().replace('\\', "/"));
        crate::services::git_sync::init(&state.store, "main").unwrap();
        crate::services::git_sync::verify_remote(&state.store, &url, "main").unwrap();
        {
            let mut settings = state.settings.lock().unwrap();
            settings.backup.git.enabled = true;
            settings.backup.git.remote = Some(url);
        }

        periodic_once(&state);

        assert!(
            server
                .object("/dav/ccswitchx/ccswitchx-backup.zip")
                .is_none(),
            "Git 自动同步生效时不应再上传 WebDAV"
        );
    }

    #[test]
    fn auto_upload_failure_is_recorded() {
        let server = FakeWebDav::start("me", "pw");
        server.set_fail_put(true);
        let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
        state.settings.lock().unwrap().backup.interval_hours = 1;

        periodic_once(&state);

        assert!(load_runtime_status(&state.store).last_error.is_some());
    }
}
