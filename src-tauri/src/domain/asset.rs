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
