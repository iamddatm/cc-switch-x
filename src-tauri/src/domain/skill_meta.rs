//! Skill 目录里的 `.meta.json`：来源、更新状态与安装时间。
//!
//! 该文件只做增量扩展，旧版 `repoOwner/...` 字段保持原样；重写时通过 `extra`
//! 保留未知键，避免未来字段被旧版本覆盖丢失。

use crate::store::atomic::write_atomic;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

pub const META_FILE: &str = ".meta.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillSourceKind {
    Github,
    Zip,
    Local,
    CcSwitch,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SkillMeta {
    #[serde(skip_serializing_if = "Option::is_none", alias = "repo_owner")]
    pub repo_owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "repo_name")]
    pub repo_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "repo_branch")]
    pub repo_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "readme_url")]
    pub readme_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "source_directory")]
    pub source_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", alias = "content_hash")]
    pub content_hash: Option<String>,
    /// skill 在仓库内的相对路径；Git 安装/更新命中后写入，下次定位优先使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<SkillSourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// 毫秒时间戳；导入的 cc-switch 数据是秒，写入时统一换算。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_check_error: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 列表展示用的来源投影；不落索引，由 `AssetService::enrich` 现场补齐。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillOrigin {
    pub kind: SkillSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<i64>,
    pub update_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_check_error: Option<String>,
    /// 有 owner/name 才能从上游检查更新。
    pub updatable: bool,
}

impl SkillMeta {
    /// 新文件显式写 `sourceKind`；旧文件按已有字段推断，保证升级后可读。
    pub fn source_kind(&self) -> SkillSourceKind {
        self.source_kind.unwrap_or_else(|| {
            if self.repo_owner.is_some()
                || self.repo_name.is_some()
                || self.source_directory.is_some()
            {
                SkillSourceKind::CcSwitch
            } else {
                SkillSourceKind::Local
            }
        })
    }

    pub fn repo_slug(&self) -> Option<String> {
        match (non_empty(&self.repo_owner), non_empty(&self.repo_name)) {
            (Some(owner), Some(name)) => Some(format!("{owner}/{name}")),
            (None, Some(name)) => Some(name),
            _ => None,
        }
    }

    pub fn is_updatable(&self) -> bool {
        non_empty(&self.repo_owner).is_some() && non_empty(&self.repo_name).is_some()
    }

    pub fn origin(&self) -> SkillOrigin {
        SkillOrigin {
            kind: self.source_kind(),
            repo: self.repo_slug(),
            branch: non_empty(&self.repo_branch),
            source_url: self.source_url.clone(),
            readme_url: self.readme_url.clone(),
            installed_at: self.installed_at,
            updated_at: self.updated_at,
            last_checked_at: self.last_checked_at,
            update_available: self.update_available.unwrap_or(false),
            last_check_error: self.last_check_error.clone(),
            updatable: self.is_updatable(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillMetaError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 处理失败：{0}")]
    Json(#[from] serde_json::Error),
}

/// 损坏或缺失都当作没有元数据：列表展示不能因此失败。
pub fn read_meta(dir: &Path) -> Option<SkillMeta> {
    let text = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_meta(dir: &Path, meta: &SkillMeta) -> Result<(), SkillMetaError> {
    write_atomic(&dir.join(META_FILE), &serde_json::to_vec_pretty(meta)?)?;
    Ok(())
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn legacy_metadata_is_inferred_and_unknown_keys_survive() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(META_FILE),
            r#"{"repoOwner":"acme","repoName":"demo","sourceDirectory":"demo","future":42}"#,
        )
        .unwrap();

        let meta = read_meta(dir.path()).unwrap();
        assert_eq!(meta.source_kind(), SkillSourceKind::CcSwitch);
        assert_eq!(meta.repo_slug().as_deref(), Some("acme/demo"));
        assert!(meta.is_updatable());
        assert_eq!(meta.extra["future"], 42);
        assert!(!meta.origin().update_available);
    }

    #[test]
    fn local_metadata_has_no_upstream() {
        let meta = SkillMeta {
            source_kind: Some(SkillSourceKind::Local),
            source_url: Some("D:/skills/demo".into()),
            ..Default::default()
        };
        let origin = meta.origin();
        assert_eq!(origin.kind, SkillSourceKind::Local);
        assert!(origin.repo.is_none());
        assert!(!origin.updatable);
    }

    #[test]
    fn writing_preserves_extra_keys_and_skips_none_fields() {
        let dir = tempdir().unwrap();
        let mut meta = SkillMeta {
            repo_owner: Some("acme".into()),
            repo_name: Some("demo".into()),
            ..Default::default()
        };
        meta.extra.insert("future".into(), Value::from("keep"));
        write_meta(dir.path(), &meta).unwrap();

        let text = std::fs::read_to_string(dir.path().join(META_FILE)).unwrap();
        assert!(text.contains("\"future\": \"keep\""), "{text}");
        assert!(!text.contains("\"repoBranch\""), "空字段不写：{text}");
    }
}
