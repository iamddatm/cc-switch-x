use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    pub name: String,
    pub event: String,
    #[serde(default)]
    pub matcher: String,
    pub command: String,
    #[serde(default)]
    pub command_windows: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

pub fn parse_hook(raw: &str) -> Result<Hook, DomainError> {
    let parsed: Hook = serde_yaml::from_str(raw).map_err(|e| DomainError::Yaml(e.to_string()))?;
    if parsed.name.trim().is_empty() {
        return Err(DomainError::EmptyName);
    }
    if parsed.event.trim().is_empty() {
        return Err(DomainError::EmptyName);
    }
    if parsed.command.trim().is_empty() {
        return Err(DomainError::EmptyName);
    }
    Ok(parsed)
}
