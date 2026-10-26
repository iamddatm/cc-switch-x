pub mod atomic;
pub mod projects;
pub mod secrets;

use crate::domain::AssetType;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("非法资产 ID：{0}")]
    InvalidId(String),
    #[error("暂不支持该资产类型的文件写入：{0:?}")]
    Unsupported(AssetType),
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// Skill 整目录备份保留份数；只清理 `backups/skills` 子树，不碰其他资产备份。
pub const SKILL_BACKUP_KEEP: usize = 20;

impl Store {
    /// 在 root 下补齐真源目录布局，并把不可同步的条目合并进 .gitignore。
    pub fn init(root: PathBuf) -> Result<Self, StoreError> {
        for name in ["mcp", "skills", "prompts", "rules", "subagents", "hooks"] {
            fs::create_dir_all(root.join("assets").join(name))?;
        }
        for name in [
            "agents",
            "local/agents",
            "state",
            "state/staging",
            "backups",
            "logs",
        ] {
            fs::create_dir_all(root.join(name))?;
        }
        let ignore = root.join(".gitignore");
        let required = [
            "index.db",
            "index.db-*",
            "secrets.local.json",
            "local/",
            "backups/",
            "logs/",
            "state/",
        ];
        let existing = fs::read_to_string(&ignore).unwrap_or_default();
        let mut content = existing.clone();
        for line in required {
            if !existing.lines().any(|l| l.trim() == line) {
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(line);
                content.push('\n');
            }
        }
        fs::write(&ignore, content)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn asset_dir(&self, asset_type: AssetType) -> PathBuf {
        self.root.join("assets").join(asset_type.dir_name())
    }

    /// `agents/<id>.json`（启用矩阵）的路径。id 只来自内置注册表，不是用户输入。
    pub fn agent_config_path(&self, agent_id: &str) -> PathBuf {
        self.root.join("agents").join(format!("{agent_id}.json"))
    }

    pub fn entry_path(&self, asset_type: AssetType, id: &str) -> Result<PathBuf, StoreError> {
        if !is_safe_id(id) {
            return Err(StoreError::InvalidId(id.to_string()));
        }
        let base = self.asset_dir(asset_type);
        Ok(match asset_type.file_extension() {
            Some(ext) => base.join(format!("{id}.{ext}")),
            None => base.join(id),
        })
    }

    pub fn write(&self, asset_type: AssetType, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let path = self.entry_path(asset_type, id)?;
        // skill 是目录资产：写的是 <id>/SKILL.md，辅助文件保持不动。
        let path = if asset_type == AssetType::Skill {
            fs::create_dir_all(&path)?;
            path.join("SKILL.md")
        } else {
            path
        };
        atomic::write_atomic(&path, bytes)?;
        Ok(())
    }

    pub fn read(&self, asset_type: AssetType, id: &str) -> Result<Vec<u8>, StoreError> {
        let path = self.entry_path(asset_type, id)?;
        Ok(fs::read(path)?)
    }

    pub fn delete(&self, asset_type: AssetType, id: &str) -> Result<bool, StoreError> {
        let path = self.entry_path(asset_type, id)?;
        if !path.exists() {
            return Ok(false);
        }
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        Ok(true)
    }

    pub fn list_ids(&self, asset_type: AssetType) -> Result<Vec<String>, StoreError> {
        let dir = self.asset_dir(asset_type);
        let mut ids = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().to_string();
            let id = match asset_type.file_extension() {
                Some(ext) => {
                    if !file_type.is_file() {
                        continue;
                    }
                    match name.strip_suffix(&format!(".{ext}")) {
                        Some(stem) => stem.to_string(),
                        None => continue,
                    }
                }
                None => {
                    if !file_type.is_dir() {
                        continue;
                    }
                    name
                }
            };
            ids.push(id);
        }
        ids.sort();
        Ok(ids)
    }

    /// 把当前内容复制到 backups/<时间戳>/<类型>/<文件名>，返回备份路径。
    pub fn backup(&self, asset_type: AssetType, id: &str) -> Result<Option<PathBuf>, StoreError> {
        let source = self.entry_path(asset_type, id)?;
        if !source.exists() {
            return Ok(None);
        }
        let stamp = chrono::Utc::now().timestamp_millis();
        let target = self
            .root
            .join("backups")
            .join(stamp.to_string())
            .join(asset_type.dir_name())
            .join(source.file_name().unwrap_or_default());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if source.is_dir() {
            copy_dir(&source, &target)?;
        } else {
            fs::copy(&source, &target)?;
        }
        Ok(Some(target))
    }

    /// Skill 的整目录备份，落到 `backups/skills/<毫秒>_<id>/`。
    /// 同一毫秒内重复备份时追加序号，绝不覆盖已有备份。
    pub fn backup_skill(&self, id: &str) -> Result<Option<PathBuf>, StoreError> {
        let source = self.entry_path(AssetType::Skill, id)?;
        if !source.exists() {
            return Ok(None);
        }
        let base = self.root.join("backups").join("skills");
        fs::create_dir_all(&base)?;
        let stamp = chrono::Utc::now().timestamp_millis();
        let mut target = base.join(format!("{stamp}_{id}"));
        let mut suffix = 1;
        while target.exists() {
            suffix += 1;
            target = base.join(format!("{stamp}_{id}_{suffix}"));
        }
        copy_dir(&source, &target)?;
        // 备份成功后尽力清理；清理失败不能把已经成功的备份报成失败。
        let _ = self.prune_skill_backups(SKILL_BACKUP_KEEP);
        Ok(Some(target))
    }

    /// 只保留最近 `keep` 份 skill 备份，返回删除的份数。
    pub fn prune_skill_backups(&self, keep: usize) -> Result<usize, StoreError> {
        let base = self.root.join("backups").join("skills");
        if !base.is_dir() {
            return Ok(0);
        }
        let mut entries = fs::read_dir(&base)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        // 目录名以 13 位毫秒时间戳开头，按名称排序即按时间排序。
        entries.sort();
        let excess = entries.len().saturating_sub(keep);
        let mut removed = 0;
        for name in entries.into_iter().take(excess) {
            fs::remove_dir_all(base.join(name))?;
            removed += 1;
        }
        Ok(removed)
    }

    /// 来源安装/更新的暂存区，必须与真源在同一卷上，rename 才是原子的。
    pub fn staging_dir(&self) -> PathBuf {
        self.root.join("state").join("staging")
    }

    /// 清理超过 `max_age` 的暂存目录，返回删除的目录数。
    /// 目录名里的毫秒时间戳优先（可测试、不受复制操作影响 mtime 的干扰）。
    pub fn cleanup_staging(&self, max_age: std::time::Duration) -> Result<usize, StoreError> {
        let root = self.staging_dir();
        if !root.is_dir() {
            return Ok(0);
        }
        let now = std::time::SystemTime::now();
        let max_age_ms = max_age.as_millis() as i64;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut removed = 0;
        for entry in fs::read_dir(&root)?.filter_map(Result::ok) {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let created_at = staging_timestamp(&name).unwrap_or_else(|| {
                entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .map(|age| now_ms - age.as_millis() as i64)
                    .unwrap_or(now_ms)
            });
            if now_ms.saturating_sub(created_at) >= max_age_ms {
                fs::remove_dir_all(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// ID 直接参与文件名拼接，因此只允许 slug 字符，且排除 . / ..
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// 暂存目录名像 `preview-1700000000000-3` / `install-1700000000000-3`，
/// 取第一个能解析成毫秒时间戳的段。
fn staging_timestamp(name: &str) -> Option<i64> {
    name.split('-')
        .filter_map(|part| part.parse::<i64>().ok())
        .find(|value| *value > 1_000_000_000_000)
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use tempfile::tempdir;

    #[test]
    fn init_creates_layout_and_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        for name in ["mcp", "skills", "prompts", "rules", "subagents", "hooks"] {
            assert!(store.root().join("assets").join(name).is_dir());
        }
        let ignore = std::fs::read_to_string(store.root().join(".gitignore")).unwrap();
        assert!(ignore.contains("node_modules/"));
        assert!(ignore.contains("index.db"));
        assert!(ignore.contains("local/"));
        assert!(ignore.contains("secrets.local.json"));
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        store
            .write(AssetType::Mcp, "pw", b"{\"id\":\"pw\"}")
            .unwrap();
        assert_eq!(
            store.read(AssetType::Mcp, "pw").unwrap(),
            b"{\"id\":\"pw\"}"
        );
        assert_eq!(store.list_ids(AssetType::Mcp).unwrap(), vec!["pw"]);
        assert!(store.delete(AssetType::Mcp, "pw").unwrap());
        assert!(!store.read(AssetType::Mcp, "pw").is_ok());
    }

    #[test]
    fn rejects_unsafe_ids() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        for id in ["../evil", "a/b", "", ".."] {
            assert!(store.entry_path(AssetType::Mcp, id).is_err(), "id={id}");
        }
    }

    #[test]
    fn overwrite_replaces_content_without_temp_files() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        store.write(AssetType::Mcp, "pw", b"one").unwrap();
        store.write(AssetType::Mcp, "pw", b"two").unwrap();
        assert_eq!(store.read(AssetType::Mcp, "pw").unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(store.asset_dir(AssetType::Mcp))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn backup_preserves_previous_content() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        store.write(AssetType::Mcp, "pw", b"one").unwrap();
        let backup = store.backup(AssetType::Mcp, "pw").unwrap().unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), b"one");
    }

    #[test]
    fn skill_backups_keep_twenty_and_leave_other_backups_alone() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        let skill = store.asset_dir(AssetType::Skill).join("demo");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "v0\n").unwrap();

        let other = store.root().join("backups/other/keep");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("note"), "别动").unwrap();

        for version in 1..=22 {
            std::fs::write(skill.join("SKILL.md"), format!("v{version}\n")).unwrap();
            store.backup_skill("demo").unwrap().unwrap();
        }

        let mut backups = std::fs::read_dir(store.root().join("backups/skills"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        backups.sort();
        assert_eq!(backups.len(), SKILL_BACKUP_KEEP);
        assert!(
            std::fs::read(other.join("note")).is_ok(),
            "只清理 backups/skills，别的资产备份不能碰"
        );
    }

    #[test]
    fn staging_cleanup_removes_only_expired_directories() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let stale = store.staging_dir().join("preview-1700000000000-1");
        let fresh = store.staging_dir().join(format!("preview-{now}-2"));
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();

        let removed = store
            .cleanup_staging(std::time::Duration::from_secs(24 * 3600))
            .unwrap();
        assert_eq!(removed, 1);
        assert!(!stale.exists());
        assert!(fresh.exists());
    }
}
