//! Skill 来源安装、上游检查/更新、卸载与批量启停命令。
//!
//! 网络与整目录文件操作都放进 `spawn_blocking`；await 期间不持 index/settings 锁，
//! 完成后单独锁 index 重建（索引始终是可丢弃的派生缓存）。

use crate::error::AppError;
use crate::import::mcp::ConflictPolicy;
use crate::net::HttpFetcher;
use crate::services::settings as settings_service;
use crate::services::skill_discovery::{
    self, SkillDiscoveryReport, SkillsShSearchResult, SkillsShTrendingResult,
};
use crate::services::skill_source::{
    self, InstallSelection, SkillBatchReport, SkillInstallOutcome, SkillInstallReport,
    SkillSourceInput, SkillSourcePreview, SkillUninstallReport,
};
use crate::services::skill_update::{self, SkillCheckReport, SkillUpdateReport};
use crate::settings::{effective_skill_repos, SkillRepo};
use crate::state::AppState;
use serde::Deserialize;
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
    let proxy = state.proxy_config()?;
    run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
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
    let proxy = state.proxy_config()?;
    run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
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
    let proxy = state.proxy_config()?;
    let report = run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
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

#[tauri::command]
pub fn get_skill_repos(state: State<'_, AppState>) -> Result<Vec<SkillRepo>, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok(effective_skill_repos(&settings))
}

#[tauri::command]
pub fn set_skill_repos(
    state: State<'_, AppState>,
    repos: Vec<SkillRepo>,
) -> Result<Vec<SkillRepo>, AppError> {
    let updated = settings_service::set_skill_repos(repos)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(effective_skill_repos(&updated))
}

#[tauri::command]
pub async fn discover_available_skills(
    state: State<'_, AppState>,
) -> Result<SkillDiscoveryReport, AppError> {
    // 发现只读仓库并写临时目录，不碰真源；不占用 Skill 写任务锁。
    let repos = {
        let settings = state
            .settings
            .lock()
            .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
        effective_skill_repos(&settings)
    };
    let store = state.store.clone();
    let proxy = state.proxy_config()?;
    run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
        Ok(skill_discovery::discover(&store, &fetcher, &repos))
    })
    .await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSkillInstallInput {
    pub repo: String,
    pub branch: Option<String>,
    #[serde(default)]
    pub relative_path: Option<String>,
    /// skills.sh 的 skillId；仓库发现结果通常已经有 relativePath。
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// 发现页直接安装：重新拉一次仓库（避免信任前端路径），定位候选后复用预览暂存安装。
#[tauri::command]
pub async fn install_discovered_skill(
    state: State<'_, AppState>,
    input: DiscoveredSkillInstallInput,
    policy: ConflictPolicy,
) -> Result<SkillInstallOutcome, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let proxy = state.proxy_config()?;
    let outcome = run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
        skill_source::install_discovered(
            &store,
            &home,
            &fetcher,
            &input.repo,
            input.branch.clone(),
            input.relative_path.as_deref(),
            input.directory.as_deref(),
            input.name.as_deref(),
            policy,
        )
    })
    .await?;
    rebuild_index(&state)?;
    Ok(outcome)
}

#[tauri::command]
pub async fn search_skills_sh(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
    offset: usize,
) -> Result<SkillsShSearchResult, AppError> {
    let proxy = state.proxy_config()?;
    run_blocking(move || skill_discovery::search_skills_sh(Some(&proxy), &query, limit, offset))
        .await
}

#[tauri::command]
pub async fn list_skills_sh_trending(
    state: State<'_, AppState>,
    page: usize,
) -> Result<SkillsShTrendingResult, AppError> {
    let proxy = state.proxy_config()?;
    run_blocking(move || skill_discovery::trending_skills_sh(Some(&proxy), page)).await
}
