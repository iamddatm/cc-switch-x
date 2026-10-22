//! 会话命令面：只做加锁与参数传递，扫描与查询在 `services::sessions`。

use crate::domain::session::{SessionDetail, SessionProviderStatus, SessionSummary};
use crate::error::AppError;
use crate::index::{SessionMatch, SessionSearchHit};
use crate::services::sessions::{self, SessionScanReport};
use crate::state::AppState;
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
