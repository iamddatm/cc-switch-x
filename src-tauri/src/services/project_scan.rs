use crate::domain::project::{
    dir_spec, ProjectDirState, ProjectLedger, ProjectSkillDirKey, ProjectSkillEntry,
    PROJECT_LEDGER_VERSION, PROJECT_SKILL_DIRS,
};
use crate::error::AppError;
use crate::index::sha256_hex;
use crate::store::atomic::write_atomic;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSkillSnapshot {
    pub id: String,
    pub dirs: BTreeMap<String, ProjectDirState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectScan {
    pub skills: BTreeMap<String, ProjectSkillSnapshot>,
    /// 项目真源 `.ccswitchx/skills/<id>/` 的当前状态；真源不是落点，不进 dirs key 空间。
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

pub fn scan(project_root: &Path, keys: &[ProjectSkillDirKey]) -> Result<ProjectScan, AppError> {
    let mut skills: BTreeMap<String, ProjectSkillSnapshot> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut unresolved_dirs: Vec<String> = Vec::new();
    let mut unresolved_skills: Vec<(String, String)> = Vec::new();
    let mut unresolved_canonical: Vec<String> = Vec::new();

    for key in keys {
        let base = project_root.join(dir_spec(*key).relative_path);
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

    let canonical = scan_canonical(project_root)?;
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

pub fn ledger_path(project_root: &Path) -> PathBuf {
    project_root.join(".ccswitchx").join("skills.json")
}

/// 真源目录的扫描结果。真源不是落点，所以单独扫，不进 `dirs` 的 key 空间。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CanonicalScan {
    pub states: BTreeMap<String, ProjectDirState>,
    pub warnings: Vec<String>,
    /// 状态取不到（链接、或目录名不是合法 ID）的 skill id：不得按「不存在」判定。
    pub unresolved: Vec<String>,
}

/// 扫 `<project>/.ccswitchx/skills/`：含 `SKILL.md` 的子目录才算一个 skill。
/// 目录名直接参与文件系统路径，非法 ID 只告警跳过，不 panic 也不当空表。
pub fn scan_canonical(project_root: &Path) -> Result<CanonicalScan, AppError> {
    let mut result = CanonicalScan::default();
    let base = crate::domain::project::canonical_root(project_root);
    let entries = match std::fs::read_dir(&base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
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
pub fn load_ledger(project_root: &Path) -> Result<ProjectLedger, AppError> {
    let path = ledger_path(project_root);
    if !path.exists() {
        return Ok(ProjectLedger::default());
    }
    let ledger: ProjectLedger = serde_json::from_slice(&std::fs::read(&path)?)?;
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

pub fn save_ledger(project_root: &Path, ledger: &ProjectLedger) -> Result<(), AppError> {
    let path = ledger_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&path, &serde_json::to_vec_pretty(ledger)?)?;
    Ok(())
}

/// 添加项目时立即写基线：targets 为空，只记录各目录与真源的当前状态，不复制任何内容。
/// 新项目直接落 v2 —— 真源此刻还不存在（canonical.present=false），矩阵显示「未跟踪」，
/// 由用户逐项「收进真源」。账本已存在时不覆盖，按现有基线继续。
pub fn ensure_baseline(
    project_root: &Path,
    scan: &ProjectScan,
) -> Result<ProjectLedger, AppError> {
    if ledger_path(project_root).exists() {
        return load_ledger(project_root);
    }
    let mut ledger = ProjectLedger {
        version: PROJECT_LEDGER_VERSION,
        updated_at: chrono::Utc::now().to_rfc3339(),
        skills: BTreeMap::new(),
    };
    for (id, snapshot) in &scan.skills {
        ledger.skills.insert(
            id.clone(),
            ProjectSkillEntry {
                targets: Vec::new(),
                source: None,
                canonical: scan.canonical.get(id).cloned().unwrap_or_default(),
                dirs: snapshot.dirs.clone(),
                last_op: None,
            },
        );
    }
    save_ledger(project_root, &ledger)?;
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

    #[test]
    fn scan_unions_skills_across_agent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_skill(&root.join(".claude/skills"), "a");
        seed_skill(&root.join(".qoder/skills"), "a");
        seed_skill(&root.join(".agents/skills"), "b");

        let scan = scan(root, &ProjectSkillDirKey::ALL).unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        seed_skill(&dir.path().join(".claude/skills"), "a");
        let scan = scan(dir.path(), &ProjectSkillDirKey::ALL).unwrap();
        let ledger = ensure_baseline(dir.path(), &scan).unwrap();

        assert!(ledger.skills["a"].targets.is_empty());
        assert!(ledger.skills["a"].dirs["claude"].present);
        assert!(!dir.path().join(".qoder/skills/a").exists());
        assert!(ledger_path(dir.path()).is_file());
    }

    #[test]
    fn scan_canonical_reads_the_project_source_of_truth() {
        let dir = tempfile::tempdir().unwrap();
        seed_skill(&dir.path().join(".ccswitchx/skills"), "mine");
        seed_skill(&dir.path().join(".claude/skills"), "only-copy");

        let scan = scan(dir.path(), &ProjectSkillDirKey::ALL).unwrap();
        assert!(scan.canonical["mine"].present);
        assert_eq!(
            scan.canonical.get("only-copy"),
            None,
            "真源扫描只看 .ccswitchx/skills，不把渲染产物当真实源"
        );
        assert!(
            !scan.skills.contains_key("mine") || scan.skills["mine"].dirs["claude"].present == false,
            "真源目录不能占用落点 key 空间"
        );
        assert!(scan.canonical["mine"].tree_hash.is_some());
    }

    #[test]
    fn scan_canonical_skips_non_skills_and_flags_unsafe_dir_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".ccswitchx/skills");
        std::fs::create_dir_all(root.join("plain")).unwrap();
        std::fs::write(root.join("plain/README.md"), "没有 SKILL.md").unwrap();
        std::fs::create_dir_all(root.join("bad name")).unwrap();
        std::fs::write(root.join("bad name/SKILL.md"), "---\nname: x\n---\n").unwrap();

        let scan = scan(dir.path(), &ProjectSkillDirKey::ALL).unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let path = ledger_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        std::fs::write(
            &path,
            r#"{"version":3,"updatedAt":"","skills":{}}"#,
        )
        .unwrap();
        let error = load_ledger(dir.path()).unwrap_err();
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
            let ledger = load_ledger(dir.path()).unwrap();
            assert_eq!(ledger.version, version);
            assert_eq!(ledger_needs_migration(&ledger), version == 1);
        }
    }

    #[test]
    fn baseline_is_version_two_and_records_an_absent_canonical() {
        let dir = tempfile::tempdir().unwrap();
        seed_skill(&dir.path().join(".claude/skills"), "a");
        let without_canonical = scan(dir.path(), &ProjectSkillDirKey::ALL).unwrap();
        let ledger = ensure_baseline(dir.path(), &without_canonical).unwrap();

        assert_eq!(ledger.version, PROJECT_LEDGER_VERSION);
        assert!(
            !ledger.skills["a"].canonical.present,
            "新项目的真源还没落地，矩阵要显示未跟踪而不是已同步"
        );
        assert!(ledger.skills["a"].targets.is_empty());

        // 真源已在磁盘上（例如另一台机器迁移过）时，基线要记下它的状态。
        std::fs::remove_file(ledger_path(dir.path())).unwrap();
        seed_skill(&dir.path().join(".ccswitchx/skills"), "a");
        let with_canonical = scan(dir.path(), &ProjectSkillDirKey::ALL).unwrap();
        let ledger = ensure_baseline(dir.path(), &with_canonical).unwrap();
        assert!(ledger.skills["a"].canonical.present);
    }

    #[test]
    fn empty_file_map_is_not_serialized() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        seed_skill(&root.join(".claude/skills"), "a");
        let scan = scan(root, &ProjectSkillDirKey::ALL).unwrap();
        ensure_baseline(root, &scan).unwrap();

        let text = std::fs::read_to_string(ledger_path(root)).unwrap();
        assert!(
            !text.contains(r#""files": {}"#),
            "台账随仓库提交，空清单只会制造 diff 噪音：{text}"
        );
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
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        let target = dir.path().join("outside");
        seed_skill(dir.path(), "outside");
        if std::os::windows::fs::symlink_dir(&target, skills.join("evil")).is_err() {
            return; // 没有开发者模式/权限时跳过，避免测试环境差异
        }

        let scan = scan(dir.path(), &[ProjectSkillDirKey::Claude]).unwrap();
        assert!(!scan.skills.contains_key("evil"));
        assert!(scan.warnings.iter().any(|w| w.contains("符号链接")));
    }

    /// 被链接跳过的目录状态未知：必须标出来，否则同步会把它当成「用户删除」。
    #[cfg(windows)]
    #[test]
    fn symlink_skill_dir_is_recorded_as_unresolved_not_absent() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        let target = dir.path().join("outside");
        seed_skill(dir.path(), "outside");
        if std::os::windows::fs::symlink_dir(&target, skills.join("evil")).is_err() {
            return; // 没有开发者模式/权限时跳过，避免测试环境差异
        }

        let scan = scan(dir.path(), &[ProjectSkillDirKey::Claude]).unwrap();
        assert!(!scan.skills.contains_key("evil"), "链接目录不进矩阵");
        assert_eq!(
            scan.unresolved_skills,
            vec![("claude".to_string(), "evil".to_string())]
        );
    }

    #[cfg(windows)]
    #[test]
    fn symlink_base_dir_marks_the_whole_key_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("shared/skills");
        std::fs::create_dir_all(&real).unwrap();
        let claude = dir.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        if std::os::windows::fs::symlink_dir(&real, claude.join("skills")).is_err() {
            return;
        }

        let scan = scan(dir.path(), &[ProjectSkillDirKey::Claude]).unwrap();
        assert_eq!(scan.unresolved_dirs, vec!["claude".to_string()]);
    }
}
