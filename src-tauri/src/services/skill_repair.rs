//! 一键处理落点核对报出的问题：把「勾选 == 磁盘事实」重新对齐。
//!
//! 与 `skill_verify` 的分工：那边只读地报出差异，这边是**唯一**会因核对结果动盘的入口，
//! 只能由用户在面板上显式触发（点之前还要过一道确认框）。启动、rescan、别的命令都不得调它。
//!
//! 三种核对状态各自的处置，安全边界一处都不放宽：
//! - `missing`：落点空着，重新部署不会覆盖任何人的东西 → 走 `skill_source::set_enabled`，
//!   能力闸、检测门禁、矩阵写入都在它里面，不另立第二套口径。
//! - `foreign`：落点被别的工具接走 → 交回 `skill_takeover::take_over_one`。它对链接**只解链**
//!   （Windows 上对 junction 递归 `remove_dir_all` 会一路删进对侧真身），对真目录必须先证明
//!   与真源同指纹并整份留底才允许替换，对不上就原样保留并报告 —— 所以「抢回」不等于无脑覆盖。
//! - `sourceGone`：真源已经不在了 → 只退账清勾选，盘上一个字节不动。
//!
//! 清单一律由本模块现算，不接收前端传来的列表：落点路径要按注册表 + `baseOverride` 重新推导，
//! 用户点确认之前可能又在对侧工具里动过一轮（与 `install_discovered_skill` 同一口径）。
//! 逐条独立成败，一项失败不影响其它项（与 `skill_takeover::import` 同一语义）。

use crate::adapters::skills;
use crate::domain::agent::{builtin_agents, AssetKind};
use crate::domain::AssetType;
use crate::error::AppError;
use crate::services::skill_local_scan::{display_path, FoundLocation};
use crate::services::skill_source;
use crate::services::skill_takeover;
use crate::services::skill_verify::{self, DeploymentStatus, SkillDeploymentIssue};
use crate::store::Store;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// 一条处置的结局。`status` 是给 UI 分档用的机器码，`detail` 是给人看的那一句。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepairItem {
    pub agent_id: String,
    pub skill_id: String,
    /// redeployed / reclaimed / already / detached / refused
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepairReport {
    pub items: Vec<SkillRepairItem>,
}

impl SkillRepairReport {
    fn push(&mut self, issue: &SkillDeploymentIssue, status: &str, detail: String) {
        self.items.push(SkillRepairItem {
            agent_id: issue.agent_id.clone(),
            skill_id: issue.skill_id.clone(),
            status: status.to_string(),
            detail,
        });
    }
}

/// 核对一遍再处置。**会动盘**：调用方必须持有 Skill 写任务锁与 maintenance 读锁。
pub fn repair(store: &Store, home: &Path) -> Result<SkillRepairReport, AppError> {
    let mut report = SkillRepairReport::default();
    let mut grouped: BTreeMap<String, Vec<SkillDeploymentIssue>> = BTreeMap::new();
    for issue in skill_verify::verify(store, home)? {
        grouped.entry(issue.agent_id.clone()).or_default().push(issue);
    }
    for (agent_id, issues) in grouped {
        repair_agent(store, home, &agent_id, &issues, &mut report);
    }
    Ok(report)
}

fn repair_agent(
    store: &Store,
    home: &Path,
    agent_id: &str,
    issues: &[SkillDeploymentIssue],
    report: &mut SkillRepairReport,
) {
    redeploy_missing(store, home, agent_id, issues, report);
    for issue in issues
        .iter()
        .filter(|issue| issue.status == DeploymentStatus::Foreign)
    {
        reclaim(store, home, issue, report);
    }
    for issue in issues
        .iter()
        .filter(|issue| issue.status == DeploymentStatus::SourceGone)
    {
        detach(store, home, issue, report);
    }
}

/// 落点空着的：一次批量重刷，矩阵写入合并成一遍。
fn redeploy_missing(
    store: &Store,
    home: &Path,
    agent_id: &str,
    issues: &[SkillDeploymentIssue],
    report: &mut SkillRepairReport,
) {
    let wanted: Vec<&SkillDeploymentIssue> = issues
        .iter()
        .filter(|issue| issue.status == DeploymentStatus::Missing)
        .collect();
    if wanted.is_empty() {
        return;
    }
    let ids = wanted
        .iter()
        .map(|issue| issue.skill_id.clone())
        .collect::<Vec<_>>();
    let by_id = wanted
        .iter()
        .map(|issue| (issue.skill_id.as_str(), *issue))
        .collect::<BTreeMap<_, _>>();
    let batch = match skill_source::set_enabled(store, home, &ids, agent_id, true) {
        Ok(batch) => batch,
        Err(error) => {
            for issue in wanted {
                report.push(issue, "refused", format!("重新部署失败：{error}"));
            }
            return;
        }
    };
    for ok in batch.ok {
        let issue = match by_id.get(ok.id.as_str()) {
            Some(issue) => *issue,
            None => continue,
        };
        // 核对时落点还空着，走到这一步只可能是刚建好；`already` 说明中途盘被人动过，照实说。
        let status = if ok.label == "already" { "already" } else { "redeployed" };
        report.push(issue, status, format!("已重新部署到落点（{}）", ok.label));
    }
    for failed in batch.failed {
        if let Some(issue) = by_id.get(failed.id.as_str()) {
            report.push(issue, "refused", failed.error);
        }
    }
}

/// 落点被别的工具接走的：走接管那一条路，抢不回来的由接管原语自己拒绝并说明原因。
fn reclaim(
    store: &Store,
    home: &Path,
    issue: &SkillDeploymentIssue,
    report: &mut SkillRepairReport,
) {
    let descriptor = match builtin_agents()
        .into_iter()
        .find(|agent| agent.id == issue.agent_id && agent.switchable(AssetKind::Skill))
    {
        Some(descriptor) => descriptor,
        None => {
            report.push(
                issue,
                "refused",
                "注册表里这个位置已经不再允许写入，未做改动".to_string(),
            );
            return;
        }
    };
    let location = FoundLocation {
        source_id: Some(issue.agent_id.clone()),
        label: descriptor.display_name.clone(),
        source: issue.target_path.clone(),
        path: display_path(&issue.target_path),
        link_target: skills::link_target(&issue.target_path).map(|raw| display_path(&raw)),
        // 能出现在核对报告里，就说明这一处过了 `switchable` 闸。
        takeover_possible: true,
    };
    let outcome = skill_takeover::take_over_one(store, home, &issue.skill_id, &location);
    let status = match outcome.status.as_str() {
        "linked" => "reclaimed",
        other => other,
    };
    let mut detail = outcome.detail.unwrap_or_else(|| "已处理".to_string());
    if let Some(backup) = outcome.backup_path {
        detail.push_str(&format!("；原目录留底在 {backup}"));
    }
    report.push(issue, status, detail);
}

/// 勾选悬空的：退账。真源都不在了，没有任何方向值得动盘。
fn detach(
    store: &Store,
    home: &Path,
    issue: &SkillDeploymentIssue,
    report: &mut SkillRepairReport,
) {
    // 拼不出真源路径的 ID，`skills::remove` 会在 entry_path 就失败；只能直接清矩阵。
    // 不绕开这一步，这条勾选就既启不了也关不掉 —— v0.18.0 修过的死路会换个形式在这里复现。
    if store.entry_path(AssetType::Skill, &issue.skill_id).is_err() {
        let skill_id = issue.skill_id.clone();
        let cleared = crate::import::update_matrix(store, &issue.agent_id, |matrix| {
            matrix.disable(AssetType::Skill, &skill_id);
        });
        return match cleared {
            Ok(()) => report.push(
                issue,
                "detached",
                "真源与落点都无从对应，已清掉这条勾选".to_string(),
            ),
            Err(error) => report.push(issue, "refused", format!("清勾选失败：{error}")),
        };
    }
    let ids = vec![issue.skill_id.clone()];
    match skill_source::set_enabled(store, home, &ids, &issue.agent_id, false) {
        Ok(batch) => {
            for ok in batch.ok {
                report.push(
                    issue,
                    "detached",
                    format!("已取消勾选（{}），盘上内容未做改动", ok.label),
                );
            }
            for failed in batch.failed {
                report.push(issue, "refused", failed.error);
            }
        }
        Err(error) => report.push(issue, "refused", format!("取消勾选失败：{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::skills::Ownership;
    use crate::import::update_matrix;
    use crate::services::agents::read_matrix;
    use std::fs;
    use std::path::PathBuf;

    struct Fixture {
        /// 只在 Drop 时保住临时目录。
        _dir: tempfile::TempDir,
        store: Store,
        home: PathBuf,
    }

    /// 沙箱：只给 `claude` 一个配置根并 `baseOverride` 指过去，其余 agent 一律「没装」，
    /// 绝不碰用户真实的 `~/.claude`。与 `skill_verify` 的 fixture 同一套写法。
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();
        crate::services::agents::save_override(&store, "claude", Some(&home)).unwrap();
        Fixture {
            _dir: dir,
            store,
            home,
        }
    }

    const OURS: &str = "---\nname: 演示\ndescription: 说明\n---\n\n我们的正文\n";
    const THEIRS: &str = "---\nname: 演示\ndescription: 说明\n---\n\n对侧的正文\n";

    impl Fixture {
        fn agent(&self) -> crate::domain::agent::AgentDescriptor {
            builtin_agents()
                .into_iter()
                .find(|agent| agent.id == "claude")
                .unwrap()
        }

        fn source(&self, id: &str) -> PathBuf {
            self.store.entry_path(AssetType::Skill, id).unwrap()
        }

        fn target(&self, id: &str) -> PathBuf {
            let agent = self.agent();
            let home = self.home.clone();
            skills::skills_dir(&agent, &home, Some(&home))
                .unwrap()
                .join(id)
        }

        fn seed_source(&self, id: &str, body: &str) -> PathBuf {
            let source = self.source(id);
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("SKILL.md"), body).unwrap();
            source
        }

        fn enable(&self, id: &str) {
            update_matrix(&self.store, "claude", |matrix| {
                matrix.enable(AssetType::Skill, id);
            })
            .unwrap();
        }

        fn deploy(&self, id: &str) -> PathBuf {
            let agent = self.agent();
            let home = self.home.clone();
            skills::deploy(&self.store, &agent, id, &home, Some(&home)).unwrap();
            self.target(id)
        }

        fn enabled(&self, id: &str) -> bool {
            read_matrix(&self.store, "claude")
                .unwrap()
                .enabled_for(AssetType::Skill)
                .iter()
                .any(|value| value == id)
        }

        fn repair(&self) -> SkillRepairReport {
            super::repair(&self.store, &self.home).unwrap()
        }

        fn remaining(&self) -> usize {
            skill_verify::verify(&self.store, &self.home).unwrap().len()
        }

        fn ownership(&self, id: &str) -> Ownership {
            skills::ownership(&self.store, &self.target(id))
        }

        /// 透过落点读正文：落点是指向我们真源的链接时读到的就是 OURS，指向别处则不是。
        fn content_at(&self, id: &str) -> String {
            fs::read_to_string(self.target(id).join("SKILL.md")).unwrap_or_default()
        }
    }

    fn status_of(report: &SkillRepairReport, skill_id: &str) -> String {
        report
            .items
            .iter()
            .find(|item| item.skill_id == skill_id)
            .unwrap_or_else(|| panic!("报告里没有 {skill_id}：{:?}", report.items))
            .status
            .clone()
    }

    #[test]
    fn a_vanished_deployment_is_relinked_and_stays_enabled() {
        let fx = fixture();
        fx.seed_source("demo", OURS);
        let target = fx.deploy("demo");
        fx.enable("demo");
        skills::remove_link(&target).unwrap();
        assert_eq!(fx.remaining(), 1, "先确认核对真的报出来了");

        let report = fx.repair();
        assert_eq!(status_of(&report, "demo"), "redeployed", "{report:?}");
        assert_eq!(fx.ownership("demo"), Ownership::OursLink);
        assert_eq!(fx.content_at("demo"), OURS);
        assert!(fx.enabled("demo"), "重刷不该把用户的勾选顺手改掉");
        assert_eq!(fx.remaining(), 0, "处理完这条不该再出现在报告里");
    }

    #[test]
    fn a_hijacked_link_is_reclaimed_without_touching_the_rival_directory() {
        let fx = fixture();
        fx.seed_source("demo", OURS);
        let target = fx.deploy("demo");
        fx.enable("demo");

        // 复刻双工具并存：对侧把自己的 SSOT 盖到同一个落点。
        let rival = fx.home.join(".cc-switch/skills/demo");
        fs::create_dir_all(&rival).unwrap();
        fs::write(rival.join("SKILL.md"), THEIRS).unwrap();
        skills::remove_link(&target).unwrap();
        skills::link_dir(&rival, &target).unwrap();
        assert_eq!(fx.remaining(), 1);

        let report = fx.repair();
        assert_eq!(status_of(&report, "demo"), "reclaimed", "{report:?}");
        assert_eq!(fx.ownership("demo"), Ownership::OursLink);
        assert_eq!(fx.content_at("demo"), OURS);
        assert_eq!(fx.remaining(), 0);
        // 只解链：对侧真身与它的正文必须逐字不变。
        assert!(
            rival.is_dir(),
            "解链不许把对侧目录一起带走"
        );
        assert_eq!(
            fs::read_to_string(rival.join("SKILL.md")).unwrap(),
            THEIRS,
            "抢回落点不许动对侧的内容"
        );
    }

    #[test]
    fn a_foreign_directory_we_cannot_match_is_left_alone() {
        let fx = fixture();
        fx.seed_source("demo", OURS);
        let target = fx.target("demo");
        fx.enable("demo");
        // 一份我们从未部署过、内容又不同的真目录：无从证明它就是我们那一份。
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), THEIRS).unwrap();

        let report = fx.repair();
        assert_eq!(status_of(&report, "demo"), "refused", "{report:?}");
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            THEIRS,
            "内容对不上的外来目录必须原样保留"
        );
        assert!(fx.enabled("demo"), "抢不回来也不该偷偷把勾选清掉");
        assert_eq!(fx.remaining(), 1, "这条仍然挂在报告里，方向由用户定");
    }

    #[test]
    fn a_dangling_checkmark_is_detached_and_its_stale_link_cleaned() {
        let fx = fixture();
        let source = fx.seed_source("demo", OURS);
        let target = fx.deploy("demo");
        fx.enable("demo");
        fs::remove_dir_all(&source).unwrap();
        assert_eq!(fx.remaining(), 1);

        let report = fx.repair();
        assert_eq!(status_of(&report, "demo"), "detached", "{report:?}");
        assert!(!fx.enabled("demo"), "悬空勾选要真的从矩阵里清掉");
        assert!(
            target.symlink_metadata().is_err(),
            "指向已删真源的坏链是我们的残留，退账时要一起清掉"
        );
        assert_eq!(fx.remaining(), 0);
    }

    #[test]
    fn an_unmappable_checkmark_can_still_be_detached() {
        let fx = fixture();
        // 拼不出真源路径的 ID：`skills::remove` 会在 entry_path 就失败，只能直接清矩阵。
        fx.enable("bad/id");
        assert_eq!(fx.remaining(), 1);

        let report = fx.repair();
        assert_eq!(status_of(&report, "bad/id"), "detached", "{report:?}");
        assert!(!fx.enabled("bad/id"));
        assert_eq!(fx.remaining(), 0);
    }

    #[test]
    fn a_consistent_matrix_produces_no_writes() {
        let fx = fixture();
        fx.seed_source("demo", OURS);
        let target = fx.deploy("demo");
        fx.enable("demo");
        let before = fs::read_to_string(target.join("SKILL.md")).unwrap();

        let report = fx.repair();
        assert!(report.items.is_empty(), "{report:?}");
        assert_eq!(fs::read_to_string(target.join("SKILL.md")).unwrap(), before);
        assert!(fx.enabled("demo"));
    }

    #[test]
    fn an_uninstalled_agent_is_never_created_for_us() {
        let fx = fixture();
        fx.seed_source("demo", OURS);
        // codex 没装（临时 home 下没有它的根），矩阵里却有历史勾选。
        update_matrix(&fx.store, "codex", |matrix| {
            matrix.enable(AssetType::Skill, "demo");
        })
        .unwrap();

        let report = fx.repair();
        assert!(report.items.is_empty(), "{report:?}");
        assert!(
            !fx.home.join(".codex").exists(),
            "检测门禁不许因为一次批量处置而被绕过"
        );
    }
}
