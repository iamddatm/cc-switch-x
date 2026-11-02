//! 扫描各 agent 的 skills 落点，找出**不是我们落的** Skill（对齐上游 cc-switch 的
//! 「未纳管发现」，见 `references/cc-switch/src-tauri/src/services/skill.rs` 的 `scan_unmanaged`）。
//!
//! 上游拿 SQLite 的已装列表当纳管集合；我们的真源是文本目录，所以纳管集合就是
//! `<store>/assets/skills/` 本身，磁盘归属沿用 `adapters::skills` 那一份判定
//! （链接落在真源目录下，或 copy 部署留下的 `.ccswitchx` 命中真源）。
//!
//! 这里**只读**：不改任何字节、不写索引 —— 未纳管清单是每次现算的派生数据。
//! 落盘一律走 `services::skill_takeover`。

use crate::adapters::skills::{self, Ownership};
use crate::domain::{describe, AssetKind, AssetType};
use crate::error::AppError;
use crate::import::slug;
use crate::net::archive;
use crate::services::agents;
use crate::store::Store;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// 一份未纳管副本被发现的位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundLocation {
    /// agent id；`None` 是跨工具共享目录，我们没有往它写过的落点。
    pub source_id: Option<String>,
    pub label: String,
    /// 打开文件用的原始路径；`path` 只是显示串。
    #[serde(skip)]
    pub source: PathBuf,
    pub path: String,
    /// 这一份是链接时给出指向，只为说明来历：接管只会解链，绝不删链接指向的内容。
    pub link_target: Option<String>,
    /// 能否接管并启用给这一处。以能力闸 `switchable(Skill)` 为准，未实测的落点只能收进真源。
    pub takeover_possible: bool,
}

/// 一个候选 Skill（按目录名跨根合并成一条）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedSkill {
    /// 磁盘上的目录名，导入定位以它为唯一入口。
    pub directory: String,
    /// 导入后会变成的真源 ID。
    pub suggested_id: String,
    pub name: String,
    pub description: Option<String>,
    /// 先被发现的那份的路径，导入的内容以它为准。
    #[serde(skip)]
    pub source: PathBuf,
    pub path: String,
    pub found_in: Vec<FoundLocation>,
    /// `suggested_id != directory`：目录名含非法字符，导入会改名，UI 要先讲清楚。
    pub id_renamed: bool,
}

/// 扫到但没当候选的东西，逐条点名，不静默丢（与 `services::scan` 的 skipped 同一纪律）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDir {
    pub dir: String,
    pub reason: String,
}

/// 扫了哪些根，让 UI 能回答「为什么这里没东西」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedRoot {
    pub source_id: Option<String>,
    pub label: String,
    pub path: String,
    pub exists: bool,
    /// 该 agent 走了 `baseOverride` 沙箱。
    pub sandboxed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedSkillReport {
    /// 可以直接收进真源的。
    pub skills: Vec<UnmanagedSkill>,
    /// 真源里已有同名 ID、但盘上这份不是我们落的：需要用户裁决，不能静默覆盖。
    pub conflicts: Vec<UnmanagedSkill>,
    pub skipped: Vec<SkippedDir>,
    pub roots: Vec<ScannedRoot>,
    pub warnings: Vec<String>,
}

impl UnmanagedSkillReport {
    /// 两个列表都按目录名找得到（导入侧的唯一入口）。
    pub fn find(&self, directory: &str) -> Option<&UnmanagedSkill> {
        self.skills
            .iter()
            .chain(self.conflicts.iter())
            .find(|entry| entry.directory == directory)
    }

    /// 真源里已有同名 ID 的那一类：只允许另存为新 ID，永不接管盘上那份。
    pub fn is_conflict(&self, directory: &str) -> bool {
        self.conflicts
            .iter()
            .any(|entry| entry.directory == directory)
    }
}

/// 一个待扫的根。
struct ScanRoot {
    source_id: Option<String>,
    label: String,
    path: PathBuf,
    /// 我们是否会往这个根写（= 该 agent 的 skills 落点已实测）。只有它可能被接管。
    takeover_possible: bool,
    sandboxed: bool,
}

/// 一个目录相对我们的结论。
enum Verdict {
    /// 已经是我们的内容，不出现在任何列表里。
    Ours,
    Candidate { skill: UnmanagedSkill, conflict: bool },
    /// 不是候选，附原因。
    Skip(String),
}

/// 跨工具约定的 `~/.agents/skills`：注册表里没有这个落点，我们从不往它写，
/// 所以它只是只读发现根，永远不会成为接管目标。
fn shared_root(home: &Path) -> PathBuf {
    home.join(".agents").join("skills")
}

/// 要扫的根：注册表里每个有 skills 落点的 agent（各自解析过 `baseOverride`），外加共享目录。
///
/// 只卡 `supported`（有没有这个落点），不卡 `switchable`：只读发现不受能力闸限制，
/// 与 `services::scan::scan_assets` 同一口径；写盘侧照旧卡 `switchable`。
fn roots(store: &Store, home: &Path) -> Result<Vec<ScanRoot>, AppError> {
    let mut out = Vec::new();
    for status in agents::detect_all(store, home)? {
        if !status.supported(AssetKind::Skill) {
            continue;
        }
        out.push(ScanRoot {
            source_id: Some(status.id.clone()),
            label: status.display_name.clone(),
            path: status.skills_dir.clone(),
            takeover_possible: status.switchable(AssetKind::Skill),
            sandboxed: agents::load_state(store, &status.id)?
                .base_override
                .is_some(),
        });
    }
    out.push(ScanRoot {
        source_id: None,
        label: ".agents/skills".to_string(),
        path: shared_root(home),
        takeover_possible: false,
        sandboxed: false,
    });
    Ok(out)
}

/// 扫全部落点。单个根读不动只记警告，不影响其它根。
pub fn scan(store: &Store, home: &Path) -> Result<UnmanagedSkillReport, AppError> {
    let managed: HashSet<String> = store.list_ids(AssetType::Skill)?.into_iter().collect();
    let mut report = UnmanagedSkillReport::default();
    // BTreeMap 按目录名合并同一个 skill 的多份副本，顺带给稳定的展示顺序。
    let mut skills: BTreeMap<String, UnmanagedSkill> = BTreeMap::new();
    let mut conflicts: BTreeMap<String, UnmanagedSkill> = BTreeMap::new();
    let mut fingerprints: BTreeMap<String, Option<String>> = BTreeMap::new();

    for root in roots(store, home)? {
        let exists = root.path.is_dir();
        report.roots.push(ScannedRoot {
            source_id: root.source_id.clone(),
            label: root.label.clone(),
            path: display_path(&root.path),
            exists,
            sandboxed: root.sandboxed,
        });
        if !exists {
            continue;
        }
        let read = match std::fs::read_dir(&root.path) {
            Ok(read) => read,
            Err(error) => {
                report
                    .warnings
                    .push(format!("读不了 {}：{error}", display_path(&root.path)));
                continue;
            }
        };
        let mut entries = read.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            // 隐藏目录不是 skill（`.git`、`.DS_Store`、编辑器残留），与上游同一口径。
            if name.starts_with('.') {
                continue;
            }
            let path = root.path.join(&name);
            let (mut skill, conflict) = match classify(store, &managed, &name, &path) {
                Verdict::Ours => continue,
                Verdict::Skip(reason) => {
                    report.skipped.push(SkippedDir {
                        dir: format!("{name} @ {}", root.label),
                        reason,
                    });
                    continue;
                }
                Verdict::Candidate { skill, conflict } => (skill, conflict),
            };
            let location = FoundLocation {
                source_id: root.source_id.clone(),
                label: root.label.clone(),
                source: path.clone(),
                path: display_path(&path),
                link_target: skills::link_target(&path)
                    .map(|target| display_path(&target.canonicalize().unwrap_or(target))),
                takeover_possible: root.takeover_possible,
            };
            let bucket = if conflict { &mut conflicts } else { &mut skills };
            match bucket.get_mut(&name) {
                Some(existing) => {
                    if ambiguous(
                        fingerprints.get(&name).and_then(|value| value.as_ref()),
                        &path,
                    ) {
                        let previous = existing
                            .found_in
                            .first()
                            .map(|first| first.label.as_str())
                            .unwrap_or("先前");
                        report.warnings.push(format!(
                            "{name} 在 {previous} 与 {} 的内容不一致，只导入先发现的那份",
                            location.label
                        ));
                    }
                    existing.found_in.push(location);
                }
                None => {
                    fingerprints.insert(name.clone(), archive::compute_dir_hash(&path).ok());
                    skill.found_in.push(location);
                    bucket.insert(name, skill);
                }
            }
        }
    }

    report.skills = skills.into_values().collect();
    report.conflicts = conflicts.into_values().collect();
    Ok(report)
}

/// 「这一份归不归我们管」的唯一判定口径，接管侧复用。
fn classify(store: &Store, managed: &HashSet<String>, name: &str, path: &Path) -> Verdict {
    let skill_md = path.join("SKILL.md");
    match skills::ownership(store, path) {
        // 指向真源的链接即使是坏链（真源那份已被删）也算我们的：它是「启用但内容缺失」，
        // 归矩阵与漂移那边说，不是外来内容。
        Ownership::OursLink | Ownership::OursCopy => return Verdict::Ours,
        Ownership::Absent => return Verdict::Skip("目录已不存在".to_string()),
        Ownership::Foreign => {}
    }
    if skills::is_link(path) {
        if !skill_md.is_file() {
            return Verdict::Skip("链接指向的不是含 SKILL.md 的目录".to_string());
        }
    } else if !path.is_dir() {
        return Verdict::Skip("不是目录".to_string());
    } else if !skill_md.is_file() {
        return Verdict::Skip("目录里没有 SKILL.md".to_string());
    }
    let Some(suggested_id) = slug(name) else {
        return Verdict::Skip("目录名推不出合法 ID".to_string());
    };
    let conflict = managed.contains(&suggested_id);
    Verdict::Candidate {
        skill: read_summary(name, path, &suggested_id),
        conflict,
    }
}

/// 名称与描述走 `domain::describe` —— 与列表页补展示字段同一口径，不另写 frontmatter 解析。
fn read_summary(name: &str, dir: &Path, suggested_id: &str) -> UnmanagedSkill {
    let bytes = std::fs::read(dir.join("SKILL.md")).unwrap_or_default();
    let details = describe(AssetType::Skill, suggested_id, &bytes);
    let trimmed = |value: Option<String>| {
        value
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    };
    UnmanagedSkill {
        directory: name.to_string(),
        suggested_id: suggested_id.to_string(),
        name: trimmed(details.name).unwrap_or_else(|| name.to_string()),
        description: trimmed(details.description),
        source: dir.to_path_buf(),
        path: display_path(dir),
        found_in: Vec::new(),
        id_renamed: suggested_id != name,
    }
}

/// 同名两份内容是否不同（目录指纹不同）。拿不到指纹时不报歧义，免得把噪音当问题。
fn ambiguous(previous: Option<&String>, path: &Path) -> bool {
    match (previous, archive::compute_dir_hash(path).ok()) {
        (Some(first), Some(second)) => *first != second,
        _ => false,
    }
}

/// Windows 上 `canonicalize` 会加 `\\?\` 前缀，那是路径内核写法，不能给用户看。
/// 判定与比较一律用 `Path`，只有回前端的显示串走这里。
pub(crate) fn display_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .trim_start_matches("//./")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::services::agents::save_override;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        store: Store,
        home: PathBuf,
    }

    /// 每个 agent 的 skills 落点都 `baseOverride` 到 `home` 下，
    /// 真实 `~/.claude` 之类绝不能被读或被写。
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
            let descriptor = descriptor(agent);
            skills::skills_dir(&descriptor, &self.home, Some(&self.home)).unwrap()
        }

        /// 往某 agent 的 skills 落点放一份手工 skill。
        fn seed_foreign(&self, agent: &str, name: &str, body: &str) -> PathBuf {
            let dir = self.skills_dir(agent).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
            dir
        }

        fn scan(&self) -> UnmanagedSkillReport {
            scan(&self.store, &self.home).unwrap()
        }
    }

    fn descriptor(id: &str) -> crate::domain::agent::AgentDescriptor {
        builtin_agents()
            .into_iter()
            .find(|agent| agent.id == id)
            .unwrap()
    }

    fn seed_canonical(store: &Store, id: &str) {
        let dir = store.asset_dir(AssetType::Skill).join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {id}\n---\n\n真源正文\n"),
        )
        .unwrap();
    }

    const SKILL: &str = "---\nname: 演示\ndescription: 说明文字\n---\n\n正文\n";

    #[test]
    fn a_foreign_directory_with_skill_md_becomes_a_candidate() {
        let fx = fixture();
        let dir = fx.seed_foreign("claude", "hand-made", SKILL);

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{report:?}");
        let skill = &report.skills[0];
        assert_eq!(skill.directory, "hand-made");
        assert_eq!(skill.suggested_id, "hand-made");
        assert_eq!(skill.name, "演示");
        assert_eq!(skill.description.as_deref(), Some("说明文字"));
        assert_eq!(skill.found_in.len(), 1);
        assert_eq!(skill.found_in[0].source_id.as_deref(), Some("claude"));
        assert!(skill.found_in[0].takeover_possible, "claude 的 skills 已实测");
        assert_eq!(skill.path, display_path(&dir));
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    }

    #[test]
    fn our_own_deployment_is_never_a_candidate() {
        let fx = fixture();
        let id = "deployed";
        seed_canonical(&fx.store, id);

        let deployed = skills::deploy(&fx.store, &descriptor("claude"), id, &fx.home, Some(&fx.home))
            .expect("自家真源要能落地");
        assert!(
            matches!(deployed, skills::Deployed::Linked(_)),
            "首次落地应当成功：{deployed:?}"
        );

        let report = fx.scan();
        assert!(
            report.skills.is_empty(),
            "自家部署不能当外来内容：{:?}",
            report.skills
        );
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    }

    #[test]
    fn our_own_copy_deployment_with_marker_is_not_a_candidate_either() {
        let fx = fixture();
        let id = "copied";
        seed_canonical(&fx.store, id);
        // 手工造一份 copy 部署（不依赖本机链接权限），带指向真源的 .ccswitchx。
        let source = fx.store.entry_path(AssetType::Skill, id).unwrap();
        let target = fx.skills_dir("claude").join(id);
        crate::store::copy_dir(&source, &target).unwrap();
        std::fs::write(target.join(".ccswitchx"), format!("{}\n", source.display())).unwrap();

        let report = fx.scan();
        assert!(report.skills.is_empty(), "{:?}", report.skills);
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
    }

    #[test]
    fn same_name_as_the_source_of_truth_but_not_ours_lands_in_conflicts() {
        let fx = fixture();
        seed_canonical(&fx.store, "twin");
        let foreign = fx.seed_foreign("claude", "twin", "---\nname: 别人的\n---\n\n别人的正文\n");

        let report = fx.scan();
        assert!(report.skills.is_empty(), "{:?}", report.skills);
        assert_eq!(report.conflicts.len(), 1, "{:?}", report.conflicts);
        assert_eq!(report.conflicts[0].directory, "twin");
        // 冲突只报告，外来那份必须一个字节都不动。
        assert_eq!(
            std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
            "---\nname: 别人的\n---\n\n别人的正文\n"
        );
    }

    #[test]
    fn things_that_are_not_skills_are_named_with_a_reason() {
        let fx = fixture();
        let root = fx.skills_dir("claude");
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/plain.md"), "没有 SKILL.md\n").unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden/SKILL.md"), SKILL).unwrap();
        std::fs::write(root.join("stray.md"), "散文件\n").unwrap();
        std::fs::create_dir_all(root.join("好")).unwrap();
        std::fs::write(root.join("好/SKILL.md"), SKILL).unwrap();

        let report = fx.scan();
        assert!(report.skills.is_empty(), "{:?}", report.skills);
        let skipped: Vec<&str> = report
            .skipped
            .iter()
            .map(|skip| skip.dir.split(" @ ").next().unwrap())
            .collect();
        assert!(skipped.contains(&"notes"), "{skipped:?}");
        assert!(skipped.contains(&"stray.md"), "{skipped:?}");
        assert!(skipped.contains(&"好"), "纯中文目录名推不出 ID：{skipped:?}");
        assert!(
            !skipped.contains(&".hidden"),
            "隐藏目录按约定直接忽略，不进 skipped：{skipped:?}"
        );
        assert!(
            report.skipped.iter().any(|skip| skip.dir == "好 @ Claude Code"
                && skip.reason.contains("合法 ID")),
            "{:?}",
            report.skipped
        );
    }

    #[test]
    fn one_skill_found_in_several_agents_merges_into_a_single_entry() {
        let fx = fixture();
        fx.seed_foreign("claude", "shared", SKILL);
        fx.seed_foreign("codex", "shared", SKILL);

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert_eq!(report.skills[0].found_in.len(), 2);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn two_different_copies_of_one_name_warn_but_still_offer_the_first() {
        let fx = fixture();
        let first = fx.seed_foreign("claude", "diverged", "---\nname: 第一份\n---\n\nA\n");
        fx.seed_foreign("codex", "diverged", "---\nname: 第二份\n---\n\nB\n");

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert_eq!(report.skills[0].found_in.len(), 2);
        assert_eq!(
            report.skills[0].path,
            display_path(&first),
            "内容不一致时只认先发现的那份"
        );
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains("不一致"));
    }

    #[test]
    fn an_unverified_landing_spot_is_still_discovered_but_cannot_be_taken_over() {
        let fx = fixture();
        // gemini 的 skills 落点在注册表里 supported 但未实测（C 档）。
        fx.seed_foreign("gemini", "from-gemini", SKILL);

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert!(
            !report.skills[0].found_in[0].takeover_possible,
            "未实测的落点不能成为接管目标：{:?}",
            report.skills[0].found_in
        );
    }

    #[test]
    fn the_shared_agents_directory_is_discovered_but_never_taken_over() {
        let fx = fixture();
        let dir = fx.home.join(".agents/skills/cross-tool");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), SKILL).unwrap();

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert_eq!(report.skills[0].found_in[0].source_id, None);
        assert!(!report.skills[0].found_in[0].takeover_possible);
    }

    #[test]
    fn a_skill_linked_outside_the_store_is_a_foreign_candidate() {
        let fx = fixture();
        // 用户自己把别处的 skill 链进来：目标不在真源目录下，属未纳管。
        let real = fx.home.join("elsewhere/my-skill");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("SKILL.md"), SKILL).unwrap();
        let link = fx.skills_dir("claude").join("my-skill");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        make_link(&real, &link);

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert!(
            report.skills[0].found_in[0].link_target.is_some(),
            "来历要能从徽标看出来：{:?}",
            report.skills[0].found_in
        );
    }

    #[test]
    fn scanning_never_touches_the_bytes_it_reports() {
        let fx = fixture();
        let dir = fx.seed_foreign("claude", "readonly", SKILL);
        let before = archive::compute_dir_hash(&dir).unwrap();

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert_eq!(
            archive::compute_dir_hash(&dir).unwrap(),
            before,
            "扫描必须一个字节都不改"
        );
    }

    #[test]
    fn a_missing_root_is_reported_as_scanned_but_empty() {
        let fx = fixture();
        let report = fx.scan();
        assert!(report.skills.is_empty(), "{:?}", report.skills);
        assert!(
            report
                .roots
                .iter()
                .any(|root| root.source_id.as_deref() == Some("claude") && !root.exists),
            "扫了哪些根要说得出来：{:?}",
            report.roots
        );
        assert!(
            report
                .roots
                .iter()
                .filter(|root| root.source_id.is_some())
                .all(|root| root.sandboxed),
            "每个 agent 根都该标明走的是沙箱：{:?}",
            report.roots
        );
    }

    #[test]
    fn a_directory_name_needing_a_slug_is_reported_as_renamed() {
        let fx = fixture();
        fx.seed_foreign("claude", "my skill", SKILL);

        let report = fx.scan();
        assert_eq!(report.skills.len(), 1, "{:?}", report.skills);
        assert_eq!(report.skills[0].suggested_id, "my-skill");
        assert!(report.skills[0].id_renamed);
    }

    /// 用生产那套链接能力造一份外来链接：本机允许符号链接就用符号链接，
    /// 只允许 junction 就用 junction —— 判定口径与被测代码保持一致。
    fn make_link(real: &Path, link: &Path) {
        skills::link_dir(real, link).expect("建链接应当成功");
        assert!(skills::is_link(link), "链接必须被 is_link 认出来");
    }
}
