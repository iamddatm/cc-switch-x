use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub added_at: String,
    pub last_opened_at: String,
    pub auto_sync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRegistry {
    pub version: u32,
    pub projects: Vec<ProjectEntry>,
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectSkillDirKey {
    Claude,
    Qoder,
    Agents,
}

impl ProjectSkillDirKey {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Qoder, Self::Agents];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Qoder => "qoder",
            Self::Agents => "agents",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectSkillDirSpec {
    pub key: ProjectSkillDirKey,
    pub relative_path: &'static str,
    pub agent_ids: &'static [&'static str],
    /// 是否已在临时项目里做过真机读取验证；false 时 UI 不允许把它选为同步目标。
    pub verified: bool,
}

pub const PROJECT_SKILL_DIRS: [ProjectSkillDirSpec; 3] = [
    ProjectSkillDirSpec {
        key: ProjectSkillDirKey::Claude,
        relative_path: ".claude/skills",
        agent_ids: &["claude"],
        verified: false,
    },
    ProjectSkillDirSpec {
        key: ProjectSkillDirKey::Qoder,
        relative_path: ".qoder/skills",
        agent_ids: &["qoder", "qoder-cn"],
        verified: false,
    },
    ProjectSkillDirSpec {
        key: ProjectSkillDirKey::Agents,
        relative_path: ".agents/skills",
        agent_ids: &["codex", "gemini", "pi"],
        verified: false,
    },
];

pub fn dir_spec(key: ProjectSkillDirKey) -> &'static ProjectSkillDirSpec {
    PROJECT_SKILL_DIRS
        .iter()
        .find(|spec| spec.key == key)
        .expect("PROJECT_SKILL_DIRS 必须覆盖所有 ProjectSkillDirKey")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirState {
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_hash: Option<String>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// 目录内文件的最大 mtime（毫秒）。只在冲突时用于推荐“最新一侧”，不写进账本。
    #[serde(skip, default)]
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillSource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillOp {
    pub dir: String,
    pub action: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillEntry {
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ProjectSkillSource>,
    #[serde(default)]
    pub dirs: BTreeMap<String, ProjectDirState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_op: Option<ProjectSkillOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLedger {
    pub version: u32,
    pub updated_at: String,
    #[serde(default)]
    pub skills: BTreeMap<String, ProjectSkillEntry>,
}

impl Default for ProjectLedger {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            skills: BTreeMap::new(),
        }
    }
}

/// 校验项目根：拒绝 home 本身、storeRoot 内部、以及用户级 agent 配置根内部。
///
/// 注意：home 的子目录是允许的（绝大多数项目都在 home 下），只拒绝 home 本身。
pub fn validate_project_root(
    path: &Path,
    home: &Path,
    store_root: &Path,
    user_level_roots: &[PathBuf],
) -> Result<PathBuf, AppError> {
    if !path.is_dir() {
        return Err(AppError::InvalidInput("项目目录不存在".into()));
    }
    let canonical = path.canonicalize()?;
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let store_root = store_root
        .canonicalize()
        .unwrap_or_else(|_| store_root.to_path_buf());

    if canonical == home {
        return Err(AppError::InvalidInput(
            "不能把用户主目录本身添加为项目".into(),
        ));
    }
    if canonical == store_root || canonical.starts_with(&store_root) {
        return Err(AppError::InvalidInput(
            "不能把 CC Switch X 的数据目录添加为项目".into(),
        ));
    }
    for root in user_level_roots {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        if canonical == root || canonical.starts_with(&root) {
            return Err(AppError::InvalidInput(format!(
                "不能把用户级 agent 配置目录添加为项目：{}",
                root.display()
            )));
        }
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_skill_dirs_are_stable() {
        let specs = &PROJECT_SKILL_DIRS;
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].key.as_str(), "claude");
        assert_eq!(specs[0].relative_path, ".claude/skills");
        assert_eq!(specs[1].key.as_str(), "qoder");
        assert_eq!(specs[1].relative_path, ".qoder/skills");
        assert_eq!(specs[2].key.as_str(), "agents");
        assert_eq!(specs[2].relative_path, ".agents/skills");
    }

    #[test]
    fn validate_rejects_home_store_root_and_user_level_roots() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let store = dir.path().join("store");
        let claude = home.join(".claude");
        let project = home.join("work/app");
        for path in [&home, &store, &claude] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::create_dir_all(&project).unwrap();

        assert!(validate_project_root(&home, &home, &store, &[claude.clone()]).is_err());
        assert!(validate_project_root(&store, &home, &store, &[claude.clone()]).is_err());
        assert!(validate_project_root(&claude, &home, &store, &[claude.clone()]).is_err());
        let ok = validate_project_root(&project, &home, &store, &[claude]).unwrap();
        assert_eq!(
            ok,
            project.canonicalize().unwrap(),
            "home 下的普通项目必须允许"
        );
    }
}
