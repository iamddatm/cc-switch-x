use crate::commands::backup::open_directory;
use crate::error::AppError;
use crate::services::agents::{detect_all, AgentStatus};
use crate::services::project_registry::{self, ProjectSkillsView, ProjectView};
use crate::services::project_sync::{self, Resolution, SyncReport};
use crate::state::AppState;
use std::path::Path;
use tauri::{AppHandle, Manager, State};

async fn run_blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| AppError::Project(format!("项目任务执行失败：{error}")))?
}

fn enabled_agents(state: &AppState) -> Result<Vec<AgentStatus>, AppError> {
    let mut agents = detect_all(&state.store, &state.home)?;
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    for agent in &mut agents {
        agent.enabled = crate::settings::is_agent_enabled(&settings, &agent.id);
    }
    Ok(agents)
}

fn enabled_agent_ids(agents: &[AgentStatus]) -> Vec<String> {
    agents
        .iter()
        .filter(|agent| agent.enabled)
        .map(|agent| agent.id.clone())
        .collect()
}

#[tauri::command]
pub fn list_projects(state: State<'_, AppState>) -> Result<Vec<ProjectView>, AppError> {
    project_registry::list_projects(&state.store)
}

#[tauri::command]
pub async fn add_project(
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectView, AppError> {
    let store = state.store.clone();
    let home = state.home.clone();
    let project_ops = state.project_ops.clone();
    let enabled = enabled_agent_ids(&enabled_agents(state.inner())?);
    run_blocking(move || {
        let _guard = project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        project_registry::add_project(&store, &home, Path::new(&path), &enabled)
    })
    .await
}

#[tauri::command]
pub async fn remove_project(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let store = state.store.clone();
    let project_ops = state.project_ops.clone();
    run_blocking(move || {
        let _guard = project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        project_registry::remove_project(&store, &id)
    })
    .await
}

#[tauri::command]
pub async fn touch_project(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProjectView, AppError> {
    let store = state.store.clone();
    let project_ops = state.project_ops.clone();
    run_blocking(move || {
        let _guard = project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        project_registry::touch_project(&store, &id)
    })
    .await
}

#[tauri::command]
pub async fn set_project_auto_sync(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<ProjectView, AppError> {
    let store = state.store.clone();
    let project_ops = state.project_ops.clone();
    run_blocking(move || {
        let _guard = project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        project_registry::set_auto_sync(&store, &id, enabled)
    })
    .await
}

#[tauri::command]
pub async fn list_project_skills(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectSkillsView, AppError> {
    let store = state.store.clone();
    let project_ops = state.project_ops.clone();
    let agents = enabled_agents(state.inner())?;
    run_blocking(move || {
        let _guard = project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        project_registry::list_project_skills(&store, &project_id, &agents)
    })
    .await
}

#[tauri::command]
pub fn reveal_project_folder(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<(), AppError> {
    let registry = crate::store::projects::load(&state.store)?;
    let entry = registry
        .projects
        .iter()
        .find(|entry| entry.id == project_id)
        .ok_or_else(|| AppError::NotFound(format!("项目不存在：{project_id}")))?;
    open_directory(&entry.path)
}

#[tauri::command]
pub async fn sync_project_skills(
    app: AppHandle,
    project_id: String,
    resolutions: Vec<Resolution>,
) -> Result<SyncReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let enabled = enabled_agent_ids(&enabled_agents(state.inner())?);
        let registry = crate::store::projects::load(&state.store)?;
        let entry = registry
            .projects
            .iter()
            .find(|entry| entry.id == project_id)
            .ok_or_else(|| AppError::NotFound(format!("项目不存在：{project_id}")))?;
        if !entry.path.is_dir() {
            return Err(AppError::NotFound(format!(
                "项目路径不存在：{}",
                entry.path.display()
            )));
        }
        project_sync::sync_project(
            &state.store,
            &project_id,
            &entry.path,
            &enabled,
            &resolutions,
        )
    })
    .await
}
