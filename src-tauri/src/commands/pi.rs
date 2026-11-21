//! pi 原生提示词资源（系统提示文件与斜杠命令模板）的 IPC 层。
//!
//! 提示库（AGENTS.md）仍走 `activate_prompt` 那条 canonical 流水线；
//! 这里只覆盖 pi 自己的 `SYSTEM.md` / `APPEND_SYSTEM.md` 与 `prompts/*.md`。

use crate::domain::AssetKind;
use crate::error::AppError;
use crate::services::agents::ensure_agent_detected;
use crate::services::pi_prompt::{self, PiPromptFileKind, PiPromptFileSnapshot, PiPromptTemplate};
use crate::state::AppState;
use tauri::State;

use super::agents::{descriptor_for, override_base};

fn pi_descriptor() -> Result<crate::domain::agent::AgentDescriptor, AppError> {
    descriptor_for("pi", AssetKind::Prompt)
}

#[tauri::command]
pub fn read_pi_prompt_file(
    state: State<'_, AppState>,
    kind: PiPromptFileKind,
) -> Result<PiPromptFileSnapshot, AppError> {
    let descriptor = pi_descriptor()?;
    let base = override_base(&state, "pi")?;
    pi_prompt::read_file(&descriptor, kind, &state.home, base.as_deref())
}

#[tauri::command]
pub fn replace_pi_prompt_file(
    state: State<'_, AppState>,
    kind: PiPromptFileKind,
    expected_revision: String,
    content: String,
) -> Result<PiPromptFileSnapshot, AppError> {
    let _guard = state.enter_write()?;
    let descriptor = pi_descriptor()?;
    ensure_agent_detected(&state.store, &state.home, "pi")?;
    let base = override_base(&state, "pi")?;
    pi_prompt::replace_file(
        &state.store,
        &descriptor,
        kind,
        &expected_revision,
        &content,
        &state.home,
        base.as_deref(),
    )
}

#[tauri::command]
pub fn delete_pi_prompt_file(
    state: State<'_, AppState>,
    kind: PiPromptFileKind,
    expected_revision: String,
) -> Result<bool, AppError> {
    let _guard = state.enter_write()?;
    let descriptor = pi_descriptor()?;
    ensure_agent_detected(&state.store, &state.home, "pi")?;
    let base = override_base(&state, "pi")?;
    pi_prompt::delete_file(
        &state.store,
        &descriptor,
        kind,
        &expected_revision,
        &state.home,
        base.as_deref(),
    )
}

#[tauri::command]
pub fn list_pi_prompt_templates(
    state: State<'_, AppState>,
) -> Result<Vec<PiPromptTemplate>, AppError> {
    let descriptor = pi_descriptor()?;
    let base = override_base(&state, "pi")?;
    pi_prompt::list_templates(&descriptor, &state.home, base.as_deref())
}

#[tauri::command]
pub fn save_pi_prompt_template(
    state: State<'_, AppState>,
    slug: String,
    original_slug: Option<String>,
    expected_revision: String,
    content: String,
) -> Result<PiPromptTemplate, AppError> {
    let _guard = state.enter_write()?;
    let descriptor = pi_descriptor()?;
    ensure_agent_detected(&state.store, &state.home, "pi")?;
    let base = override_base(&state, "pi")?;
    pi_prompt::save_template(
        &state.store,
        &descriptor,
        &slug,
        original_slug.as_deref(),
        &expected_revision,
        &content,
        &state.home,
        base.as_deref(),
    )
}

#[tauri::command]
pub fn delete_pi_prompt_template(
    state: State<'_, AppState>,
    slug: String,
    expected_revision: String,
) -> Result<bool, AppError> {
    let _guard = state.enter_write()?;
    let descriptor = pi_descriptor()?;
    ensure_agent_detected(&state.store, &state.home, "pi")?;
    let base = override_base(&state, "pi")?;
    pi_prompt::delete_template(
        &state.store,
        &descriptor,
        &slug,
        &expected_revision,
        &state.home,
        base.as_deref(),
    )
}
