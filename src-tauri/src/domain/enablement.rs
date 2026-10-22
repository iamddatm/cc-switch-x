use super::AssetType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `agents/<agent-id>.json`：跨设备同步的用户态启用矩阵（spec 4.3）。
/// 本机专属的东西（路径覆盖、检测结果、链接方式）一律在 `local/agents/`，不在这个文件里。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEnablement {
    /// 键用 `AssetType::dir_name()`（mcp / skills / prompts …），与真源目录名保持一致。
    #[serde(default)]
    pub enabled: BTreeMap<String, Vec<String>>,
    /// 每类里 Prompts 同时只能有一个激活项；其余为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_prompt: Option<String>,
}

impl AgentEnablement {
    pub fn enabled_for(&self, asset_type: AssetType) -> Vec<String> {
        self.enabled
            .get(asset_type.dir_name())
            .cloned()
            .unwrap_or_default()
    }

    pub fn enable(&mut self, asset_type: AssetType, id: &str) {
        let ids = self
            .enabled
            .entry(asset_type.dir_name().to_string())
            .or_default();
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
            ids.sort();
        }
    }

    pub fn disable(&mut self, asset_type: AssetType, id: &str) {
        let entry = self
            .enabled
            .entry(asset_type.dir_name().to_string())
            .or_default();
        entry.retain(|existing| existing != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_is_idempotent_and_sorted() {
        let mut matrix = AgentEnablement::default();
        matrix.enable(AssetType::Mcp, "zz");
        matrix.enable(AssetType::Mcp, "aa");
        matrix.enable(AssetType::Mcp, "aa");
        assert_eq!(
            matrix.enabled_for(AssetType::Mcp),
            vec!["aa".to_string(), "zz".to_string()]
        );
        matrix.disable(AssetType::Mcp, "zz");
        assert_eq!(matrix.enabled_for(AssetType::Mcp), vec!["aa".to_string()]);
    }

    #[test]
    fn json_shape_matches_the_spec_example() {
        let mut matrix = AgentEnablement::default();
        matrix.enable(AssetType::Skill, "lark-wiki");
        matrix.active_prompt = Some("team-default".into());
        let text = serde_json::to_string(&matrix).unwrap();
        assert!(text.contains(r#""skills":["lark-wiki"]"#), "{text}");
        assert!(text.contains(r#""activePrompt":"team-default""#), "{text}");
    }
}
