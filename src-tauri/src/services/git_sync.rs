//! Git 备份：把可移植真源变成工作区，自动提交并在配置了远端时同步。
//!
//! 安全边界：
//! - 所有 git 调用都用参数数组，不经过 shell；
//! - 身份用 `-c` 临时注入，不改用户全局配置；自动提交禁用交互式签名，避免弹窗挂起；
//! - 拉取/覆盖前先做快照；冲突立即 `rebase --abort` 并暂停自动同步；
//! - Git 失败只记录状态，绝不让用户已经成功的数据写入失败。

use crate::error::AppError;
use crate::index::Index;
use crate::services::backup::{self, SnapshotKind};
use crate::settings::{validate_git_branch, validate_git_remote, GitBackupSettings};
use crate::state::AppState;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const STATE_FILE: &str = "state/git.json";
const PULL_INTERVAL: Duration = Duration::from_secs(30 * 60);
const COMMIT_DEBOUNCE: Duration = Duration::from_secs(3);
const COMMIT_MAX_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum GitEvent {
    /// 真源刚被改过；worker 防抖后合并成一次提交。
    Changed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitSyncState {
    Unavailable,
    Disabled,
    Idle,
    Syncing,
    Paused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub available: bool,
    pub initialized: bool,
    pub enabled: bool,
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub dirty: u64,
    pub ahead: u64,
    pub behind: u64,
    pub last_commit_at: Option<i64>,
    pub last_pull_at: Option<i64>,
    pub state: GitSyncState,
    pub last_error: Option<String>,
    pub conflict_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub hash: String,
    pub short_hash: String,
    pub message: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRestoreReport {
    pub safety_backup: String,
    pub commit: String,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PullOutcome {
    UpToDate,
    NoRemote,
    Pulled { commits: u64, safety_backup: String },
    Conflict { files: Vec<String>, safety_backup: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Persisted {
    last_commit_at: Option<i64>,
    last_pull_at: Option<i64>,
    last_error: Option<String>,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    conflict_files: Vec<String>,
}

impl Persisted {
    fn load(store: &Store) -> Self {
        let path = store.root().join(STATE_FILE);
        fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, store: &Store) -> Result<(), AppError> {
        let path = store.root().join(STATE_FILE);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&path, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    fn clear_error(&mut self) {
        self.last_error = None;
        self.paused = false;
        self.conflict_files.clear();
    }

    fn mark_error(&mut self, message: impl Into<String>) {
        self.last_error = Some(message.into());
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 拉取/恢复可能带回旧版 `.gitignore`；补齐必需行，避免 secrets/local/backups 被误提交。
fn ensure_layout(store: &Store) -> Result<(), AppError> {
    Store::init(store.root().to_path_buf())?;
    Ok(())
}

pub fn is_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// 必须是我们自己 `git init` 出来的工作区，不能只凭父目录是仓库就认。
fn initialized(store: &Store) -> bool {
    store.root().join(".git").is_dir()
        && run_git(store.root(), &["rev-parse", "--is-inside-work-tree"])
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "true"
            })
            .unwrap_or(false)
}

fn run_git(root: &Path, args: &[&str]) -> Result<Output, AppError> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| AppError::Git(format!("无法运行 git：{error}")))
}

fn git_out(root: &Path, args: &[&str]) -> Result<String, AppError> {
    let output = run_git(root, args)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Git(format!(
            "git {} 失败：{}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_ok(root: &Path, args: &[&str]) -> bool {
    run_git(root, args)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn current_branch(root: &Path) -> Option<String> {
    git_out(root, &["symbolic-ref", "--short", "HEAD"]).ok()
}

fn has_head(root: &Path) -> bool {
    git_ok(root, &["rev-parse", "--verify", "HEAD"])
}

fn set_branch(root: &Path, branch: &str) -> Result<(), AppError> {
    if has_head(root) {
        git_out(root, &["branch", "-M", branch])?;
    } else {
        git_out(root, &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])?;
    }
    Ok(())
}

fn dirty_paths(root: &Path) -> Vec<String> {
    git_out(root, &["status", "--porcelain"])
        .map(|text| {
            text.lines()
                .filter_map(|line| line.get(3..).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn commit_count(root: &Path, range: &str) -> u64 {
    git_out(root, &["rev-list", "--count", range])
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

pub fn status(store: &Store, git: &GitBackupSettings) -> GitStatus {
    let available = is_available();
    let initialized = available && initialized(store);
    let persisted = Persisted::load(store);
    let branch = if initialized {
        current_branch(store.root()).or_else(|| Some(git.branch.clone()))
    } else {
        Some(git.branch.clone())
    };
    let remote = git.remote.clone();
    let dirty = if initialized {
        dirty_paths(store.root()).len() as u64
    } else {
        0
    };
    let (ahead, behind) = if initialized {
        match (&remote, &branch) {
            (Some(_), Some(branch)) if git_ok(store.root(), &["rev-parse", "--verify", &format!("origin/{branch}")]) => {
                let text = git_out(
                    store.root(),
                    &["rev-list", "--left-right", "--count", &format!("HEAD...origin/{branch}")],
                )
                .unwrap_or_default();
                let mut values = text.split_whitespace();
                (
                    values.next().and_then(|value| value.parse().ok()).unwrap_or(0),
                    values.next().and_then(|value| value.parse().ok()).unwrap_or(0),
                )
            }
            _ => (0, 0),
        }
    } else {
        (0, 0)
    };
    let last_commit_at = if initialized {
        git_out(store.root(), &["log", "-1", "--format=%ct"])
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .map(|seconds| seconds * 1000)
    } else {
        None
    };
    let state = if !available {
        GitSyncState::Unavailable
    } else if !initialized || !git.enabled {
        GitSyncState::Disabled
    } else if persisted.paused {
        GitSyncState::Paused
    } else if persisted.last_error.is_some() {
        GitSyncState::Error
    } else {
        GitSyncState::Idle
    };
    GitStatus {
        available,
        initialized,
        enabled: git.enabled,
        branch,
        remote,
        dirty,
        ahead,
        behind,
        last_commit_at,
        last_pull_at: persisted.last_pull_at,
        state,
        last_error: persisted.last_error,
        conflict_files: persisted.conflict_files,
    }
}

/// 初始化仓库并提交第一版；调用方负责写回 `git.enabled = true`。
pub fn init(store: &Store, branch: &str) -> Result<(), AppError> {
    if !is_available() {
        return Err(AppError::Git("未检测到 git，请先安装 Git 并加入 PATH".into()));
    }
    validate_git_branch(branch).map_err(AppError::InvalidInput)?;
    if !initialized(store) {
        git_out(store.root(), &["init"])?;
    }
    ensure_layout(store)?;
    set_branch(store.root(), branch)?;
    commit_all(store, "init: CC Switch X 备份")?;
    Ok(())
}

pub fn set_remote(
    store: &Store,
    remote: Option<&str>,
    branch: &str,
) -> Result<(), AppError> {
    if !initialized(store) {
        return Err(AppError::Git("请先初始化 Git 备份".into()));
    }
    validate_git_branch(branch).map_err(AppError::InvalidInput)?;
    match remote.map(str::trim).filter(|value| !value.is_empty()) {
        Some(remote) => {
            validate_git_remote(remote).map_err(AppError::InvalidInput)?;
            if git_ok(store.root(), &["remote", "get-url", "origin"]) {
                git_out(store.root(), &["remote", "set-url", "origin", remote])?;
            } else {
                git_out(store.root(), &["remote", "add", "origin", remote])?;
            }
        }
        None => {
            if git_ok(store.root(), &["remote", "get-url", "origin"]) {
                git_out(store.root(), &["remote", "remove", "origin"])?;
            }
        }
    }
    set_branch(store.root(), branch)?;
    Ok(())
}

/// 成功提交返回 true；没有变更返回 false。
pub fn commit_all(store: &Store, message: &str) -> Result<bool, AppError> {
    if !initialized(store) {
        return Err(AppError::Git("Git 备份尚未初始化".into()));
    }
    if dirty_paths(store.root()).is_empty() {
        return Ok(false);
    }
    git_out(store.root(), &["add", "-A", "--", "."])?;
    let output = run_git(
        store.root(),
        &[
            "-c",
            "user.name=CC Switch X",
            "-c",
            "user.email=ccswitchx@localhost",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            message,
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if stderr.contains("nothing to commit") || stderr.contains("no changes added") {
            return Ok(false);
        }
        return Err(AppError::Git(format!(
            "提交失败：{}",
            stderr.trim()
        )));
    }
    let mut persisted = Persisted::load(store);
    persisted.last_commit_at = Some(now_ms());
    persisted.clear_error();
    persisted.save(store)?;
    Ok(true)
}

pub fn push(store: &Store, branch: &str) -> Result<(), AppError> {
    if !initialized(store) {
        return Err(AppError::Git("Git 备份尚未初始化".into()));
    }
    validate_git_branch(branch).map_err(AppError::InvalidInput)?;
    git_out(
        store.root(),
        &["push", "origin", &format!("HEAD:{branch}")],
    )?;
    Ok(())
}

fn fetch(store: &Store, branch: &str) -> Result<bool, AppError> {
    let output = run_git(store.root(), &["fetch", "--prune", "origin", branch])?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("couldn't find remote ref") || stderr.contains("could not find remote ref") {
        return Ok(false);
    }
    Err(AppError::Git(format!("拉取远端失败：{}", stderr.trim())))
}

fn rebase_onto_remote(store: &Store, branch: &str) -> Result<(), AppError> {
    git_out(store.root(), &["rebase", &format!("origin/{branch}")])?;
    Ok(())
}

pub fn conflict_files(store: &Store) -> Vec<String> {
    git_out(store.root(), &["diff", "--name-only", "--diff-filter=U"])
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// 拉取：先提交本地变更，再快照，最后 rebase。调用方持 maintenance 写锁与 index 锁。
pub fn pull(
    store: &Store,
    index: &mut Index,
    git: &GitBackupSettings,
    retain: usize,
) -> Result<PullOutcome, AppError> {
    if !initialized(store) {
        return Err(AppError::Git("Git 备份尚未初始化".into()));
    }
    let Some(_) = git.remote else {
        return Ok(PullOutcome::NoRemote);
    };
    let branch = git.branch.clone();
    validate_git_branch(&branch).map_err(AppError::InvalidInput)?;
    if !dirty_paths(store.root()).is_empty() {
        commit_all(store, "sync: 拉取前保存本地变更")?;
    }
    if !fetch(store, &branch)? {
        return Ok(PullOutcome::NoRemote);
    }
    let behind = commit_count(store.root(), &format!("HEAD..origin/{branch}"));
    if behind == 0 {
        let mut persisted = Persisted::load(store);
        persisted.last_pull_at = Some(now_ms());
        if !persisted.paused {
            persisted.clear_error();
        }
        persisted.save(store)?;
        return Ok(PullOutcome::UpToDate);
    }
    let safety = backup::create(store, None, SnapshotKind::PrePull)?;
    let _ = backup::prune(store, retain.max(1));

    if let Err(error) = rebase_onto_remote(store, &branch) {
        let files = conflict_files(store);
        let _ = run_git(store.root(), &["rebase", "--abort"]);
        let mut persisted = Persisted::load(store);
        persisted.paused = true;
        persisted.conflict_files = files.clone();
        persisted.mark_error(format!("拉取冲突：{error}"));
        persisted.save(store)?;
        if files.is_empty() {
            return Err(AppError::Git(format!(
                "拉取失败且没有可列出的冲突文件：{error}"
            )));
        }
        return Ok(PullOutcome::Conflict {
            files,
            safety_backup: safety.name,
        });
    }

    ensure_layout(store)?;
    let mut persisted = Persisted::load(store);
    persisted.last_pull_at = Some(now_ms());
    persisted.clear_error();
    persisted.save(store)?;
    index.reset_agent_state_and_drift()?;
    index.rebuild(store)?;
    Ok(PullOutcome::Pulled {
        commits: behind,
        safety_backup: safety.name,
    })
}

/// 冲突时选择保留本地：强推覆盖远端。必须由 UI 二次确认。
pub fn keep_local(store: &Store, git: &GitBackupSettings) -> Result<(), AppError> {
    let branch = git.branch.clone();
    validate_git_branch(&branch).map_err(AppError::InvalidInput)?;
    git_out(
        store.root(),
        &[
            "push",
            "--force-with-lease",
            "origin",
            &format!("HEAD:{branch}"),
        ],
    )?;
    let mut persisted = Persisted::load(store);
    persisted.last_pull_at = Some(now_ms());
    persisted.clear_error();
    persisted.save(store)?;
    Ok(())
}

/// 冲突时选择保留远端：先快照，再 reset --hard。调用方持 maintenance 写锁与 index 锁。
pub fn keep_remote(
    store: &Store,
    index: &mut Index,
    git: &GitBackupSettings,
    retain: usize,
) -> Result<String, AppError> {
    let branch = git.branch.clone();
    validate_git_branch(&branch).map_err(AppError::InvalidInput)?;
    if !fetch(store, &branch)? {
        return Err(AppError::Git("远端分支不存在".into()));
    }
    let safety = backup::create(store, None, SnapshotKind::PrePull)?;
    let _ = backup::prune(store, retain.max(1));
    git_out(store.root(), &["reset", "--hard", &format!("origin/{branch}")])?;
    ensure_layout(store)?;
    index.reset_agent_state_and_drift()?;
    index.rebuild(store)?;
    let mut persisted = Persisted::load(store);
    persisted.last_pull_at = Some(now_ms());
    persisted.clear_error();
    persisted.save(store)?;
    Ok(safety.name)
}

/// 恢复到某次提交：先快照、提交当前变更，再 read-tree 到一个新提交。调用方持写锁与 index 锁。
pub fn restore_commit(
    store: &Store,
    index: &mut Index,
    hash: &str,
    retain: usize,
) -> Result<GitRestoreReport, AppError> {
    if !is_valid_hash(hash) {
        return Err(AppError::InvalidInput("提交哈希格式不对".into()));
    }
    if !initialized(store) {
        return Err(AppError::Git("Git 备份尚未初始化".into()));
    }
    if !git_ok(store.root(), &["cat-file", "-e", &format!("{hash}^{{commit}}")]) {
        return Err(AppError::NotFound(format!("提交不存在：{hash}")));
    }
    let safety = backup::create(store, None, SnapshotKind::PreRestore)?;
    let _ = backup::prune(store, retain.max(1));
    if !dirty_paths(store.root()).is_empty() {
        commit_all(store, "restore: 恢复前保存当前变更")?;
    }
    let old_head = git_out(store.root(), &["rev-parse", "HEAD"]).ok();
    git_out(store.root(), &["read-tree", "--reset", "-u", hash])?;
    // 旧提交里的 .gitignore 可能缺少必需行；在生成恢复提交前补齐。
    ensure_layout(store)?;
    let subject = git_out(store.root(), &["log", "-1", "--format=%s", hash]).unwrap_or_default();
    let short = &hash[..hash.len().min(8)];
    let changed = commit_all(
        store,
        &format!("restore: {short} {}", subject.trim()),
    )?;
    let new_head = git_out(store.root(), &["rev-parse", "HEAD"]).unwrap_or_default();
    let files = old_head
        .as_deref()
        .map(|old| {
            git_out(store.root(), &["diff", "--name-only", old, "HEAD"])
                .map(|text| text.lines().count() as u64)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    index.reset_agent_state_and_drift()?;
    index.rebuild(store)?;
    Ok(GitRestoreReport {
        safety_backup: safety.name,
        commit: if changed { new_head } else { hash.to_string() },
        files,
    })
}

fn is_valid_hash(hash: &str) -> bool {
    (7..=40).contains(&hash.len()) && hash.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn list_commits(store: &Store, limit: usize) -> Result<Vec<GitCommit>, AppError> {
    if !initialized(store) {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 200);
    let text = git_out(
        store.root(),
        &[
            "log",
            "--pretty=format:%H%x1f%h%x1f%at%x1f%s",
            "-n",
            &limit.to_string(),
        ],
    )?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let hash = parts.next()?.to_string();
            let short_hash = parts.next()?.to_string();
            let timestamp = parts.next()?.parse::<i64>().ok()? * 1000;
            let message = parts.next().unwrap_or_default().to_string();
            Some(GitCommit {
                hash,
                short_hash,
                message,
                timestamp,
            })
        })
        .collect())
}

pub fn record_error(store: &Store, message: impl Into<String>) {
    let mut persisted = Persisted::load(store);
    persisted.mark_error(message);
    let _ = persisted.save(store);
}

fn git_snapshot(state: &AppState) -> Option<(GitBackupSettings, u32)> {
    let settings = state.settings.lock().ok()?;
    Some((settings.backup.git.clone(), settings.backup.retain_count))
}

fn auto_commit_allowed(git: &GitBackupSettings) -> bool {
    git.enabled && git.auto_commit
}

fn auto_pull_allowed(git: &GitBackupSettings) -> bool {
    git.enabled && git.auto_pull && git.remote.is_some()
}

pub fn spawn_worker(app: AppHandle, rx: Receiver<GitEvent>) {
    let _ = std::thread::Builder::new()
        .name("cc-switch-x-git".into())
        .spawn(move || {
            startup_sync(&app);
            let mut pending: Option<(String, Instant)> = None;
            let mut next_pull = Instant::now();
            loop {
                let now = Instant::now();
                let timeout = match &pending {
                    Some((_, since)) => {
                        let elapsed = now.saturating_duration_since(*since);
                        COMMIT_DEBOUNCE.min(COMMIT_MAX_DELAY.saturating_sub(elapsed))
                    }
                    None => next_pull
                        .saturating_duration_since(now)
                        .max(Duration::from_secs(1)),
                };
                match rx.recv_timeout(timeout) {
                    Ok(GitEvent::Changed(message)) => {
                        if pending.is_none() {
                            pending = Some((message.clone(), Instant::now()));
                        } else if let Some((slot, _)) = pending.as_mut() {
                            *slot = message;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if let Some((message, _)) = pending.take() {
                            run_auto_commit(&app, &message);
                        } else if Instant::now() >= next_pull {
                            run_auto_pull(&app);
                            next_pull = Instant::now() + PULL_INTERVAL;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
}

fn run_auto_commit(app: &AppHandle, message: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some((git, _retain)) = git_snapshot(&state) else {
        return;
    };
    if !auto_commit_allowed(&git) || !initialized(&state.store) {
        return;
    }
    let changed = {
        let Ok(_maintenance) = state.maintenance.write() else {
            return;
        };
        let Ok(_ops) = state.git_ops.lock() else {
            return;
        };
        match commit_all(&state.store, message) {
            Ok(changed) => changed,
            Err(error) => {
                record_error(&state.store, error.to_string());
                return;
            }
        }
    };
    // push 只碰 .git 与网络，不需要挡真源写操作；失败留给下一次或手动重试。
    if changed && git.remote.is_some() && !Persisted::load(&state.store).paused {
        let Ok(_ops) = state.git_ops.lock() else {
            return;
        };
        if let Err(error) = push(&state.store, &git.branch) {
            record_error(&state.store, error.to_string());
        }
    }
}

fn run_auto_pull(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some((git, retain)) = git_snapshot(&state) else {
        return;
    };
    if !auto_pull_allowed(&git) || !initialized(&state.store) {
        return;
    }
    if Persisted::load(&state.store).paused {
        return;
    }
    let Ok(_maintenance) = state.maintenance.write() else {
        return;
    };
    let Ok(_ops) = state.git_ops.lock() else {
        return;
    };
    let Ok(mut index) = state.index.lock() else {
        return;
    };
    match pull(&state.store, &mut index, &git, retain as usize) {
        Ok(PullOutcome::Conflict { .. }) | Ok(_) => {}
        Err(error) => record_error(&state.store, error.to_string()),
    }
}

/// 启动时补交上次退出前可能遗留的变更，并触发一次拉取。
pub fn startup_sync(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some((git, _retain)) = git_snapshot(&state) else {
        return;
    };
    if !git.enabled || !initialized(&state.store) {
        return;
    }
    if !dirty_paths(state.store.root()).is_empty() {
        let Ok(_maintenance) = state.maintenance.write() else {
            return;
        };
        let Ok(_ops) = state.git_ops.lock() else {
            return;
        };
        if let Err(error) = commit_all(&state.store, "startup: 捕获上次退出前的变更") {
            record_error(&state.store, error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::settings::GitBackupSettings;
    use tempfile::tempdir;

    fn git_fixture() -> Option<(tempfile::TempDir, Store, Index, std::path::PathBuf)> {
        if !is_available() {
            return None;
        }
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&store.root().join("index.db")).unwrap();
        let remote = dir.path().join("remote.git");
        let output = Command::new("git")
            .args(["init", "--bare"])
            .arg(&remote)
            .output()
            .unwrap();
        assert!(output.status.success());
        Some((dir, store, index, remote))
    }

    fn remote_url(path: &Path) -> String {
        format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
    }

    fn clone_remote(remote: &Path, dest: &Path) -> std::path::PathBuf {
        let output = Command::new("git")
            .args(["clone", "-b", "main"])
            .arg(remote)
            .arg(dest)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "clone 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        dest.to_path_buf()
    }

    fn write_mcp(store: &Store, name: &str) {
        store
            .write(
                AssetType::Mcp,
                "pw",
                format!(
                    r#"{{"id":"pw","name":"{name}","transport":"stdio","command":"npx"}}"#
                )
                .as_bytes(),
            )
            .unwrap();
    }

    fn git_settings(remote: String) -> GitBackupSettings {
        GitBackupSettings {
            enabled: true,
            auto_commit: true,
            auto_pull: true,
            remote: Some(remote),
            branch: "main".into(),
        }
    }

    #[test]
    fn init_commit_push_and_pull_roundtrip() {
        let Some((_dir, store, mut index, remote)) = git_fixture() else {
            return;
        };
        write_mcp(&store, "Playwright");
        init(&store, "main").unwrap();
        set_remote(&store, Some(&remote_url(&remote)), "main").unwrap();
        push(&store, "main").unwrap();

        let settings = git_settings(remote_url(&remote));
        let status = status(&store, &settings);
        assert!(status.initialized);
        assert_eq!(status.dirty, 0);
        assert_eq!(status.branch.as_deref(), Some("main"));
        let commits = list_commits(&store, 10).unwrap();
        assert!(!commits.is_empty());

        // 远端出现新提交后，本地 pull 应更新文件并重建索引。
        let other = clone_remote(&remote, &_dir.path().join("other"));
        std::fs::write(
            other.join("assets/mcp/pw.json"),
            r#"{"id":"pw","name":"Remote","transport":"stdio","command":"npx"}"#,
        )
        .unwrap();
        for args in [
            vec!["-C", other.to_str().unwrap(), "add", "-A"],
            vec![
                "-C",
                other.to_str().unwrap(),
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "remote",
            ],
            vec!["-C", other.to_str().unwrap(), "push", "origin", "main"],
        ] {
            let output = Command::new("git").args(args).output().unwrap();
            assert!(output.status.success());
        }

        let outcome = pull(&store, &mut index, &settings, 10).unwrap();
        assert!(matches!(outcome, PullOutcome::Pulled { .. }));
        let text = String::from_utf8(store.read(AssetType::Mcp, "pw").unwrap()).unwrap();
        assert!(text.contains("Remote"));
        assert_eq!(index.list(Some(AssetType::Mcp)).unwrap().len(), 1);
    }

    #[test]
    fn conflict_pauses_and_keep_remote_restores() {
        let Some((_dir, store, mut index, remote)) = git_fixture() else {
            return;
        };
        write_mcp(&store, "Playwright");
        init(&store, "main").unwrap();
        set_remote(&store, Some(&remote_url(&remote)), "main").unwrap();
        push(&store, "main").unwrap();

        let other = clone_remote(&remote, &_dir.path().join("other"));
        std::fs::write(
            other.join("assets/mcp/pw.json"),
            r#"{"id":"pw","name":"Remote","transport":"stdio","command":"npx"}"#,
        )
        .unwrap();
        for args in [
            vec!["-C", other.to_str().unwrap(), "add", "-A"],
            vec![
                "-C",
                other.to_str().unwrap(),
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "remote",
            ],
            vec!["-C", other.to_str().unwrap(), "push", "origin", "main"],
        ] {
            let output = Command::new("git").args(args).output().unwrap();
            assert!(output.status.success());
        }
        write_mcp(&store, "Local");
        let settings = git_settings(remote_url(&remote));
        let outcome = pull(&store, &mut index, &settings, 10).unwrap();
        let PullOutcome::Conflict { files, .. } = outcome else {
            panic!("应当检测到冲突");
        };
        assert!(files.iter().any(|file| file.contains("pw.json")));
        assert!(status(&store, &settings).paused_is_true());

        let safety = keep_remote(&store, &mut index, &settings, 10).unwrap();
        assert!(!safety.is_empty());
        let text = String::from_utf8(store.read(AssetType::Mcp, "pw").unwrap()).unwrap();
        assert!(text.contains("Remote"));
        assert!(!status(&store, &settings).conflict_files.iter().any(|file| file.contains("pw.json")));
    }

    #[test]
    fn restore_commit_creates_a_new_commit_and_keeps_local_state() {
        let Some((_dir, store, mut index, _remote)) = git_fixture() else {
            return;
        };
        write_mcp(&store, "v1");
        std::fs::write(
            store.root().join("local/agents/claude.json"),
            r#"{"baseOverride":"D:/local"}"#,
        )
        .unwrap();
        init(&store, "main").unwrap();
        let first = list_commits(&store, 1).unwrap()[0].hash.clone();

        write_mcp(&store, "v2");
        commit_all(&store, "second").unwrap();
        let report = restore_commit(&store, &mut index, &first, 10).unwrap();
        assert!(!report.safety_backup.is_empty());
        let text = String::from_utf8(store.read(AssetType::Mcp, "pw").unwrap()).unwrap();
        assert!(text.contains("v1"));
        assert_eq!(
            std::fs::read_to_string(store.root().join("local/agents/claude.json")).unwrap(),
            r#"{"baseOverride":"D:/local"}"#
        );
        assert_eq!(list_commits(&store, 1).unwrap()[0].hash, report.commit);
    }

    trait StatusExt {
        fn paused_is_true(&self) -> bool;
    }

    impl StatusExt for GitStatus {
        fn paused_is_true(&self) -> bool {
            self.state == GitSyncState::Paused
                && !self.conflict_files.is_empty()
        }
    }
}
