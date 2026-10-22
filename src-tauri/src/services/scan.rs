//! 扫描 agent 侧的既有文件，把它们变成真源里的资产（spec 7.3 的「采纳磁盘」）。
//!
//! 与导入器的区别：导入读的是 cc-switch 的库，扫描读的是工具自己的落点，
//! 而且必须**幂等**——刚 deploy 完再扫一遍应当什么都不变，否则会造出假 diff。

use crate::adapters::{rules, subagents};
use crate::domain::agent::AgentDescriptor;
use crate::domain::rule::{parse_rule, render_canonical as render_rule_canonical};
use crate::domain::subagent::{parse_subagent, render_canonical as render_subagent_canonical};
use crate::domain::{AssetKind, AssetType, DomainError};
use crate::error::AppError;
use crate::import::slug;
use crate::index::Index;
use crate::services::assets::AssetService;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// 一类「整文件资产」的扫描映射。实现者只回答类型特有的问题（目录、扩展名、
/// 工具文件 ↔ canonical 的两条边），找文件、推 ID、撞名、写真源、记索引都走同一份代码。
trait FileAsset {
    /// 报错与跳过原因里的类型名。
    const LABEL: &'static str;
    const ASSET_TYPE: AssetType;

    fn kind(&self) -> AssetKind;
    fn directory(
        &self,
        agent: &AgentDescriptor,
        home: &Path,
        base_override: Option<&Path>,
    ) -> Result<PathBuf, AppError>;
    fn extension(&self, agent: &AgentDescriptor) -> &'static str;

    /// 该写进真源的文本。`existing` 是真源里已有的原文；None 表示还没有这个条目。
    fn canonical_text(
        &self,
        agent: &AgentDescriptor,
        id: &str,
        text: &str,
        existing: Option<&str>,
    ) -> Result<String, DomainError>;

    /// 工具文件与既有真源是否等价（等价就不写，计 unchanged）。
    fn matches_existing(
        &self,
        agent: &AgentDescriptor,
        id: &str,
        text: &str,
        existing: &str,
    ) -> Result<bool, DomainError>;
}

struct RuleAsset;

impl FileAsset for RuleAsset {
    const LABEL: &'static str = "规则";
    const ASSET_TYPE: AssetType = AssetType::Rule;

    fn kind(&self) -> AssetKind {
        AssetKind::Rule
    }

    fn directory(
        &self,
        agent: &AgentDescriptor,
        home: &Path,
        base_override: Option<&Path>,
    ) -> Result<PathBuf, AppError> {
        rules::rules_dir(agent, home, base_override)
    }

    fn extension(&self, agent: &AgentDescriptor) -> &'static str {
        rules::format_for_agent(agent).map_or(".md", |format| format.extension)
    }

    fn canonical_text(
        &self,
        _agent: &AgentDescriptor,
        id: &str,
        text: &str,
        existing: Option<&str>,
    ) -> Result<String, DomainError> {
        let Some(existing_raw) = existing else {
            return render_rule_canonical(&rules::rule_from_tool_file(id, text)?);
        };
        // 已有真源：只把正文换回去，frontmatter（paths 等）由真源自己保留
        let existing_rule = parse_rule(id, existing_raw)?;
        rules::merge_body_into_team(text, Some(&existing_rule))
    }

    fn matches_existing(
        &self,
        _agent: &AgentDescriptor,
        id: &str,
        text: &str,
        existing: &str,
    ) -> Result<bool, DomainError> {
        // 只比正文：工具侧的 trigger/glob 是渲染派生物，比 frontmatter 会造出假 diff
        let existing_rule = parse_rule(id, existing)?;
        rules::body_equals(text, &existing_rule)
    }
}

struct SubagentAsset;

fn subagent_from_tool(
    agent: &AgentDescriptor,
    id: &str,
    text: &str,
) -> Result<crate::domain::subagent::Subagent, DomainError> {
    let file_name = format!("{id}{}", subagents::extension_for(agent));
    subagents::reverse_subagent(agent, &file_name, text)
}

impl FileAsset for SubagentAsset {
    const LABEL: &'static str = "子代理";
    const ASSET_TYPE: AssetType = AssetType::Subagent;

    fn kind(&self) -> AssetKind {
        AssetKind::Subagent
    }

    fn directory(
        &self,
        agent: &AgentDescriptor,
        home: &Path,
        base_override: Option<&Path>,
    ) -> Result<PathBuf, AppError> {
        subagents::agents_dir(agent, home, base_override)
    }

    fn extension(&self, agent: &AgentDescriptor) -> &'static str {
        subagents::extension_for(agent)
    }

    fn canonical_text(
        &self,
        agent: &AgentDescriptor,
        id: &str,
        text: &str,
        existing: Option<&str>,
    ) -> Result<String, DomainError> {
        let mut recovered = subagent_from_tool(agent, id, text)?;
        if let Some(raw) = existing {
            // targets 只存在于真源侧（工具文件里根本没这个概念），反推不能把它抹掉
            recovered.targets = parse_subagent(raw)?.targets;
        }
        render_subagent_canonical(&recovered)
    }

    fn matches_existing(
        &self,
        agent: &AgentDescriptor,
        id: &str,
        text: &str,
        existing: &str,
    ) -> Result<bool, DomainError> {
        let current = parse_subagent(existing)?;
        let mut recovered = subagent_from_tool(agent, id, text)?;
        recovered.targets = current.targets.clone();
        Ok(recovered == current)
    }
}

fn scan_assets<M: FileAsset>(
    mapping: &M,
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ScanReport, AppError> {
    if !agent.supported(mapping.kind()) {
        return Err(AppError::Unsupported(format!(
            "{} 没有{}落点，扫描无意义",
            agent.id,
            M::LABEL
        )));
    }
    let directory = mapping.directory(agent, home, base_override)?;
    let extension = mapping.extension(agent);
    let mut report = ScanReport::default();
    if !directory.is_dir() {
        return Ok(report);
    }

    let mut files = std::fs::read_dir(&directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();

    let mut seen: Vec<String> = Vec::new();
    for path in files {
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

        let text = match read_if_utf8(&path, M::LABEL) {
            Ok(text) => text,
            Err(error) => {
                report.skipped.push(SkippedFile {
                    file: file_name,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let existing_raw = store
            .read(M::ASSET_TYPE, &id)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok());

        // 真源读不出来的时候不能拿它当「没有」直接覆盖，所以 matches_existing 的
        // Err 与 canonical_text 的 Err 都要点名跳过，不静默替换。
        let (canonical, is_new) = match &existing_raw {
            None => match mapping.canonical_text(agent, &id, &text, None) {
                Ok(canonical) => (canonical, true),
                Err(error) => {
                    report.skipped.push(SkippedFile {
                        file: file_name,
                        reason: error.to_string(),
                    });
                    continue;
                }
            },
            Some(raw) => match mapping.matches_existing(agent, &id, &text, raw) {
                Ok(true) => {
                    report.unchanged += 1;
                    continue;
                }
                Ok(false) => match mapping.canonical_text(agent, &id, &text, Some(raw)) {
                    Ok(canonical) => (canonical, false),
                    Err(error) => {
                        report.skipped.push(SkippedFile {
                            file: file_name,
                            reason: error.to_string(),
                        });
                        continue;
                    }
                },
                Err(error) => {
                    report.skipped.push(SkippedFile {
                        file: file_name,
                        reason: error.to_string(),
                    });
                    continue;
                }
            },
        };

        AssetService::write(store, index, M::ASSET_TYPE, &id, &canonical)?;
        if is_new {
            report.imported += 1;
        } else {
            report.updated += 1;
        }
    }
    Ok(report)
}

fn read_if_utf8(path: &Path, label: &str) -> Result<String, AppError> {
    let bytes = std::fs::read(path)?;
    String::from_utf8(bytes).map_err(|_| AppError::InvalidInput(format!("{label}文件不是 UTF-8")))
}

/// 扫 `<rules_dir>/*<ext>`：新文件建条目，已有文件只在正文不同时更新。
pub fn rules(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ScanReport, AppError> {
    scan_assets(&RuleAsset, store, index, agent, home, base_override)
}

/// 扫 `<agents_dir>/*<ext>`（claude/qoder 是 `.md`，Codex 是 `.toml`）。
pub fn subagents(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ScanReport, AppError> {
    scan_assets(&SubagentAsset, store, index, agent, home, base_override)
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

#[cfg(test)]
mod subagent_tests {
    use super::*;
    use crate::adapters::subagents::deploy as deploy_subagent;
    use crate::adapters::subagents::target_path as subagent_target;
    use crate::domain::agent::builtin_agents;
    use crate::domain::subagent::{render_canonical, Subagent};
    use serde_json::{Map, Value as JsonValue};
    use std::path::PathBuf;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        index: Index,
        home: PathBuf,
        agent: AgentDescriptor,
    }

    fn fixture(id: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        Fixture {
            _dir: dir,
            store,
            index,
            home,
            agent: builtin_agents().into_iter().find(|a| a.id == id).unwrap(),
        }
    }

    fn spec() -> Subagent {
        Subagent {
            name: "reviewer".into(),
            description: "代码审查".into(),
            instructions: "先看 diff".into(),
            model: None,
            tools: vec!["Read".into()],
            tool_extras: JsonValue::Object(Map::new()),
            targets: Vec::new(),
        }
    }

    impl Fixture {
        fn agents_dir(&self) -> PathBuf {
            subagent_target(&self.agent, "x", &self.home, None)
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf()
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::create_dir_all(self.agents_dir()).unwrap();
            std::fs::write(self.agents_dir().join(name), text).unwrap();
        }

        fn seed_canonical(&self, targets: &[&str]) {
            let mut sub = spec();
            sub.targets = targets.iter().map(|t| t.to_string()).collect();
            self.store
                .write(
                    AssetType::Subagent,
                    "reviewer",
                    render_canonical(&sub).unwrap().as_bytes(),
                )
                .unwrap();
        }

        fn canonical(&self) -> Subagent {
            let text = String::from_utf8(self.store.read(AssetType::Subagent, "reviewer").unwrap())
                .unwrap();
            parse_subagent(&text).unwrap()
        }

        fn scan(&mut self) -> Result<ScanReport, AppError> {
            subagents(&self.store, &mut self.index, &self.agent, &self.home, None)
        }
    }

    #[test]
    fn scanning_a_codex_toml_agent_becomes_a_canonical_subagent() {
        let mut fx = fixture("codex");
        fx.write(
            "reviewer.toml",
            "name = \"reviewer\"\ndescription = \"代码审查\"\ndeveloper_instructions = \"先看 diff\"\nmodel_reasoning_effort = \"high\"\n",
        );

        let report = fx.scan().unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        let sub = fx.canonical();
        assert_eq!(sub.instructions, "先看 diff");
        assert_eq!(sub.tools, Vec::<String>::new(), "Codex 没有 tools 概念");
        assert_eq!(
            sub.tool_extras["codex"]["model_reasoning_effort"], "high",
            "私有字段要留在 tool_extras 里"
        );
    }

    #[test]
    fn scanning_is_idempotent_right_after_a_deploy() {
        let mut fx = fixture("qoder-cn");
        fx.seed_canonical(&[]);
        deploy_subagent(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            "reviewer",
            &fx.home,
            None,
        )
        .unwrap();

        let report = fx.scan().unwrap();
        assert_eq!(
            (report.imported, report.updated, report.unchanged),
            (0, 0, 1),
            "{report:?}"
        );
    }

    #[test]
    fn a_source_only_field_is_not_losing_its_values_on_rescan() {
        let mut fx = fixture("claude");
        // 真源里有 targets，工具文件里没有这个概念
        fx.seed_canonical(&["claude", "codex"]);
        deploy_subagent(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            "reviewer",
            &fx.home,
            None,
        )
        .unwrap();
        // 用户在工具侧改了提示词
        fx.write(
            "reviewer.md",
            "---\nname: reviewer\ndescription: 代码审查\ntools:\n- Read\n---\n\n改过的指令\n",
        );

        let report = fx.scan().unwrap();
        assert_eq!(report.updated, 1, "{report:?}");
        let sub = fx.canonical();
        assert_eq!(sub.instructions, "改过的指令");
        assert_eq!(
            sub.targets,
            vec!["claude".to_string(), "codex".to_string()],
            "targets 只存在于真源侧，反推不能把它抹掉"
        );
    }

    #[test]
    fn files_that_cannot_become_a_valid_subagent_are_named_not_half_imported() {
        let mut fx = fixture("claude");
        fx.write("no-description.md", "---\nname: x\n---\n\n正文\n");
        fx.write("no-body.md", "---\nname: y\ndescription: d\n---\n\n");
        fx.write("notes.txt", "不是子代理文件");
        fx.write(
            "reviewer.md",
            "---\nname: reviewer\ndescription: d\n---\n\n正文\n",
        );

        let report = fx.scan().unwrap();
        assert_eq!(report.imported, 1, "{report:?}");
        let skipped: Vec<&str> = report
            .skipped
            .iter()
            .map(|skip| skip.file.as_str())
            .collect();
        for expected in ["no-description.md", "no-body.md", "notes.txt"] {
            assert!(skipped.contains(&expected), "{skipped:?}");
        }
        assert!(report
            .skipped
            .iter()
            .any(|s| s.file == "no-description.md" && s.reason.contains("description")));
    }

    #[test]
    fn a_broken_toml_agent_is_skipped_with_the_parser_error() {
        let mut fx = fixture("codex");
        fx.write("reviewer.toml", "name = \"x\" description = 断掉的行");
        let report = fx.scan().unwrap();
        assert_eq!(report.imported, 0, "{report:?}");
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0].reason.contains("TOML"),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn scanning_a_subagent_capable_agent_only_needs_the_agents_dir() {
        let mut fx = fixture("gemini");
        let error = fx.scan().unwrap_err();
        assert!(
            matches!(error, AppError::Unsupported(_)),
            "Gemini 注册表里没有 agents_dir：{error}"
        );
    }
}
