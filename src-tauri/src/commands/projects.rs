use crate::commands::backup::open_directory;
use crate::domain::project::ProjectSkillDirKey;
use crate::error::AppError;
use crate::services::agents::{detect_all, AgentStatus};
use crate::services::project_migrate::{self, MigrationDecision, ProjectMigrationReport};
use crate::services::project_registry::{
    self, ProjectSkillsView, ProjectSkillView, ProjectView,
};
use crate::services::project_scan;
use crate::services::project_sync::{self, Resolution, SyncReport};
use crate::state::AppState;
use std::path::{Path, PathBuf};
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

/// 项目必须在注册表里且路径真实存在；所有写命令都先过这一关。
fn registered_project_root(state: &AppState, project_id: &str) -> Result<PathBuf, AppError> {
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
    Ok(entry.path.clone())
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
        let agents = enabled_agents(state.inner())?;
        let enabled = enabled_agent_ids(&agents);
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::sync_project(
            &state.store,
            &project_id,
            &root,
            &enabled,
            &project_registry::selectable_dir_keys(&agents),
            &resolutions,
        )
    })
    .await
}

#[tauri::command]
pub async fn set_project_skill_targets(
    app: AppHandle,
    project_id: String,
    skill_id: String,
    targets: Vec<String>,
) -> Result<ProjectSkillView, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let agents = enabled_agents(state.inner())?;
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::set_targets(
            &root,
            &skill_id,
            &targets,
            &project_registry::selectable_dir_keys(&agents),
        )?;
        let view = project_registry::list_project_skills(&state.store, &project_id, &agents)?;
        view.skills
            .into_iter()
            .find(|skill| skill.id == skill_id)
            .ok_or_else(|| AppError::Project(format!("项目里没有该 skill：{skill_id}")))
    })
    .await
}

#[tauri::command]
pub async fn remove_project_skill_copy(
    app: AppHandle,
    project_id: String,
    skill_id: String,
    dir: String,
) -> Result<SyncReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::remove_copy(&state.store, &project_id, &root, &skill_id, &dir)
    })
    .await
}

#[tauri::command]
pub async fn uninstall_project_skill(
    app: AppHandle,
    project_id: String,
    skill_id: String,
) -> Result<SyncReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::uninstall(&state.store, &project_id, &root, &skill_id)
    })
    .await
}

/// 收进真源：把某个目录里现有的 skill 目录变成项目真源（先备份旧真源）。
#[tauri::command]
pub async fn adopt_project_skill(
    app: AppHandle,
    project_id: String,
    skill_id: String,
    dir: String,
) -> Result<SyncReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::adopt(&state.store, &project_id, &root, &skill_id, &dir)
    })
    .await
}

/// 读项目真源的 `SKILL.md` 正文（应用内编辑器用）。
#[tauri::command]
pub async fn read_project_skill_body(
    app: AppHandle,
    project_id: String,
    skill_id: String,
) -> Result<String, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::read_body(&root, &skill_id)
    })
    .await
}

/// 写项目真源的 `SKILL.md`：只替换正文，附件不动；渲染留给下一次同步。
#[tauri::command]
pub async fn write_project_skill_body(
    app: AppHandle,
    project_id: String,
    skill_id: String,
    content: String,
) -> Result<SyncReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let root = registered_project_root(state.inner(), &project_id)?;
        project_sync::write_body(&state.store, &project_id, &root, &skill_id, &content)
    })
    .await
}

/// v1 台账 → v2 项目真源的显式迁移。
/// 迁移看的是仓库里实际有什么，所以扫全部落点而不是只扫已启用的 agent：
/// 只按启用项扫会把停用了 agent 的副本误判成「不存在」而丢掉条目。
#[tauri::command]
pub async fn migrate_project_skills(
    app: AppHandle,
    project_id: String,
    decisions: Vec<MigrationDecision>,
) -> Result<ProjectMigrationReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _project_guard = state
            .project_ops
            .lock()
            .map_err(|_| AppError::Project("项目锁失败".into()))?;
        let _write = state.enter_write()?;
        let root = registered_project_root(state.inner(), &project_id)?;
        let ledger = project_scan::load_ledger(&root)?;
        let scan = project_scan::scan(&root, &ProjectSkillDirKey::ALL)?;
        project_migrate::apply_migration(
            &state.store,
            &project_id,
            &root,
            &ledger,
            &scan,
            &decisions,
        )
    })
    .await
}
