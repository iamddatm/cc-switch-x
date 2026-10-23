//! 会话统一模型：各 agent 的原生会话文件解析后都落到这里。
//!
//! 会话是**只读**的本地浏览对象，不进真源目录、不参与同步；这些类型只在
//! `index.db`（派生缓存）与前端之间流动。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionRole {
    User,
    Assistant,
    Tool,
    System,
}

impl SessionRole {
    pub const ALL: [SessionRole; 4] = [Self::User, Self::Assistant, Self::Tool, Self::System];

    pub fn as_db(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::System => "system",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, SessionError> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            "system" => Ok(Self::System),
            other => Err(SessionError::Unsupported(format!("未知会话角色：{other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMessageKind {
    Text,
    Thinking,
    ToolCall,
    ToolResult,
    Image,
    Event,
}

impl SessionMessageKind {
    pub const ALL: [SessionMessageKind; 6] = [
        Self::Text,
        Self::Thinking,
        Self::ToolCall,
        Self::ToolResult,
        Self::Image,
        Self::Event,
    ];

    pub fn as_db(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Thinking => "thinking",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::Image => "image",
            Self::Event => "event",
        }
    }

    pub fn from_db(value: &str) -> Result<Self, SessionError> {
        match value {
            "text" => Ok(Self::Text),
            "thinking" => Ok(Self::Thinking),
            "tool_call" => Ok(Self::ToolCall),
            "tool_result" => Ok(Self::ToolResult),
            "image" => Ok(Self::Image),
            "event" => Ok(Self::Event),
            other => Err(SessionError::Unsupported(format!(
                "未知会话消息类型：{other}"
            ))),
        }
    }
}

/// 解析器跳过一行时留下的记录；`line` 从 1 开始。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWarning {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMessage {
    pub role: SessionRole,
    pub kind: SessionMessageKind,
    pub content: String,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSession {
    pub session_id: String,
    pub title: String,
    pub project_path: Option<PathBuf>,
    pub created_at: Option<i64>,
    pub updated_at: i64,
    pub messages: Vec<ParsedMessage>,
    pub warnings: Vec<SessionWarning>,
}

/// 列表项。`parse_error` 由 `session_files` 按 `source_path` 派生，不是 `sessions` 表的列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub agent_id: String,
    pub session_id: String,
    pub title: String,
    pub project_path: Option<String>,
    pub source_path: String,
    pub created_at: Option<i64>,
    pub updated_at: i64,
    pub message_count: u32,
    pub parse_error: Option<String>,
    /// 最后一条文本消息的摘要，列表行用于「最后：…」。
    #[serde(default)]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub seq: u32,
    pub role: SessionRole,
    pub kind: SessionMessageKind,
    pub content: String,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionSummary,
    pub messages: Vec<SessionMessage>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProviderStatus {
    pub agent_id: String,
    pub display_name: String,
    pub verified: bool,
    pub parse_ready: bool,
    /// 展开后的绝对路径（含 baseOverride），空状态直接展示给用户。
    pub roots: Vec<String>,
    pub root_exists: bool,
    pub found_files: usize,
    pub session_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("缺少会话 ID")]
    MissingId,
    #[error("暂不支持：{0}")]
    Unsupported(String),
}
