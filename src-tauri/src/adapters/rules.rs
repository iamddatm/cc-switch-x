//! 规则文件按 agent 的落盘形态。canonical 真源永远是工具中立的 `.md`
//! （`domain/rule.rs`），这里只负责「渲染出去」与「把工具侧的正文推回来」。
//!
//! 机制借自 teamai-cli 的 `RuleFormat`（MIT，见 THIRD_PARTY_NOTICES.md）：
//! 三段式 render / body_equals / merge_body_into_team，加 `previous_renders`
//! 认出早期版本写的文件，否则改了渲染格式就再也不能安全替换自家旧产物。

use crate::domain::agent::AgentDescriptor;
use crate::domain::frontmatter::split_frontmatter;
use crate::domain::rule::{render_canonical, Rule, RuleFrontmatter, RuleTrigger};
use crate::domain::DomainError;

/// 渲染一个工具形态的规则文件。失败只可能是 canonical 序列化本身出错。
pub type Render = fn(&Rule) -> Result<String, DomainError>;

pub struct RuleFormat {
    /// 落盘用的扩展名。扫描与删除都要按它找文件，不能写死 `.md`。
    pub extension: &'static str,
    pub render: Render,
    /// 该工具靠哪几个 frontmatter 字段窄化规则范围——只用于报告文案。
    pub scope_fields: &'static [&'static str],
    /// 早期版本写过的形态。磁盘上是其中之一 → 仍算我们家的，可替换。
    pub previous_renders: &'static [Render],
}

/// `{a,b}` 里的逗号不算分隔：`src/{a,b}/**` 是一条 glob。
fn split_top_level_commas(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                parts.push(value[start..index].to_string());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].to_string());
    parts
}

/// 展开 `{a,b}` 交替，供「按逗号切 glob 列表」的工具使用。
/// 组内不足两个候选时保持原样（`{a}` 是字面量，不是交替）。
pub fn expand_braces(glob: &str) -> Vec<String> {
    let mut depth = 0usize;
    let mut open: Option<usize> = None;
    for (index, ch) in glob.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    open = Some(index);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth > 0 {
                    continue;
                }
                let group_open = open.unwrap_or(0);
                let alternatives = split_top_level_commas(&glob[group_open + 1..index]);
                if alternatives.len() < 2 {
                    continue;
                }
                let prefix = &glob[..group_open];
                let suffix = &glob[index + 1..];
                return alternatives
                    .iter()
                    .flat_map(|alternative| {
                        expand_braces(&format!("{prefix}{alternative}{suffix}"))
                    })
                    .collect();
            }
            _ => {}
        }
    }
    vec![glob.to_string()]
}

pub fn verbatim_render(rule: &Rule) -> Result<String, DomainError> {
    render_canonical(rule)
}

/// Qoder Desktop 写规则的形状（CLI 也读同一份）：`paths` 展开后写成单行
/// 不加引号的 `glob: a, b`；没有 `paths` 就是 `trigger: always_on`。
/// Qoder 不发布 schema，这个形态取自本机 Qoder CN CLI 二进制里的解析器
/// 与 alibaba/tron-one-agent 的真实规则文件（两源互证）。
fn render_qoder(rule: &Rule) -> Result<String, DomainError> {
    let frontmatter = match rule.effective_trigger() {
        RuleTrigger::Glob => {
            let globs = expand_globs(&rule.meta.paths);
            format!("trigger: glob\nglob: {}", globs.join(", "))
        }
        RuleTrigger::AlwaysOn => "trigger: always_on".to_string(),
        RuleTrigger::Manual => "trigger: manual".to_string(),
        // 缺 description 时 Qoder 自己会降级成 manual，我们不改写、如实透传
        RuleTrigger::ModelDecision => {
            if rule.meta.description.trim().is_empty() {
                "trigger: model_decision".to_string()
            } else {
                format!(
                    "trigger: model_decision\ndescription: {}",
                    rule.meta.description
                )
            }
        }
    };
    Ok(format!(
        "---\n{frontmatter}\n---\n\n{}\n",
        normalize_body(&rule.body)
    ))
}

static QODER_RULE_FORMAT: RuleFormat = RuleFormat {
    extension: ".md",
    render: render_qoder,
    scope_fields: &["trigger", "glob"],
    // 早期版本直接把真源正文抄过去（带 canonical frontmatter）
    previous_renders: &[verbatim_render],
};

/// 该 agent 自己的规则形态；None 表示它直接读工具中立的 canonical `.md`。
pub fn format_for_agent(agent: &AgentDescriptor) -> Option<&'static RuleFormat> {
    match agent.id.as_str() {
        "qoder" | "qoder-cn" => Some(&QODER_RULE_FORMAT),
        _ => None,
    }
}

pub fn render_rule(agent: &AgentDescriptor, rule: &Rule) -> Result<String, DomainError> {
    match format_for_agent(agent) {
        Some(format) => (format.render)(rule),
        None => verbatim_render(rule),
    }
}

/// 一个 agent 可能收到过的全部形态：当前渲染 + 早期渲染。
/// 认「盘上这份是不是我们写的」时对每一个都比一遍。
pub fn delivered_renders(agent: &AgentDescriptor) -> Vec<Render> {
    match format_for_agent(agent) {
        Some(format) => {
            let mut renders = vec![format.render];
            renders.extend_from_slice(format.previous_renders);
            renders
        }
        None => vec![verbatim_render],
    }
}

/// canonical 真源里 paths 的顺序与 brace 写法都可能被用户在工具侧改掉，
/// 所以比对前双方都展开去重。
pub fn expand_globs(paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        for glob in expand_braces(path.trim()) {
            let glob = glob.trim();
            if !glob.is_empty() && !out.iter().any(|existing| existing == glob) {
                out.push(glob.to_string());
            }
        }
    }
    out
}

/// 比正文，不比 frontmatter：工具侧的 trigger/glob 是渲染派生物，
/// 拉下来再推回去必然重写一遍，拿它比会造出假 diff。
pub fn body_equals(tool_text: &str, rule: &Rule) -> Result<bool, DomainError> {
    Ok(normalize_body(&body_of(tool_text)?) == normalize_body(&rule.body))
}

fn body_of(text: &str) -> Result<String, DomainError> {
    Ok(split_frontmatter(text)?.body)
}

fn normalize_body(body: &str) -> String {
    body.trim().to_string()
}

/// 工具侧的正文换回 canonical，frontmatter 由真源自己保留：
/// 拿工具派生的 `trigger: glob` 反向覆盖 `paths:`，下次同步会把规则的范围弄丢。
/// `existing = None` 只在 push 新文件时用到，产物是纯正文（没有真源元数据可依）。
pub fn merge_body_into_team(
    tool_text: &str,
    existing: Option<&Rule>,
) -> Result<String, DomainError> {
    let body = normalize_body(&body_of(tool_text)?);
    let Some(existing) = existing else {
        return Ok(format!("{body}\n"));
    };
    if normalize_body(&existing.body) == body {
        // 正文没变就逐字节交还原文件，空推一次不该显示成 diff
        return render_canonical(existing);
    }
    let mut merged = existing.clone();
    // 正文规范成「trim 后带一个结尾换行」，与真源里其他 .md 同形
    merged.body = format!("{body}\n");
    render_canonical(&merged)
}

/// 从工具侧文件反推 canonical（扫描导入用）：`glob` 行拿回 `paths`。
/// 正文补回结尾换行，让反推出来的 canonical 与真源里自己写的文件同形。
pub fn rule_from_tool_file(id: &str, tool_text: &str) -> Result<Rule, DomainError> {
    let parts = split_frontmatter(tool_text)?;
    let body = format!("{}\n", normalize_body(&parts.body));
    let Some(yaml) = parts.yaml else {
        return Ok(Rule {
            id: id.to_string(),
            meta: RuleFrontmatter {
                name: id.to_string(),
                description: String::new(),
                paths: Vec::new(),
                trigger: None,
            },
            body,
        });
    };
    let value: serde_yaml::Value =
        serde_yaml::from_str(&yaml).map_err(|e| DomainError::Yaml(e.to_string()))?;
    let read = |key: &str| -> Option<String> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
    };
    let trigger = match read("trigger").as_deref() {
        Some("always_on") => Some(RuleTrigger::AlwaysOn),
        Some("glob") => Some(RuleTrigger::Glob),
        Some("manual") => Some(RuleTrigger::Manual),
        Some("model_decision") => Some(RuleTrigger::ModelDecision),
        // 认不出的 trigger 不当成没写：留给 canonical 派生，报告里点名
        _ => None,
    };
    let paths = match read("glob") {
        Some(glob) => expand_globs(
            &glob
                .split(',')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect::<Vec<String>>(),
        ),
        None => Vec::new(),
    };
    Ok(Rule {
        id: id.to_string(),
        meta: crate::domain::rule::RuleFrontmatter {
            name: read("name").unwrap_or_else(|| id.to_string()),
            description: read("description").unwrap_or_default(),
            paths,
            trigger,
        },
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::domain::rule::parse_rule;

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn qoder() -> AgentDescriptor {
        agent("qoder")
    }

    fn rule_with_paths(paths: &[&str]) -> Rule {
        Rule {
            id: "style".into(),
            meta: RuleFrontmatter {
                name: "style".into(),
                description: "写法".into(),
                paths: paths.iter().map(|p| p.to_string()).collect(),
                trigger: None,
            },
            body: "正文\n".into(),
        }
    }

    fn rule_without_paths() -> Rule {
        rule_with_paths(&[])
    }

    #[test]
    fn paths_become_a_single_unquoted_glob_line() {
        let rendered = render_rule(&qoder(), &rule_with_paths(&["src/**", "docs/**"])).unwrap();
        assert_eq!(
            rendered,
            "---\ntrigger: glob\nglob: src/**, docs/**\n---\n\n正文\n"
        );
    }

    #[test]
    fn brace_alternation_is_expanded_and_deduplicated() {
        let rendered = render_rule(
            &qoder(),
            &rule_with_paths(&["src/{a,b}/*.ts", "src/a/*.ts"]),
        )
        .unwrap();
        assert!(
            rendered.contains("glob: src/a/*.ts, src/b/*.ts"),
            "{rendered}"
        );
        assert_eq!(rendered.matches("src/a/*.ts").count(), 1, "展开后不能重复");
    }

    #[test]
    fn a_brace_group_without_a_comma_stays_literal() {
        assert_eq!(expand_braces("src/{a}/*.ts"), vec!["src/{a}/*.ts"]);
        assert_eq!(
            expand_braces("src/{a,b}/{c,d}.ts"),
            vec!["src/a/c.ts", "src/a/d.ts", "src/b/c.ts", "src/b/d.ts"]
        );
    }

    #[test]
    fn a_rule_without_paths_is_always_on() {
        let rendered = render_rule(&qoder(), &rule_without_paths()).unwrap();
        assert_eq!(rendered, "---\ntrigger: always_on\n---\n\n正文\n");
    }

    #[test]
    fn an_explicit_trigger_survives_the_render() {
        let mut rule = rule_with_paths(&["src/**"]);
        rule.meta.trigger = Some(RuleTrigger::Manual);
        let rendered = render_rule(&qoder(), &rule).unwrap();
        assert!(rendered.contains("trigger: manual"), "{rendered}");
        assert!(!rendered.contains("glob:"), "manual 规则不该被带上 glob");

        rule.meta.trigger = Some(RuleTrigger::ModelDecision);
        let rendered = render_rule(&qoder(), &rule).unwrap();
        assert!(
            rendered.contains("trigger: model_decision\ndescription: 写法"),
            "{rendered}"
        );
    }

    #[test]
    fn body_survives_the_round_trip_and_frontmatter_stays_owned_by_the_source() {
        let team = rule_with_paths(&["src/**"]);
        let tool_file = render_rule(&qoder(), &team).unwrap();
        // 用户在 Qoder 那边只改了正文
        let edited = tool_file.replace("正文", "改过的正文");
        assert!(!body_equals(&edited, &team).unwrap());
        let merged = merge_body_into_team(&edited, Some(&team)).unwrap();
        assert_eq!(parse_rule("style", &merged).unwrap().body, "改过的正文\n");
        // 真源自己的 paths 不能被工具侧派生的 trigger/glob 反向覆盖
        assert_eq!(
            parse_rule("style", &merged).unwrap().meta.paths,
            vec!["src/**".to_string()]
        );
    }

    #[test]
    fn pushing_an_unchanged_body_produces_no_diff() {
        let team = rule_with_paths(&["src/**"]);
        let tool_file = render_rule(&qoder(), &team).unwrap();
        let merged = merge_body_into_team(&tool_file, Some(&team)).unwrap();
        assert_eq!(
            merged,
            render_canonical(&team).unwrap(),
            "正文没变时必须逐字节交还原文件"
        );
        assert!(body_equals(&tool_file, &team).unwrap());
    }

    #[test]
    fn an_older_render_is_still_recognised_as_ours() {
        let team = rule_with_paths(&["src/**"]);
        let legacy = verbatim_render(&team).unwrap();
        assert!(delivered_renders(&qoder())
            .iter()
            .any(|render| render(&team).unwrap() == legacy));
        // 当前渲染也在名单里，否则新写的文件反倒不认
        assert!(delivered_renders(&qoder())
            .iter()
            .any(|render| render(&team).unwrap() == render_rule(&qoder(), &team).unwrap()));
    }

    #[test]
    fn only_qoder_gets_a_derived_frontmatter() {
        for id in ["claude", "codex", "gemini", "pi"] {
            assert!(
                format_for_agent(&agent(id)).is_none(),
                "{id} 没有自己的规则形态"
            );
        }
        assert_eq!(
            format_for_agent(&agent("qoder-cn")).unwrap().extension,
            ".md"
        );
    }

    #[test]
    fn scanning_a_qoder_rule_recovers_paths_from_the_glob_line() {
        let tool_text = "---\ntrigger: glob\nglob: src/**, docs/**\n---\n\n正文\n";
        let rule = rule_from_tool_file("style", tool_text).unwrap();
        assert_eq!(rule.meta.name, "style");
        assert_eq!(rule.meta.trigger, Some(RuleTrigger::Glob));
        assert_eq!(
            rule.meta.paths,
            vec!["src/**".to_string(), "docs/**".to_string()]
        );
        assert_eq!(rule.body, "正文\n");

        let always = rule_from_tool_file("x", "---\ntrigger: always_on\n---\n\n正文\n").unwrap();
        assert_eq!(always.meta.trigger, Some(RuleTrigger::AlwaysOn));
        assert!(always.meta.paths.is_empty());
    }

    #[test]
    fn the_canonical_render_round_trips_through_parse() {
        let team = rule_with_paths(&["src/{a,b}/*.ts"]);
        let canonical = render_canonical(&team).unwrap();
        let parsed = parse_rule("style", &canonical).unwrap();
        assert_eq!(parsed.meta.paths, team.meta.paths);
        assert_eq!(parsed.body, team.body);
    }

    #[test]
    fn an_unparsable_frontmatter_is_an_error_not_an_empty_rule() {
        let broken = "---\ntrigger: [unclosed\n---\n\n正文\n";
        assert!(rule_from_tool_file("style", broken).is_err());
    }
}
