use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subagent {
    pub name: String,
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tool_extras: serde_json::Value,
    #[serde(default)]
    pub targets: Vec<String>,
}

pub fn parse_subagent(raw: &str) -> Result<Subagent, DomainError> {
    let parsed: Subagent =
        serde_yaml::from_str(raw).map_err(|e| DomainError::Yaml(e.to_string()))?;
    if parsed.name.trim().is_empty() {
        return Err(DomainError::EmptyName);
    }
    Ok(parsed)
}
