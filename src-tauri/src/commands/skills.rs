//! Skill 来源安装、上游检查/更新、卸载与批量启停命令，
//! 外加本地未纳管 Skill 的扫描（只读）与导入接管（写盘）。
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
use crate::services::skill_local_scan::{self, UnmanagedSkillReport};
use crate::services::skill_repair::{self, SkillRepairReport};
use crate::services::skill_source::{
    self, InstallSelection, SkillBatchReport, SkillInstallOutcome, SkillInstallReport,
    SkillSourceInput, SkillSourcePreview, SkillUninstallReport,
};
use crate::services::skill_takeover::{
    self, UnmanagedImportInput, UnmanagedImportReport,
};
use crate::services::skill_update::{self, SkillCheckReport, SkillUpdateReport};
use crate::services::skill_verify::{self, SkillDeploymentIssue};
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
    let maintenance = state.maintenance.clone();
    let report = run_blocking(move || {
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_source::install(
            skill_source::SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            staging_id.as_deref(),
            &selections,
            policy,
        )
    })
    .await?;
    rebuild_index(&state)?;
    state.notify_git("skill: install");
    Ok(report)
}

#[tauri::command]
pub async fn check_skill_updates(
    state: State<'_, AppState>,
    ids: Option<Vec<String>>,
) -> Result<SkillCheckReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let proxy = state.proxy_config()?;
    let maintenance = state.maintenance.clone();
    let report = run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
        // 检查结果会写回 `.meta.json`，属于真源变更。
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_update::check(
            skill_update::UpdateSink::Store {
                store: &store,
                home: &home,
            },
            &fetcher,
            ids.as_deref(),
        )
    })
    .await?;
    state.notify_git("skill: check updates");
    Ok(report)
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
    let maintenance = state.maintenance.clone();
    let report = run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_update::update(
            skill_update::UpdateSink::Store {
                store: &store,
                home: &home,
            },
            &fetcher,
            &ids,
        )
    })
    .await?;
    rebuild_index(&state)?;
    state.notify_git("skill: update");
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
    let maintenance = state.maintenance.clone();
    let report = run_blocking(move || {
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_source::uninstall(&store, &home, &ids)
    })
    .await?;
    rebuild_index(&state)?;
    state.notify_git("skill: uninstall");
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
    let maintenance = state.maintenance.clone();
    let label_agent = agent_id.clone();
    let report = run_blocking(move || {
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_source::set_enabled(&store, &home, &ids, &agent_id, enabled)
    })
    .await?;
    state.notify_git(format!(
        "{}: skills on {label_agent}",
        if enabled { "enable" } else { "disable" }
    ));
    Ok(report)
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
        Ok(skill_discovery::discover_with_installed(
            &store, &fetcher, &repos,
        ))
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
    let maintenance = state.maintenance.clone();
    let outcome = run_blocking(move || {
        let fetcher = HttpFetcher::new(Some(&proxy))?;
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_source::install_discovered(
            skill_source::SkillSink::Store {
                store: &store,
                home: &home,
            },
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
    state.notify_git(format!("skill: install {}", outcome.id));
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

/// 扫描各 agent 的 skills 落点，列出**不是我们落的** Skill。
///
/// 只读遍历本地目录：不占 Skill 写任务锁，也不碰真源（与 `discover_available_skills` 同一口径）。
#[tauri::command]
pub async fn scan_unmanaged_skills(
    state: State<'_, AppState>,
) -> Result<UnmanagedSkillReport, AppError> {
    let store = state.store.clone();
    let home = state.home.clone();
    run_blocking(move || skill_local_scan::scan(&store, &home)).await
}

/// 核对启用矩阵里的 Skill 勾选：落点还在不在、还是不是我们那一份。
///
/// 与 `scan_unmanaged_skills` 是同一枚硬币的两面（那个从未勾选项出发，这个从矩阵出发）。
/// 纯只读 stat 遍历，不占 Skill 写任务锁；报告只给用户看，不会顺手补链接。
#[tauri::command]
pub async fn verify_skill_deployments(
    state: State<'_, AppState>,
) -> Result<Vec<SkillDeploymentIssue>, AppError> {
    let store = state.store.clone();
    let home = state.home.clone();
    run_blocking(move || skill_verify::verify(&store, &home)).await
}

/// 一键处理核对报出的差异：重刷空落点、抢回被别处接走的链接、清掉真源已删的悬空勾选。
///
/// **这是唯一会因核对结果动盘的入口**，清单由服务端现算（不信任前端传来的列表），
/// 前端调用前必须先过确认框。写任务互斥与 maintenance 锁照 `import_unmanaged_skills` 那一套。
#[tauri::command]
pub async fn repair_skill_deployments(
    state: State<'_, AppState>,
) -> Result<SkillRepairReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let maintenance = state.maintenance.clone();
    let report = run_blocking(move || {
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_repair::repair(&store, &home)
    })
    .await?;
    rebuild_index(&state)?;
    state.notify_git("skill: repair deployments");
    Ok(report)
}

/// 导入未纳管的 Skill：收进中央库真源，并按用户选择接管盘上那份外来目录。
#[tauri::command]
pub async fn import_unmanaged_skills(
    state: State<'_, AppState>,
    inputs: Vec<UnmanagedImportInput>,
    policy: ConflictPolicy,
) -> Result<UnmanagedImportReport, AppError> {
    let _guard = state.begin_skill_job()?;
    let store = state.store.clone();
    let home = state.home.clone();
    let maintenance = state.maintenance.clone();
    let report = run_blocking(move || {
        let _write = maintenance
            .read()
            .map_err(|_| AppError::InvalidInput("maintenance 锁失败".into()))?;
        skill_takeover::import(&store, &home, &inputs, policy)
    })
    .await?;
    rebuild_index(&state)?;
    state.notify_git("skill: import unmanaged");
    Ok(report)
}
