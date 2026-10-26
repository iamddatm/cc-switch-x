//! 项目 skill 的「检查更新」状态缓存。
//!
//! 为什么不写进 `<project>/.ccswitchx/skills.json`：那个文件随仓库提交，而
//! `lastCheckedAt` / `updateAvailable` 是每次检查都会变的机器局部状态，写进去会把
//! 用户的 git diff 刷成噪音，也会在两台机器间来回打架。所以真源与台账保持干净，
//! 易变状态只存本机 `local/`（store 的 .gitignore 已覆盖，不同步、不进备份快照）。
//!
//! 它是纯派生数据（与 `index.db` 同类）：缺失或损坏都按空表处理，下次检查自然重建。

use crate::error::AppError;
use crate::store::{atomic::write_atomic, is_safe_id, Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectSkillCheck {
    pub update_available: bool,
    /// 上游候选目录的 `compute_dir_hash`（与全局同一算法）。
    pub latest_hash: Option<String>,
    pub last_checked_at: Option<i64>,
    pub last_check_error: Option<String>,
    /// 有 owner/name 才能从上游检查更新。
    pub updatable: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectCheckCache {
    pub version: u32,
    pub skills: BTreeMap<String, ProjectSkillCheck>,
}

impl ProjectCheckCache {
    fn path(store: &Store, project_id: &str) -> Option<PathBuf> {
        // project_id 由我们自己生成（slug + 路径哈希），仍然过一遍 ID 安全，
        // 免得哪天有人把外部字符串当 id 传进来就变成路径穿越。
        if !is_safe_id(project_id) {
            return None;
        }
        Some(
            store
                .root()
                .join("local")
                .join("project-skill-checks")
                .join(format!("{project_id}.json")),
        )
    }

    pub fn load(store: &Store, project_id: &str) -> Self {
        let loaded = Self::path(store, project_id)
            .filter(|path| path.is_file())
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        loaded.unwrap_or_default()
    }

    pub fn save(&self, store: &Store, project_id: &str) -> Result<(), AppError> {
        let Some(path) = Self::path(store, project_id) else {
            return Err(AppError::InvalidInput(format!(
                "项目 ID 不能用作文件名：{project_id}"
            )));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&path, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn record(&mut self, skill_id: &str, check: ProjectSkillCheck) {
        self.skills.insert(skill_id.to_string(), check);
    }

    pub fn get(&self, skill_id: &str) -> Option<&ProjectSkillCheck> {
        self.skills.get(skill_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_corrupt_cache_is_an_empty_table_but_a_bad_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();

        assert!(ProjectCheckCache::load(&store, "app-1").skills.is_empty());
        assert!(ProjectCheckCache::load(&store, "../escape").skills.is_empty());

        let path = ProjectCheckCache::path(&store, "app-1").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ 不是 JSON").unwrap();
        assert!(
            ProjectCheckCache::load(&store, "app-1").skills.is_empty(),
            "派生缓存损坏不该让功能挂掉"
        );
        assert!(ProjectCheckCache::default().save(&store, "../escape").is_err());
    }

    #[test]
    fn cache_roundtrips_and_stays_out_of_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let mut cache = ProjectCheckCache::default();
        cache.record(
            "demo",
            ProjectSkillCheck {
                update_available: true,
                latest_hash: Some("abc".into()),
                last_checked_at: Some(1),
                last_check_error: None,
                updatable: true,
            },
        );
        cache.save(&store, "app-1").unwrap();

        let loaded = ProjectCheckCache::load(&store, "app-1");
        assert_eq!(loaded.skills.len(), 1);
        assert!(loaded.get("demo").unwrap().update_available);
        assert!(
            ProjectCheckCache::path(&store, "app-1")
                .unwrap()
                .starts_with(store.root().join("local")),
            "必须落在 local/ 下，才不会进同步与备份"
        );
    }
}
