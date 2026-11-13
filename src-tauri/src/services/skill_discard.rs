//! 删除未纳管的 Skill 副本：用户在「未纳管」页显式点删除时才走这里。
//!
//! 与相邻模块的分工：`skill_local_scan` 只读地发现盘上不归我们管的内容，
//! `skill_takeover` 把它收进真源并换成我们的部署，这里只做清理 —— 那份内容既不收进真源，
//! 也不接管，原地删掉。
//!
//! 安全边界一处都不放宽：
//! - 目录名是唯一入口，实际路径由本次重新扫描定位，前端传的路径不可信；
//! - 动手前复核 `skills::ownership`，仍是 `Foreign` 才删：指向真源的链接、我们落的副本一律拒绝；
//! - 链接（含 junction）只解链。Windows 上 `remove_dir_all` 会沿 junction 删进对侧真身，
//!   所以真目录必须先整份留底到 `backups/skills`，留底成功才允许删；
//! - 不碰真源、不写启用矩阵：删除外来副本不是「停用」，两者不能混。

use crate::adapters::skills::{self, Ownership};
use crate::error::AppError;
use crate::services::skill_local_scan::{display_path, FoundLocation, UnmanagedSkillReport};
use crate::store::Store;
use serde::Serialize;
use std::path::Path;

/// 一处的删除结局。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardLocationOutcome {
    /// agent id；共享目录（`.agents/skills`）这类位置为空。
    pub source_id: Option<String>,
    pub label: String,
    pub path: String,
    /// `removedLink` / `removedDirectory` / `missing` / `refused`
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 真目录删除前的整份留底；链接只解链，不产生备份。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

/// 一个未纳管条目的删除结局。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardOutcome {
    pub directory: String,
    pub deleted: Vec<DiscardLocationOutcome>,
    /// 本次扫描里找不到该目录（已被处理或已删除）时的原因。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardReport {
    pub outcomes: Vec<DiscardOutcome>,
}

/// 删除若干未纳管条目在盘上的全部副本。逐条独立成败，一项失败不影响其它项。
pub fn discard(
    store: &Store,
    home: &Path,
    directories: &[String],
) -> Result<DiscardReport, AppError> {
    if directories.is_empty() {
        return Err(AppError::InvalidInput("没有选择要删除的 Skill".into()));
    }
    // 与导入同一纪律：整批只扫一次，定位与归属判定取自同一份事实。
    let report = crate::services::skill_local_scan::scan(store, home)?;
    Ok(DiscardReport {
        outcomes: directories
            .iter()
            .map(|directory| discard_one(store, &report, directory.trim()))
            .collect(),
    })
}

fn discard_one(store: &Store, report: &UnmanagedSkillReport, directory: &str) -> DiscardOutcome {
    let Some(candidate) = report.find(directory) else {
        return DiscardOutcome {
            directory: directory.to_string(),
            deleted: Vec::new(),
            error: Some(format!(
                "未纳管列表里没有 {directory}，可能已经处理过或已被删除，请刷新后重试"
            )),
        };
    };
    // 先解链、后删真身：`.agents/skills` 这类共享根在扫描结果里排在 agent 落点之后，
    // 但删除顺序不能靠这个巧合 —— 排序保证指向真身的链接先断开，再删真身。
    let mut locations = candidate.found_in.clone();
    locations.sort_by_key(|location| !skills::is_link(&location.source));
    DiscardOutcome {
        directory: directory.to_string(),
        deleted: locations
            .iter()
            .map(|location| discard_location(store, location))
            .collect(),
        error: None,
    }
}

fn discard_location(store: &Store, location: &FoundLocation) -> DiscardLocationOutcome {
    let target = location.source.as_path();
    let outcome = |status: &str, detail: Option<String>, backup_path: Option<String>| {
        DiscardLocationOutcome {
            source_id: location.source_id.clone(),
            label: location.label.clone(),
            path: display_path(target),
            status: status.to_string(),
            detail,
            backup_path,
        }
    };
    let refuse = |detail: String| outcome("refused", Some(detail), None);

    match skills::ownership(store, target) {
        Ownership::Absent => return outcome("missing", Some("这一处已经不在了".into()), None),
        // 我们落的副本归「已安装」面板管：那里停用/卸载有整条写入流水线，不能在这里删。
        Ownership::OursLink | Ownership::OursCopy => {
            return refuse("这一处归我们管，不走删除；请在已安装里停用或卸载".into())
        }
        Ownership::Foreign => {}
    }

    if skills::is_link(target) {
        let previous = skills::link_target(target)
            .map(|raw| display_path(&raw.canonicalize().unwrap_or(raw)));
        return match skills::remove_link(target) {
            Ok(()) => outcome(
                "removedLink",
                Some(match previous {
                    Some(previous) => format!("已解除链接（原指向 {previous}），目标内容未动"),
                    None => "已解除链接，目标内容未动".to_string(),
                }),
                None,
            ),
            Err(error) => refuse(format!("解链失败：{error}")),
        };
    }
    if !target.is_dir() {
        return refuse("不是目录".to_string());
    }
    // 共享根没有 agent id，用固定标签进备份名，保证 `backup_adopted` 的 ID 校验能过。
    let owner = location.source_id.as_deref().unwrap_or("shared");
    let backup = match store.backup_adopted(owner, target) {
        Ok(path) => path,
        Err(error) => return refuse(format!("删除前留底失败：{error}")),
    };
    let backup_path = display_path(&backup);
    match std::fs::remove_dir_all(target) {
        Ok(()) => outcome(
            "removedDirectory",
            Some("已删除原目录（删除前已整份留底）".into()),
            Some(backup_path),
        ),
        Err(error) => outcome(
            "refused",
            Some(format!("删除原目录失败：{error}；原目录已保留")),
            Some(backup_path),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{agent::builtin_agents, AssetType};
    use crate::services::agents::save_override;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        store: Store,
        home: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        for id in ["claude", "codex", "gemini", "qoder", "qoder-cn", "pi"] {
            save_override(&store, id, Some(&home)).unwrap();
        }
        Fixture {
            _dir: dir,
            store,
            home,
        }
    }

    impl Fixture {
        fn skills_dir(&self, agent: &str) -> PathBuf {
            let descriptor = builtin_agents()
                .into_iter()
                .find(|descriptor| descriptor.id == agent)
                .unwrap();
            skills::skills_dir(&descriptor, &self.home, Some(&self.home)).unwrap()
        }

        fn seed_foreign(&self, agent: &str, name: &str) -> PathBuf {
            let dir = self.skills_dir(agent).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), "---\nname: 演示\n---\n\n正文\n").unwrap();
            dir
        }

        fn discard(&self, directories: &[&str]) -> DiscardReport {
            let directories = directories.iter().map(|dir| dir.to_string()).collect::<Vec<_>>();
            discard(&self.store, &self.home, &directories).unwrap()
        }
    }

    #[test]
    fn deleting_a_link_only_removes_the_link() {
        let fx = fixture();
        let real = fx.home.join("elsewhere/hand-made");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("SKILL.md"), "正文\n").unwrap();
        let link = fx.skills_dir("claude").join("hand-made");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        skills::link_dir(&real, &link).unwrap();

        let report = fx.discard(&["hand-made"]);
        assert_eq!(report.outcomes.len(), 1);
        let outcome = &report.outcomes[0];
        assert_eq!(outcome.deleted[0].status, "removedLink", "{outcome:?}");
        assert!(link.symlink_metadata().is_err(), "链接应当被解开");
        assert_eq!(
            std::fs::read_to_string(real.join("SKILL.md")).unwrap(),
            "正文\n",
            "链接指向的内容一个字节都不能动"
        );
    }

    #[test]
    fn deleting_a_real_directory_backs_it_up_first() {
        let fx = fixture();
        let dir = fx.seed_foreign("claude", "hand-made");

        let report = fx.discard(&["hand-made"]);
        let outcome = &report.outcomes[0];
        assert_eq!(outcome.deleted[0].status, "removedDirectory", "{outcome:?}");
        assert!(!dir.exists(), "原目录应当被删掉");
        let backup = outcome.deleted[0].backup_path.as_ref().unwrap();
        assert!(
            Path::new(backup).join("SKILL.md").is_file(),
            "留底必须能还原内容：{backup}"
        );
    }

    #[test]
    fn our_own_deployment_is_refused_rather_than_deleted() {
        let fx = fixture();
        let id = "deployed";
        let source = fx.store.asset_dir(AssetType::Skill).join(id);
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "---\nname: demo\n---\n\n正文\n").unwrap();
        let descriptor = builtin_agents()
            .into_iter()
            .find(|agent| agent.id == "claude")
            .unwrap();
        skills::deploy(&fx.store, &descriptor, id, &fx.home, Some(&fx.home)).unwrap();
        let target = fx.skills_dir("claude").join(id);
        let location = FoundLocation {
            source_id: Some("claude".to_string()),
            label: "Claude Code".to_string(),
            source: target.clone(),
            path: display_path(&target),
            link_target: None,
            takeover_possible: true,
        };

        let outcome = discard_location(&fx.store, &location);
        assert_eq!(outcome.status, "refused", "{outcome:?}");
        assert!(target.symlink_metadata().is_ok(), "我们的部署不能被删");
    }

    #[test]
    fn a_directory_that_vanished_is_reported_not_silently_dropped() {
        let fx = fixture();
        let report = fx.discard(&["hand-made"]);
        let outcome = &report.outcomes[0];
        assert!(outcome.deleted.is_empty());
        assert!(outcome.error.as_deref().unwrap_or_default().contains("没有"));
    }

    #[test]
    fn one_missing_entry_does_not_block_the_rest() {
        let fx = fixture();
        let dir = fx.seed_foreign("claude", "hand-made");

        let report = fx.discard(&["hand-made", "does-not-exist"]);
        assert_eq!(report.outcomes.len(), 2);
        assert!(report.outcomes[0].error.is_none(), "{:?}", report.outcomes);
        assert!(!dir.exists());
        assert!(report.outcomes[1].error.is_some(), "{:?}", report.outcomes[1]);
    }
}
