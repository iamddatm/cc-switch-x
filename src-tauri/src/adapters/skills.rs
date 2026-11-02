use crate::adapters::paths::expand_root;
use crate::domain::agent::AgentDescriptor;
use crate::domain::AssetType;
use crate::error::AppError;
use crate::process::hidden_command;
use crate::services::agents::save_link_kind;
use crate::store::{copy_dir, Store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// 真源更新后的分发刷新结果。链接天然指向新内容，只有 copy 需要重拷。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    Linked,
    Copied,
    Foreign,
    Absent,
}

/// 复制落地时留在目录里的归属标记：只有它才允许我们整目录删掉。
const MARKER: &str = ".ccswitchx";

pub(crate) fn skills_dir(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    Ok(expand_root(&agent.paths.skills_dir, base_override, home))
}

/// 目录本身是不是链接（symlink 或 Windows junction）。
///
/// Windows 上 `DirEntry::file_type()` 认不出 junction，必须 `symlink_metadata`
/// 加 `read_link` 双保险；项目级扫描也用这一份，别再复制第二份判定。
pub(crate) fn is_link(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_symlink() || std::fs::read_link(path).is_ok()
}

/// 链接指向的目标（原样返回，未规范化）；普通目录读不到链接就是 None。junction 同样有效。
pub(crate) fn link_target(path: &Path) -> Option<PathBuf> {
    std::fs::read_link(path).ok()
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

    // 链接能力按符号链接 → junction 依次实测；都不行才降级成 copy。
    if let Ok(kind) = link_dir(&source, &target) {
        save_link_kind(store, &agent.id, kind)?;
        return Ok(Deployed::Linked(kind));
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

/// 真源更新后刷新已落地的副本：链接不用动，copy 必须整份重拷。
/// 目标缺失时什么都不建（矩阵可能只是陈旧记录，更新不应顺手创建目录）。
pub fn refresh(
    store: &Store,
    agent: &AgentDescriptor,
    skill_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<RefreshOutcome, AppError> {
    let source = store.entry_path(AssetType::Skill, skill_id)?;
    let target = skills_dir(agent, home, base_override)?.join(skill_id);
    if target.symlink_metadata().is_err() {
        return Ok(RefreshOutcome::Absent);
    }
    if links_to(&target, &source) {
        return Ok(RefreshOutcome::Linked);
    }
    if marker_matches(&target, &source) {
        std::fs::remove_dir_all(&target)?;
        copy_dir(&source, &target)?;
        write_marker(&target, &source)?;
        return Ok(RefreshOutcome::Copied);
    }
    Ok(RefreshOutcome::Foreign)
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
    fold_path(normalize_path(left)) == fold_path(normalize_path(right))
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// 统一成可比较的字符串形态：分隔符归一，Windows 再忽略大小写。
fn fold_path(path: PathBuf) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn is_under(root: &Path, path: &Path) -> bool {
    let root = fold_path(normalize_path(root));
    let path = fold_path(normalize_path(path));
    path == root || path.starts_with(&format!("{root}/"))
}

/// 一份目录相对中央库真源的归属。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ownership {
    /// 链接指向 `<store>/assets/skills/` 底下。改名后留下的旧链接也算我们的。
    OursLink,
    /// 复制落地，`.ccswitchx` 指向一个仍然存在的真源目录。
    OursCopy,
    /// 存在，但不是我们落的：手工放的、别的工具装的、或指向别处的链接。
    Foreign,
    /// 不存在。
    Absent,
}

/// 「这一份是不是我们管理的内容」——扫描与接管用的宽判定。
///
/// 与 `deploy`/`remove`/`refresh` 里的 `links_to`/`marker_matches` 有意不同：
/// 那两个问的是「这个 ID 的目标是否正好指向它的真源」，必须精确，否则改名后的
/// 旧链接会被当成已完成；这里问的是「这块地盘上的内容归不归我们管」，
/// 只要落在真源目录下就算，免得把自家副本当成外来内容去接管。
pub(crate) fn ownership(store: &Store, dir: &Path) -> Ownership {
    if dir.symlink_metadata().is_err() {
        return Ownership::Absent;
    }
    let skills_root = store.asset_dir(AssetType::Skill);
    if let Some(target) = link_target(dir) {
        return if is_under(&skills_root, &target) {
            Ownership::OursLink
        } else {
            Ownership::Foreign
        };
    }
    let Ok(text) = std::fs::read_to_string(dir.join(MARKER)) else {
        return Ownership::Foreign;
    };
    let marked = Path::new(text.trim());
    if is_under(&skills_root, marked) && marked.join("SKILL.md").is_file() {
        return Ownership::OursCopy;
    }
    Ownership::Foreign
}

/// 清除部署标记。外来目录里的 `.ccswitchx` 记的是**别人那次部署**的绝对路径，
/// 跟着收进真源既会让新副本自称是他人的部署，也会污染目录指纹造成假 diff。
pub(crate) fn strip_marker(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(MARKER)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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

/// 把 `target` 链到 `source`：先试符号链接，再试 junction，回报实测用到的方式。
/// `deploy` 与「接管失败后还原原链接」共用这一份链接能力口径。
pub(crate) fn link_dir(source: &Path, target: &Path) -> Result<LinkKind, AppError> {
    for kind in [LinkKind::Symlink, LinkKind::Junction] {
        if try_link(kind, source, target) {
            return Ok(kind);
        }
    }
    Err(AppError::InvalidInput(format!(
        "本机既建不了符号链接也建不了 junction：{}",
        target.display()
    )))
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
    let link = for_cmd(target);
    let origin = for_cmd(source);
    hidden_command("cmd")
        .args(["/C", "mklink", "/J", &link, &origin])
        .output()
        .map(|output| output.status.success() && target.symlink_metadata().is_ok())
        .unwrap_or(false)
}

/// `expand_root` 会保留声明里的 `/`（Windows API 能吃），但 `cmd` 的 `mklink`
/// 把混用分隔符的路径判成「无效参数」，junction 就会永远悄悄降级成 copy。
/// 交给 cmd 前一律换成反斜杠。
#[cfg(windows)]
fn for_cmd(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

#[cfg(not(windows))]
fn junction(_source: &Path, _target: &Path) -> bool {
    false
}

/// 删链接：Windows 上 symlink 与 junction 都可能出现 `remove_dir` 失败，退回 `remove_file`。
/// 只能用于链接本身 —— 对 junction 递归 `remove_dir_all` 会一路删进链接指向的真身。
pub(crate) fn remove_link(target: &Path) -> Result<(), AppError> {
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
    fn refresh_recopies_our_copy_but_never_touches_foreign_directories() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let source = seed_skill(&store, "demo");
        let home = dir.path().join("home");
        let descriptor = agent("claude");
        let target = home.join(".claude/skills/demo");

        // 手工造一个我们自己的 copy 部署（不依赖 Windows 链接权限）
        copy_dir(&source, &target).unwrap();
        write_marker(&target, &source).unwrap();
        std::fs::write(source.join("SKILL.md"), "---\nname: demo\n---\n\nv2\n").unwrap();

        assert_eq!(
            refresh(&store, &descriptor, "demo", &home, None).unwrap(),
            RefreshOutcome::Copied
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "---\nname: demo\n---\n\nv2\n",
            "copy 部署必须跟着真源更新"
        );
        assert_eq!(
            std::fs::read_to_string(target.join(".ccswitchx"))
                .unwrap()
                .trim(),
            source.display().to_string()
        );

        // 外来目录：不删、不改、不认领
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "别人的\n").unwrap();
        std::fs::remove_file(target.join(".ccswitchx")).unwrap();
        assert_eq!(
            refresh(&store, &descriptor, "demo", &home, None).unwrap(),
            RefreshOutcome::Foreign
        );
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "别人的\n"
        );

        // 目标缺失：不凭空创建
        std::fs::remove_dir_all(&target).unwrap();
        assert_eq!(
            refresh(&store, &descriptor, "demo", &home, None).unwrap(),
            RefreshOutcome::Absent
        );
        assert!(!target.exists());
    }

    /// `expand_root` 会保留声明里的 `/`。Windows API 能吃这种混用分隔符的路径，
    /// 但 `cmd` 的 `mklink` 判它「无效参数」—— 只有符号链接不可用时才走 junction，
    /// 那条路不能被分隔符悄悄打死，否则这台机器永远只能 copy 部署。
    #[cfg(windows)]
    #[test]
    fn junction_accepts_the_mixed_separators_expand_root_leaves_behind() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real").join("skill");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("box").join("skills").join("skill");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        let mixed = PathBuf::from(link.to_string_lossy().replace('\\', "/"));
        assert!(mixed.to_string_lossy().contains('/'), "样本必须是混用分隔符");

        std::fs::write(real.join("SKILL.md"), "真身\n").unwrap();
        assert!(junction(&real, &mixed), "混用分隔符也要能建 junction");
        assert!(is_link(&mixed));
        assert_eq!(
            std::fs::read_to_string(mixed.join("SKILL.md")).unwrap(),
            "真身\n",
            "建好的 junction 要能读到真身内容"
        );
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
