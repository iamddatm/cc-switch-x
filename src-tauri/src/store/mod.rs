pub mod atomic;
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

pub struct Store {
    root: PathBuf,
}

impl Store {
    /// 在 root 下补齐真源目录布局，并把不可同步的条目合并进 .gitignore。
    pub fn init(root: PathBuf) -> Result<Self, StoreError> {
        for name in ["mcp", "skills", "prompts", "rules", "subagents", "hooks"] {
            fs::create_dir_all(root.join("assets").join(name))?;
        }
        for name in ["agents", "local/agents", "state", "backups", "logs"] {
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
        if asset_type == AssetType::Skill {
            return Err(StoreError::Unsupported(asset_type));
        }
        let path = self.entry_path(asset_type, id)?;
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
}
