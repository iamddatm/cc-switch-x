//! v2 → v3 布局搬迁：把项目仓库里的 `.ccswitchx/` 搬进客户端 store。
//!
//! v2 把真源与台账放在 `<project>/.ccswitchx/` 里随仓库提交；v3 改成
//! `<storeRoot>/local/projects/<project-id>/`（`local/` 不进任何同步）。
//! 这个模块只做一件事：把老项目的字节安全搬过去，然后删掉项目里的旧目录。
//!
//! 纪律：
//! 1. **先备份再动手**：整份 `.ccswitchx/` 先快照进 `backups/projects/<id>/<ts>/legacy/`。
//! 2. **写后校验**：逐个 skill 比对树哈希，台账回读反序列化；任一不一致就整体放弃，
//!    不写 `layout.json`、不删任何东西。
//! 3. **删除是最后一步，且有三个前提**：布局戳记已写、每个 skill 的哈希都与来源一致、
//!    `.ccswitchx/` 里除了 `skills/` 与 `skills.json` 没有别的东西。缺一不可。
//! 4. **幂等**：`layout.json` 存在即视为已迁完；重复进入只会做「清理遗留」或什么都不做。

use crate::domain::project::{
    ProjectLayoutStatus, ProjectPaths, LEGACY_DIR_RELATIVE, PROJECT_LAYOUT_MIN_READABLE,
    PROJECT_LAYOUT_VERSION,
};
use crate::error::AppError;
use crate::services::project_scan;
use crate::store::{copy_dir, is_safe_id, Store};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// `layout.json` 的内容。只记「这个项目已经用上新布局」这个事实与来路，
/// 不复制任何业务数据 —— 真源与台账各有各的文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutStamp {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migrated_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migrated_at: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRelocationReport {
    /// 搬迁后的最终状态。等于 `Native` 才代表「项目里已经没有我们的文件了」。
    pub status: ProjectLayoutStatus,
    /// 本次从项目搬进 store 的 skill id。
    pub migrated: Vec<String>,
    /// 旧目录是否已被删掉。
    pub legacy_removed: bool,
    pub warnings: Vec<String>,
}

impl ProjectRelocationReport {
    fn unchanged(status: ProjectLayoutStatus) -> Self {
        Self {
            status,
            ..Default::default()
        }
    }
}

fn read_layout(paths: &ProjectPaths) -> Result<Option<u32>, AppError> {
    let path = paths.layout_path();
    if !path.is_file() {
        return Ok(None);
    }
    let stamp: LayoutStamp = serde_json::from_slice(&std::fs::read(path)?).map_err(|error| {
        AppError::InvalidInput(format!(
            "项目的布局戳记 {} 解析失败：{error}",
            path.display()
        ))
    })?;
    Ok(Some(stamp.version))
}

/// 旧目录里的条目名。空目录返回空表。
fn legacy_entries(paths: &ProjectPaths) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(paths.legacy_dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// 除 `skills/` 与 `skills.json` 之外，旧目录里还有没有别的东西。
fn foreign_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|name| name.as_str() != "skills" && name.as_str() != "skills.json")
        .cloned()
        .collect()
}

/// store 侧是否已经有这个项目的数据（台账或真源目录）。
fn store_has_data(paths: &ProjectPaths) -> bool {
    if paths.ledger_path().is_file() {
        return true;
    }
    !store_skill_ids(paths).unwrap_or_default().is_empty()
}

/// store 里的真源 skill id（只认目录，点开头的 staging 不算）。
fn store_skill_ids(paths: &ProjectPaths) -> Result<Vec<String>, AppError> {
    let Ok(read) = std::fs::read_dir(paths.canonical_root()) else {
        return Ok(Vec::new());
    };
    let mut ids = Vec::new();
    for entry in read {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        ids.push(name);
    }
    ids.sort();
    Ok(ids)
}

/// 纯读判定：项目当前处于哪种布局。不会创建、移动或删除任何东西。
pub fn status(paths: &ProjectPaths) -> Result<ProjectLayoutStatus, AppError> {
    let layout = read_layout(paths)?;
    if let Some(version) = layout {
        // 高版本是「未来格式」，低版本从未被写过：两者都按不可理解处理。
        if !(PROJECT_LAYOUT_MIN_READABLE..=PROJECT_LAYOUT_VERSION).contains(&version) {
            return Ok(ProjectLayoutStatus::Unsupported { version });
        }
    }
    let legacy_exists = paths.legacy_dir().is_dir();
    let entries = if legacy_exists {
        legacy_entries(paths)
    } else {
        Vec::new()
    };
    let foreign = foreign_entries(&entries);

    match layout {
        Some(_) if !legacy_exists => Ok(ProjectLayoutStatus::Native),
        Some(_) if !foreign.is_empty() => Ok(ProjectLayoutStatus::Foreign { entries: foreign }),
        Some(_) => {
            // 已迁完但旧的还留着：内容与 store 一致只是没删干净，不一致就得让人裁决。
            // 只看同名 skill 与台账，未跟踪的额外目录不影响判定。
            let mut collisions = legacy_skill_ids(paths)?;
            collisions.retain(|id| paths.canonical_skill_dir(id).is_ok_and(|dir| dir.is_dir()));
            if has_divergence(paths, &collisions)? {
                Ok(ProjectLayoutStatus::Hybrid { collisions })
            } else {
                Ok(ProjectLayoutStatus::RedundantLegacy { entries })
            }
        }
        None => {
            if !legacy_exists {
                return Ok(ProjectLayoutStatus::Native);
            }
            if store_has_data(paths) {
                // 两边都有数据：可能上次搬到一半崩了，也可能有人手工放过东西。
                // 交给 `auto` 逐项比对，一致就补完，不一致就停下来等人裁决。
                let mut collisions = legacy_skill_ids(paths)?;
                collisions.retain(|id| paths.canonical_skill_dir(id).is_ok_and(|dir| dir.is_dir()));
                return Ok(ProjectLayoutStatus::Hybrid { collisions });
            }
            if !foreign.is_empty() {
                // 旧目录里有不认识的东西：数据照搬（复制不改来源），但绝不删那个目录。
                return Ok(ProjectLayoutStatus::Foreign { entries: foreign });
            }
            Ok(ProjectLayoutStatus::Legacy)
        }
    }
}

/// 旧真源里的 skill id：含 `SKILL.md` 的子目录。非法 ID 会出现在结果里，
/// 由调用方在搬迁时拒绝，而不是静默跳过（跳过等于把用户的文件留在原地不解释）。
fn legacy_skill_ids(paths: &ProjectPaths) -> Result<Vec<String>, AppError> {
    let base = paths.legacy_dir().join("skills");
    let Ok(read) = std::fs::read_dir(&base) else {
        return Ok(Vec::new());
    };
    let mut ids = Vec::new();
    for entry in read {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if !entry.file_type()?.is_dir() || !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        ids.push(name);
    }
    ids.sort();
    Ok(ids)
}

/// 两侧同名 skill 是否有内容分歧；台账两边都有但内容不同也算分歧。
/// 只比较、不写入，用来决定 `Hybrid` 能不能自动补完。
fn has_divergence(paths: &ProjectPaths, collisions: &[String]) -> Result<bool, AppError> {
    for id in collisions {
        let source = project_scan::tree_state(&paths.legacy_dir().join("skills").join(id))?;
        let target = project_scan::tree_state(&paths.canonical_skill_dir(id)?)?;
        if source.tree_hash != target.tree_hash {
            return Ok(true);
        }
    }
    let legacy_ledger = paths.legacy_dir().join("skills.json");
    if legacy_ledger.is_file() && paths.ledger_path().is_file() {
        let left: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&legacy_ledger)?)?;
        let right: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.ledger_path())?)?;
        if left != right {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 从项目里把字节搬进 store（第一次搬迁，或崩溃后的补完）。
    Migrate,
    /// store 已是权威副本：只校验旧目录一致后删除，不再复制。
    Cleanup,
}

/// 打开项目、添加项目时调用：该搬的搬，该清遗留的清遗留。
/// 已是最新布局（或没有旧目录）且 store 尚无数据时是纯读，不产生任何写入。
pub fn auto(store: &Store, paths: &ProjectPaths) -> Result<ProjectRelocationReport, AppError> {
    match status(paths)? {
        ProjectLayoutStatus::Unsupported { version } => Err(AppError::InvalidInput(format!(
            "这个项目的数据由更新版本的 CC Switch X 写入（布局版本 {version}，本程序支持 {PROJECT_LAYOUT_VERSION}），请升级后再打开"
        ))),
        ProjectLayoutStatus::Native => {
            // 补写布局戳记：有这个文件，下次启动的判定就不用再猜。
            if !paths.layout_path().is_file() && store_has_data(paths) {
                write_layout(paths)?;
            }
            Ok(ProjectRelocationReport::unchanged(
                ProjectLayoutStatus::Native,
            ))
        }
        ProjectLayoutStatus::Legacy => relocate(store, paths, Mode::Migrate),
        ProjectLayoutStatus::Foreign { .. } => {
            if paths.layout_path().is_file() {
                // 已经搬过，只剩用户自己决定要不要清理。
                Ok(ProjectRelocationReport::unchanged(status(paths)?))
            } else {
                relocate(store, paths, Mode::Migrate)
            }
        }
        ProjectLayoutStatus::Hybrid { collisions } => {
            if has_divergence(paths, &collisions)? {
                // 两边真有分歧：一个字节都不动，等用户在面板上裁决。
                Ok(ProjectRelocationReport::unchanged(
                    ProjectLayoutStatus::Hybrid { collisions },
                ))
            } else {
                relocate(store, paths, Mode::Migrate)
            }
        }
        ProjectLayoutStatus::RedundantLegacy { .. } => relocate(store, paths, Mode::Cleanup),
    }
}

fn write_layout(paths: &ProjectPaths) -> Result<(), AppError> {
    let stamp = LayoutStamp {
        version: PROJECT_LAYOUT_VERSION,
        migrated_from: Some("in-project".to_string()),
        migrated_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    if let Some(parent) = paths.layout_path().parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::store::atomic::write_atomic(paths.layout_path(), &serde_json::to_vec_pretty(&stamp)?)?;
    Ok(())
}

/// 真正落盘。只有 `Mode::Migrate` 会复制字节与做备份；`Mode::Cleanup` 只校验后删除。
fn relocate(
    store: &Store,
    paths: &ProjectPaths,
    mode: Mode,
) -> Result<ProjectRelocationReport, AppError> {
    // 双保险：旧目录绝不能与客户端数据目录重叠，否则「删旧目录」会删到 store。
    if paths.legacy_dir().starts_with(store.root()) || store.root().starts_with(paths.legacy_dir())
    {
        return Err(AppError::InvalidInput(format!(
            "项目里的 {} 与 CC Switch X 的数据目录重叠，拒绝搬迁",
            LEGACY_DIR_RELATIVE
        )));
    }
    if !paths.legacy_dir().is_dir() {
        return Ok(ProjectRelocationReport::unchanged(
            ProjectLayoutStatus::Native,
        ));
    }

    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut report = ProjectRelocationReport::default();
    let ids = legacy_skill_ids(paths)?;

    if mode == Mode::Migrate {
        // 1. 整份快照。放在最前面，后面任何一步失败都能从这里取回原始字节。
        let snapshot = store.project_backup_dir(&paths.id, timestamp)?.join("legacy");
        std::fs::create_dir_all(snapshot.parent().unwrap_or(&snapshot))?;
        copy_dir(paths.legacy_dir(), &snapshot)?;

        // 2. 逐个复制真源。先落临时名再 rename，避免中途留下半个 skill。
        std::fs::create_dir_all(paths.canonical_root())?;
        for id in &ids {
            if !is_safe_id(id) {
                return Err(AppError::InvalidInput(format!(
                    "旧真源里的目录名不能用作 skill ID：{id}"
                )));
            }
            let source = paths.legacy_dir().join("skills").join(id);
            let source_state = project_scan::tree_state(&source)?;
            let target = paths.canonical_skill_dir(id)?;
            if target.exists() {
                // 幂等：内容一致当作已搬；不一致说明两边真有分歧，交给人裁决。
                // 只比树哈希 —— `mtime` 是冲突推荐用的观察量，不该参与「是不是同一份内容」。
                if project_scan::tree_state(&target)?.tree_hash != source_state.tree_hash {
                    return Err(AppError::Project(format!(
                        "store 里已有内容不同的 {id}，拒绝覆盖：{}",
                        target.display()
                    )));
                }
                continue;
            }
            let staging = paths
                .canonical_root()
                .join(format!(".staging-relocate-{timestamp}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&staging);
            copy_dir(&source, &staging)?;
            std::fs::rename(&staging, &target).map_err(|error| {
                let _ = std::fs::remove_dir_all(&staging);
                AppError::Io(error)
            })?;
            report.migrated.push(id.clone());
        }

        // 3. 台账：旧文件可能是 v1 或 v2，原样搬过去，让 v1→v2 迁移器继续生效。
        let legacy_ledger = paths.legacy_dir().join("skills.json");
        if legacy_ledger.is_file() {
            let ledger = project_scan::load_ledger_file(&legacy_ledger)?;
            paths
                .ledger_path()
                .parent()
                .map(std::fs::create_dir_all)
                .transpose()?;
            crate::store::atomic::write_atomic(
                paths.ledger_path(),
                &serde_json::to_vec_pretty(&ledger)?,
            )?;
        }
    }

    // 4. 写后校验：旧目录里每个 skill 都必须在 store 里有同哈希的副本。
    //    校验不过就停在这里 —— 不写戳记、不删任何东西。
    let mut verified = true;
    for id in &ids {
        let source = project_scan::tree_state(&paths.legacy_dir().join("skills").join(id))?;
        let target = project_scan::tree_state(&paths.canonical_skill_dir(id)?)?;
        if !target.present || target.tree_hash != source.tree_hash {
            if mode == Mode::Migrate {
                return Err(AppError::Project(format!(
                    "{id} 搬迁后回读校验与旧真源不一致，已保留项目里的 {}",
                    LEGACY_DIR_RELATIVE
                )));
            }
            verified = false;
            report.warnings.push(format!(
                "{} 里的 {id} 与客户端副本不一致，已保留未删",
                LEGACY_DIR_RELATIVE
            ));
        }
    }

    let entries = legacy_entries(paths);
    let foreign = foreign_entries(&entries);
    if verified && foreign.is_empty() {
        // 5. 布局戳记与删除都只在完全证实之后发生。
        write_layout(paths)?;
        match std::fs::remove_dir_all(paths.legacy_dir()) {
            Ok(()) => {
                report.legacy_removed = true;
                report.status = ProjectLayoutStatus::Native;
            }
            Err(error) => {
                report.warnings.push(format!(
                    "旧目录 {} 删除失败（store 侧数据已就绪，可手动删除）：{error}",
                    paths.legacy_dir().display()
                ));
                report.status = ProjectLayoutStatus::RedundantLegacy { entries };
            }
        }
        return Ok(report);
    }

    if !verified {
        report.status = ProjectLayoutStatus::RedundantLegacy { entries };
        return Ok(report);
    }

    // 有陌生条目：数据已就绪、戳记已写，但那个目录留给用户自己处理。
    write_layout(paths)?;
    report.warnings.push(format!(
        "{} 里还有不认识的东西，已保留未删；v3 数据已就绪",
        LEGACY_DIR_RELATIVE
    ));
    report.status = ProjectLayoutStatus::Foreign { entries: foreign };
    Ok(report)
}

/// 供界面显式清理遗留目录（`RedundantLegacy`）：内容与 store 完全一致才会删。
pub fn purge_legacy(
    store: &Store,
    paths: &ProjectPaths,
) -> Result<ProjectRelocationReport, AppError> {
    match status(paths)? {
        ProjectLayoutStatus::RedundantLegacy { .. } => relocate(store, paths, Mode::Cleanup),
        other => Err(AppError::Project(format!(
            "当前布局状态不能用这个操作清理：{other:?}"
        ))),
    }
}

/// 两边数据有分歧时，用户必须显式选一边。两条路都先备份再动手。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutChoice {
    /// 以客户端 store 为准：备份后删掉项目里的旧目录。
    Store,
    /// 以项目里的旧数据为准：备份后覆盖 store 侧，再删旧目录。
    Legacy,
}

pub fn resolve(
    store: &Store,
    paths: &ProjectPaths,
    choice: LayoutChoice,
) -> Result<ProjectRelocationReport, AppError> {
    let status = status(paths)?;
    if let ProjectLayoutStatus::Unsupported { version } = status {
        return Err(AppError::InvalidInput(format!(
            "这个项目的数据由更新版本的 CC Switch X 写入（布局版本 {version}），请升级后再打开"
        )));
    }
    if !paths.legacy_dir().is_dir() {
        return Ok(ProjectRelocationReport::unchanged(
            ProjectLayoutStatus::Native,
        ));
    }
    match choice {
        LayoutChoice::Store => keep_store(store, paths),
        LayoutChoice::Legacy => take_legacy(store, paths),
    }
}

fn snapshot_into(store: &Store, paths: &ProjectPaths, source: &Path, slot: &str) -> Result<(), AppError> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let dest = store.project_backup_dir(&paths.id, timestamp)?.join(slot);
    std::fs::create_dir_all(dest.parent().unwrap_or(&dest))?;
    copy_dir(source, &dest)?;
    Ok(())
}

/// 以 store 为准：先备份旧目录，再删。
fn keep_store(store: &Store, paths: &ProjectPaths) -> Result<ProjectRelocationReport, AppError> {
    snapshot_into(store, paths, paths.legacy_dir(), "legacy")?;
    std::fs::remove_dir_all(paths.legacy_dir())?;
    write_layout(paths)?;
    Ok(ProjectRelocationReport {
        status: ProjectLayoutStatus::Native,
        migrated: Vec::new(),
        legacy_removed: true,
        warnings: Vec::new(),
    })
}

/// 以项目里的旧数据为准：先备份 store 侧，再用旧目录重新搬一遍。
fn take_legacy(store: &Store, paths: &ProjectPaths) -> Result<ProjectRelocationReport, AppError> {
    if paths.data_dir().is_dir() {
        snapshot_into(store, paths, paths.data_dir(), "store-before-legacy")?;
    }
    let _ = std::fs::remove_dir_all(paths.canonical_root());
    let _ = std::fs::remove_file(paths.ledger_path());
    let _ = std::fs::remove_file(paths.layout_path());
    relocate(store, paths, Mode::Migrate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::ProjectSkillDirKey;

    struct Fixture {
        _temp: tempfile::TempDir,
        store: Store,
        paths: ProjectPaths,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::init(temp.path().join("store")).unwrap();
        let root = temp.path().join("home/work/app");
        std::fs::create_dir_all(&root).unwrap();
        let paths = ProjectPaths::resolve(store.root(), "app-1", &root).unwrap();
        Fixture {
            _temp: temp,
            store,
            paths,
        }
    }

    fn seed(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
        std::fs::write(dir.join("references/a.md"), "附件").unwrap();
    }

    /// 铺一个 v2 现场：项目里的真源 + 台账 + 一份渲染产物。
    fn seed_v2(fx: &Fixture, ledger_version: u32) {
        seed(&fx.paths.legacy_dir().join("skills/demo"), "真源 v1");
        seed(
            &fx.paths.root.join(".claude/skills/demo"),
            "渲染产物（不该被动）",
        );
        std::fs::create_dir_all(fx.paths.legacy_dir()).unwrap();
        std::fs::write(
            fx.paths.legacy_dir().join("skills.json"),
            format!(
                r#"{{"version":{ledger_version},"updatedAt":"","skills":{{"demo":{{"targets":["claude"]}}}}}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_fresh_project_is_native_and_untouched() {
        let fx = fixture();
        assert_eq!(status(&fx.paths).unwrap(), ProjectLayoutStatus::Native);
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert_eq!(report.migrated.len(), 0);
        assert!(!report.legacy_removed);
        assert!(!fx.paths.data_dir().exists(), "纯读不该建出数据目录");
    }

    #[test]
    fn legacy_layout_moves_the_source_and_ledger_then_removes_the_old_dir() {
        let fx = fixture();
        seed_v2(&fx, 2);
        assert_eq!(status(&fx.paths).unwrap(), ProjectLayoutStatus::Legacy);

        let report = auto(&fx.store, &fx.paths).unwrap();
        assert_eq!(report.status, ProjectLayoutStatus::Native);
        assert_eq!(report.migrated, vec!["demo".to_string()]);
        assert!(report.legacy_removed);

        // 真源与台账都到了 store，内容逐字一致。
        let canonical = fx.paths.canonical_root().join("demo");
        assert_eq!(
            std::fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "真源 v1"
        );
        assert!(canonical.join("references/a.md").is_file(), "整目录搬走");
        let ledger = project_scan::load_ledger(&fx.paths).unwrap();
        assert_eq!(ledger.skills["demo"].targets, vec!["claude".to_string()]);

        // 项目里只剩下渲染产物，旧目录消失。
        assert!(!fx.paths.legacy_dir().exists());
        assert_eq!(
            std::fs::read_to_string(fx.paths.root.join(".claude/skills/demo/SKILL.md")).unwrap(),
            "渲染产物（不该被动）"
        );

        // 备份里留有整份旧目录，出问题能取回。
        let backups = fx.store.root().join("backups/projects/app-1");
        let snapshot = std::fs::read_dir(&backups).unwrap().next().unwrap().unwrap();
        assert!(snapshot
            .path()
            .join("legacy/skills/demo/SKILL.md")
            .is_file());
    }

    #[test]
    fn relocation_is_idempotent() {
        let fx = fixture();
        seed_v2(&fx, 2);
        auto(&fx.store, &fx.paths).unwrap();
        let again = auto(&fx.store, &fx.paths).unwrap();
        assert_eq!(again.status, ProjectLayoutStatus::Native);
        assert!(again.migrated.is_empty());
        assert!(!again.legacy_removed);
    }

    /// 旧目录里有我们不认识的东西：搬走数据但**绝不删**那个目录。
    #[test]
    fn a_foreign_entry_blocks_deletion_but_not_the_migration() {
        let fx = fixture();
        seed_v2(&fx, 2);
        std::fs::write(fx.paths.legacy_dir().join("notes.md"), "我自己放的").unwrap();

        let report = auto(&fx.store, &fx.paths).unwrap();
        assert!(matches!(
            report.status,
            ProjectLayoutStatus::Foreign { .. }
        ));
        assert!(!report.legacy_removed);
        assert!(fx.paths.legacy_dir().join("notes.md").is_file());
        assert!(fx.paths.legacy_dir().join("skills/demo").is_dir(), "旧内容留在原地");
        assert!(fx.paths.canonical_root().join("demo/SKILL.md").is_file(), "数据已经搬过去");
    }

    /// 两边都有同名真源且内容不同：不自动覆盖，保持 Hybrid 等人裁决。
    #[test]
    fn conflicting_content_is_left_for_a_human_decision() {
        let fx = fixture();
        seed_v2(&fx, 2);
        seed(&fx.paths.canonical_root().join("demo"), "store 里已有");

        assert_eq!(
            status(&fx.paths).unwrap(),
            ProjectLayoutStatus::Hybrid {
                collisions: vec!["demo".to_string()]
            }
        );
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert!(matches!(
            report.status,
            ProjectLayoutStatus::Hybrid { .. }
        ));
        assert_eq!(
            std::fs::read_to_string(fx.paths.canonical_root().join("demo/SKILL.md")).unwrap(),
            "store 里已有",
            "store 侧的字节不能被覆盖"
        );
        assert!(fx.paths.legacy_dir().join("skills/demo/SKILL.md").is_file());
        assert!(!fx.paths.layout_path().exists(), "分歧时不能写布局戳记");
    }

    /// 用户选「以客户端为准」：旧目录先整份备份，再删除。
    #[test]
    fn store_wins_backs_up_the_legacy_dir_before_deleting_it() {
        let fx = fixture();
        seed_v2(&fx, 2);
        seed(&fx.paths.canonical_root().join("demo"), "store 里已有");

        let report = resolve(&fx.store, &fx.paths, LayoutChoice::Store).unwrap();
        assert_eq!(report.status, ProjectLayoutStatus::Native);
        assert!(report.legacy_removed);
        assert!(!fx.paths.legacy_dir().exists());
        assert_eq!(
            std::fs::read_to_string(fx.paths.canonical_root().join("demo/SKILL.md")).unwrap(),
            "store 里已有"
        );

        let backups = fx.store.root().join("backups/projects/app-1");
        let snapshot = std::fs::read_dir(&backups).unwrap().next().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(
                snapshot.path().join("legacy/skills/demo/SKILL.md")
            )
            .unwrap(),
            "真源 v1",
            "删除前必须留下旧目录的完整快照"
        );
    }

    /// 用户选「以项目为准」：store 侧先整份备份，再被旧数据覆盖。
    #[test]
    fn legacy_wins_replaces_the_store_copy_after_a_backup() {
        let fx = fixture();
        seed_v2(&fx, 2);
        seed(&fx.paths.canonical_root().join("demo"), "store 里已有");

        let report = resolve(&fx.store, &fx.paths, LayoutChoice::Legacy).unwrap();
        assert_eq!(report.status, ProjectLayoutStatus::Native);
        assert!(!fx.paths.legacy_dir().exists());
        assert_eq!(
            std::fs::read_to_string(fx.paths.canonical_root().join("demo/SKILL.md")).unwrap(),
            "真源 v1",
            "store 侧应被旧数据覆盖"
        );

        let backups = fx.store.root().join("backups/projects/app-1");
        let snapshot = std::fs::read_dir(&backups).unwrap().next().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(
                snapshot
                    .path()
                    .join("store-before-legacy/skills/demo/SKILL.md")
            )
            .unwrap(),
            "store 里已有",
            "覆盖前必须留下 store 侧的完整快照"
        );
    }

    /// 布局版本高于本程序：任何自动动作都必须拒绝，不能按旧语义删用户的目录。
    #[test]
    fn a_future_layout_version_blocks_everything() {
        let fx = fixture();
        seed_v2(&fx, 2);
        std::fs::create_dir_all(fx.paths.data_dir()).unwrap();
        std::fs::write(
            fx.paths.layout_path(),
            r#"{"version":9,"migratedFrom":"in-project"}"#,
        )
        .unwrap();

        assert_eq!(
            status(&fx.paths).unwrap(),
            ProjectLayoutStatus::Unsupported { version: 9 }
        );
        assert!(auto(&fx.store, &fx.paths).is_err());
        assert!(resolve(&fx.store, &fx.paths, LayoutChoice::Store).is_err());
        assert!(fx.paths.legacy_dir().is_dir(), "旧目录一个字节都不能动");
    }

    /// 崩溃残留：复制已完成、戳记未写。两侧内容一致时必须自动补完并清理。
    #[test]
    fn a_crashed_migration_with_matching_content_is_completed() {
        let fx = fixture();
        seed_v2(&fx, 2);
        // 模拟「复制完、还没写戳记」：store 侧已有同内容的真源与台账。
        seed(&fx.paths.canonical_root().join("demo"), "真源 v1");
        std::fs::create_dir_all(fx.paths.data_dir()).unwrap();
        std::fs::copy(
            fx.paths.legacy_dir().join("skills.json"),
            fx.paths.ledger_path(),
        )
        .unwrap();

        assert!(matches!(
            status(&fx.paths).unwrap(),
            ProjectLayoutStatus::Hybrid { .. }
        ));
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert_eq!(report.status, ProjectLayoutStatus::Native);
        assert!(report.legacy_removed);
        assert!(!fx.paths.legacy_dir().exists());
        assert!(fx.paths.layout_path().is_file());
    }

    /// 崩溃残留：数据已经搬完、戳记也写了，但旧目录没删干净。
    #[test]
    fn a_leftover_legacy_dir_after_a_successful_move_is_purged() {
        let fx = fixture();
        seed_v2(&fx, 2);
        auto(&fx.store, &fx.paths).unwrap();
        // 手工恢复一份一模一样的旧目录，模拟「上次删到一半」。
        seed(&fx.paths.legacy_dir().join("skills/demo"), "真源 v1");
        std::fs::copy(
            fx.paths.ledger_path(),
            fx.paths.legacy_dir().join("skills.json"),
        )
        .unwrap();

        assert!(matches!(
            status(&fx.paths).unwrap(),
            ProjectLayoutStatus::RedundantLegacy { .. }
        ));
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert!(report.legacy_removed);
        assert!(!fx.paths.legacy_dir().exists());
    }

    /// 已迁完但旧目录内容与 store 不同：必须判 Hybrid 交给用户选，
    /// 不能当成「没删干净的残留」直接清掉。
    #[test]
    fn a_divergent_leftover_after_a_successful_move_needs_a_decision() {
        let fx = fixture();
        seed_v2(&fx, 2);
        auto(&fx.store, &fx.paths).unwrap();
        seed(&fx.paths.legacy_dir().join("skills/demo"), "项目侧改过");
        std::fs::copy(
            fx.paths.ledger_path(),
            fx.paths.legacy_dir().join("skills.json"),
        )
        .unwrap();

        assert_eq!(
            status(&fx.paths).unwrap(),
            ProjectLayoutStatus::Hybrid {
                collisions: vec!["demo".to_string()]
            }
        );
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert!(matches!(
            report.status,
            ProjectLayoutStatus::Hybrid { .. }
        ));
        assert!(fx.paths.legacy_dir().join("skills/demo").is_dir(), "分歧时不能删");
        assert_eq!(
            std::fs::read_to_string(fx.paths.canonical_root().join("demo/SKILL.md")).unwrap(),
            "真源 v1"
        );

        // 用户选择项目一侧后才覆盖。
        resolve(&fx.store, &fx.paths, LayoutChoice::Legacy).unwrap();
        assert!(!fx.paths.legacy_dir().exists());
        assert_eq!(
            std::fs::read_to_string(fx.paths.canonical_root().join("demo/SKILL.md")).unwrap(),
            "项目侧改过"
        );
    }

    /// 全新项目：项目根下不能出现 `.ccswitchx`，布局戳记要落进 store。
    #[test]
    fn native_projects_get_a_layout_stamp_only_when_written_by_us() {
        let fx = fixture();
        seed(&fx.paths.root.join(".claude/skills/demo"), "副本");
        // 这里只验证判定：没有旧目录就是 Native，且 auto 不产生任何写入。
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert_eq!(report.status, ProjectLayoutStatus::Native);
        assert!(!fx.paths.layout_path().exists());
    }

    /// v1 台账原样搬进 store，交给 v1→v2 迁移器继续处理。
    #[test]
    fn a_v1_ledger_is_carried_over_untouched() {
        let fx = fixture();
        seed_v2(&fx, 1);
        let report = auto(&fx.store, &fx.paths).unwrap();
        assert_eq!(report.status, ProjectLayoutStatus::Native);
        let ledger = project_scan::load_ledger(&fx.paths).unwrap();
        assert_eq!(ledger.version, 1, "搬迁不改台账版本");
        assert!(project_scan::ledger_needs_migration(&ledger));
    }

    /// 旧目录与学生渲染目录互不影响：搬迁不碰项目里的 agent 目录。
    #[test]
    fn rendering_dirs_are_not_touched_by_relocation() {
        let fx = fixture();
        seed_v2(&fx, 2);
        let rendered = fx.paths.root.join(".qoder/skills/demo");
        seed(&rendered, "qoder 副本");
        auto(&fx.store, &fx.paths).unwrap();
        let scan = project_scan::scan(&fx.paths, &ProjectSkillDirKey::ALL).unwrap();
        assert!(scan.skills["demo"].dirs["qoder"].present);
        assert_eq!(
            std::fs::read_to_string(rendered.join("SKILL.md")).unwrap(),
            "qoder 副本"
        );
    }
}
