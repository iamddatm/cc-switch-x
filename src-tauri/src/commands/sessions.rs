//! 会话命令面：只做加锁与参数传递，扫描与查询在 `services::sessions`。

use crate::domain::session::{SessionDetail, SessionProviderStatus, SessionSummary};
use crate::error::AppError;
use crate::index::{SessionMatch, SessionSearchHit};
use crate::services::sessions::{self, SessionScanReport};
use crate::sessions::export::{render_markdown, resume_info, ResumeInfo};
use crate::state::AppState;
use std::path::Path;
use tauri::State;

fn lock_index(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::index::Index>, AppError> {
    state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))
}

/// 全量增量扫描。必须放在阻塞线程里：本机 Claude 主会话约 710 MB。
#[tauri::command]
pub async fn scan_sessions(app: tauri::AppHandle) -> Result<SessionScanReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let mut index = lock_index(&state)?;
        sessions::scan_all(&state.store, &mut index, &state.home)
    })
    .await
    .map_err(|error| AppError::InvalidInput(format!("会话扫描任务失败：{error}")))?
}

#[tauri::command]
pub fn list_sessions(
    state: State<'_, AppState>,
    agent_id: Option<String>,
    query: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<Vec<SessionSummary>, AppError> {
    let index = lock_index(&state)?;
    sessions::list_sessions(&index, agent_id.as_deref(), query.as_deref(), offset, limit)
}

#[tauri::command]
pub fn get_session(
    state: State<'_, AppState>,
    agent_id: String,
    session_id: String,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<SessionDetail, AppError> {
    let index = lock_index(&state)?;
    sessions::get_session(&index, &agent_id, &session_id, offset, limit)
}

#[tauri::command]
pub fn search_sessions(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<SessionSearchHit>, AppError> {
    let index = lock_index(&state)?;
    sessions::search_sessions(&index, &query, limit)
}

#[tauri::command]
pub fn search_in_session(
    state: State<'_, AppState>,
    agent_id: String,
    session_id: String,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<SessionMatch>, AppError> {
    let index = lock_index(&state)?;
    sessions::search_in_session(&index, &agent_id, &session_id, &query, limit)
}

#[tauri::command]
pub fn session_providers(
    state: State<'_, AppState>,
) -> Result<Vec<SessionProviderStatus>, AppError> {
    let index = lock_index(&state)?;
    sessions::providers_status(&index, &state.store, &state.home)
}

#[tauri::command]
pub fn session_resume(
    state: State<'_, AppState>,
    agent_id: String,
    session_id: String,
) -> Result<ResumeInfo, AppError> {
    let index = lock_index(&state)?;
    let session = index
        .session_by_id(&agent_id, &session_id)?
        .ok_or_else(|| AppError::NotFound(format!("会话不存在：{session_id}")))?;
    let provider = crate::sessions::provider(&agent_id)
        .ok_or_else(|| AppError::Unsupported(format!("未知会话来源：{agent_id}")))?;
    resume_info(provider, &session)
        .ok_or_else(|| AppError::Unsupported("该会话没有可用的恢复命令".into()))
}

#[tauri::command]
pub fn export_session_markdown(
    state: State<'_, AppState>,
    agent_id: String,
    session_id: String,
    dest: String,
) -> Result<String, AppError> {
    let index = lock_index(&state)?;
    let detail = index.session_detail(&agent_id, &session_id, 0, u32::MAX)?;
    let markdown = render_markdown(&detail);
    let path = std::path::absolute(Path::new(&dest))?;
    let parent = path
        .parent()
        .ok_or_else(|| AppError::InvalidInput(format!("导出路径没有父目录：{}", path.display())))?;
    if !parent.is_dir() {
        return Err(AppError::InvalidInput(format!(
            "导出目录不存在：{}",
            parent.display()
        )));
    }
    crate::store::atomic::write_atomic(&path, markdown.as_bytes())?;
    Ok(path.to_string_lossy().into_owned())
}
