//! 把未纳管的 Skill 收进中央库真源，并按用户选择接管盘上那份外来目录。
//!
//! 「收进真源」这一步不自己实现：走 `skill_source::install` 的本地目录分支，
//! 备份 / 暂存 / 原子换目录 / `.meta.json` 来源戳都是现成的那条写入流水线。
//! 这里只加**接管**：把 agent 目录里那份外来内容换成指向真源的部署，
//! 好让「面板上的勾选 == 磁盘事实」，不留一条永远启不起来的死路。
//!
//! 接管的底线：
//! - 链接只解链，绝不 `remove_dir_all` —— Windows 上对 junction 递归删会一路删进链接的真身；
//! - 真目录必须先证明真源保存的就是这一份（目录指纹一致），并整份留过底，才允许删；
//! - 替换完一律交回 `skill_source::set_enabled` 落启用矩阵，能力闸与检测门禁都在它里面；
//! - 启用失败就把原样还回去，绝不留下「原目录没了、部署也没建成」的半截状态。

use crate::adapters::skills::{self, Ownership};
use crate::domain::AssetType;
use crate::error::AppError;
use crate::import::mcp::ConflictPolicy;
use crate::net::archive;
use crate::services::skill_local_scan::{display_path, FoundLocation, UnmanagedSkillReport};
use crate::services::skill_source::{self, InstallSelection, SkillSink, SkillSourceInput};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 一项导入请求。目录名是唯一入口，实际路径一律由扫描重新定位 —— 前端传的路径不可信。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedImportInput {
    pub directory: String,
    /// 要接管并启用给的位置（agent id）。空数组就是「只收进真源，不改盘」。
    #[serde(default)]
    pub take_over: Vec<String>,
}

/// 一处接管的结局。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeoverOutcome {
    /// agent id；共享目录（`.agents/skills`）这类不可写的位置为空串，永远只收进真源。
    pub source_id: String,
    pub label: String,
    /// linked / already / refused
    pub status: String,
    /// 实测落地方式（symlink / junction / copy / already）或拒绝原因。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedImportOutcome {
    pub directory: String,
    /// 真源里最终落地的 ID（`KeepBoth` 时可能与目录名不同）。
    pub id: String,
    /// installed / overwritten / skipped / failed
    pub status: String,
    pub taken_over: Vec<TakeoverOutcome>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedImportReport {
    pub outcomes: Vec<UnmanagedImportOutcome>,
}

/// 批量导入。每一项独立成败，一项失败不影响其它项（与 `skill_source::install` 同一语义）。
pub fn import(
    store: &Store,
    home: &Path,
    inputs: &[UnmanagedImportInput],
    policy: ConflictPolicy,
) -> Result<UnmanagedImportReport, AppError> {
    if inputs.is_empty() {
        return Err(AppError::InvalidInput("没有选择要导入的 Skill".into()));
    }
    // 整批只扫一次：定位与「是否冲突」都取自同一份事实，避免中途磁盘变了却各说一套。
    let report = crate::services::skill_local_scan::scan(store, home)?;
    Ok(UnmanagedImportReport {
        outcomes: inputs
            .iter()
            .map(|input| import_one(store, home, &report, input, policy))
            .collect(),
    })
}

fn import_one(
    store: &Store,
    home: &Path,
    report: &UnmanagedSkillReport,
    input: &UnmanagedImportInput,
    policy: ConflictPolicy,
) -> UnmanagedImportOutcome {
    let directory = input.directory.trim();
    let Some(candidate) = report.find(directory) else {
        return UnmanagedImportOutcome {
            directory: directory.to_string(),
            id: String::new(),
            status: "failed".to_string(),
            taken_over: Vec::new(),
            warnings: Vec::new(),
            error: Some(format!(
                "未纳管列表里没有 {directory}，可能已经处理过或已被删除，请刷新后重试"
            )),
        };
    };

    // 1) 收进真源：整条写入流水线复用来源安装那一套。
    let installed = match skill_source::install(
        SkillSink::Store { store, home },
        &SkillSourceInput::Directory {
            path: candidate.source.clone(),
        },
        None,
        &[InstallSelection {
            relative_path: ".".to_string(),
            id: candidate.suggested_id.clone(),
        }],
        policy,
    ) {
        Err(error) => return failure(candidate, format!("写入真源失败：{error}")),
        Ok(installed) => match installed.outcomes.into_iter().next() {
            Some(outcome) => outcome,
            None => return failure(candidate, "安装没有返回结果".to_string()),
        },
    };
    let id = installed.id.clone();

    // 2) 接管。冲突项永不接管：要删的那份内容不在真源里，真源那份来自别的来源。
    let mut taken_over = Vec::new();
    let mut warnings = installed.warnings;
    if report.is_conflict(directory) {
        if !input.take_over.is_empty() {
            warnings.push(
                "真源里已有同名条目，本次只按冲突策略另存，没有接管盘上那份目录".to_string(),
            );
        }
    } else {
        for source_id in &input.take_over {
            let Some(location) = candidate
                .found_in
                .iter()
                .find(|found| found.source_id.as_deref() == Some(source_id.as_str()))
            else {
                taken_over.push(refused(
                    source_id,
                    source_id,
                    "这一处不在本次扫描结果里，请刷新后重试".to_string(),
                ));
                continue;
            };
            taken_over.push(take_over_one(store, home, &id, location));
        }
    }

    UnmanagedImportOutcome {
        directory: directory.to_string(),
        id,
        status: installed.status,
        taken_over,
        warnings,
        error: installed.error,
    }
}

fn failure(
    candidate: &crate::services::skill_local_scan::UnmanagedSkill,
    error: String,
) -> UnmanagedImportOutcome {
    UnmanagedImportOutcome {
        directory: candidate.directory.clone(),
        id: candidate.suggested_id.clone(),
        status: "failed".to_string(),
        taken_over: Vec::new(),
        warnings: Vec::new(),
        error: Some(error),
    }
}

fn refused(source_id: &str, label: &str, detail: String) -> TakeoverOutcome {
    TakeoverOutcome {
        source_id: source_id.to_string(),
        label: label.to_string(),
        status: "refused".to_string(),
        detail: Some(detail),
        backup_path: None,
    }
}

fn already(location: &FoundLocation, agent_id: &str, detail: &str) -> TakeoverOutcome {
    TakeoverOutcome {
        source_id: agent_id.to_string(),
        label: location.label.clone(),
        status: "already".to_string(),
        detail: Some(detail.to_string()),
        backup_path: None,
    }
}

/// 接管一处外来副本。
///
/// `skill_repair` 抢回被别的工具接走的落点走的是同一份逻辑：安全边界（只解链、
/// 真目录先比指纹再留底、失败原位还原）必须只有一处，不能各写一遍。
pub(crate) fn take_over_one(
    store: &Store,
    home: &Path,
    id: &str,
    location: &FoundLocation,
) -> TakeoverOutcome {
    let Some(agent_id) = location.source_id.clone() else {
        return refused(
            "",
            &location.label,
            format!("{} 不是我们的写入落点，只收进真源，没有改动盘上那份", location.label),
        );
    };
    if !location.takeover_possible {
        return refused(
            &agent_id,
            &location.label,
            format!("{} 的 skills 落点未实测，不能接管", location.label),
        );
    }
    // 这里不再自己解析 descriptor / baseOverride：`skill_source::set_enabled` 会按注册表
    // 重新核 `switchable` 与检测门禁，两处判定必须只有一个口径。
    let target = location.source.as_path();
    match skills::ownership(store, target) {
        Ownership::OursLink | Ownership::OursCopy => {
            return already(location, &agent_id, "这一处已经指向真源")
        }
        Ownership::Absent => {
            return already(location, &agent_id, "这一处已经不在了，交给正常启用流程落地")
        }
        Ownership::Foreign => {}
    }

    // 链接：解链不丢内容（真身在别处，原样留着），所以不需要内容核对，也不需要留底。
    if skills::is_link(target) {
        let previous = skills::link_target(target);
        let restorable = previous
            .as_ref()
            .map(|raw| raw.canonicalize().unwrap_or_else(|_| raw.clone()))
            .map(Restore::Link);
        if let Err(error) = skills::remove_link(target) {
            return refused(&agent_id, &location.label, format!("解链失败：{error}"));
        }
        let detail = match previous.as_ref().map(|raw| display_path(raw)) {
            Some(previous) => format!("已解除原链接（原指向 {previous}）"),
            None => "已解除原链接".to_string(),
        };
        return enable_or_restore(
            store,
            home,
            id,
            &agent_id,
            &location.label,
            target,
            restorable,
            None,
            detail,
        );
    }

    // 真目录：先证明真源保存的就是这一份，再整份留底，才允许删。
    let Ok(canonical) = store.entry_path(AssetType::Skill, id) else {
        return refused(&agent_id, &location.label, format!("非法的 Skill ID：{id}"));
    };
    if !canonical.join("SKILL.md").is_file() {
        return refused(
            &agent_id,
            &location.label,
            format!("真源里没有 {id}，不能接管"),
        );
    }
    match (
        archive::compute_dir_hash(target),
        archive::compute_dir_hash(&canonical),
    ) {
        (Ok(ours), Ok(theirs)) if ours == theirs => {}
        (Ok(_), Ok(_)) => {
            return refused(
                &agent_id,
                &location.label,
                "盘上这份与真源内容不一致，已保留原目录；请先比对清楚再处理".to_string(),
            )
        }
        (Err(error), _) | (_, Err(error)) => {
            return refused(
                &agent_id,
                &location.label,
                format!("无法比对内容：{error}"),
            )
        }
    }
    let backup = match store.backup_adopted(&agent_id, target) {
        Ok(path) => path,
        Err(error) => {
            return refused(
                &agent_id,
                &location.label,
                format!("接管前留底失败：{error}"),
            )
        }
    };
    if let Err(error) = std::fs::remove_dir_all(target) {
        return TakeoverOutcome {
            backup_path: Some(display_path(&backup)),
            ..refused(
                &agent_id,
                &location.label,
                format!("删除原目录失败：{error}；原目录已保留"),
            )
        };
    }
    enable_or_restore(
        store,
        home,
        id,
        &agent_id,
        &location.label,
        target,
        Some(Restore::Directory(backup.clone())),
        Some(backup),
        "已用指向真源的部署替换原目录".to_string(),
    )
}

/// 外来副本已经不在原位，把它变成我们的部署并写上启用矩阵；失败就还原原样。
fn enable_or_restore(
    store: &Store,
    home: &Path,
    id: &str,
    agent_id: &str,
    label: &str,
    target: &Path,
    restorable: Option<Restore>,
    backup: Option<PathBuf>,
    detail: String,
) -> TakeoverOutcome {
    let backup_path = backup.as_ref().map(|path| display_path(path));
    let ids = vec![id.to_string()];
    let failure = match skill_source::set_enabled(store, home, &ids, agent_id, true) {
        Ok(batch) if batch.failed.is_empty() => {
            let used = batch
                .ok
                .first()
                .map(|ok| ok.label.clone())
                .unwrap_or_else(|| "linked".to_string());
            return TakeoverOutcome {
                source_id: agent_id.to_string(),
                label: label.to_string(),
                status: "linked".to_string(),
                detail: Some(format!("{detail}；启用方式：{used}")),
                backup_path,
            };
        }
        Ok(batch) => batch
            .failed
            .first()
            .map(|failed| failed.error.clone())
            .unwrap_or_else(|| "启用失败".to_string()),
        Err(error) => error.to_string(),
    };
    // 还原：宁可回到接管前，也不留下「原目录没了、部署也没建成」。
    let restored = match restorable {
        Some(Restore::Link(source)) => skills::link_dir(&source, target)
            .map(|_| "已还原原链接".to_string())
            .map_err(|error| format!("还原原链接失败：{error}")),
        Some(Restore::Directory(backup)) => crate::store::copy_dir(&backup, target)
            .map(|_| "已从备份还原原目录".to_string())
            .map_err(|error| format!("从备份还原失败：{error}")),
        None => Ok("无需还原".to_string()),
    };
    let detail = match restored {
        Ok(note) => format!("{failure}；{note}"),
        Err(error) => format!(
            "{failure}；{error}，需要手工处理：{}",
            display_path(target)
        ),
    };
    TakeoverOutcome {
        source_id: agent_id.to_string(),
        label: label.to_string(),
        status: "refused".to_string(),
        detail: Some(detail),
        backup_path,
    }
}

/// 还原手段：链接按原目标重链，真目录从刚才的备份拷回。
enum Restore {
    Link(PathBuf),
    Directory(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::services::agents::{self, save_override};
    use crate::services::skill_local_scan;
    use crate::store::Store;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        store: Store,
        home: PathBuf,
    }

    /// 所有 agent 的落点都 `baseOverride` 到临时 home，真实 `~/.claude` 一律不碰。
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
                .find(|candidate| candidate.id == agent)
                .unwrap();
            skills::skills_dir(&descriptor, &self.home, Some(&self.home)).unwrap()
        }

        fn seed_foreign(&self, agent: &str, name: &str, body: &str) -> PathBuf {
            let dir = self.skills_dir(agent).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
            dir
        }

        fn canonical(&self, id: &str) -> PathBuf {
            self.store.entry_path(AssetType::Skill, id).unwrap()
        }

        fn import(&self, directory: &str, take_over: &[&str]) -> UnmanagedImportOutcome {
            let report = import(
                &self.store,
                &self.home,
                &[UnmanagedImportInput {
                    directory: directory.to_string(),
                    take_over: take_over.iter().map(|id| id.to_string()).collect(),
                }],
                ConflictPolicy::Skip,
            )
            .unwrap();
            report.outcomes.into_iter().next().unwrap()
        }

        fn enabled(&self, agent: &str, id: &str) -> bool {
            agents::read_matrix(&self.store, agent)
                .unwrap()
                .enabled_for(AssetType::Skill)
                .contains(&id.to_string())
        }
    }

    const SKILL: &str = "---\nname: 演示\ndescription: 说明\n---\n\n正文\n";

    #[test]
    fn importing_lands_the_source_of_truth_with_its_origin() {
        let fx = fixture();
        let foreign = fx.seed_foreign("claude", "hand-made", SKILL);

        let outcome = fx.import("hand-made", &[]);
        assert_eq!(outcome.status, "installed", "{outcome:?}");
        assert_eq!(outcome.taken_over.len(), 0);
        assert!(fx.canonical("hand-made").join("SKILL.md").is_file());
        let meta = crate::domain::skill_meta::read_meta(&fx.canonical("hand-made"))
            .expect("导入要留下来源信息");
        assert_eq!(meta.source_kind(), crate::domain::skill_meta::SkillSourceKind::Local);
        assert!(meta.source_url.unwrap().contains("hand-made"));
        // 没勾接管：盘上那份一个字节都不动。
        assert_eq!(
            std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
            SKILL
        );
        assert!(!fx.enabled("claude", "hand-made"));
    }

    #[test]
    fn taking_over_replaces_the_directory_with_our_deployment() {
        let fx = fixture();
        let foreign = fx.seed_foreign("claude", "adopt-me", SKILL);
        std::fs::write(foreign.join("extra.md"), "附件\n").unwrap();

        let outcome = fx.import("adopt-me", &["claude"]);
        let taken = &outcome.taken_over[0];
        assert_eq!(taken.status, "linked", "{taken:?}");
        assert_eq!(taken.source_id, "claude");

        // 真源内容齐、且盘上这份已经是指向真源的链接或带标记的副本。
        assert!(fx.canonical("adopt-me").join("extra.md").is_file());
        assert!(matches!(
            skills::ownership(&fx.store, &fx.skills_dir("claude").join("adopt-me")),
            Ownership::OursLink | Ownership::OursCopy
        ));
        assert!(fx.enabled("claude", "adopt-me"), "接管必须同时写启用矩阵");
        // 再部署一次应当是什么都不做，证明接管落的是自家形态。
        let descriptor = builtin_agents()
            .into_iter()
            .find(|agent| agent.id == "claude")
            .unwrap();
        assert_eq!(
            skills::deploy(
                &fx.store,
                &descriptor,
                "adopt-me",
                &fx.home,
                Some(&fx.home)
            )
            .unwrap(),
            skills::Deployed::Already
        );
    }

    #[test]
    fn a_taken_over_directory_keeps_a_readable_backup() {
        let fx = fixture();
        let foreign = fx.seed_foreign("claude", "backed-up", "---\nname: 原始\n---\n\n原样\n");

        let outcome = fx.import("backed-up", &["claude"]);
        let backup = outcome.taken_over[0]
            .backup_path
            .clone()
            .expect("删真目录前必须留底");
        assert!(Path::new(&backup).join("SKILL.md").is_file());
        assert_eq!(
            std::fs::read_to_string(Path::new(&backup).join("SKILL.md")).unwrap(),
            "---\nname: 原始\n---\n\n原样\n"
        );
        assert!(backup.contains("adopt_claude_backed-up"), "{backup}");
        // 原位现在是我们的部署，内容就是刚才留底的那份。
        assert!(matches!(
            skills::ownership(&fx.store, &foreign),
            Ownership::OursLink | Ownership::OursCopy
        ));
    }

    #[test]
    fn the_store_copy_never_keeps_someone_elses_deployment_marker() {
        let fx = fixture();
        let foreign = fx.seed_foreign("claude", "marked", SKILL);
        // 这份外来目录本身是别的 store  copy 部署过的，带着别人的标记。
        std::fs::write(
            foreign.join(".ccswitchx"),
            "D:/someone-else/.ccswitchx/assets/skills/marked\n",
        )
        .unwrap();

        let outcome = fx.import("marked", &["claude"]);
        assert_eq!(outcome.status, "installed", "{outcome:?}");
        assert!(
            !fx.canonical("marked").join(".ccswitchx").exists(),
            "别人的部署标记不能进真源"
        );
    }

    #[test]
    fn taking_over_a_foreign_link_only_unlinks_it() {
        let fx = fixture();
        let real = fx.home.join("elsewhere/linked-skill");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("SKILL.md"), SKILL).unwrap();
        let link = fx.skills_dir("claude").join("linked-skill");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        make_link(&real, &link);

        let outcome = fx.import("linked-skill", &["claude"]);
        let taken = &outcome.taken_over[0];
        assert_eq!(taken.status, "linked", "{taken:?}");
        assert!(taken.backup_path.is_none(), "解链不丢内容，不需要留底：{taken:?}");
        // 链接真身必须完整保留。
        assert_eq!(
            std::fs::read_to_string(real.join("SKILL.md")).unwrap(),
            SKILL
        );
        assert!(matches!(
            skills::ownership(&fx.store, &link),
            Ownership::OursLink | Ownership::OursCopy
        ));
        assert!(fx.enabled("claude", "linked-skill"));
    }

    #[test]
    fn a_conflict_is_never_taken_over() {
        let fx = fixture();
        // 真源先有同名条目，盘上这份是别人的内容。
        std::fs::create_dir_all(fx.canonical("twin")).unwrap();
        std::fs::write(fx.canonical("twin").join("SKILL.md"), "---\nname: 真源\n---\n\nA\n").unwrap();
        let foreign = fx.seed_foreign("claude", "twin", "---\nname: 别人\n---\n\nB\n");

        let outcome = fx.import("twin", &["claude"]);
        assert_eq!(outcome.status, "skipped", "{outcome:?}");
        // 冲突项压根不产生接管动作：要删的那份内容不在真源里。
        assert!(outcome.taken_over.is_empty(), "{:?}", outcome.taken_over);
        assert!(
            outcome.warnings.iter().any(|warning| warning.contains("没有接管")),
            "{:?}",
            outcome.warnings
        );
        assert_eq!(std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(), "---\nname: 别人\n---\n\nB\n");
        assert_eq!(
            std::fs::read_to_string(fx.canonical("twin").join("SKILL.md")).unwrap(),
            "---\nname: 真源\n---\n\nA\n"
        );
    }

    #[test]
    fn an_unverified_landing_spot_refuses_takeover_but_still_installs() {
        let fx = fixture();
        fx.seed_foreign("gemini", "gemini-skill", SKILL);

        let outcome = fx.import("gemini-skill", &["gemini"]);
        assert_eq!(outcome.status, "installed", "{outcome:?}");
        assert_eq!(outcome.taken_over[0].status, "refused");
        assert!(
            outcome.taken_over[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("未实测"),
            "{:?}",
            outcome.taken_over[0]
        );
        assert!(fx.canonical("gemini-skill").is_dir());
    }

    #[test]
    fn takeover_refuses_when_the_copy_does_not_match_the_source_of_truth() {
        let fx = fixture();
        // 直接测护栏：真源内容已经与盘上那份不同了（例如并发改过）。
        std::fs::create_dir_all(fx.canonical("drifted")).unwrap();
        std::fs::write(fx.canonical("drifted").join("SKILL.md"), "真源另一版\n").unwrap();
        let foreign = fx.seed_foreign("claude", "drifted", SKILL);
        let report = skill_local_scan::scan(&fx.store, &fx.home).unwrap();
        let candidate = report.find("drifted").expect("冲突项也要被扫出来");
        let location = &candidate.found_in[0];

        let outcome = take_over_one(&fx.store, &fx.home, "drifted", location);
        assert_eq!(outcome.status, "refused", "{outcome:?}");
        assert!(
            outcome.detail.as_deref().unwrap().contains("不一致"),
            "{:?}",
            outcome.detail
        );
        assert_eq!(std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(), SKILL);
    }

    #[test]
    fn a_failed_enable_restores_the_directory_from_the_backup() {
        let fx = fixture();
        let foreign = fx.seed_foreign("claude", "rollback", SKILL);
        // 直接测回滚这一手：原目录已经留过底并被删掉，而启用注定失败（注册表里没有这个 agent）。
        let backup = fx.store.backup_adopted("claude", &foreign).unwrap();
        std::fs::remove_dir_all(&foreign).unwrap();

        let outcome = enable_or_restore(
            &fx.store,
            &fx.home,
            "rollback",
            "no-such-agent",
            "Claude Code",
            &foreign,
            Some(Restore::Directory(backup.clone())),
            Some(backup),
            "已用指向真源的部署替换原目录".to_string(),
        );
        assert_eq!(outcome.status, "refused", "{outcome:?}");
        assert!(
            outcome.detail.as_deref().unwrap().contains("已从备份还原"),
            "{:?}",
            outcome.detail
        );
        assert!(
            foreign.join("SKILL.md").is_file(),
            "绝不能留下「原目录没了、部署也没建成」"
        );
        assert_eq!(
            std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
            SKILL
        );
    }

    #[test]
    fn an_imported_and_taken_over_skill_leaves_the_unmanaged_list() {
        let fx = fixture();
        let foreign = fx.seed_foreign("claude", "stale", SKILL);

        let outcome = fx.import("stale", &["claude"]);
        assert_eq!(outcome.status, "installed", "{outcome:?}");
        assert_eq!(outcome.taken_over[0].status, "linked", "{:?}", outcome.taken_over);

        // 盘上这份已经是我们的部署，扫描不再把它当未纳管；重复导入必须点名失败，
        // 而不是又建一条真源条目或再删一次目录。
        let again = fx.import("stale", &["claude"]);
        assert_eq!(again.status, "failed", "{again:?}");
        assert!(again.error.unwrap().contains("没有"), "错误要说清已不在列表");
        let unknown = fx.import("never-existed", &["claude"]);
        assert_eq!(unknown.status, "failed");
        assert_eq!(unknown.id, "");
        assert!(!fx.skills_dir("claude").join("never-existed").exists());
        assert!(foreign.join("SKILL.md").is_file(), "重复请求不能把已部署的内容弄丢");
        assert!(fx.enabled("claude", "stale"));
    }

    #[test]
    fn an_empty_batch_is_refused() {
        let fx = fixture();
        let error = import(&fx.store, &fx.home, &[], ConflictPolicy::Skip).unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)), "{error}");
    }

    #[test]
    fn takeover_outcomes_carry_no_ambiguous_directories() {
        // found_in 里的 source_id 才是接管的合法目标；空串（共享目录）必须被拒。
        let fx = fixture();
        let dir = fx.home.join(".agents/skills/cross-tool");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), SKILL).unwrap();

        let outcome = fx.import("cross-tool", &[""]);
        assert_eq!(outcome.status, "installed", "{outcome:?}");
        assert_eq!(outcome.taken_over[0].status, "refused");
        assert!(fx.canonical("cross-tool").is_dir(), "只收进真源仍要成功");
    }

    /// 用生产那套链接能力造外来链接（本机允许哪种就用哪种）。
    fn make_link(real: &Path, link: &Path) {
        skills::link_dir(real, link).expect("建链接应当成功");
        assert!(skills::is_link(link), "链接必须被 is_link 认出来");
    }
}
