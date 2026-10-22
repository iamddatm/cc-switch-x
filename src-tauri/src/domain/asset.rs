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
