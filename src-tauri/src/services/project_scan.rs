use crate::domain::project::{
    ProjectDirState, ProjectLedger, ProjectPaths, ProjectSkillDirKey, ProjectSkillEntry,
    PROJECT_LEDGER_VERSION, PROJECT_SKILL_DIRS,
};
use crate::error::AppError;
use crate::index::sha256_hex;
use crate::store::atomic::write_atomic;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSkillSnapshot {
    pub id: String,
    pub dirs: BTreeMap<String, ProjectDirState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectScan {
    pub skills: BTreeMap<String, ProjectSkillSnapshot>,
    /// 项目真源（客户端数据目录里的 `skills/<id>/`）的当前状态；真源不是落点，不进 dirs key 空间。
    pub canonical: BTreeMap<String, ProjectDirState>,
    pub dirs: Vec<ProjectSkillDirKey>,
    pub warnings: Vec<String>,
    /// 基目录本身是链接/junction 而被跳过的目录 key：该 key 下所有 skill 状态都未知。
    pub unresolved_dirs: Vec<String>,
    /// 因链接被跳过的 (目录 key, skill id)：这些格子状态未知，
    /// 绝不能按「不存在」参与差异判定，否则会把别的健康副本判成删除。
    pub unresolved_skills: Vec<(String, String)>,
    /// 因链接或非法 ID 而无法取到状态的真源 skill id：同样不得按「不存在」判定。
    pub unresolved_canonical: Vec<String>,
}

impl ProjectScan {
    /// 某目录 key 下某 skill 的当前状态是否不可解析（链接跳过或整个目录是链接）。
    pub fn blocks(&self, dir: &str, skill_id: &str) -> bool {
        self.unresolved_dirs.iter().any(|key| key == dir)
            || self
                .unresolved_skills
                .iter()
                .any(|(key, id)| key == dir && id == skill_id)
    }

    /// 真源状态是否不可解析：不可解析时禁止渲染，也禁止据此判定「真源已删除」。
    pub fn blocks_canonical(&self, skill_id: &str) -> bool {
        self.unresolved_canonical
            .iter()
            .any(|id| id == skill_id)
    }
}

/// 把 Settings 里启用的 agent 映射到项目物理目录；同一目录只保留一次。
pub fn enabled_dir_keys(enabled_agent_ids: &[String]) -> Vec<ProjectSkillDirKey> {
    let mut keys = Vec::new();
    for spec in &PROJECT_SKILL_DIRS {
        let enabled = spec
            .agent_ids
            .iter()
            .any(|id| enabled_agent_ids.iter().any(|enabled| enabled == id));
        if enabled && !keys.contains(&spec.key) {
            keys.push(spec.key);
        }
    }
    keys
}

/// 目录树指纹：排序后的相对路径 + 每个文件 SHA-256，再对整份清单求 SHA-256。
/// 路径统一用 `/`，保证 Windows 与 Unix 的清单一致。
pub fn tree_state(dir: &Path) -> Result<ProjectDirState, AppError> {
    if !dir.exists() {
        return Ok(ProjectDirState::default());
    }
    if is_link(dir) {
        return Err(AppError::InvalidInput(format!(
            "拒绝符号链接目录：{}",
            dir.display()
        )));
    }

    let mut files = BTreeMap::new();
    let mut mtime = 0_i64;
    collect_files(dir, dir, &mut files, &mut mtime)?;

    let mut manifest = String::new();
    for (path, hash) in &files {
        manifest.push_str(path);
        manifest.push('\0');
        manifest.push_str(hash);
        manifest.push('\n');
    }
    let tree_hash = format!("sha256:{}", sha256_hex(manifest.as_bytes()));
    Ok(ProjectDirState {
        present: true,
        tree_hash: Some(tree_hash),
        files,
        mtime,
    })
}

fn collect_files(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, String>,
    mtime: &mut i64,
) -> Result<(), AppError> {
    let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        // 点开头条目不计入指纹：`.DS_Store`、编辑器临时文件、`.ccswitchx` 标记都不该
        // 让一个 skill 永远显示「内容不一致」并因此被反复重渲染。
        // 这也是与 `net::archive::compute_dir_hash` 对齐的口径。
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if is_link(&path) {
            return Err(AppError::InvalidInput(format!(
                "拒绝符号链接：{}",
                path.display()
            )));
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(root, &path, files, mtime)?;
        } else if file_type.is_file() {
            if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                if let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH) {
                    *mtime = (*mtime).max(elapsed.as_millis() as i64);
                }
            }
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, sha256_hex(&std::fs::read(&path)?));
        }
    }
    Ok(())
}

/// 普通目录的 `read_link` 会失败；Windows junction 能读到目标，必须一并拒绝。
pub(crate) fn is_link(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_symlink() || std::fs::read_link(path).is_ok()
}

pub fn scan(paths: &ProjectPaths, keys: &[ProjectSkillDirKey]) -> Result<ProjectScan, AppError> {
    let mut skills: BTreeMap<String, ProjectSkillSnapshot> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut unresolved_dirs: Vec<String> = Vec::new();
    let mut unresolved_skills: Vec<(String, String)> = Vec::new();
    let mut unresolved_canonical: Vec<String> = Vec::new();

    for key in keys {
        let base = paths.render_dir(*key);
        if is_link(&base) {
            warnings.push(format!("跳过符号链接：{}", base.display()));
            unresolved_dirs.push(key.as_str().to_string());
            continue;
        }
        let entries = match std::fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if is_link(&path) {
                warnings.push(format!("跳过符号链接：{}", path.display()));
                mark_unresolved(&mut unresolved_skills, key.as_str(), &name);
                continue;
            }
            if !entry.file_type()?.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let state = match tree_state(&path) {
                Ok(state) => state,
                Err(error) => {
                    warnings.push(format!("跳过包含符号链接的 skill {name}：{error}"));
                    mark_unresolved(&mut unresolved_skills, key.as_str(), &name);
                    continue;
                }
            };
            skills
                .entry(name.clone())
                .or_insert_with(|| ProjectSkillSnapshot {
                    id: name.clone(),
                    dirs: BTreeMap::new(),
                })
                .dirs
                .insert(key.as_str().to_string(), state);
        }
    }

    for snapshot in skills.values_mut() {
        for key in keys {
            snapshot
                .dirs
                .entry(key.as_str().to_string())
                .or_default();
        }
    }

    let canonical = scan_canonical(paths)?;
    warnings.extend(canonical.warnings);
    unresolved_canonical.extend(canonical.unresolved);

    Ok(ProjectScan {
        skills,
        canonical: canonical.states,
        dirs: keys.to_vec(),
        warnings,
        unresolved_dirs,
        unresolved_skills,
        unresolved_canonical,
    })
}

fn mark_unresolved(unresolved: &mut Vec<(String, String)>, dir_key: &str, skill_id: &str) {
    let pair = (dir_key.to_string(), skill_id.to_string());
    if !unresolved.contains(&pair) {
        unresolved.push(pair);
    }
}

/// 台账与真源在**项目数据根**（`<storeRoot>/local/projects/<project-id>/`）里的相对位置。
/// 只给 UI 展示「会新建/重写哪些文件」用，绝不暴露绝对路径；v3 起这些字节不在用户仓库里，
/// 所以这里不再是 `.ccswitchx/…` 那种看起来像仓库路径的串。
pub const LEDGER_DISPLAY: &str = "skills.json";
pub const CANONICAL_DISPLAY: &str = "skills";

/// 真源目录的扫描结果。真源不是落点，所以单独扫，不进 `dirs` 的 key 空间。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CanonicalScan {
    pub states: BTreeMap<String, ProjectDirState>,
    pub warnings: Vec<String>,
    /// 状态取不到（链接、或目录名不是合法 ID）的 skill id：不得按「不存在」判定。
    pub unresolved: Vec<String>,
}

/// 扫项目数据根下的 `skills/`：含 `SKILL.md` 的子目录才算一个 skill。
/// 目录名直接参与文件系统路径，非法 ID 只告警跳过，不 panic 也不当空表。
pub fn scan_canonical(paths: &ProjectPaths) -> Result<CanonicalScan, AppError> {
    let mut result = CanonicalScan::default();
    let base = paths.canonical_root();
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        // 隐藏目录不是 skill：提交用的 staging 也以点开头，不能混进真源。
        if name.starts_with('.') {
            continue;
        }
        if is_link(&path) {
            result
                .warnings
                .push(format!("跳过符号链接：{}", path.display()));
            result.unresolved.push(name);
            continue;
        }
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if !crate::store::is_safe_id(&name) {
            result
                .warnings
                .push(format!("真源里的目录名不能用作 skill ID：{name}"));
            result.unresolved.push(name);
            continue;
        }
        if !path.join("SKILL.md").is_file() {
            continue;
        }
        match tree_state(&path) {
            Ok(state) => {
                result.states.insert(name, state);
            }
            Err(error) => {
                result
                    .warnings
                    .push(format!("跳过包含符号链接的真源 skill {name}：{error}"));
                result.unresolved.push(name);
            }
        }
    }
    Ok(result)
}

/// 账本缺失按空表处理；存在但解析不了必须报错，避免把归属当空表覆盖。
/// 版本高于本程序支持的版本也必须报错：按旧模型解释会把渲染产物当成对等副本互相覆盖。
pub fn load_ledger(paths: &ProjectPaths) -> Result<ProjectLedger, AppError> {
    load_ledger_file(paths.ledger_path())
}

/// 从任意路径读一份台账（v3 搬迁要读项目里遗留的旧台账）。版本闸与 `load_ledger` 完全一致。
pub fn load_ledger_file(path: &Path) -> Result<ProjectLedger, AppError> {
    if !path.exists() {
        return Ok(ProjectLedger::default());
    }
    let ledger: ProjectLedger = serde_json::from_slice(&std::fs::read(path)?)?;
    if ledger.version > PROJECT_LEDGER_VERSION {
        return Err(AppError::InvalidInput(format!(
            "该项目的台账 {} 由更新版本的 CC Switch X 写入（版本 {}，本程序最高支持 {}），请升级后再打开",
            path.display(),
            ledger.version,
            PROJECT_LEDGER_VERSION
        )));
    }
    Ok(ledger)
}

/// v1 是「对等副本」模型，必须经显式迁移才能按真源模型渲染。
/// 迁移不在读路径里做：读命令不持写锁，且「空跑不改仓库文件」是既有不变量。
pub fn ledger_needs_migration(ledger: &ProjectLedger) -> bool {
    ledger.version < PROJECT_LEDGER_VERSION
}

pub fn save_ledger(paths: &ProjectPaths, ledger: &ProjectLedger) -> Result<(), AppError> {
    let path = paths.ledger_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&path, &serde_json::to_vec_pretty(ledger)?)?;
    Ok(())
}

/// 添加项目时立即写基线：targets 为空，只记录各目录与真源的当前状态，不复制任何内容。
/// 新项目直接落当前台账版本 —— 真源此刻还不存在（canonical.present=false），矩阵显示「未跟踪」，
/// 由用户逐项「收进真源」。账本已存在时不覆盖，按现有基线继续。
pub fn ensure_baseline(paths: &ProjectPaths, scan: &ProjectScan) -> Result<ProjectLedger, AppError> {
    if paths.ledger_path().exists() {
        return load_ledger(paths);
    }
    // 项目里还留着旧布局时绝不能写空台账：那会把「还没搬」变成「两边都有」，
    // 之后自动搬迁既不敢删也不敢覆盖，项目就永久卡在 Hybrid。
    if paths.legacy_dir().is_dir() {
        return Err(AppError::Project(format!(
            "项目里还有没搬完的 {}，不能在这里新建空台账",
            crate::domain::project::LEGACY_DIR_RELATIVE
        )));
    }
    let mut ledger = ProjectLedger {
        version: PROJECT_LEDGER_VERSION,
        updated_at: chrono::Utc::now().to_rfc3339(),
        skills: BTreeMap::new(),
    };
    for id in scan.skills.keys() {
        ledger.skills.insert(
            id.clone(),
            ProjectSkillEntry {
                targets: Vec::new(),
                source: None,
                canonical: scan.canonical.get(id).cloned().unwrap_or_default(),
                // 只记 targets 的渲染基线；此刻 targets 为空，所以什么都不记。
                // 把「观察到的目录现状」写进 dirs 会让外来副本看起来像我们写的，
                // 之后被勾选时就会被静默覆盖。未跟踪副本的状态由扫描现场给 UI。
                dirs: BTreeMap::new(),
                last_op: None,
            },
        );
    }
    save_ledger(paths, &ledger)?;
    Ok(ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::ProjectSkillDirKey;
    use std::path::Path;

    fn seed_skill(root: &Path, id: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: demo\n---\n").unwrap();
    }

    /// 一个「store 根 + 项目根」的最小现场。v3 起真源与台账住在 store 里，
    /// 扫描器要同时知道两边才能算出落点。
    struct Sandbox {
        _temp: tempfile::TempDir,
        paths: ProjectPaths,
    }

    fn sandbox() -> Sandbox {
        let temp = tempfile::tempdir().unwrap();
        let store_root = temp.path().join("store");
        let project_root = temp.path().join("home/work/app");
        std::fs::create_dir_all(&project_root).unwrap();
        Sandbox {
            paths: ProjectPaths::resolve(&store_root, "app-1", &project_root).unwrap(),
            _temp: temp,
        }
    }

    /// 项目里的渲染目录。
    fn render(sbx: &Sandbox, dir: &str) -> std::path::PathBuf {
        sbx.paths.root.join(dir)
    }

    /// store 里的项目真源根。
    fn canonical_root(sbx: &Sandbox) -> std::path::PathBuf {
        sbx.paths.canonical_root().to_path_buf()
    }

    #[test]
    fn scan_unions_skills_across_agent_dirs() {
        let sbx = sandbox();
        seed_skill(&render(&sbx, ".claude/skills").as_path(), "a");
        seed_skill(&render(&sbx, ".qoder/skills").as_path(), "a");
        seed_skill(&render(&sbx, ".agents/skills").as_path(), "b");

        let scan = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        assert_eq!(
            scan.skills.keys().cloned().collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(scan.skills["a"].dirs["claude"].present, true);
        assert_eq!(scan.skills["a"].dirs["qoder"].present, true);
        assert_eq!(scan.skills["a"].dirs["agents"].present, false);
    }

    #[test]
    fn baseline_records_targets_empty_and_never_copies_content() {
        let sbx = sandbox();
        seed_skill(&render(&sbx, ".claude/skills").as_path(), "a");
        let scan = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        let ledger = ensure_baseline(&sbx.paths, &scan).unwrap();

        assert!(ledger.skills["a"].targets.is_empty());
        assert!(
            ledger.skills["a"].dirs.is_empty(),
            "dirs 只记渲染基线；把观察到的现状写进去会让外来副本看起来像我们写的"
        );
        assert!(
            render(&sbx, ".claude/skills/a").is_dir(),
            "扫描与建基线都不动项目里的磁盘内容"
        );
        assert!(!render(&sbx, ".qoder/skills/a").exists());
        assert!(sbx.paths.ledger_path().is_file());
        assert!(
            !sbx.paths.legacy_dir().exists(),
            "v3 不再往项目里写任何东西"
        );
    }

    #[test]
    fn scan_canonical_reads_the_project_source_of_truth() {
        let sbx = sandbox();
        seed_skill(&canonical_root(&sbx), "mine");
        seed_skill(&render(&sbx, ".claude/skills").as_path(), "only-copy");

        let scan = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        assert!(scan.canonical["mine"].present);
        assert_eq!(
            scan.canonical.get("only-copy"),
            None,
            "真源扫描只看 store 里的 skills/，不把渲染产物当真实源"
        );
        assert!(
            !scan.skills.contains_key("mine") || scan.skills["mine"].dirs["claude"].present == false,
            "真源目录不能占用落点 key 空间"
        );
        assert!(scan.canonical["mine"].tree_hash.is_some());
    }

    #[test]
    fn scan_canonical_skips_non_skills_and_flags_unsafe_dir_names() {
        let sbx = sandbox();
        let root = canonical_root(&sbx);
        std::fs::create_dir_all(root.join("plain")).unwrap();
        std::fs::write(root.join("plain/README.md"), "没有 SKILL.md").unwrap();
        std::fs::create_dir_all(root.join("bad name")).unwrap();
        std::fs::write(root.join("bad name/SKILL.md"), "---\nname: x\n---\n").unwrap();

        let scan = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        assert!(
            !scan.canonical.contains_key("plain"),
            "没有 SKILL.md 的目录不是一个 skill"
        );
        assert!(
            scan.blocks_canonical("bad name"),
            "目录名不能当 ID 时必须报出来，不能静默当成不存在"
        );
        assert!(scan.warnings.iter().any(|w| w.contains("bad name")));
    }

    #[test]
    fn load_ledger_rejects_a_newer_version_but_reads_v1_and_v2() {
        let sbx = sandbox();
        let path = sbx.paths.ledger_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        std::fs::write(&path, r#"{"version":3,"updatedAt":"","skills":{}}"#).unwrap();
        let error = load_ledger(&sbx.paths).unwrap_err();
        assert!(
            matches!(error, AppError::InvalidInput(_)),
            "更高版本的台账不能按旧模型解释：{error:?}"
        );

        for version in [1, 2] {
            std::fs::write(
                &path,
                format!(r#"{{"version":{version},"updatedAt":"","skills":{{}}}}"#),
            )
            .unwrap();
            let ledger = load_ledger(&sbx.paths).unwrap();
            assert_eq!(ledger.version, version);
            assert_eq!(ledger_needs_migration(&ledger), version == 1);
        }
    }

    #[test]
    fn baseline_is_version_two_and_records_an_absent_canonical() {
        let sbx = sandbox();
        seed_skill(&render(&sbx, ".claude/skills").as_path(), "a");
        let without_canonical = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        let ledger = ensure_baseline(&sbx.paths, &without_canonical).unwrap();

        assert_eq!(ledger.version, PROJECT_LEDGER_VERSION);
        assert!(
            !ledger.skills["a"].canonical.present,
            "新项目的真源还没落地，矩阵要显示未跟踪而不是已同步"
        );
        assert!(ledger.skills["a"].targets.is_empty());

        // 真源已在磁盘上（例如从别处搬来）时，基线要记下它的状态。
        std::fs::remove_file(sbx.paths.ledger_path()).unwrap();
        seed_skill(&canonical_root(&sbx), "a");
        let with_canonical = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        let ledger = ensure_baseline(&sbx.paths, &with_canonical).unwrap();
        assert!(ledger.skills["a"].canonical.present);
    }

    #[test]
    fn empty_file_map_is_not_serialized() {
        let sbx = sandbox();
        seed_skill(&render(&sbx, ".claude/skills").as_path(), "a");
        let scan = scan(&sbx.paths, &ProjectSkillDirKey::ALL).unwrap();
        ensure_baseline(&sbx.paths, &scan).unwrap();

        let text = std::fs::read_to_string(sbx.paths.ledger_path()).unwrap();
        assert!(
            !text.contains(r#""files": {}"#),
            "空清单是纯噪音，不该写进台账：{text}"
        );
    }

    /// 点开头条目不计入指纹，否则 `.DS_Store` 会让格子永远「不一致」并被反复重渲染。
    #[test]
    fn tree_state_skips_dot_entries() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("a");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "v1").unwrap();

        let clean = tree_state(&skill).unwrap();
        std::fs::write(skill.join(".DS_Store"), "junk").unwrap();
        std::fs::create_dir_all(skill.join(".idea")).unwrap();
        std::fs::write(skill.join(".idea/workspace.xml"), "junk").unwrap();
        assert_eq!(
            tree_state(&skill).unwrap().tree_hash,
            clean.tree_hash,
            "隐藏文件与隐藏目录都不能改变指纹"
        );

        std::fs::write(skill.join("SKILL.md"), "v2").unwrap();
        assert_ne!(tree_state(&skill).unwrap().tree_hash, clean.tree_hash);
    }

    #[test]
    fn tree_state_is_order_independent_and_tracks_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("a");
        std::fs::create_dir_all(skill.join("references")).unwrap();
        std::fs::write(skill.join("SKILL.md"), "v1").unwrap();
        std::fs::write(skill.join("references/x.md"), "x").unwrap();

        let first = tree_state(&skill).unwrap();
        std::fs::write(skill.join("references/y.md"), "y").unwrap();
        let added = tree_state(&skill).unwrap();
        assert_ne!(first.tree_hash, added.tree_hash);
        std::fs::write(skill.join("SKILL.md"), "v2").unwrap();
        let updated = tree_state(&skill).unwrap();
        assert_ne!(added.tree_hash, updated.tree_hash);
        std::fs::remove_file(skill.join("references/x.md")).unwrap();
        let removed = tree_state(&skill).unwrap();
        assert_ne!(updated.tree_hash, removed.tree_hash);
    }

    #[cfg(windows)]
    #[test]
    fn symlink_skill_dirs_are_skipped_with_a_warning() {
        let sbx = sandbox();
        let skills = render(&sbx, ".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        let target = sbx.paths.root.join("outside");
        seed_skill(target.parent().unwrap(), "outside");
        if std::os::windows::fs::symlink_dir(&target, skills.join("evil")).is_err() {
            return; // 没有开发者模式/权限时跳过，避免测试环境差异
        }

        let scan = scan(&sbx.paths, &[ProjectSkillDirKey::Claude]).unwrap();
        assert!(!scan.skills.contains_key("evil"));
        assert!(scan.warnings.iter().any(|w| w.contains("符号链接")));
    }

    /// 被链接跳过的目录状态未知：必须标出来，否则同步会把它当成「用户删除」。
    #[cfg(windows)]
    #[test]
    fn symlink_skill_dir_is_recorded_as_unresolved_not_absent() {
        let sbx = sandbox();
        let skills = render(&sbx, ".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        let target = sbx.paths.root.join("outside");
        seed_skill(target.parent().unwrap(), "outside");
        if std::os::windows::fs::symlink_dir(&target, skills.join("evil")).is_err() {
            return; // 没有开发者模式/权限时跳过，避免测试环境差异
        }

        let scan = scan(&sbx.paths, &[ProjectSkillDirKey::Claude]).unwrap();
        assert!(!scan.skills.contains_key("evil"), "链接目录不进矩阵");
        assert_eq!(
            scan.unresolved_skills,
            vec![("claude".to_string(), "evil".to_string())]
        );
    }

    #[cfg(windows)]
    #[test]
    fn symlink_base_dir_marks_the_whole_key_unresolved() {
        let sbx = sandbox();
        let real = sbx.paths.root.join("shared/skills");
        std::fs::create_dir_all(&real).unwrap();
        let claude = render(&sbx, ".claude");
        std::fs::create_dir_all(&claude).unwrap();
        if std::os::windows::fs::symlink_dir(&real, claude.join("skills")).is_err() {
            return;
        }

        let scan = scan(&sbx.paths, &[ProjectSkillDirKey::Claude]).unwrap();
        assert_eq!(scan.unresolved_dirs, vec!["claude".to_string()]);
    }
}
