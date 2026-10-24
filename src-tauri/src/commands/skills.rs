//! Skill 来源安装、上游检查/更新、卸载与批量启停命令。
//!
//! 网络与整目录文件操作都放进 `spawn_blocking`；await 期间不持 index/settings 锁，
//! 完成后单独锁 index 重建（索引始终是可丢弃的派生缓存）。

use crate::error::AppError;
use crate::import::mcp::ConflictPolicy;
use crate::net::HttpFetcher;
use crate::services::skill_source::{
    self, InstallSelection, SkillBatchReport, SkillInstallReport, SkillSourceInput,
    SkillSourcePreview, SkillUninstallReport,
};
use crate::services::skill_update::{self, SkillCheckReport, SkillUpdateReport};
use crate::state::AppState;
use tauri::State;

async fn run_blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| AppError::InvalidInput(format!("Skill 任务执行失败：{error}")))?
}

fn rebuild_index(state: &AppState) -> Result<(), AppError> {
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    index.rebuild(&state.store)?;
    Ok(())
}

#[tauri::command]
pub async fn preview_skill_source(
    state: State<'_, AppState>,
    input: SkillSourceInput,
) -> Result<SkillSourcePreview, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    run_blocking(move || {
        let fetcher = HttpFetcher::new()?;
        skill_source::preview(&store, &fetcher, &input)
    })
    .await
}

#[tauri::command]
pub async fn install_skill(
    state: State<'_, AppState>,
    input: SkillSourceInput,
    staging_id: Option<String>,
    selections: Vec<InstallSelection>,
    policy: ConflictPolicy,
) -> Result<SkillInstallReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let report = run_blocking(move || {
        skill_source::install(
            &store,
            &home,
            &input,
            staging_id.as_deref(),
            &selections,
            policy,
        )
    })
    .await?;
    rebuild_index(&state)?;
    Ok(report)
}

#[tauri::command]
pub async fn check_skill_updates(
    state: State<'_, AppState>,
    ids: Option<Vec<String>>,
) -> Result<SkillCheckReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    run_blocking(move || {
        let fetcher = HttpFetcher::new()?;
        skill_update::check(&store, &fetcher, ids.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn update_skills(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<SkillUpdateReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let report = run_blocking(move || {
        let fetcher = HttpFetcher::new()?;
        skill_update::update(&store, &home, &fetcher, &ids)
    })
    .await?;
    rebuild_index(&state)?;
    Ok(report)
}

#[tauri::command]
pub async fn uninstall_skills(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<SkillUninstallReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let report = run_blocking(move || skill_source::uninstall(&store, &home, &ids)).await?;
    rebuild_index(&state)?;
    Ok(report)
}

#[tauri::command]
pub async fn set_skills_enabled(
    state: State<'_, AppState>,
    ids: Vec<String>,
    agent_id: String,
    enabled: bool,
) -> Result<SkillBatchReport, AppError> {
    let store = state.store.clone();
    let home = state.home.clone();
    run_blocking(move || skill_source::set_enabled(&store, &home, &ids, &agent_id, enabled)).await
}
