use super::frontmatter::split_frontmatter;
use super::DomainError;
use serde::{Deserialize, Serialize};

/// Qoder 侧的窄化方式。真源里可以显式写 `trigger`，也可以由 `paths` 派生。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleTrigger {
    AlwaysOn,
    Glob,
    Manual,
    ModelDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleFrontmatter {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub trigger: Option<RuleTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub meta: RuleFrontmatter,
    pub body: String,
}

impl Rule {
    /// 派生规则与 teamai 一致：写了 `trigger` 就照它，没写则「有 paths → glob，
    /// 无 paths → always_on」。显式 `glob` 但 paths 为空时退成 always_on ——
    /// 空 glob 行在 Qoder 侧没有意义，不能为了保住字面值写出一份无效文件。
    pub fn effective_trigger(&self) -> RuleTrigger {
        let scoped = !self.meta.paths.is_empty();
        match self.meta.trigger {
            Some(
                trigger
                @ (RuleTrigger::Manual | RuleTrigger::ModelDecision | RuleTrigger::AlwaysOn),
            ) => trigger,
            // 显式 glob 但没有 paths：空 glob 行在 Qoder 侧无效，退成 always_on
            Some(RuleTrigger::Glob) if !scoped => RuleTrigger::AlwaysOn,
            Some(RuleTrigger::Glob) | None => {
                if scoped {
                    RuleTrigger::Glob
                } else {
                    RuleTrigger::AlwaysOn
                }
            }
        }
    }
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

/// canonical 真源的 `.md`。序列化失败必须如实报出来：这里原先是
/// `unwrap_or_default()`，一旦失败会写出没有 frontmatter 的空文件。
pub fn render_canonical(rule: &Rule) -> Result<String, DomainError> {
    let yaml = serde_yaml::to_string(&rule.meta).map_err(|e| DomainError::Yaml(e.to_string()))?;
    Ok(format!("---\n{}---\n\n{}", yaml, rule.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(trigger: Option<RuleTrigger>, paths: &[&str]) -> Rule {
        Rule {
            id: "style".into(),
            meta: RuleFrontmatter {
                name: "style".into(),
                description: String::new(),
                paths: paths.iter().map(|p| p.to_string()).collect(),
                trigger,
            },
            body: "正文\n".into(),
        }
    }

    #[test]
    fn trigger_is_derived_from_paths_when_absent() {
        assert_eq!(
            rule(None, &["src/**"]).effective_trigger(),
            RuleTrigger::Glob
        );
        assert_eq!(rule(None, &[]).effective_trigger(), RuleTrigger::AlwaysOn);
        assert_eq!(
            rule(Some(RuleTrigger::Manual), &["src/**"]).effective_trigger(),
            RuleTrigger::Manual
        );
        assert_eq!(
            rule(Some(RuleTrigger::AlwaysOn), &["src/**"]).effective_trigger(),
            RuleTrigger::AlwaysOn,
            "显式 always_on 不能被 paths 反过来改成 glob"
        );
    }

    #[test]
    fn an_empty_glob_list_cannot_produce_an_invalid_file() {
        assert_eq!(
            rule(Some(RuleTrigger::Glob), &[]).effective_trigger(),
            RuleTrigger::AlwaysOn
        );
    }

    #[test]
    fn canonical_round_trip_keeps_the_trigger() {
        let original = rule(Some(RuleTrigger::ModelDecision), &["src/**"]);
        let text = render_canonical(&original).unwrap();
        let parsed = parse_rule("style", &text).unwrap();
        assert_eq!(parsed.meta.trigger, Some(RuleTrigger::ModelDecision));
        assert_eq!(parsed.meta.paths, original.meta.paths);
        assert!(text.contains("trigger: model_decision"), "{text}");
    }

    #[test]
    fn a_rule_without_frontmatter_is_rejected() {
        assert!(matches!(
            parse_rule("style", "只有正文"),
            Err(DomainError::Yaml(_))
        ));
    }

    #[test]
    fn an_unknown_trigger_is_a_parse_error_not_a_silent_default() {
        assert!(matches!(
            parse_rule("style", "---\nname: x\ntrigger: sometimes\n---\n\n正文\n"),
            Err(DomainError::Yaml(_))
        ));
    }
}
