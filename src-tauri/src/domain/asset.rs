use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Mcp,
    Skill,
    Prompt,
    Rule,
    Subagent,
    Hook,
}

impl AssetType {
    pub const ALL: [AssetType; 6] = [
        Self::Mcp,
        Self::Skill,
        Self::Prompt,
        Self::Rule,
        Self::Subagent,
        Self::Hook,
    ];

    /// 能力表键回到真源资产类型：Session 不是真源资产，返回 Err 让调用方必须处理。
    pub fn from_kind(
        kind: crate::domain::agent::AssetKind,
    ) -> Result<Self, crate::domain::agent::AssetKind> {
        kind.try_as_asset_type()
    }

    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skill => "skills",
            Self::Prompt => "prompts",
            Self::Rule => "rules",
            Self::Subagent => "subagents",
            Self::Hook => "hooks",
        }
    }

    /// None 表示该资产在真源里是一个目录（skills/<id>/）而非单文件。
    pub fn file_extension(self) -> Option<&'static str> {
        match self {
            Self::Mcp => Some("json"),
            Self::Skill => None,
            Self::Prompt | Self::Rule => Some("md"),
            Self::Subagent | Self::Hook => Some("yaml"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
    pub asset_type: AssetType,
    pub id: String,
    pub name: String,
    pub updated_at: i64,
    pub hash: String,
    /// 列表行的副标题；由 `AssetService` 从真源内容补齐，不落索引表。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Skill 的来源仓库（owner/repo）；其他资产或本地 skill 为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 真源文件的真实修改时间（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
    /// 真源文件字节数，列表行展示大小用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// MCP 的传输方式，用于行内 badge。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
}

/// 「资产 × agent」四态（spec 3.3）：未同步 / 已写入 / 已生效 / 有漂移。
/// 落库用 `as_str()`，改名等于改数据，必须一并处理旧值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetState {
    Off,
    Written,
    Effective,
    Drifted,
}

impl AssetState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Written => "written",
            Self::Effective => "effective",
            Self::Drifted => "drifted",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "written" => Some(Self::Written),
            "effective" => Some(Self::Effective),
            "drifted" => Some(Self::Drifted),
            _ => None,
        }
    }
}

/// 漂移的两种形态（spec 5.4）。**只有我们落过盘的东西才谈漂移**：
/// 用户自己新建的文件不在台账里，那是漏管，不是漂移。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftKind {
    /// 磁盘内容与我们期望的不一致（通常是用户手改）
    Modified,
    /// 落盘的文件不见了
    Missing,
}

impl DriftKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Missing => "missing",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "modified" => Some(Self::Modified),
            "missing" => Some(Self::Missing),
            _ => None,
        }
    }
}
