//! Hooks 落进目标工具的 JSON 子树，并解决「哪几条是我们写的」。
//!
//! 与 rules / subagents 不同，hooks 住在共享文件里（`settings.json` 的 `hooks` 子树），
//! 删一条就可能碰到用户自己的条目。所以这里只允许两种归属依据之外的第三种信号：
//! 条目自带的归属标记 `description = "[ccswitchx:hook:<id>]"`——`description` 是这些工具
//! 本就容忍、不参与执行的字段（实测形状），拿它当标记不发明私有键。

use crate::adapters::paths::expand_root;
use crate::domain::agent::{AgentDescriptor, HooksTarget};
use crate::domain::hook::{Hook, HOOK_TYPE};
use crate::error::AppError;
use crate::services::apply::{Action, PlanItem};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 归属标记前缀。识别时按前缀取 `<id>`，允许后面还有说明文字（用户可能自己补了话）。
pub const MARKER_PREFIX: &str = "[ccswitchx:hook:";

pub fn marker(hook_id: &str) -> String {
    format!("{MARKER_PREFIX}{hook_id}]")
}

/// 这条目是谁家的。没有标记、标记不闭合、或 `]` 后面直接接了别的字符的都不算，
/// 只容忍 `]` 之后跟空白再跟说明文字（用户可能自己补了话，那还是我们那条）。
pub fn entry_owner(description: &str) -> Option<&str> {
    let rest = description.strip_prefix(MARKER_PREFIX)?;
    let (id, tail) = rest.split_once(']')?;
    if id.is_empty() {
        return None;
    }
    if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
        return None;
    }
    Some(id)
}

fn json_target(agent: &AgentDescriptor) -> Result<(&str, &str, bool), AppError> {
    match &agent.paths.hooks {
        HooksTarget::Json {
            file, key, matcher, ..
        } => Ok((file.as_str(), key.as_str(), *matcher)),
        HooksTarget::Unsupported => Err(AppError::Unsupported(format!(
            "{} 没有 hook 落点，未做任何改动",
            agent.id
        ))),
    }
}

fn gate_of(agent: &AgentDescriptor) -> Option<&crate::domain::agent::HooksGate> {
    match &agent.paths.hooks {
        HooksTarget::Json { gate, .. } => gate.as_ref(),
        HooksTarget::Unsupported => None,
    }
}

/// 该工具的 hooks 要用户自己先打开功能开关。**只报告，不代写**：
/// `[features].hooks` 是用户级功能开关，替他打开等于改他可感知的行为。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksNeedUserAction {
    /// UI 直接显示的绝对路径（不是声明式的 `~/...`）。
    pub file: PathBuf,
    pub why: String,
}

pub fn check_gate(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Option<HooksNeedUserAction>, AppError> {
    let Some(gate) = gate_of(agent) else {
        return Ok(None);
    };
    let path = expand_root(&gate.file, base_override, home);
    let required = format!("[{}].{} = true", gate.table, gate.key);
    let needed = |why: String| HooksNeedUserAction {
        file: path.clone(),
        why: format!(
            "请在 {} 里把 {required} 设为 true（我们不代改这个开关）——{why}",
            path.display()
        ),
    };

    let text = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(needed(format!("文件还不存在（{error}）"))))
        }
        Err(error) => return Ok(Some(needed(format!("读不到文件（{error}）")))),
    };
    let document = match text.parse::<toml_edit::DocumentMut>() {
        Ok(document) => document,
        Err(error) => return Ok(Some(needed(format!("文件解析失败：{error}")))),
    };
    let flag = document
        .get(&gate.table)
        .and_then(toml_edit::Item::as_table)
        .and_then(|table| table.get(&gate.key))
        .and_then(toml_edit::Item::as_value)
        .and_then(toml_edit::Value::as_bool);
    Ok(match flag {
        Some(true) => None,
        Some(false) => Some(needed("当前明确是 false".into())),
        None => Some(needed("没有这一项".into())),
    })
}

pub fn target_path(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    let (file, _, _) = json_target(agent)?;
    Ok(expand_root(file, base_override, home))
}

/// 该工具的一条 hook 条目。`matcher` 只在该工具认这个字段且真源写了值时才出现
/// （Codex 的条目没有 matcher）。
pub fn render_entry(
    agent: &AgentDescriptor,
    hook_id: &str,
    hook: &Hook,
) -> Result<Value, AppError> {
    hook.validate().map_err(AppError::Domain)?;
    let (_, _, matcher_allowed) = json_target(agent)?;

    let mut command = Map::new();
    command.insert("type".into(), Value::String(HOOK_TYPE.into()));
    let chosen = match (&hook.command_windows, cfg!(target_os = "windows")) {
        (Some(windows_command), true) if !windows_command.trim().is_empty() => windows_command,
        _ => &hook.command,
    };
    command.insert("command".into(), Value::String(chosen.clone()));
    if let Some(timeout) = hook.timeout {
        // 单位是秒，与两个工具的字段同名同义（canonical 里也是秒，不做换算）
        command.insert("timeout".into(), Value::from(timeout));
    }

    let mut entry = Map::new();
    if matcher_allowed && !hook.matcher.trim().is_empty() {
        entry.insert("matcher".into(), Value::String(hook.matcher.clone()));
    }
    entry.insert("description".into(), Value::String(marker(hook_id)));
    entry.insert("hooks".into(), Value::Array(vec![Value::Object(command)]));
    Ok(Value::Object(entry))
}

fn parse_document(current: &[u8]) -> Result<Value, AppError> {
    let text = String::from_utf8_lossy(current).to_string();
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(&text).map_err(|error| {
        AppError::InvalidInput(format!("目标 JSON 解析失败，未做任何改动：{error}"))
    })
}

fn serialize(document: &Value) -> Result<Vec<u8>, AppError> {
    // preserve_order 开着：非管辖键与它们在文件里的顺序原样带回去
    let mut out = serde_json::to_string_pretty(document)?;
    out.push('\n');
    Ok(out.into_bytes())
}

/// 开关没打开就一个字节都不写，把「要用户做什么」原话报出去。
/// disable 也一样拦：写了半套只会让 hooks.json 与 config.toml 的状态互相矛盾。
fn gate_allows_writing(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<(), AppError> {
    if let Some(action) = check_gate(agent, home, base_override)? {
        return Err(AppError::Unsupported(action.why));
    }
    Ok(())
}

/// 只在 `hooks` 子树里增删自己那一条；其余条目与其他顶层键一律不碰。
fn merge_hooks(
    current: &[u8],
    key: &str,
    event: &str,
    hook_id: &str,
    entry: Option<&Value>,
) -> Result<Vec<u8>, AppError> {
    let mut document = parse_document(current)?;
    let root = document
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidInput("目标 JSON 根节点不是对象".into()))?;
    let subtree = root
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let subtree = subtree.as_object_mut().ok_or_else(|| {
        AppError::InvalidInput(format!("{key} 不是对象，无法安全地往里加 hook 条目"))
    })?;

    let events = subtree
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let entries = events
        .as_array_mut()
        .ok_or_else(|| AppError::InvalidInput(format!("{key}.{event} 不是数组，不改这个形状")))?;

    // 先把自家的旧条目摘掉（幂等：重复 enable 不堆叠），再按需放新的回去
    let mut kept: Vec<Value> = entries
        .drain(..)
        .filter(|item| {
            item.get("description")
                .and_then(Value::as_str)
                .and_then(entry_owner)
                .map(|owner| owner != hook_id)
                .unwrap_or(true)
        })
        .collect();
    if let Some(entry) = entry {
        kept.push(entry.clone());
    }
    *entries = kept;
    let emptied = entries.is_empty();

    // 空掉的 event / 空掉的子树都收掉：留一个 `"hooks": {}` 在用户配置里是噪音
    if emptied {
        subtree.remove(event);
    }
    if subtree.is_empty() {
        root.remove(key);
    }
    serialize(&document)
}

fn plan_item(
    agent: &AgentDescriptor,
    hook_id: &str,
    path: PathBuf,
    contents: Vec<u8>,
    action: Action,
    event: &str,
) -> PlanItem {
    let marker_sign = match action {
        Action::Disable => '-',
        _ => '+',
    };
    PlanItem {
        agent_id: agent.id.clone(),
        asset_type: crate::domain::AssetType::Hook,
        asset_id: hook_id.to_string(),
        action,
        target_path: path,
        contents,
        diff: vec![format!("{marker_sign} {event}:{}", marker(hook_id))],
        // hook 的热加载语义没有实测依据，按「要重启」处理
        needs_restart: true,
    }
}

pub fn plan_enable(
    agent: &AgentDescriptor,
    hook_id: &str,
    hook: &Hook,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PlanItem, AppError> {
    let (_, key, _) = json_target(agent)?;
    gate_allows_writing(agent, home, base_override)?;
    let entry = render_entry(agent, hook_id, hook)?;
    let path = target_path(agent, home, base_override)?;
    let current = if path.exists() {
        std::fs::read(&path)?
    } else {
        Vec::new()
    };
    let contents = merge_hooks(&current, key, &hook.event, hook_id, Some(&entry))?;
    Ok(plan_item(
        agent,
        hook_id,
        path,
        contents,
        Action::Enable,
        &hook.event,
    ))
}

pub fn plan_disable(
    agent: &AgentDescriptor,
    hook_id: &str,
    event: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PlanItem, AppError> {
    let (_, key, _) = json_target(agent)?;
    gate_allows_writing(agent, home, base_override)?;
    let path = target_path(agent, home, base_override)?;
    let current = if path.exists() {
        std::fs::read(&path)?
    } else {
        Vec::new()
    };
    let contents = merge_hooks(&current, key, event, hook_id, None)?;
    Ok(plan_item(
        agent,
        hook_id,
        path,
        contents,
        Action::Disable,
        event,
    ))
}

/// 该文件里不是我们写的条目数。UI 用它明示「这些是你自己的，我们不动」。
pub fn foreign_count(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<usize, AppError> {
    let (_, key, _) = json_target(agent)?;
    let path = target_path(agent, home, base_override)?;
    if !path.exists() {
        return Ok(0);
    }
    let document = parse_document(&std::fs::read(&path)?)?;
    let Some(subtree) = document.get(key).and_then(Value::as_object) else {
        return Ok(0);
    };
    Ok(subtree
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter(|item| {
            item.get("description")
                .and_then(Value::as_str)
                .and_then(entry_owner)
                .is_none()
        })
        .count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use serde_json::json;

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn hook(event: &str, command: &str) -> Hook {
        Hook {
            name: "fmt".into(),
            event: event.into(),
            matcher: String::new(),
            command: command.into(),
            command_windows: None,
            timeout: Some(30),
        }
    }

    /// 一份「用户已经在用 hooks」的 settings.json：别的顶层键 + 同事件下他自己的一条。
    /// 内容是编的，不是从真实文件抄的（真实文件里有明文凭据）。
    fn user_settings() -> Value {
        json!({
            "permissions": { "defaultMode": "acceptEdits" },
            "statusLine": { "command": "echo hi" },
            "hooks": {
                "SessionStart": [
                    { "description": "用户自己的一条", "hooks": [{ "type": "command", "command": "echo mine" }] }
                ]
            }
        })
    }

    struct Sandbox {
        _dir: tempfile::TempDir,
        home: PathBuf,
        base: PathBuf,
    }

    fn sandbox(current: Option<&Value>) -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("sandbox");
        let home = dir.path().join("home");
        std::fs::create_dir_all(base.join(".claude")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        if let Some(current) = current {
            std::fs::write(
                base.join(".claude/settings.json"),
                serde_json::to_vec_pretty(current).unwrap(),
            )
            .unwrap();
        }
        Sandbox {
            _dir: dir,
            home,
            base,
        }
    }

    fn write_settings(box_: &Sandbox, contents: &[u8]) {
        std::fs::write(box_.base.join(".claude/settings.json"), contents).unwrap();
    }

    fn contents_of(plan: &PlanItem) -> Value {
        serde_json::from_slice(&plan.contents).unwrap()
    }

    #[test]
    fn enabling_appends_to_the_event_and_keeps_every_foreign_entry() {
        let box_ = sandbox(Some(&user_settings()));
        let plan = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        let after = contents_of(&plan);
        let entries = after["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "不能把用户自己的 hook 挤掉：{after}");
        assert_eq!(entries[0]["description"], "用户自己的一条");
        assert_eq!(entries[1]["description"], "[ccswitchx:hook:fmt]");
        assert_eq!(entries[1]["hooks"][0]["command"], "cargo fmt");
        assert_eq!(entries[1]["hooks"][0]["type"], HOOK_TYPE);
        assert_eq!(entries[1]["hooks"][0]["timeout"], 30);
        assert_eq!(
            after["statusLine"]["command"], "echo hi",
            "非管辖键必须不变"
        );
        assert_eq!(after["permissions"]["defaultMode"], "acceptEdits");
        let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["permissions", "statusLine", "hooks"]);
    }

    #[test]
    fn disabling_removes_only_the_marked_entry() {
        let box_ = sandbox(Some(&user_settings()));
        let enable = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        write_settings(&box_, &enable.contents);

        let disable = plan_disable(
            &agent("claude"),
            "fmt",
            "SessionStart",
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        let after = contents_of(&disable);
        let entries = after["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{after}");
        assert_eq!(entries[0]["description"], "用户自己的一条");
        assert_eq!(after["statusLine"]["command"], "echo hi");
    }

    #[test]
    fn enabling_twice_does_not_stack_our_own_entry() {
        let box_ = sandbox(Some(&user_settings()));
        let first = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        write_settings(&box_, &first.contents);
        let second = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        assert_eq!(
            second.contents, first.contents,
            "同一计划重复执行结果必须不变（幂等）"
        );
        let after = contents_of(&second);
        assert_eq!(after["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn our_entry_is_replaced_in_place_when_the_command_changes() {
        let box_ = sandbox(Some(&user_settings()));
        let first = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        write_settings(&box_, &first.contents);
        let changed = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt --all"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        let after = contents_of(&changed);
        let entries = after["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "换命令是替换不是新增：{after}");
        assert_eq!(
            entries[0]["description"], "用户自己的一条",
            "用户那条仍在原位"
        );
        assert_eq!(entries[1]["hooks"][0]["command"], "cargo fmt --all");
    }

    #[test]
    fn a_file_without_the_marker_is_left_alone_even_if_the_command_matches() {
        let box_ = sandbox(None);
        // 用户自己复制了同一条命令，但没有归属标记
        let theirs = json!({
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "cargo fmt" }] }
                ]
            }
        });
        write_settings(&box_, &serde_json::to_vec_pretty(&theirs).unwrap());

        let disable = plan_disable(
            &agent("claude"),
            "fmt",
            "PreToolUse",
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        let after = String::from_utf8(disable.contents).unwrap();
        assert!(after.contains("cargo fmt"), "没有归属标记就不许删：{after}");
    }

    #[test]
    fn an_unknown_event_is_rejected_instead_of_silently_written() {
        let box_ = sandbox(Some(&user_settings()));
        let error = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("PreTool", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap_err();
        assert!(
            matches!(
                error,
                AppError::Domain(crate::domain::DomainError::UnknownHookEvent(_))
            ),
            "拼错的事件名不能写进用户配置：{error}"
        );
        assert!(
            !std::fs::read_to_string(box_.base.join(".claude/settings.json"))
                .unwrap()
                .contains("PreTool")
        );
    }

    #[test]
    fn the_matcher_is_written_only_for_tools_that_read_it() {
        let mut with_matcher = hook("PreToolUse", "cargo fmt");
        with_matcher.matcher = "Bash".into();
        let claude_entry = render_entry(&agent("claude"), "fmt", &with_matcher).unwrap();
        assert_eq!(claude_entry["matcher"], "Bash");

        // Codex 的条目形状里没有 matcher（本机 hooks.json 实测）
        let codex_entry = render_entry(&agent("codex"), "fmt", &with_matcher).unwrap();
        assert!(
            codex_entry.get("matcher").is_none(),
            "Codex 不认 matcher：{codex_entry}"
        );
        assert_eq!(codex_entry["description"], "[ccswitchx:hook:fmt]");
    }

    #[test]
    fn the_windows_command_is_used_only_on_windows() {
        let mut both = hook("SessionStart", "cargo fmt");
        both.command_windows = Some("pwsh -NoProfile -File a.ps1".into());
        let entry = render_entry(&agent("qoder-cn"), "fmt", &both).unwrap();
        let expected = if cfg!(target_os = "windows") {
            "pwsh -NoProfile -File a.ps1"
        } else {
            "cargo fmt"
        };
        assert_eq!(entry["hooks"][0]["command"], expected);

        let single = hook("SessionStart", "cargo fmt");
        let entry = render_entry(&agent("qoder-cn"), "fmt", &single).unwrap();
        assert_eq!(entry["hooks"][0]["command"], "cargo fmt");
    }

    #[test]
    fn a_missing_hooks_subtree_is_created_without_touching_the_rest() {
        let box_ = sandbox(Some(&json!({ "model": "haiku" })));
        let plan = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        let after = contents_of(&plan);
        assert_eq!(after["model"], "haiku");
        assert_eq!(after["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_event_whose_shape_we_do_not_understand_is_refused() {
        let box_ = sandbox(Some(
            &json!({ "hooks": { "SessionStart": "not an array" } }),
        ));
        let error = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap_err();
        assert!(
            matches!(error, AppError::InvalidInput(_)),
            "改不动的形状要报错，不能顺手覆盖：{error}"
        );
    }

    #[test]
    fn disabling_our_last_entry_removes_the_now_empty_subtree() {
        let box_ = sandbox(Some(&json!({ "model": "haiku" })));
        let enable = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        write_settings(&box_, &enable.contents);

        let disable = plan_disable(
            &agent("claude"),
            "fmt",
            "SessionStart",
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        let after = contents_of(&disable);
        assert_eq!(after["model"], "haiku");
        assert!(
            after.get("hooks").is_none(),
            "空掉的子树不该留在用户配置里：{after}"
        );
    }

    #[test]
    fn foreign_count_names_the_users_own_entries() {
        let box_ = sandbox(Some(&user_settings()));
        assert_eq!(
            foreign_count(&agent("claude"), &box_.home, Some(&box_.base)).unwrap(),
            1
        );
        let enable = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        write_settings(&box_, &enable.contents);
        assert_eq!(
            foreign_count(&agent("claude"), &box_.home, Some(&box_.base)).unwrap(),
            1,
            "我们自己的那条不能被算成用户的"
        );
    }

    #[test]
    fn marker_parsing_tolerates_appended_text_but_not_lookalikes() {
        assert_eq!(entry_owner("[ccswitchx:hook:fmt]"), Some("fmt"));
        assert_eq!(
            entry_owner("[ccswitchx:hook:fmt] 用户补的说明"),
            Some("fmt")
        );
        assert_eq!(entry_owner("[ccswitchx:hook:fmt2]"), Some("fmt2"));
        assert_eq!(entry_owner("用户自己的一条"), None);
        assert_eq!(entry_owner("[ccswitchx:hook:]"), None);
        assert_eq!(
            entry_owner("[ccswitchx:hook:fmt"),
            None,
            "未闭合的标记不算我们的"
        );
        assert_eq!(
            entry_owner("[ccswitchx:hook:fmt]尾巴"),
            None,
            "] 后直接跟字符的是别人的写法"
        );
    }

    #[test]
    fn agents_without_a_hook_target_are_refused() {
        let box_ = sandbox(Some(&user_settings()));
        for id in ["gemini", "pi"] {
            let error = plan_enable(
                &agent(id),
                "fmt",
                &hook("SessionStart", "cargo fmt"),
                &box_.home,
                Some(&box_.base),
            )
            .unwrap_err();
            assert!(
                matches!(error, AppError::Unsupported(_)),
                "{id} 没有 hook 落点：{error}"
            );
        }
    }

    #[test]
    fn the_plan_carries_the_restart_flag_and_the_marked_diff() {
        let box_ = sandbox(Some(&user_settings()));
        let plan = plan_enable(
            &agent("claude"),
            "fmt",
            &hook("SessionStart", "cargo fmt"),
            &box_.home,
            Some(&box_.base),
        )
        .unwrap();
        assert!(
            plan.needs_restart,
            "hook 的热加载语义没实测，不能报成已生效"
        );
        assert_eq!(plan.diff, vec!["+ SessionStart:[ccswitchx:hook:fmt]"]);
        assert_eq!(plan.target_path, box_.base.join(".claude/settings.json"));
    }
}

#[cfg(test)]
mod codex_tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use serde_json::json;
    use std::path::PathBuf;

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn hook(event: &str) -> Hook {
        Hook {
            name: "fmt".into(),
            event: event.into(),
            matcher: "Bash".into(),
            command: "cargo fmt".into(),
            command_windows: None,
            timeout: None,
        }
    }

    struct Sandbox {
        _dir: tempfile::TempDir,
        home: PathBuf,
        base: PathBuf,
    }

    /// Codex 的两个文件：`config.toml`（功能开关，属于用户）与 `hooks.json`（我们写）。
    fn sandbox(config: Option<&str>) -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("sandbox");
        let home = dir.path().join("home");
        std::fs::create_dir_all(base.join(".codex")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        if let Some(config) = config {
            std::fs::write(base.join(".codex/config.toml"), config).unwrap();
        }
        Sandbox {
            _dir: dir,
            home,
            base,
        }
    }

    fn plan_enable_in(box_: &Sandbox) -> Result<PlanItem, AppError> {
        plan_enable(
            &agent("codex"),
            "fmt",
            &hook("SessionStart"),
            &box_.home,
            Some(&box_.base),
        )
    }

    #[test]
    fn enabling_a_codex_hook_writes_hooks_json_not_config_toml() {
        let box_ = sandbox(Some("[features]\nhooks = true\nmodel = \"gpt-5\"\n"));
        let before = std::fs::read(box_.base.join(".codex/config.toml")).unwrap();

        let plan = plan_enable_in(&box_).unwrap();
        assert_eq!(plan.target_path, box_.base.join(".codex/hooks.json"));
        let after: Value = serde_json::from_slice(&plan.contents).unwrap();
        let entries = after["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{after}");
        assert_eq!(entries[0]["description"], "[ccswitchx:hook:fmt]");
        assert_eq!(
            std::fs::read(box_.base.join(".codex/config.toml")).unwrap(),
            before,
            "config.toml 是用户的文件，一个字节都不许动"
        );
    }

    #[test]
    fn codex_entries_carry_no_matcher() {
        let box_ = sandbox(Some("[features]\nhooks = true\n"));
        let plan = plan_enable_in(&box_).unwrap();
        let after: Value = serde_json::from_slice(&plan.contents).unwrap();
        let entry = &after["hooks"]["SessionStart"][0];
        assert!(
            entry.get("matcher").is_none(),
            "本机 hooks.json 的条目里没有 matcher，写了会被忽略：{entry}"
        );
        assert_eq!(entry["hooks"][0]["command"], "cargo fmt");
    }

    #[test]
    fn a_missing_features_flag_is_reported_not_written() {
        let box_ = sandbox(Some("model = \"gpt-5\"\n"));
        let error = plan_enable_in(&box_).unwrap_err();
        let message = error.to_string();
        assert!(
            matches!(error, AppError::Unsupported(_)),
            "没打开开关就不能写：{message}"
        );
        assert!(message.contains("[features].hooks = true"), "{message}");
        assert!(
            message.contains(&box_.base.join(".codex/config.toml").display().to_string()),
            "报告里必须给出该改哪个文件的绝对路径：{message}"
        );
        assert!(
            !box_.base.join(".codex/hooks.json").exists(),
            "被拦下时一个文件都不该建出来"
        );
    }

    #[test]
    fn an_absent_config_file_and_an_explicit_false_are_both_reported() {
        let none = sandbox(None);
        let message = plan_enable_in(&none).unwrap_err().to_string();
        assert!(message.contains("文件还不存在"), "{message}");

        let off = sandbox(Some("[features]\nhooks = false\n"));
        let message = plan_enable_in(&off).unwrap_err().to_string();
        assert!(message.contains("当前明确是 false"), "{message}");
    }

    #[test]
    fn a_broken_config_toml_is_not_assumed_enabled() {
        let box_ = sandbox(Some("[features\nhooks = true"));
        let message = plan_enable_in(&box_).unwrap_err().to_string();
        assert!(message.contains("解析失败"), "{message}");
        assert!(!box_.base.join(".codex/hooks.json").exists());
    }

    #[test]
    fn disabling_is_gated_too() {
        let box_ = sandbox(Some("model = \"gpt-5\"\n"));
        let error = plan_disable(
            &agent("codex"),
            "fmt",
            "SessionStart",
            &box_.home,
            Some(&box_.base),
        )
        .unwrap_err();
        assert!(
            matches!(error, AppError::Unsupported(_)),
            "开关没打开时写半套只会让两个文件的状态互相矛盾：{error}"
        );
    }

    #[test]
    fn the_gate_only_applies_to_tools_that_declare_one() {
        let box_ = sandbox(Some("model = \"gpt-5\"\n"));
        for id in ["claude", "qoder-cn"] {
            assert_eq!(
                check_gate(&agent(id), &box_.home, Some(&box_.base)).unwrap(),
                None,
                "{id} 没有功能开关这回事"
            );
        }
        assert!(check_gate(&agent("codex"), &box_.home, Some(&box_.base))
            .unwrap()
            .is_some());
    }

    #[test]
    fn check_gate_follows_the_base_override() {
        let box_ = sandbox(Some("[features]\nhooks = true\n"));
        assert_eq!(
            check_gate(&agent("codex"), &box_.home, Some(&box_.base)).unwrap(),
            None
        );
        // 没覆盖时看的是真实家目录，那边的开关与我们沙箱无关
        let reported = check_gate(&agent("codex"), &box_.home, None)
            .unwrap()
            .expect("真实 home 下的 config.toml 不该被当成已确认");
        assert_eq!(
            reported.file,
            box_.home.join(".codex/config.toml"),
            "报告的路径必须是展开后的绝对路径"
        );
    }

    #[test]
    fn an_existing_user_entry_in_hooks_json_is_kept() {
        let box_ = sandbox(Some("[features]\nhooks = true\n"));
        std::fs::write(
            box_.base.join(".codex/hooks.json"),
            serde_json::to_vec_pretty(&json!({
                "hooks": {
                    "SessionStart": [
                        { "hooks": [{ "type": "command", "command": "echo mine" }] }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let plan = plan_enable_in(&box_).unwrap();
        let after: Value = serde_json::from_slice(&plan.contents).unwrap();
        let entries = after["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "{after}");
        assert_eq!(entries[0]["hooks"][0]["command"], "echo mine");
    }
}
