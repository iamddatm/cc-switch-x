use crate::domain::project::{
    dir_spec, ProjectDirState, ProjectLedger, ProjectSkillDirKey, ProjectSkillEntry,
    PROJECT_SKILL_DIRS,
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
    pub dirs: Vec<ProjectSkillDirKey>,
    pub warnings: Vec<String>,
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

    for key in keys {
        let base = project_root.join(dir_spec(*key).relative_path);
        if is_link(&base) {
            warnings.push(format!("跳过符号链接：{}", base.display()));
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
                continue;
            }
            if !entry.file_type()?.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let state = match tree_state(&path) {
                Ok(state) => state,
                Err(error) => {
                    warnings.push(format!("跳过包含符号链接的 skill {name}：{error}"));
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

    Ok(ProjectScan {
        skills,
        dirs: keys.to_vec(),
        warnings,
    })
}

pub fn ledger_path(project_root: &Path) -> PathBuf {
    project_root.join(".ccswitchx").join("skills.json")
}

/// 账本缺失按空表处理；存在但解析不了必须报错，避免把归属当空表覆盖。
pub fn load_ledger(project_root: &Path) -> Result<ProjectLedger, AppError> {
    let path = ledger_path(project_root);
    if !path.exists() {
        return Ok(ProjectLedger::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

pub fn save_ledger(project_root: &Path, ledger: &ProjectLedger) -> Result<(), AppError> {
    let path = ledger_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&path, &serde_json::to_vec_pretty(ledger)?)?;
    Ok(())
}

/// 添加项目时立即写基线：targets 为空，只记录各目录当前状态，不复制任何内容。
/// 账本已存在时不覆盖，按现有基线继续。
pub fn ensure_baseline(
    project_root: &Path,
    scan: &ProjectScan,
) -> Result<ProjectLedger, AppError> {
    if ledger_path(project_root).exists() {
        return load_ledger(project_root);
    }
    let mut ledger = ProjectLedger {
        version: 1,
        updated_at: chrono::Utc::now().to_rfc3339(),
        skills: BTreeMap::new(),
    };
    for (id, snapshot) in &scan.skills {
        ledger.skills.insert(
            id.clone(),
            ProjectSkillEntry {
                targets: Vec::new(),
                source: None,
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
}
