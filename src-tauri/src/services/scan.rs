//! 扫描 agent 侧的既有文件，把它们变成真源里的资产（spec 7.3 的「采纳磁盘」）。
//!
//! 与导入器的区别：导入读的是 cc-switch 的库，扫描读的是工具自己的落点，
//! 而且必须**幂等**——刚 deploy 完再扫一遍应当什么都不变，否则会造出假 diff。

use crate::adapters::rules;
use crate::domain::agent::AgentDescriptor;
use crate::domain::rule::parse_rule;
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::slug;
use crate::index::Index;
use crate::services::assets::AssetService;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 一次扫描的结果。`unchanged` 单独计数：幂等要能被断言，混进 skipped 就看不出来了。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub imported: usize,
    pub updated: usize,
    pub unchanged: usize,
    /// 没动的文件与原因，UI 逐条列出来，不静默丢。
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFile {
    pub file: String,
    pub reason: String,
}

fn read_if_utf8(path: &Path) -> Result<String, AppError> {
    let bytes = std::fs::read(path)?;
    String::from_utf8(bytes).map_err(|_| AppError::InvalidInput("规则文件不是 UTF-8".into()))
}

/// 扫 `<rules_dir>/*<ext>`：新文件建条目，已有文件只在正文不同时更新。
pub fn rules(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ScanReport, AppError> {
    if !agent.supported(AssetKind::Rule) {
        return Err(AppError::Unsupported(format!(
            "{} 没有 markdown 规则落点，扫描无意义",
            agent.id
        )));
    }
    let directory = rules::rules_dir(agent, home, base_override)?;
    let extension = rules::format_for_agent(agent).map_or(".md", |format| format.extension);
    let mut report = ScanReport::default();
    if !directory.is_dir() {
        return Ok(report);
    }

    let mut seen: Vec<String> = Vec::new();
    let mut entries = std::fs::read_dir(&directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if !file_name.ends_with(extension) {
            // 工具目录里别的扩展名（例如 Codex 的 *.rules）不是我们的对象
            report.skipped.push(SkippedFile {
                file: file_name,
                reason: format!("不是 {extension} 文件"),
            });
            continue;
        }
        let stem = file_name.trim_end_matches(extension);
        let Some(id) = slug(stem) else {
            report.skipped.push(SkippedFile {
                file: file_name,
                reason: "文件名推导不出合法 ID".into(),
            });
            continue;
        };
        if seen.iter().any(|existing| existing == &id) {
            report.skipped.push(SkippedFile {
                file: file_name,
                reason: format!("与已导入的同名条目冲突（ID {id}）"),
            });
            continue;
        }
        seen.push(id.clone());

        let text = match read_if_utf8(&path) {
            Ok(text) => text,
            Err(error) => {
                report.skipped.push(SkippedFile {
                    file: file_name,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let recovered = match rules::rule_from_tool_file(&id, &text) {
            Ok(rule) => rule,
            Err(error) => {
                report.skipped.push(SkippedFile {
                    file: file_name,
                    reason: error.to_string(),
                });
                continue;
            }
        };

        let existing = store
            .read(AssetType::Rule, &id)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|raw| parse_rule(&id, &raw).ok());

        match existing {
            None => {
                let canonical = crate::domain::rule::render_canonical(&recovered)?;
                AssetService::write(store, index, AssetType::Rule, &id, &canonical)?;
                report.imported += 1;
            }
            Some(existing) => {
                if rules::body_equals(&text, &existing)? {
                    report.unchanged += 1;
                    continue;
                }
                // 只把正文换回真源，frontmatter 保持真源自己那份
                let merged = rules::merge_body_into_team(&text, Some(&existing))?;
                AssetService::write(store, index, AssetType::Rule, &id, &merged)?;
                report.updated += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::rules::deploy;
    use crate::domain::agent::builtin_agents;
    use crate::domain::rule::{render_canonical, Rule, RuleFrontmatter};
    use crate::index::Index;
    use crate::store::Store;
    use tempfile::tempdir;

    struct Fixture {
        /// 只在 Drop 时保住临时目录，测试结束前不能让它被删掉。
        _dir: tempfile::TempDir,
        store: Store,
        index: Index,
        home: std::path::PathBuf,
        agent: AgentDescriptor,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".qoder-cn/rules")).unwrap();
        Fixture {
            _dir: dir,
            store,
            index,
            home,
            agent: builtin_agents()
                .into_iter()
                .find(|a| a.id == "qoder-cn")
                .unwrap(),
        }
    }

    impl Fixture {
        fn rules_dir(&self) -> std::path::PathBuf {
            self.home.join(".qoder-cn/rules")
        }

        fn write_tool_rule(&self, name: &str, text: &str) {
            std::fs::write(self.rules_dir().join(name), text).unwrap();
        }

        fn seed_canonical(&self, id: &str, paths: &[&str], body: &str) {
            let rule = Rule {
                id: id.into(),
                meta: RuleFrontmatter {
                    name: id.into(),
                    description: String::new(),
                    paths: paths.iter().map(|p| p.to_string()).collect(),
                    trigger: None,
                },
                body: body.into(),
            };
            self.store
                .write(
                    AssetType::Rule,
                    id,
                    render_canonical(&rule).unwrap().as_bytes(),
                )
                .unwrap();
        }

        fn canonical(&self, id: &str) -> Rule {
            let text = String::from_utf8(self.store.read(AssetType::Rule, id).unwrap()).unwrap();
            parse_rule(id, &text).unwrap()
        }
    }

    #[test]
    fn scanning_a_tool_rule_becomes_a_canonical_rule_with_the_same_body() {
        let mut fx = fixture();
        // Qoder 侧 trigger: glob + glob: a, b → canonical paths 应当拿回两个值
        fx.write_tool_rule(
            "style.md",
            "---\ntrigger: glob\nglob: src/**\n---\n\n正文\n",
        );

        let report = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(report.skipped.len(), 0, "{:?}", report.skipped);
        let rule = fx.canonical("style");
        assert_eq!(rule.meta.paths, vec!["src/**".to_string()]);
        assert_eq!(rule.body, "正文\n");
        assert_eq!(
            rule.meta.trigger,
            Some(crate::domain::rule::RuleTrigger::Glob)
        );
        // 索引里也得有，面板才列得出来
        let listed = fx.index.list(Some(AssetType::Rule)).unwrap();
        assert_eq!(
            listed.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["style"]
        );
    }

    #[test]
    fn scanning_is_idempotent_right_after_a_deploy() {
        let mut fx = fixture();
        fx.seed_canonical("style", &["src/**"], "正文\n");
        deploy(&fx.store, &mut fx.index, &fx.agent, "style", &fx.home, None).unwrap();

        let before = fx.store.read(AssetType::Rule, "style").unwrap();
        let report = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(
            (report.imported, report.updated, report.unchanged),
            (0, 0, 1),
            "刚部署完再扫一遍必须什么都不改：{report:?}"
        );
        assert_eq!(fx.store.read(AssetType::Rule, "style").unwrap(), before);
    }

    #[test]
    fn a_tool_side_body_edit_is_pulled_back_without_losing_the_source_frontmatter() {
        let mut fx = fixture();
        fx.seed_canonical("style", &["src/**"], "旧正文\n");
        fx.write_tool_rule("style.md", "---\ntrigger: always_on\n---\n\n新正文\n");

        let report = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(report.updated, 1, "{report:?}");
        let rule = fx.canonical("style");
        assert_eq!(rule.body, "新正文\n");
        assert_eq!(
            rule.meta.paths,
            vec!["src/**".to_string()],
            "工具侧的 trigger: always_on 不能反过来把真源的 paths 抹掉"
        );
        assert_eq!(rule.meta.trigger, None);
    }

    #[test]
    fn other_extensions_broken_files_and_unslammable_names_are_named_not_dropped() {
        let mut fx = fixture();
        fx.write_tool_rule("default.rules", "prefix_rule(pattern=[\"git\"])");
        fx.write_tool_rule("broken.md", "---\ntrigger: [unclosed\n---\n\n正文\n");
        fx.write_tool_rule("好.md", "---\ntrigger: always_on\n---\n\n正文\n");
        fx.write_tool_rule("style.md", "---\ntrigger: always_on\n---\n\n正文\n");

        let report = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        let skipped: Vec<&str> = report
            .skipped
            .iter()
            .map(|skip| skip.file.as_str())
            .collect();
        assert_eq!(skipped.len(), 3, "{skipped:?}");
        for expected in ["broken.md", "default.rules", "好.md"] {
            assert!(
                skipped.contains(&expected),
                "{expected} 该被点名：{skipped:?}"
            );
        }
        assert!(
            report
                .skipped
                .iter()
                .any(|s| s.file == "好.md" && s.reason.contains("合法 ID")),
            "纯中文文件名要说明为什么进不来：{:?}",
            report.skipped
        );
    }

    #[test]
    fn two_files_colliding_on_one_id_leave_the_first_and_name_the_second() {
        let mut fx = fixture();
        fx.write_tool_rule("a b.md", "---\ntrigger: always_on\n---\n\n第一份\n");
        fx.write_tool_rule("a-b.md", "---\ntrigger: always_on\n---\n\n第二份\n");

        let report = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
        assert!(
            report.skipped[0].reason.contains("冲突"),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn an_agent_without_rules_is_refused_rather_than_scanning_nothing() {
        let mut fx = fixture();
        fx.agent = builtin_agents()
            .into_iter()
            .find(|a| a.id == "claude")
            .unwrap();
        let error = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap_err();
        assert!(
            matches!(error, AppError::Unsupported(_)),
            "Claude 没有用户级 rules 落点，扫描请求必须被拒：{error}"
        );
    }

    #[test]
    fn a_missing_rules_dir_scans_to_an_empty_report() {
        let mut fx = fixture();
        std::fs::remove_dir_all(fx.rules_dir()).unwrap();
        let report = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(report, ScanReport::default());
    }

    #[test]
    fn scanning_honours_the_base_override() {
        let fx_tmp = tempdir().unwrap();
        let mut fx = fixture();
        let sandbox = fx_tmp.path().join("sandbox");
        std::fs::create_dir_all(sandbox.join(".qoder-cn/rules")).unwrap();
        std::fs::write(
            sandbox.join(".qoder-cn/rules/from-sandbox.md"),
            "---\ntrigger: always_on\n---\n\n沙箱里的规则\n",
        )
        .unwrap();

        let report = rules(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            &fx.home,
            Some(&sandbox),
        )
        .unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(fx.canonical("from-sandbox").body, "沙箱里的规则\n");
        // 没被覆盖时读的是真实 home，那边是空的
        let untouched = rules(&fx.store, &mut fx.index, &fx.agent, &fx.home, None).unwrap();
        assert_eq!(untouched, ScanReport::default());
    }
}
