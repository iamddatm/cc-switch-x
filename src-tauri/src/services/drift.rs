//! 漂移检测：期望内容 vs 磁盘实际（spec 5.4）。
//!
//! 只查台账里有的东西——用户自己新建的文件不在台账里，那是我们漏管，不是漂移。
//! `rescan` **只报告不动盘**；改动要么被采纳进真源（AdoptDisk），要么被真源重刷
//! （RepushSource），两者都由用户点出来，绝不在写入路径上偷偷做。

use crate::adapters::{ledger, prompt, rules, subagents};
use crate::domain::agent::{builtin_agents, AgentDescriptor, AssetKind};
use crate::domain::prompt::{parse_prompt, render_prompt, Prompt};
use crate::domain::rule::parse_rule;
use crate::domain::subagent::{parse_subagent, render_canonical as render_subagent_canonical};
use crate::domain::{AssetState, AssetType, DriftKind};
use crate::error::AppError;
use crate::index::{sha256_hex, DriftRow, Index};
use crate::services::assets::AssetService;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 给 UI 的一条待解决漂移：除 id 与形态，还带上落点路径，用户才知道去哪儿看。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftRecord {
    pub id: i64,
    pub asset_type: AssetType,
    pub asset_id: String,
    pub agent_id: String,
    pub kind: DriftKind,
    pub detected_at: i64,
    pub target_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolveAction {
    /// 把磁盘上的改动收进真源（落盘文件本身不动）
    AdoptDisk,
    /// 用真源重刷磁盘（磁盘内容被覆盖，写入前照例备份）
    RepushSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolveOutcome {
    /// 真源已更新成磁盘的内容；落盘文件保持用户写的那份，一个字节没动
    Adopted { path: PathBuf },
    /// 磁盘已重刷成真源渲染的结果
    Repushed {
        path: PathBuf,
        written: usize,
        skipped: usize,
    },
}

fn descriptor(agent_id: &str) -> Result<AgentDescriptor, AppError> {
    builtin_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| AppError::NotFound(format!("注册表里没有 {agent_id}")))
}

fn kind_of(entry: &ledger::AppliedEntry) -> Result<AssetKind, AppError> {
    AssetKind::from_snake_name(&entry.asset_type).ok_or_else(|| {
        AppError::InvalidInput(format!("台账里的资产类型认不出来：{}", entry.asset_type))
    })
}

fn asset_type_of(kind: AssetKind) -> Result<AssetType, AppError> {
    AssetType::from_kind(kind).map_err(|_| AppError::InvalidInput("Session 不落真源".into()))
}

/// 真源里的原文。`Ok(None)` = 真源已经不在了，这时算不出期望，也就无从判断漂移。
fn read_canonical(
    store: &Store,
    kind: AssetKind,
    asset_id: &str,
) -> Result<Option<String>, AppError> {
    match store.read(asset_type_of(kind)?, asset_id) {
        Ok(bytes) => Ok(Some(String::from_utf8(bytes).map_err(|_| {
            AppError::InvalidInput(format!("真源里的 {asset_id} 不是 UTF-8"))
        })?)),
        Err(crate::store::StoreError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(None)
        }
        Err(other) => Err(other.into()),
    }
}

/// 磁盘这份内容与期望一致吗。
///
/// 比的是**语义**而不是字节：工具侧的 `trigger` / `glob` 那些是渲染派生物，
/// 拿字节比会把「用户没改内容」报成漂移。（字节级的归属判定是
/// `ledger::bytes_are_ours` 的活，两回事。）
fn disk_matches_expectation(
    store: &Store,
    agent: &AgentDescriptor,
    entry: &ledger::AppliedEntry,
) -> Result<Option<bool>, AppError> {
    let kind = kind_of(entry)?;
    let Some(canonical) = read_canonical(store, kind, &entry.asset_id)? else {
        return Ok(None);
    };
    let disk = match std::fs::read(&entry.target_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Some(false)),
        Err(error) => return Err(error.into()),
    };
    let Ok(disk_text) = String::from_utf8(disk) else {
        // 读不成 UTF-8 就绝不是我们写的那份内容
        return Ok(Some(false));
    };
    let matches = match kind {
        AssetKind::Rule => {
            let rule = parse_rule(&entry.asset_id, &canonical)?;
            rules::body_equals(&disk_text, &rule)?
        }
        AssetKind::Subagent => {
            let current = parse_subagent(&canonical)?;
            let file_name = format!("{}{}", entry.asset_id, subagents::extension_for(agent));
            let mut recovered = match subagents::reverse_subagent(agent, &file_name, &disk_text) {
                Ok(recovered) => recovered,
                // 解析不出合法子代理 = 内容确实不是我们落的那份
                Err(_) => return Ok(Some(false)),
            };
            // name 与 targets 在工具侧没有对应字段，比对时以真源为准
            recovered.name = current.name.clone();
            recovered.targets = current.targets.clone();
            recovered == current
        }
        AssetKind::Prompt => {
            let parsed = parse_prompt(&entry.asset_id, &canonical)?;
            // 提示词是整文件接管，没有派生 frontmatter，按字节比
            disk_text == parsed.body
        }
        other => {
            return Err(AppError::InvalidInput(format!(
                "{} 不该出现在落盘台账里，无法判断漂移",
                other.snake_name()
            )))
        }
    };
    Ok(Some(matches))
}

fn ledger_key(kind: AssetKind, asset_id: &str) -> String {
    format!("{}/{}", kind.snake_name(), asset_id)
}

/// 重新检查所有 agent 的落盘结果，返回当前待解决的全部漂移。**不动磁盘**。
/// 落点路径都记在台账里（写盘那天展开好的绝对路径），所以这里不需要 `home`。
pub fn rescan(store: &Store, index: &mut Index) -> Result<Vec<DriftRecord>, AppError> {
    let mut live: Vec<(AssetKind, String, String)> = Vec::new();
    for agent in builtin_agents() {
        let table = ledger::load(store, &agent.id)?;
        for entry in table.entries.values() {
            let kind = kind_of(entry)?;
            live.push((kind, entry.asset_id.clone(), agent.id.clone()));
            let asset_type = asset_type_of(kind)?;
            match disk_matches_expectation(store, &agent, entry)? {
                None => {}
                Some(true) => index.clear_drift(asset_type, &entry.asset_id, &agent.id)?,
                Some(false) => {
                    let drift_kind = if entry.target_path.exists() {
                        DriftKind::Modified
                    } else {
                        DriftKind::Missing
                    };
                    index.record_drift(asset_type, &entry.asset_id, &agent.id, drift_kind)?;
                }
            }
        }
    }

    // 台账里已经没有这条落盘记录了（被 remove 掉、或真源被删）：这条漂移不再有意义，
    // 留着会让 UI 一直挂着一条无处可处置的记录。
    for row in index.open_drifts()? {
        let key = ledger_key(AssetKind::from_asset_type(row.asset_type), &row.asset_id);
        if !live.iter().any(|(kind, asset_id, agent_id)| {
            ledger_key(*kind, asset_id) == key && agent_id == &row.agent_id
        }) {
            index.clear_drift(row.asset_type, &row.asset_id, &row.agent_id)?;
        }
    }

    open_records(store, index)
}

/// 当前待解决的漂移，带上各自的落点路径。`rescan` 与只读列表都用它，
/// 免得两处各写一遍「记录 → 台账 → 路径」的对应关系。
pub fn open_records(store: &Store, index: &Index) -> Result<Vec<DriftRecord>, AppError> {
    let mut targets = std::collections::HashMap::new();
    for agent in builtin_agents() {
        for (key, entry) in ledger::load(store, &agent.id)?.entries {
            targets.insert((agent.id.clone(), key), entry.target_path);
        }
    }
    let mut out = Vec::new();
    for row in index.open_drifts()? {
        let key = ledger_key(AssetKind::from_asset_type(row.asset_type), &row.asset_id);
        let target_path = targets
            .get(&(row.agent_id.clone(), key))
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidInput(format!("漂移记录 {} 找不到对应的落点路径", row.id))
            })?;
        out.push(DriftRecord {
            id: row.id,
            asset_type: row.asset_type,
            asset_id: row.asset_id,
            agent_id: row.agent_id,
            kind: row.kind,
            detected_at: row.detected_at,
            target_path,
        });
    }
    Ok(out)
}

/// 写命令前的闸：有未解决的漂移就先让用户处置，不许静默覆盖（spec 5.4）。
pub fn ensure_no_open_drift(
    index: &Index,
    kind: AssetKind,
    asset_id: &str,
    agent_id: &str,
) -> Result<(), AppError> {
    if index.has_open_drift(asset_type_of(kind)?, asset_id, agent_id)? {
        return Err(AppError::DriftUnresolved(format!(
            "{agent_id} 的 {}/{} 有未解决的漂移：先选「采纳磁盘」或「重刷真源」",
            kind.snake_name(),
            asset_id
        )));
    }
    Ok(())
}

/// 提示词按 agent + 类型判：一个 agent 只有一个提示词文件，
/// 换激活哪一条同样会覆盖那份被手改过的文件。
pub fn ensure_no_open_drift_for_agent(
    index: &Index,
    kind: AssetKind,
    agent_id: &str,
) -> Result<(), AppError> {
    if index.has_open_drift_for_agent(asset_type_of(kind)?, agent_id)? {
        return Err(AppError::DriftUnresolved(format!(
            "{agent_id} 的提示词文件有未解决的漂移：先选「采纳磁盘」或「重刷真源」"
        )));
    }
    Ok(())
}

fn ledger_entry(store: &Store, row: &DriftRow) -> Result<ledger::AppliedEntry, AppError> {
    ledger::load(store, &row.agent_id)?
        .get(AssetKind::from_asset_type(row.asset_type), &row.asset_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "台账里没有 {}/{}/{} 的落盘记录",
                row.agent_id,
                row.asset_type.dir_name(),
                row.asset_id
            ))
        })
}

/// 用户在两种处置里点了一个。`AdoptDisk` 不动落盘文件；`RepushSource` 走写入流水线。
pub fn resolve(
    store: &Store,
    index: &mut Index,
    home: &Path,
    base_override: Option<&Path>,
    drift_id: i64,
    action: ResolveAction,
) -> Result<ResolveOutcome, AppError> {
    let row = index.open_drift_by_id(drift_id)?.ok_or_else(|| {
        AppError::NotFound(format!(
            "漂移记录 {drift_id} 不存在或已经解决，未做任何改动"
        ))
    })?;
    let agent = descriptor(&row.agent_id)?;
    let entry = ledger_entry(store, &row)?;
    let kind = AssetKind::from_asset_type(row.asset_type);

    match action {
        ResolveAction::AdoptDisk => {
            let disk = std::fs::read(&entry.target_path).map_err(|_| {
                AppError::InvalidInput(format!(
                    "{} 已经不在了，没有可采纳的内容；要恢复请选「重刷真源」",
                    entry.target_path.display()
                ))
            })?;
            let disk_text = String::from_utf8(disk.clone())
                .map_err(|_| AppError::InvalidInput("落盘文件不是 UTF-8，无法采纳".into()))?;
            let canonical = read_canonical(store, kind, &row.asset_id)?.ok_or_else(|| {
                AppError::NotFound(format!("真源里的 {} 已不存在，无法采纳", row.asset_id))
            })?;

            let updated = match kind {
                AssetKind::Rule => {
                    let existing = parse_rule(&row.asset_id, &canonical)?;
                    rules::merge_body_into_team(&disk_text, Some(&existing))?
                }
                AssetKind::Subagent => {
                    let existing = parse_subagent(&canonical)?;
                    let file_name = format!("{}{}", row.asset_id, subagents::extension_for(&agent));
                    let mut recovered =
                        subagents::reverse_subagent(&agent, &file_name, &disk_text)?;
                    // 这两个字段工具侧没有，采纳时保住真源自己那份
                    recovered.name = existing.name;
                    recovered.targets = existing.targets;
                    render_subagent_canonical(&recovered)?
                }
                AssetKind::Prompt => {
                    let existing = parse_prompt(&row.asset_id, &canonical)?;
                    render_prompt(&Prompt {
                        id: row.asset_id.clone(),
                        meta: existing.meta,
                        body: disk_text,
                    })
                }
                other => {
                    return Err(AppError::Unsupported(format!(
                        "{} 的采纳磁盘没有可靠的落点格式，未做任何改动",
                        other.snake_name()
                    )))
                }
            };

            AssetService::write(store, index, row.asset_type, &row.asset_id, &updated)?;
            // 盘上这份现在是用户的内容：把它记成台账里的期望，别下次又报同一处。
            // 落盘文件一个字节都不动 —— 这正是「采纳磁盘」的承诺。
            ledger::record(
                store,
                &row.agent_id,
                kind,
                &row.asset_id,
                &entry.target_path,
                &sha256_hex(&disk),
                entry.rendered_by,
            )?;
            index.resolve_drift(drift_id, "adopted")?;
            set_state(index, &row, AssetState::Written, None)?;
            Ok(ResolveOutcome::Adopted {
                path: entry.target_path,
            })
        }
        ResolveAction::RepushSource => {
            let (written, skipped) = match kind {
                AssetKind::Rule => {
                    let report =
                        rules::repush(store, index, &agent, &row.asset_id, home, base_override)?;
                    (report.written, report.skipped)
                }
                AssetKind::Subagent => {
                    let report = subagents::repush(
                        store,
                        index,
                        &agent,
                        &row.asset_id,
                        home,
                        base_override,
                    )?;
                    (report.written, report.skipped)
                }
                AssetKind::Prompt => {
                    // Takeover::Confirm 是用户已经点过「重刷」，覆盖外来内容不再追问
                    match prompt::activate(
                        store,
                        index,
                        &agent,
                        &row.asset_id,
                        &prompt::Takeover::Confirm,
                        home,
                        base_override,
                    )? {
                        prompt::ActivateOutcome::Activated { .. } => (1usize, 0usize),
                        prompt::ActivateOutcome::AlreadyActive { .. } => (0, 1),
                        prompt::ActivateOutcome::NeedsConfirmation { path } => {
                            return Err(AppError::InvalidInput(format!(
                                "{} 仍需要用户确认才能覆盖，未做任何改动",
                                path.display()
                            )))
                        }
                    }
                }
                other => {
                    return Err(AppError::Unsupported(format!(
                        "{} 的重刷真源没有可靠的落点格式，未做任何改动",
                        other.snake_name()
                    )))
                }
            };
            index.resolve_drift(drift_id, "repushed")?;
            set_state(index, &row, AssetState::Written, None)?;
            Ok(ResolveOutcome::Repushed {
                path: entry.target_path,
                written,
                skipped,
            })
        }
    }
}

fn set_state(
    index: &mut Index,
    row: &DriftRow,
    state: AssetState,
    last_error: Option<String>,
) -> Result<(), AppError> {
    index.set_agent_state(&crate::index::AgentStateRow {
        agent_id: row.agent_id.clone(),
        asset_type: row.asset_type,
        asset_id: row.asset_id.clone(),
        state,
        last_error,
        applied_at: chrono::Utc::now().timestamp_millis(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::rules::{deploy as deploy_rule, render_rule, target_path as rule_target};
    use crate::adapters::subagents::{deploy as deploy_subagent, target_path as subagent_target};
    use crate::domain::agent::builtin_agents;
    use crate::domain::prompt::{render_prompt, Prompt, PromptMetadata};
    use crate::domain::rule::{render_canonical as render_rule_canonical, Rule, RuleFrontmatter};
    use crate::domain::subagent::Subagent;
    use crate::index::Index;
    use serde_json::{Map, Value as JsonValue};
    use std::path::PathBuf;

    struct Fixture {
        /// 只在 Drop 时保住临时目录。
        _dir: tempfile::TempDir,
        store: Store,
        index: Index,
        home: PathBuf,
    }

    fn fixture() -> Fixture {
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
        }
    }

    impl Fixture {
        fn agent(&self, id: &str) -> AgentDescriptor {
            builtin_agents().into_iter().find(|a| a.id == id).unwrap()
        }

        fn seed_rule(&self) {
            let rule = Rule {
                id: "style".into(),
                meta: RuleFrontmatter {
                    name: "style".into(),
                    description: "写法".into(),
                    paths: vec!["src/**".into()],
                    trigger: None,
                },
                body: "正文\n".into(),
            };
            self.store
                .write(
                    AssetType::Rule,
                    "style",
                    render_rule_canonical(&rule).unwrap().as_bytes(),
                )
                .unwrap();
        }

        fn seed_subagent(&self) {
            let sub = Subagent {
                name: "reviewer".into(),
                description: "代码审查".into(),
                instructions: "先看 diff".into(),
                model: None,
                tools: vec!["Read".into()],
                tool_extras: JsonValue::Object(Map::new()),
                targets: vec!["claude".into()],
            };
            self.store
                .write(
                    AssetType::Subagent,
                    "reviewer",
                    crate::domain::subagent::render_canonical(&sub)
                        .unwrap()
                        .as_bytes(),
                )
                .unwrap();
        }

        fn seed_prompt(&self, id: &str, body: &str) {
            let prompt = Prompt {
                id: id.into(),
                meta: PromptMetadata {
                    title: Some(id.into()),
                    description: None,
                },
                body: body.into(),
            };
            self.store
                .write(AssetType::Prompt, id, render_prompt(&prompt).as_bytes())
                .unwrap();
        }

        fn rule_target(&self) -> PathBuf {
            rule_target(&self.agent("qoder-cn"), "style", &self.home, None).unwrap()
        }

        /// 落盘一条规则。单独包一层是因为 `&mut fx.index` 与 `fx.agent()` 同语句会撞借用。
        fn deploy_rule(&mut self) {
            let agent = self.agent("qoder-cn");
            deploy_rule(
                &self.store,
                &mut self.index,
                &agent,
                "style",
                &self.home,
                None,
            )
            .unwrap();
        }

        fn remove_rule(&mut self) {
            let agent = self.agent("qoder-cn");
            crate::adapters::rules::remove(
                &self.store,
                &mut self.index,
                &agent,
                "style",
                &self.home,
                None,
            )
            .unwrap();
        }

        fn canonical_rule(&self) -> Rule {
            let text =
                String::from_utf8(self.store.read(AssetType::Rule, "style").unwrap()).unwrap();
            crate::domain::rule::parse_rule("style", &text).unwrap()
        }

        fn resolve(
            &mut self,
            drift_id: i64,
            action: ResolveAction,
        ) -> Result<ResolveOutcome, AppError> {
            super::resolve(
                &self.store,
                &mut self.index,
                &self.home,
                None,
                drift_id,
                action,
            )
        }

        fn rescan(&mut self) -> Vec<DriftRecord> {
            super::rescan(&self.store, &mut self.index).unwrap()
        }
    }

    #[test]
    fn a_hand_edit_becomes_drift_and_blocks_silent_reapply() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        let target = fx.rule_target();
        std::fs::write(&target, "---\ntrigger: always_on\n---\n\n用户手改\n").unwrap();

        let drifts = fx.rescan();
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        assert_eq!(drifts[0].kind, DriftKind::Modified);
        assert_eq!(drifts[0].target_path, target);

        let blocked = ensure_no_open_drift(&fx.index, AssetKind::Rule, "style", "qoder-cn");
        assert!(
            matches!(blocked, Err(AppError::DriftUnresolved(_))),
            "有未解决漂移时不能直接覆盖：{blocked:?}"
        );
        // 命令层就是靠这个闸拒绝的；取消落盘方向不拦
        assert!(ensure_no_open_drift(&fx.index, AssetKind::Rule, "other", "qoder-cn").is_ok());
    }

    #[test]
    fn adopting_the_disk_pulls_the_edit_back_into_the_source_instead_of_losing_it() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        let target = fx.rule_target();
        let edited = "---\ntrigger: always_on\n---\n\n用户手改\n";
        std::fs::write(&target, edited).unwrap();
        let drift = &fx.rescan()[0];
        let drift_id = drift.id;

        let outcome = fx.resolve(drift_id, ResolveAction::AdoptDisk).unwrap();
        assert!(
            matches!(outcome, ResolveOutcome::Adopted { .. }),
            "{outcome:?}"
        );
        let text = String::from_utf8(fx.store.read(AssetType::Rule, "style").unwrap()).unwrap();
        assert!(
            text.contains("用户手改"),
            "采纳磁盘要真的把改动收进真源：{text}"
        );
        assert!(
            text.contains("src/**"),
            "真源自己的 paths 不能被工具侧派生的 frontmatter 换掉：{text}"
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            edited,
            "采纳磁盘不许动落盘文件"
        );
        assert!(fx.rescan().is_empty(), "解决后不该还挂着漂移");
    }

    #[test]
    fn repushing_the_source_restores_the_render() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        let target = fx.rule_target();
        let expected = render_rule(&fx.agent("qoder-cn"), &fx.canonical_rule()).unwrap();
        std::fs::write(&target, "---\ntrigger: always_on\n---\n\n用户手改\n").unwrap();
        let drift_id = fx.rescan()[0].id;

        let outcome = fx.resolve(drift_id, ResolveAction::RepushSource).unwrap();
        match &outcome {
            ResolveOutcome::Repushed { written, .. } => assert_eq!(*written, 1),
            other => panic!("应该是重刷：{other:?}"),
        }
        assert_eq!(std::fs::read_to_string(&target).unwrap(), expected);
        assert!(fx.rescan().is_empty());
        // 重刷前照样备份了被覆盖的内容
        let backups = std::fs::read_dir(fx.store.root().join("backups"))
            .unwrap()
            .count();
        assert!(backups > 0, "写入流水线必须留备份");
    }

    #[test]
    fn a_deleted_file_is_reported_as_missing_not_silently_recreated() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        let target = fx.rule_target();
        std::fs::remove_file(&target).unwrap();

        let drifts = fx.rescan();
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        assert_eq!(drifts[0].kind, DriftKind::Missing);
        assert!(!target.exists(), "rescan 只报告，不动盘");

        // 文件都不在，采纳磁盘没有意义，要说清楚该选哪条
        let drift_id = drifts[0].id;
        let error = fx.resolve(drift_id, ResolveAction::AdoptDisk).unwrap_err();
        assert!(error.to_string().contains("重刷真源"), "{error}");
        assert!(!target.exists(), "失败的一条路径也不能把文件变回来");

        fx.resolve(drift_id, ResolveAction::RepushSource).unwrap();
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            render_rule(&fx.agent("qoder-cn"), &fx.canonical_rule()).unwrap()
        );
    }

    #[test]
    fn an_untracked_file_is_never_drift() {
        let mut fx = fixture();
        fx.seed_rule();
        // 用户自己新建的规则文件不在台账里 → 那是我们漏管，不是漂移
        let dir = fx.rule_target().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mine.md"), "用户自己的规则").unwrap();

        assert!(fx.rescan().is_empty());
    }

    #[test]
    fn a_derived_frontmatter_change_is_not_drift() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        let target = fx.rule_target();
        let written = std::fs::read_to_string(&target).unwrap();
        // 正文一模一样，只有派生的 trigger 行被用户改成 manual
        let tweaked = written.replace("trigger: glob", "trigger: manual");
        assert_ne!(tweaked, written);
        std::fs::write(&target, &tweaked).unwrap();

        assert!(
            fx.rescan().is_empty(),
            "工具侧的派生字段不该被报成漂移，否则每次同步都在自我否定"
        );
    }

    #[test]
    fn rescan_clears_a_drift_the_user_undid_themselves() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        let target = fx.rule_target();
        let written = std::fs::read_to_string(&target).unwrap();
        std::fs::write(&target, "改掉了").unwrap();
        assert_eq!(fx.rescan().len(), 1);
        std::fs::write(&target, &written).unwrap();
        assert!(fx.rescan().is_empty(), "用户自己改回去之后记录必须消失");
    }

    #[test]
    fn removing_the_asset_leaves_no_orphan_drift_behind() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        // 文件被删掉 → 记一条 Missing
        std::fs::remove_file(fx.rule_target()).unwrap();
        let drifts = fx.rescan();
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        assert_eq!(drifts[0].kind, DriftKind::Missing);

        // 用户接着取消落盘：Absent（本来就没有），台账忘掉这条记录
        fx.remove_rule();
        assert!(
            fx.rescan().is_empty(),
            "台账已经不认这条了，漂移记录不能挂着让用户无处处置"
        );
    }

    #[test]
    fn a_hand_edited_file_must_be_resolved_before_it_can_be_removed() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        std::fs::write(fx.rule_target(), "改掉了").unwrap();
        assert_eq!(fx.rescan().len(), 1);

        // remove 的归属检查同样挡着：不是「我们落的、没被改过」的内容就不许删
        let agent = fx.agent("qoder-cn");
        assert_eq!(
            crate::adapters::rules::remove(
                &fx.store,
                &mut fx.index,
                &agent,
                "style",
                &fx.home,
                None
            )
            .unwrap(),
            crate::adapters::Removed::Foreign
        );
        // 先采纳再取消才走得通
        let drift_id = fx.rescan()[0].id;
        fx.resolve(drift_id, ResolveAction::AdoptDisk).unwrap();
        fx.remove_rule();
        assert!(!fx.rule_target().exists());
    }

    #[test]
    fn subagent_drift_round_trips_both_ways() {
        let mut fx = fixture();
        fx.seed_subagent();
        let agent = fx.agent("claude");
        deploy_subagent(&fx.store, &mut fx.index, &agent, "reviewer", &fx.home, None).unwrap();
        let target = subagent_target(&agent, "reviewer", &fx.home, None).unwrap();

        // 用户改了提示词正文
        std::fs::write(
            &target,
            "---\nname: reviewer\ndescription: 代码审查\ntools:\n- Read\n---\n\n改过的指令\n",
        )
        .unwrap();
        let drifts = fx.rescan();
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        assert_eq!(drifts[0].asset_type, AssetType::Subagent);
        let drift_id = drifts[0].id;
        fx.resolve(drift_id, ResolveAction::AdoptDisk).unwrap();
        let text =
            String::from_utf8(fx.store.read(AssetType::Subagent, "reviewer").unwrap()).unwrap();
        assert!(text.contains("改过的指令"), "{text}");
        assert!(
            text.contains("targets:\n- claude") || text.contains("targets:\n  - claude"),
            "真源侧的 targets 不能被采纳掉：{text}"
        );
        assert!(fx.rescan().is_empty());
    }

    #[test]
    fn prompt_drift_is_judged_per_agent_and_switching_is_blocked() {
        let mut fx = fixture();
        fx.seed_prompt("a", "A 的正文");
        let agent = fx.agent("claude");
        crate::adapters::prompt::activate(
            &fx.store,
            &mut fx.index,
            &agent,
            "a",
            &crate::adapters::prompt::Takeover::Confirm,
            &fx.home,
            None,
        )
        .unwrap();
        let target = fx.home.join(".claude/CLAUDE.md");
        std::fs::write(&target, "用户手改的提示词").unwrap();
        let drifts = fx.rescan();
        assert_eq!(drifts.len(), 1, "{drifts:?}");
        assert_eq!(drifts[0].asset_type, AssetType::Prompt);

        assert!(
            matches!(
                ensure_no_open_drift_for_agent(&fx.index, AssetKind::Prompt, "claude"),
                Err(AppError::DriftUnresolved(_))
            ),
            "换激活另一条提示词同样会覆盖那份被手改的文件"
        );

        let drift_id = drifts[0].id;
        fx.seed_prompt("b", "B 的正文");
        fx.resolve(drift_id, ResolveAction::AdoptDisk).unwrap();
        let text = String::from_utf8(fx.store.read(AssetType::Prompt, "a").unwrap()).unwrap();
        assert!(text.contains("用户手改的提示词"), "{text}");
        assert!(fx.rescan().is_empty());
    }

    #[test]
    fn a_deleted_source_asset_is_skipped_rather_than_crashing_the_scan() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        // 真源被删掉：没有期望可比，只能跳过（清理是另一件事，这里不假装成没漂移也没法比对）
        fx.store.delete(AssetType::Rule, "style").unwrap();
        assert!(fx.rescan().is_empty());
    }

    #[test]
    fn resolving_the_same_drift_twice_is_refused() {
        let mut fx = fixture();
        fx.seed_rule();
        fx.deploy_rule();
        std::fs::write(fx.rule_target(), "改掉了").unwrap();
        let drift_id = fx.rescan()[0].id;
        fx.resolve(drift_id, ResolveAction::AdoptDisk).unwrap();

        let error = fx
            .resolve(drift_id, ResolveAction::RepushSource)
            .unwrap_err();
        assert!(
            matches!(error, AppError::NotFound(_)),
            "已解决的记录不能再处置：{error}"
        );
    }
}
