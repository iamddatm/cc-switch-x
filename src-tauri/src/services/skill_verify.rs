//! 勾选核对：启用矩阵说「已启用」的 Skill，落点上还是不是我们那一份。
//!
//! 台账类资产（Prompt / Rule / Subagent）由 `services::drift` 逐条比字节；Skill 走链接部署，
//! 没有「期望字节」可比，可比的是**归属** —— 落点是否仍指向中央库真源。
//! 这里只报告，不动盘，也不产漂移记录：处置方向只有「在我们面板里重新勾选部署」和
//! 「去另一个管同一落点的工具里取消同名条目」两种，方向必须由用户定，工具猜一次就错一次。
//!
//! 与 `skill_local_scan` 的分工：那边问「盘上有哪些不是我们落的」（从未勾选项出发），
//! 这边问「我们勾了的为什么不在盘上/不归我们了」（从矩阵出发），两边共用 `skills::ownership`。

use crate::adapters::skills::{self, Ownership};
use crate::domain::agent::{builtin_agents, AssetKind};
use crate::domain::AssetType;
use crate::error::AppError;
use crate::services::agents::{ensure_agent_detected, load_state, read_matrix};
use crate::services::skill_local_scan::display_path;
use crate::store::Store;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 落点与勾选不符的形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeploymentStatus {
    /// 落点在，但已经不归我们：别的工具建的链接、或我们没有部署过的目录
    Foreign,
    /// 落点整个不见了
    Missing,
    /// 真源已经不在了，矩阵里那条勾选是悬空记录
    SourceGone,
}

/// 一条待核对结果。`detail` 写清落点当前的实际状态，用户据此判断是谁动的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDeploymentIssue {
    pub agent_id: String,
    pub skill_id: String,
    pub status: DeploymentStatus,
    pub target_path: PathBuf,
    pub detail: String,
}

/// 核对全部 agent 的 Skill 勾选。**只读**：一个字节都不写，也不建链接。
pub fn verify(store: &Store, home: &Path) -> Result<Vec<SkillDeploymentIssue>, AppError> {
    let mut issues = Vec::new();
    for agent in builtin_agents() {
        if !agent.switchable(AssetKind::Skill) {
            continue;
        }
        let enabled = read_matrix(store, &agent.id)?.enabled_for(AssetType::Skill);
        if enabled.is_empty() {
            continue;
        }
        match ensure_agent_detected(store, home, &agent.id) {
            Ok(()) => {}
            // 工具没装（配置根不存在）：矩阵里的勾选是历史残留，报出来只会淹没真问题
            Err(AppError::NotDetected(_)) => continue,
            Err(error) => return Err(error),
        }
        let base = load_state(store, &agent.id)?.base_override;
        let dir = skills::skills_dir(&agent, home, base.as_deref())?;
        for skill_id in enabled {
            if let Some(issue) = check_one(store, &agent.id, &skill_id, &dir) {
                issues.push(issue);
            }
        }
    }
    Ok(issues)
}

fn check_one(store: &Store, agent_id: &str, skill_id: &str, dir: &Path) -> Option<SkillDeploymentIssue> {
    let target = dir.join(skill_id);
    let source = match store.entry_path(AssetType::Skill, skill_id) {
        Ok(path) => path,
        // 认不出的 ID 拼不出真源路径，也就永远不可能有落点，直接按悬空勾选报出来
        Err(_) => {
            return Some(issue(
                agent_id,
                skill_id,
                DeploymentStatus::SourceGone,
                target,
                "这条勾选不是合法的真源 ID，真源与落点都无从对应".to_string(),
            ))
        }
    };
    if !source.join("SKILL.md").is_file() {
        return Some(issue(
            agent_id,
            skill_id,
            DeploymentStatus::SourceGone,
            target,
            format!("真源里的 {} 已经不在了，只剩勾选", source.display()),
        ));
    }
    match skills::ownership(store, &target) {
        Ownership::OursLink | Ownership::OursCopy => None,
        Ownership::Absent => Some(issue(
            agent_id,
            skill_id,
            DeploymentStatus::Missing,
            target.clone(),
            "落点上已经没有这个目录（被外部删掉，或从未部署成功）".to_string(),
        )),
        Ownership::Foreign => Some(issue(
            agent_id,
            skill_id,
            DeploymentStatus::Foreign,
            target.clone(),
            match skills::link_target(&target) {
                Some(link) => format!(
                    "落点现在是指向 {} 的链接，不再是我们的真源",
                    display_path(&link)
                ),
                None => "落点是一份我们没有部署过的目录".to_string(),
            },
        )),
    }
}

fn issue(
    agent_id: &str,
    skill_id: &str,
    status: DeploymentStatus,
    target_path: PathBuf,
    detail: String,
) -> SkillDeploymentIssue {
    SkillDeploymentIssue {
        agent_id: agent_id.to_string(),
        skill_id: skill_id.to_string(),
        status,
        target_path,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::import::update_matrix;
    use std::fs;

    struct Fixture {
        /// 只在 Drop 时保住临时目录。
        _dir: tempfile::TempDir,
        store: Store,
        home: PathBuf,
    }

    /// 沙箱：`baseOverride` 把 Claude 的配置根整个指到临时 home，
    /// 绝不碰用户真实的 `~/.claude`。其余 agent 没有根，会被检测门禁跳过。
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

    impl Fixture {
        fn agent(&self) -> crate::domain::agent::AgentDescriptor {
            builtin_agents().into_iter().find(|a| a.id == "claude").unwrap()
        }

        fn seed_source(&self, id: &str) -> PathBuf {
            let source = self.store.entry_path(AssetType::Skill, id).unwrap();
            fs::create_dir_all(&source).unwrap();
            fs::write(source.join("SKILL.md"), "---\nname: Demo\n---\nv1\n").unwrap();
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
            let base = home.clone();
            let target = skills::skills_dir(&agent, &home, Some(&base))
                .unwrap()
                .join(id);
            skills::deploy(&self.store, &agent, id, &home, Some(&base)).unwrap();
            target
        }

        /// 用生产那套链接能力造外来链接：本机允许哪种就用哪种，口径与被测代码一致。
        fn make_link(&self, real: &Path, link: &Path) {
            skills::link_dir(real, link).expect("建链接应当成功");
        }

        fn verify(&self) -> Vec<SkillDeploymentIssue> {
            super::verify(&self.store, &self.home).unwrap()
        }
    }

    #[test]
    fn a_matching_deployment_stays_silent() {
        let fx = fixture();
        fx.seed_source("demo");
        fx.deploy("demo");
        fx.enable("demo");

        assert!(fx.verify().is_empty(), "勾选与落点一致时不该有任何报告");
    }

    #[test]
    fn a_link_hijacked_by_another_tool_names_where_it_went() {
        let fx = fixture();
        fx.seed_source("demo");
        let target = fx.deploy("demo");
        fx.enable("demo");

        // 复刻双工具并存：对侧把自己的 SSOT 链接盖到同一个落点
        let rival = fx.home.join(".other-tool/skills/demo");
        fs::create_dir_all(&rival).unwrap();
        fs::write(rival.join("SKILL.md"), "---\nname: Demo\n---\n旧版\n").unwrap();
        skills::remove_link(&target).unwrap();
        fx.make_link(&rival, &target);

        let issues = fx.verify();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].status, DeploymentStatus::Foreign);
        assert!(
            issues[0].detail.contains(".other-tool/skills/demo"),
            "报告必须指出落点被指去了哪里：{}",
            issues[0].detail
        );
        assert!(
            fx.store
                .entry_path(AssetType::Skill, "demo")
                .unwrap()
                .join("SKILL.md")
                .is_file(),
            "核对只读，不许碰真源"
        );
    }

    #[test]
    fn a_directory_we_never_deployed_is_foreign() {
        let fx = fixture();
        fx.seed_source("demo");
        fx.enable("demo");
        let agent = fx.agent();
        let base = fx.home.clone();
        let target = skills::skills_dir(&agent, &fx.home, Some(&base))
            .unwrap()
            .join("demo");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "---\nname: Demo\n---\n手工放的\n").unwrap();

        let issues = fx.verify();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].status, DeploymentStatus::Foreign);
        assert_eq!(issues[0].detail, "落点是一份我们没有部署过的目录");
    }

    #[test]
    fn a_vanished_deployment_is_reported_without_being_recreated() {
        let fx = fixture();
        fx.seed_source("demo");
        let target = fx.deploy("demo");
        fx.enable("demo");
        skills::remove_link(&target).unwrap();

        let issues = fx.verify();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].status, DeploymentStatus::Missing);
        assert!(
            target.symlink_metadata().is_err(),
            "只报告：核对不许顺手把链接补回来"
        );
    }

    #[test]
    fn a_dangling_checkmark_is_reported_as_source_gone() {
        let fx = fixture();
        fx.seed_source("demo");
        fx.deploy("demo");
        fx.enable("demo");
        // 真源目录被删（矩阵里还挂着勾选）—— 这条正是「面板说已启用、盘上无从对应」
        fs::remove_dir_all(
            fx.store.entry_path(AssetType::Skill, "demo").unwrap(),
        )
        .unwrap();

        let issues = fx.verify();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert_eq!(issues[0].status, DeploymentStatus::SourceGone);
    }

    #[test]
    fn an_uninstalled_agent_is_skipped_instead_of_crying_missing() {
        let fx = fixture();
        fx.seed_source("demo");
        // codex 没装（临时 home 下没有它的根），但矩阵里有历史勾选
        update_matrix(&fx.store, "codex", |matrix| {
            matrix.enable(AssetType::Skill, "demo");
        })
        .unwrap();

        assert!(fx.verify().is_empty(), "{:?}", fx.verify());
    }
}
