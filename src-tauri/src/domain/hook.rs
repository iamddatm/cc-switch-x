use super::DomainError;
use serde::{Deserialize, Serialize};

/// 已确证的 hook 事件名。这份名单来自本机 `qoderclicn` 二进制里的事件枚举
/// （`X.PreToolUse="PreToolUse"` 那一段，源码级确证，共 28 个）；Claude 文档里的
/// 事件名全在这份之内，所以它同时当 canonical 的事件白名单。
///
/// 拼错的事件名（`PreTool`）必须被拒：静默写进去等于往用户配置里塞一条永远不会
/// 触发的条目，用户看不出区别。
pub const KNOWN_EVENTS: [&str; 28] = [
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "SubagentStart",
    "Notification",
    "SubagentStop",
    "SessionStart",
    "Setup",
    "SessionEnd",
    "PreCompact",
    "PostCompact",
    "Stop",
    "QueryEnd",
    "UserPromptSubmit",
    "ConfigChange",
    "StopFailure",
    "TeammateIdle",
    "InstructionsLoaded",
    "CwdChanged",
    "FileChanged",
    "WorktreeCreate",
    "WorktreeRemove",
    "Elicitation",
    "ElicitationResult",
    "TaskCreated",
    "TaskCompleted",
    "PermissionRequest",
    "PermissionDenied",
];

/// hook 的 `type` 枚举里我们唯一会写的值：其余（`runtime|http|prompt|agent`）
/// 需要各自的载荷结构，没有实测依据就不发明。
pub const HOOK_TYPE: &str = "command";

pub fn is_known_event(event: &str) -> bool {
    KNOWN_EVENTS.contains(&event)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    pub name: String,
    pub event: String,
    #[serde(default)]
    pub matcher: String,
    pub command: String,
    /// Windows 上的替代命令（例如需要 `pwsh -File` 的那些）。没有就两边用同一条。
    #[serde(default)]
    pub command_windows: Option<String>,
    /// 单位是**秒**，与两个目标工具的字段同名同义（Claude / Qoder 的 `timeout` 都是秒）。
    #[serde(default)]
    pub timeout: Option<u64>,
}

pub fn parse_hook(raw: &str) -> Result<Hook, DomainError> {
    let parsed: Hook = serde_yaml::from_str(raw).map_err(|e| DomainError::Yaml(e.to_string()))?;
    parsed.validate()?;
    Ok(parsed)
}

impl Hook {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::EmptyName);
        }
        if !is_known_event(self.event.trim()) {
            return Err(DomainError::UnknownHookEvent(self.event.clone()));
        }
        if self.command.trim().is_empty() {
            return Err(DomainError::EmptyCommand);
        }
        Ok(())
    }
}

/// 真源里的 hook 存成 YAML。与 rule / subagent 同理：序列化失败不能写成空文件。
pub fn render_canonical(hook: &Hook) -> Result<String, DomainError> {
    serde_yaml::to_string(hook).map_err(|e| DomainError::Yaml(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: &str) -> Hook {
        Hook {
            name: "fmt".into(),
            event: event.into(),
            matcher: String::new(),
            command: "cargo fmt".into(),
            command_windows: None,
            timeout: Some(30),
        }
    }

    #[test]
    fn the_whitelist_is_the_events_the_cli_enum_declares() {
        assert_eq!(KNOWN_EVENTS.len(), 28);
        // 白名单里不能有重复：重复会让「28 个」这个断言失去意义
        let mut unique = KNOWN_EVENTS.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), KNOWN_EVENTS.len(), "事件名重复了");
        assert!(is_known_event("PreToolUse"));
        assert!(is_known_event("PermissionDenied"));
    }

    #[test]
    fn a_misspelled_event_is_rejected_not_silently_written() {
        assert_eq!(
            hook("PreTool").validate().unwrap_err(),
            DomainError::UnknownHookEvent("PreTool".into())
        );
        assert_eq!(
            hook("").validate().unwrap_err(),
            DomainError::UnknownHookEvent(String::new())
        );
    }

    #[test]
    fn an_empty_command_is_named_not_confused_with_the_name() {
        let mut blank = hook("SessionStart");
        blank.command = "  ".into();
        assert_eq!(blank.validate().unwrap_err(), DomainError::EmptyCommand);
    }

    #[test]
    fn canonical_round_trip_keeps_the_windows_command() {
        let mut both = hook("SessionStart");
        both.command_windows = Some("pwsh -NoProfile -File a.ps1".into());
        let text = render_canonical(&both).unwrap();
        assert_eq!(parse_hook(&text).unwrap(), both);
    }
}
