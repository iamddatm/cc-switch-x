use super::DomainError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
    Sse,
    Ws,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl McpServer {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.trim().is_empty() {
            return Err(DomainError::EmptyId);
        }
        if self.name.trim().is_empty() {
            return Err(DomainError::EmptyName);
        }
        match self.transport {
            Transport::Stdio => {
                if self.command.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(DomainError::McpCommandRequired);
                }
            }
            _ => {
                if self.url.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(DomainError::McpUrlRequired);
                }
            }
        }
        Ok(())
    }
}
