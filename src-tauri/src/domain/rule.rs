use super::frontmatter::split_frontmatter;
use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleFrontmatter {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub meta: RuleFrontmatter,
    pub body: String,
}

pub fn parse_rule(id: &str, raw: &str) -> Result<Rule, DomainError> {
    if id.trim().is_empty() {
        return Err(DomainError::EmptyId);
    }
    let parts = split_frontmatter(raw)?;
    let Some(yaml) = parts.yaml else {
        return Err(DomainError::Yaml("rule 缺少 frontmatter".into()));
    };
    let meta: RuleFrontmatter =
        serde_yaml::from_str(&yaml).map_err(|e| DomainError::Yaml(e.to_string()))?;
    Ok(Rule {
        id: id.to_string(),
        meta,
        body: parts.body,
    })
}

pub fn render_rule(rule: &Rule) -> String {
    let yaml = serde_yaml::to_string(&rule.meta).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, rule.body)
}
