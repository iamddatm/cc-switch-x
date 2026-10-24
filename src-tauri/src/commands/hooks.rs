use crate::adapters::hooks;
use crate::commands::agents::{descriptor_for, override_base};
use crate::domain::agent::builtin_agents;
use crate::domain::hook::Hook;
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::update_matrix;
use crate::services::agents::ensure_agent_detected;
use crate::services::apply::{ApplyReport, ApplyService};
use crate::state::AppState;
use tauri::State;

fn read_hook(state: &AppState, id: &str) -> Result<Hook, AppError> {
    let canonical = state.store.read(AssetType::Hook, id)?;
    let text = String::from_utf8(canonical)
        .map_err(|_| AppError::InvalidInput("真源里的 hook 不是 UTF-8".into()))?;
    Ok(crate::domain::hook::parse_hook(&text)?)
}

/// 往目标工具的 `hooks` 子树里加/删**我们自己那一条**（按 `description` 标记认）。
/// 返回逐项报告；失败原因里会说清楚是哪种形状没敢动。
#[tauri::command]
pub fn set_hook_enabled(
    state: State<'_, AppState>,
    id: String,
    agent_id: String,
    enabled: bool,
) -> Result<ApplyReport, AppError> {
    let _guard = state.enter_write()?;
    let descriptor = descriptor_for(&agent_id, AssetKind::Hook)?;
    let hook = read_hook(&state, &id)?;
    let base = override_base(&state, &agent_id)?;
    // 取消也会把合并后的文件写回去；没检测到就两边都不写。
    ensure_agent_detected(&state.store, &state.home, &agent_id)?;

    let plan = if enabled {
        hooks::plan_enable(&descriptor, &id, &hook, &state.home, base.as_deref())?
    } else {
        hooks::plan_disable(&descriptor, &id, &hook.event, &state.home, base.as_deref())?
    };

    let report = {
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        ApplyService::apply(&state.store, &mut index, &[plan])?
    };
    update_matrix(&state.store, &agent_id, |matrix| {
        if enabled {
            matrix.enable(AssetType::Hook, &id);
        } else {
            matrix.disable(AssetType::Hook, &id);
        }
    })?;
    state.notify_git(format!(
        "{}: hooks/{id} on {agent_id}",
        if enabled { "enable" } else { "disable" }
    ));
    Ok(report)
}

/// 该 agent 的 hook 文件里不是我们写的条目数——面板要明示「这些是你自己的，我们不动」。
#[tauri::command]
pub fn foreign_hook_count(state: State<'_, AppState>, agent_id: String) -> Result<usize, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Hook)?;
    let base = override_base(&state, &agent_id)?;
    hooks::foreign_count(&descriptor, &state.home, base.as_deref())
}

/// 该工具要不要用户自己先打开功能开关（Codex 的 `[features].hooks`）。
/// `None` = 不需要；`Some` 里带着要改的**绝对路径**与原因，UI 照着显示——我们不代改。
/// 这是只读查询，所以不受能力闸约束：面板要在「能不能开」之前就告诉用户该做什么。
#[tauri::command]
pub fn hooks_gate_status(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<hooks::HooksNeedUserAction>, AppError> {
    let descriptor = builtin_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| AppError::NotFound(format!("注册表里没有 {agent_id}")))?;
    let base = override_base(&state, &agent_id)?;
    hooks::check_gate(&descriptor, &state.home, base.as_deref())
}
