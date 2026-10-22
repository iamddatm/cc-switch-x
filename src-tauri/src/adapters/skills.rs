use crate::adapters::paths::expand_root;
use crate::domain::agent::AgentDescriptor;
use crate::domain::AssetType;
use crate::error::AppError;
use crate::services::agents::save_link_kind;
use crate::store::{copy_dir, Store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 落地方式。`link_kind` 记的是**实测结果**而不是配置意图：Windows 上符号链接
/// 需要开发者模式或管理员权限，拿不到就得降级，UI 必须显示真正用的那种。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Symlink,
    Junction,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployed {
    /// 本次落地成功，带实测方式
    Linked(LinkKind),
    /// 已经指向我们的真源目录，什么都没做
    Already,
    /// 目标位置是别人的目录，拒绝碰它
    Foreign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    Ours(LinkKind),
    Foreign,
    Absent,
}

/// 复制落地时留在目录里的归属标记：只有它才允许我们整目录删掉。
const MARKER: &str = ".ccswitchx";

fn skills_dir(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    Ok(expand_root(&agent.paths.skills_dir, base_override, home))
}

pub fn deploy(
    store: &Store,
    agent: &AgentDescriptor,
    skill_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Deployed, AppError> {
    let source = store.entry_path(AssetType::Skill, skill_id)?;
    if !source.join("SKILL.md").is_file() {
        return Err(AppError::NotFound(format!(
            "真源里没有这个 skill：{skill_id}"
        )));
    }
    let target = skills_dir(agent, home, base_override)?.join(skill_id);

    if links_to(&target, &source) {
        return Ok(Deployed::Already);
    }
    if marker_matches(&target, &source) {
        return Ok(Deployed::Already);
    }
    if target.symlink_metadata().is_ok() {
        return Ok(Deployed::Foreign);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    for kind in [LinkKind::Symlink, LinkKind::Junction] {
        if try_link(kind, &source, &target) {
            save_link_kind(store, &agent.id, kind)?;
            return Ok(Deployed::Linked(kind));
        }
    }
    copy_dir(&source, &target)?;
    write_marker(&target, &source)?;
    save_link_kind(store, &agent.id, LinkKind::Copy)?;
    Ok(Deployed::Linked(LinkKind::Copy))
}

pub fn remove(
    store: &Store,
    agent: &AgentDescriptor,
    skill_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Removed, AppError> {
    let source = store.entry_path(AssetType::Skill, skill_id)?;
    let target = skills_dir(agent, home, base_override)?.join(skill_id);
    if target.symlink_metadata().is_err() {
        return Ok(Removed::Absent);
    }
    if links_to(&target, &source) {
        remove_link(&target)?;
        return Ok(Removed::Ours(LinkKind::Symlink));
    }
    if marker_matches(&target, &source) {
        std::fs::remove_dir_all(&target)?;
        return Ok(Removed::Ours(LinkKind::Copy));
    }
    Ok(Removed::Foreign)
}

/// 链接是否指向我们的真源目录。`read_link` 对 symlink 与 junction 都有效。
fn links_to(target: &Path, source: &Path) -> bool {
    let Ok(link) = std::fs::read_link(target) else {
        return false;
    };
    same_path(&link, source)
}

fn marker_matches(target: &Path, source: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(target.join(MARKER)) else {
        return false;
    };
    same_path(Path::new(text.trim()), source)
}

fn same_path(left: &Path, right: &Path) -> bool {
    let normalize =
        |path: &Path| -> PathBuf { path.canonicalize().unwrap_or_else(|_| path.to_path_buf()) };
    let (left, right) = (normalize(left), normalize(right));
    let fold = |value: &Path| -> String {
        let text = value.to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    fold(&left) == fold(&right)
}

fn write_marker(target: &Path, source: &Path) -> Result<(), AppError> {
    std::fs::write(target.join(MARKER), format!("{}\n", source.display()))?;
    Ok(())
}

fn try_link(kind: LinkKind, source: &Path, target: &Path) -> bool {
    match kind {
        LinkKind::Copy => false,
        LinkKind::Symlink => symlink_dir(source, target),
        LinkKind::Junction => junction(source, target),
    }
}

#[cfg(windows)]
fn symlink_dir(source: &Path, target: &Path) -> bool {
    std::os::windows::fs::symlink_dir(source, target).is_ok()
}

#[cfg(not(windows))]
fn symlink_dir(source: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(source, target).is_ok()
}

/// junction 没有 std 支持，只能借 `mklink /J`；报告里注明来源是命令。
#[cfg(windows)]
fn junction(source: &Path, target: &Path) -> bool {
    let link = target.to_string_lossy().to_string();
    let origin = source.to_string_lossy().to_string();
    Command::new("cmd")
        .args(["/C", "mklink", "/J", &link, &origin])
        .output()
        .map(|output| output.status.success() && target.symlink_metadata().is_ok())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn junction(_source: &Path, _target: &Path) -> bool {
    false
}

/// 删链接：Windows 上 symlink 与 junction 都可能出现 `remove_dir` 失败，退回 `remove_file`。
fn remove_link(target: &Path) -> Result<(), AppError> {
    match std::fs::remove_dir(target) {
        Ok(()) => Ok(()),
        Err(first) => std::fs::remove_file(target)
            .map_err(|_| AppError::Io(first))
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::agents::load_state;
    use tempfile::tempdir;

    fn agent(id: &str) -> AgentDescriptor {
        crate::domain::agent::builtin_agents()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap()
    }

    /// 真源里造一个 skill 目录
    fn seed_skill(store: &Store, id: &str) -> PathBuf {
        let dir = store.asset_dir(AssetType::Skill).join(id);
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: demo\n---\n\n正文\n").unwrap();
        std::fs::write(dir.join("references/note.md"), "补充\n").unwrap();
        dir
    }

    #[test]
    fn deploy_lands_the_skill_and_records_the_kind_it_actually_used() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let source = seed_skill(&store, "demo");
        let home = dir.path().join("home");
        let descriptor = agent("claude");

        let deployed = deploy(&store, &descriptor, "demo", &home, None).unwrap();
        let kind = match deployed {
            Deployed::Linked(kind) => kind,
            other => panic!("首次落地不该是 {other:?}"),
        };
        let target = home.join(".claude/skills").join("demo");
        assert!(
            target.join("SKILL.md").is_file(),
            "按 {kind:?} 落地后要能读到 SKILL.md"
        );
        assert!(
            target.join("references/note.md").is_file(),
            "子目录要一起可用"
        );
        assert!(source.is_dir(), "复制不移动：真源目录必须还在");

        // 记录的必须是实测用到的方式，不能把 copy 报成 symlink
        let state = load_state(&store, "claude").unwrap();
        assert_eq!(state.link_kind, Some(kind));
    }

    #[test]
    fn deploying_twice_changes_nothing() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        seed_skill(&store, "demo");
        let home = dir.path().join("home");
        let descriptor = agent("claude");

        deploy(&store, &descriptor, "demo", &home, None).unwrap();
        let target = home.join(".claude/skills").join("demo");
        let before = std::fs::read_to_string(target.join("SKILL.md")).unwrap();
        assert_eq!(
            deploy(&store, &descriptor, "demo", &home, None).unwrap(),
            Deployed::Already
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            before
        );
    }

    #[test]
    fn deploy_refuses_to_touch_someone_elses_directory() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        seed_skill(&store, "demo");
        let home = dir.path().join("home");
        let foreign = home.join(".claude/skills/demo");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("SKILL.md"), "别人写的\n").unwrap();

        let deployed = deploy(&store, &agent("claude"), "demo", &home, None).unwrap();
        assert_eq!(deployed, Deployed::Foreign);
        assert_eq!(
            std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
            "别人写的\n",
            "非自家目录必须一个字节都不动"
        );
    }

    #[test]
    fn remove_deletes_ours_and_keeps_foreign() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        seed_skill(&store, "demo");
        let home = dir.path().join("home");
        let descriptor = agent("claude");

        deploy(&store, &descriptor, "demo", &home, None).unwrap();
        let ours = home.join(".claude/skills/demo");
        assert!(matches!(
            remove(&store, &descriptor, "demo", &home, None).unwrap(),
            Removed::Ours(_)
        ));
        assert!(!ours.exists());
        assert!(
            store.asset_dir(AssetType::Skill).join("demo").is_dir(),
            "卸载分发不能把真源删了"
        );

        std::fs::create_dir_all(&ours).unwrap();
        std::fs::write(ours.join("SKILL.md"), "别人的\n").unwrap();
        assert_eq!(
            remove(&store, &descriptor, "demo", &home, None).unwrap(),
            Removed::Foreign
        );
        assert!(ours.join("SKILL.md").is_file());
    }

    #[test]
    fn deploying_an_unknown_skill_fails_loudly() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let error = deploy(
            &store,
            &agent("claude"),
            "nope",
            &dir.path().join("home"),
            None,
        )
        .expect_err("真源里没有的 skill 不能凭空部署");
        assert!(error.to_string().contains("nope"), "{error}");
    }

    #[test]
    fn an_override_root_relocates_the_deployment() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        seed_skill(&store, "demo");
        let sandbox = dir.path().join("sandbox");

        deploy(
            &store,
            &agent("claude"),
            "demo",
            &dir.path().join("real-home"),
            Some(&sandbox),
        )
        .unwrap();
        assert!(sandbox.join(".claude/skills/demo/SKILL.md").is_file());
        assert!(
            !dir.path().join("real-home").exists(),
            "绝不能写到真实 home"
        );
    }
}
