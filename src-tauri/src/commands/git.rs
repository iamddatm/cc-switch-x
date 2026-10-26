//! Git 备份命令。网络操作放进 `spawn_blocking`，并通过 `git_ops` 串行。

use crate::error::AppError;
use crate::services::git_sync::{
    self, GitCommit, GitRestoreReport, GitStatus, PullOutcome,
};
use crate::services::settings as settings_service;
use crate::settings::{self as settings_file, AppSettings, GitBackupSettings};
use crate::state::AppState;
use tauri::{AppHandle, Manager};

fn git_settings(state: &AppState) -> Result<GitBackupSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok(settings.backup.git.clone())
}

fn save_git_settings(
    state: &AppState,
    change: impl FnOnce(&mut GitBackupSettings),
) -> Result<AppSettings, AppError> {
    let mut backup = {
        let settings = state
            .settings
            .lock()
            .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
        settings.backup.clone()
    };
    change(&mut backup.git);
    let updated =
        settings_service::set_backup_settings_at(&settings_file::settings_path()?, backup)?;
    let mut current = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *current = updated.clone();
    Ok(updated)
}

async fn run_blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| AppError::Git(format!("Git 任务执行失败：{error}")))?
}

#[tauri::command]
pub async fn get_git_status(app: AppHandle) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let git = git_settings(&state)?;
        Ok(git_sync::status(&state.store, &git))
    })
    .await
}

#[tauri::command]
pub async fn git_list_commits(
    app: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<GitCommit>, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        git_sync::list_commits(&state.store, limit.unwrap_or(50) as usize)
    })
    .await
}

#[tauri::command]
pub async fn git_init(app: AppHandle, branch: Option<String>) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let branch = branch.unwrap_or_else(|| {
            git_settings(&state)
                .map(|git| git.branch)
                .unwrap_or_else(|_| "main".into())
        });
        {
            let _maintenance = state.enter_maintenance()?;
            let _ops = state
                .git_ops
                .lock()
                .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
            git_sync::init(&state.store, &branch)?;
        }
        let updated = save_git_settings(&state, |git| {
            git.branch = branch;
        })?;
        Ok(git_sync::status(&state.store, &updated.backup.git))
    })
    .await
}

/// 保存远端与分支。地址变化会让此前的验证失效并停用自动同步，必须重新验证。
fn apply_remote(
    state: &AppState,
    remote: Option<String>,
    branch: String,
) -> Result<AppSettings, AppError> {
    let remote = remote
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    {
        let _ops = state
            .git_ops
            .lock()
            .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
        git_sync::set_remote(&state.store, remote.as_deref(), &branch)?;
    }
    save_git_settings(state, |git| {
        if git.remote != remote {
            git.enabled = false;
        }
        git.remote = remote;
        git.branch = branch;
    })
}

#[tauri::command]
pub async fn git_set_remote(
    app: AppHandle,
    remote: Option<String>,
    branch: String,
) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let updated = apply_remote(&state, remote, branch)?;
        Ok(git_sync::status(&state.store, &updated.backup.git))
    })
    .await
}

/// 保存表单里的远端并执行 `git ls-remote`；验证通过后才允许开启 Git 备份。
#[tauri::command]
pub async fn git_verify_remote(
    app: AppHandle,
    remote: Option<String>,
    branch: String,
) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let Some(candidate) = remote
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            return Err(AppError::InvalidInput("请先填写远端地址".into()));
        };
        // 先保存地址（失败也会保留以便修正），再联网验证；验证结果与地址绑定。
        let updated = apply_remote(&state, remote, branch.clone())?;
        {
            let _ops = state
                .git_ops
                .lock()
                .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
            git_sync::verify_remote(&state.store, &candidate, &branch)?;
        }
        Ok(git_sync::status(&state.store, &updated.backup.git))
    })
    .await
}

/// 立即提交当前真源；不自动推送，推送走「立即推送」或自动同步。
#[tauri::command]
pub async fn git_commit_now(
    app: AppHandle,
    message: Option<String>,
) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let message = message
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "backup: {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                )
            });
        {
            let _maintenance = state.enter_maintenance()?;
            let _ops = state
                .git_ops
                .lock()
                .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
            git_sync::commit_all(&state.store, &message)?;
        }
        let git = git_settings(&state)?;
        Ok(git_sync::status(&state.store, &git))
    })
    .await
}

#[tauri::command]
pub async fn git_push_now(app: AppHandle) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let git = git_settings(&state)?;
        {
            let _ops = state
                .git_ops
                .lock()
                .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
            if let Err(error) = git_sync::push(&state.store, &git.branch) {
                git_sync::record_error(&state.store, error.to_string());
                return Err(error);
            }
        }
        Ok(git_sync::status(&state.store, &git))
    })
    .await
}

#[tauri::command]
pub async fn git_pull_now(app: AppHandle) -> Result<PullOutcome, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let git = git_settings(&state)?;
        let retain = {
            let settings = state
                .settings
                .lock()
                .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
            settings.backup.retain_count.max(1) as usize
        };
        let _maintenance = state.enter_maintenance()?;
        let _ops = state
            .git_ops
            .lock()
            .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        match git_sync::pull(&state.store, &mut index, &git, retain) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                git_sync::record_error(&state.store, error.to_string());
                Err(error)
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn git_resolve_conflict(
    app: AppHandle,
    action: String,
) -> Result<GitStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let git = git_settings(&state)?;
        match action.as_str() {
            "keepLocal" => {
                let _ops = state
                    .git_ops
                    .lock()
                    .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
                if let Err(error) = git_sync::keep_local(&state.store, &git) {
                    git_sync::record_error(&state.store, error.to_string());
                    return Err(error);
                }
            }
            "keepRemote" => {
                let retain = {
                    let settings = state
                        .settings
                        .lock()
                        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
                    settings.backup.retain_count.max(1) as usize
                };
                let _maintenance = state.enter_maintenance()?;
                let _ops = state
                    .git_ops
                    .lock()
                    .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
                let mut index = state
                    .index
                    .lock()
                    .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
                if let Err(error) = git_sync::keep_remote(&state.store, &mut index, &git, retain) {
                    git_sync::record_error(&state.store, error.to_string());
                    return Err(error);
                }
            }
            other => {
                return Err(AppError::InvalidInput(format!(
                    "未知的冲突处置：{other}"
                )))
            }
        }
        Ok(git_sync::status(&state.store, &git))
    })
    .await
}

#[tauri::command]
pub async fn git_restore_commit(
    app: AppHandle,
    hash: String,
) -> Result<GitRestoreReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let retain = {
            let settings = state
                .settings
                .lock()
                .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
            settings.backup.retain_count.max(1) as usize
        };
        let _maintenance = state.enter_maintenance()?;
        let _ops = state
            .git_ops
            .lock()
            .map_err(|_| AppError::InvalidInput("git 锁失败".into()))?;
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        match git_sync::restore_commit(&state.store, &mut index, &hash, retain) {
            Ok(report) => Ok(report),
            Err(error) => {
                git_sync::record_error(&state.store, error.to_string());
                Err(error)
            }
        }
    })
    .await
}
