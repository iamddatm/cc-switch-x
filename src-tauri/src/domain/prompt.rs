use super::frontmatter::split_frontmatter;
use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PromptMetadata {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub id: String,
    pub meta: PromptMetadata,
    pub body: String,
}

pub fn parse_prompt(id: &str, raw: &str) -> Result<Prompt, DomainError> {
    if id.trim().is_empty() {
        return Err(DomainError::EmptyId);
    }
    let parts = split_frontmatter(raw)?;
    let meta = match parts.yaml {
        Some(yaml) => serde_yaml::from_str(&yaml).map_err(|e| DomainError::Yaml(e.to_string()))?,
        None => PromptMetadata::default(),
    };
    Ok(Prompt {
        id: id.to_string(),
        meta,
        body: parts.body,
    })
}

pub fn render_prompt(prompt: &Prompt) -> String {
    let yaml = serde_yaml::to_string(&prompt.meta).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, prompt.body)
}
