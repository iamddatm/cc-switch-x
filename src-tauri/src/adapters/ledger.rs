use crate::domain::AssetKind;
use crate::error::AppError;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 一次落盘的期望状态。`hash` 是**写到磁盘上那份字节**的指纹（不是真源文件的指纹），
/// 漂移检测拿「磁盘实际」跟它比；`rendered_by` 是渲染器版本号，改了渲染格式时用来
/// 判断旧条目是否还是「我们家早期写的那一版」（对应 teamai 的 `previousRenders`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedEntry {
    pub asset_type: String,
    pub asset_id: String,
    pub target_path: PathBuf,
    pub hash: String,
    pub rendered_by: u32,
}

/// `state/applied/<agent>.json`：我们往某个 agent 落过哪些文件、落成了什么。
/// 文件型资产（prompts / rules / subagents）的归属只认这份台账，
/// 不往资产正文里塞标记 —— 正文是 agent 要读的内容，不能污染。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppliedLedger {
    pub entries: BTreeMap<String, AppliedEntry>,
}

fn key(kind: AssetKind, asset_id: &str) -> String {
    format!("{}/{}", kind.snake_name(), asset_id)
}

impl AppliedLedger {
    pub fn get(&self, kind: AssetKind, asset_id: &str) -> Option<&AppliedEntry> {
        self.entries.get(&key(kind, asset_id))
    }

    /// 按目标文件找条目：同一文件被不同资产先后写过（切换激活的提示词）时，
    /// 归属看的是「这份字节是不是我们写的」，而不是「是不是这个资产 id 写的」。
    pub fn find_by_path<'a>(&'a self, path: &Path, hash: &str) -> Option<&'a AppliedEntry> {
        self.entries
            .values()
            .find(|entry| entry.target_path == path && entry.hash == hash)
    }

    pub fn set(&mut self, kind: AssetKind, entry: AppliedEntry) {
        self.entries.insert(key(kind, &entry.asset_id), entry);
    }

    pub fn remove(&mut self, kind: AssetKind, asset_id: &str) -> Option<AppliedEntry> {
        self.entries.remove(&key(kind, asset_id))
    }
}

pub fn ledger_path(store: &Store, agent_id: &str) -> PathBuf {
    store
        .root()
        .join("state")
        .join("applied")
        .join(format!("{agent_id}.json"))
}

/// 台账缺失按空表处理；存在但解析不了必须报错 —— 静默当空表等于忘掉全部归属，
/// 之后 disable 会把用户自己写的文件误判成别人的。
pub fn load(store: &Store, agent_id: &str) -> Result<AppliedLedger, AppError> {
    let path = ledger_path(store, agent_id);
    if !path.exists() {
        return Ok(AppliedLedger::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn save(store: &Store, agent_id: &str, ledger: &AppliedLedger) -> Result<(), AppError> {
    write_atomic(
        &ledger_path(store, agent_id),
        &serde_json::to_vec_pretty(ledger)?,
    )?;
    Ok(())
}

/// 记录（或更新）一次落盘；只动自己那一条。
pub fn record(
    store: &Store,
    agent_id: &str,
    kind: AssetKind,
    asset_id: &str,
    target_path: &Path,
    hash: &str,
    rendered_by: u32,
) -> Result<(), AppError> {
    let mut ledger = load(store, agent_id)?;
    ledger.set(
        kind,
        AppliedEntry {
            asset_type: kind.snake_name().to_string(),
            asset_id: asset_id.to_string(),
            target_path: target_path.to_path_buf(),
            hash: hash.to_string(),
            rendered_by,
        },
    );
    save(store, agent_id, &ledger)
}

pub fn forget(
    store: &Store,
    agent_id: &str,
    kind: AssetKind,
    asset_id: &str,
) -> Result<(), AppError> {
    let mut ledger = load(store, agent_id)?;
    if ledger.remove(kind, asset_id).is_none() {
        return Ok(());
    }
    save(store, agent_id, &ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetKind;
    use tempfile::tempdir;

    #[test]
    fn ledger_roundtrips_and_replaces_only_its_own_key() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        record(
            &store,
            "qoder-cn",
            AssetKind::Rule,
            "style",
            &dir.path().join("r.md"),
            "h1",
            1,
        )
        .unwrap();
        record(
            &store,
            "qoder-cn",
            AssetKind::Hook,
            "fmt",
            &dir.path().join("s.json"),
            "h2",
            1,
        )
        .unwrap();
        let text =
            std::fs::read_to_string(store.root().join("state/applied/qoder-cn.json")).unwrap();
        assert_eq!(text.matches("\"assetId\"").count(), 2, "{text}");

        // 同一资产重新落盘只更新自己那条，别的不该被抹掉
        record(
            &store,
            "qoder-cn",
            AssetKind::Rule,
            "style",
            &dir.path().join("r.md"),
            "h3",
            2,
        )
        .unwrap();
        let ledger = load(&store, "qoder-cn").unwrap();
        assert_eq!(ledger.get(AssetKind::Rule, "style").unwrap().hash, "h3");
        assert_eq!(ledger.get(AssetKind::Rule, "style").unwrap().rendered_by, 2);
        assert_eq!(ledger.get(AssetKind::Hook, "fmt").unwrap().hash, "h2");
    }

    #[test]
    fn same_id_under_two_capabilities_do_not_collide() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        record(
            &store,
            "claude",
            AssetKind::Rule,
            "same",
            &dir.path().join("a"),
            "ha",
            1,
        )
        .unwrap();
        record(
            &store,
            "claude",
            AssetKind::Subagent,
            "same",
            &dir.path().join("b"),
            "hb",
            1,
        )
        .unwrap();
        let ledger = load(&store, "claude").unwrap();
        assert_eq!(ledger.get(AssetKind::Rule, "same").unwrap().hash, "ha");
        assert_eq!(ledger.get(AssetKind::Subagent, "same").unwrap().hash, "hb");
    }

    #[test]
    fn ownership_is_matched_by_bytes_at_the_path_not_by_asset_id() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        let target = dir.path().join("CLAUDE.md");
        record(&store, "claude", AssetKind::Prompt, "a", &target, "h-a", 1).unwrap();

        let ledger = load(&store, "claude").unwrap();
        assert!(ledger.find_by_path(&target, "h-a").is_some());
        assert_eq!(
            ledger
                .find_by_path(&target, "h-a")
                .unwrap()
                .asset_id
                .as_str(),
            "a"
        );
        // 用户手改过：字节对不上，谁都不是
        assert!(ledger.find_by_path(&target, "h-tampered").is_none());
        // 另一个文件里的相同字节不算：归属必须同时命中路径与内容
        assert!(ledger
            .find_by_path(&dir.path().join("other"), "h-a")
            .is_none());
    }

    #[test]
    fn forget_removes_only_the_named_entry() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        record(
            &store,
            "claude",
            AssetKind::Rule,
            "a",
            &dir.path().join("a"),
            "ha",
            1,
        )
        .unwrap();
        record(
            &store,
            "claude",
            AssetKind::Rule,
            "b",
            &dir.path().join("b"),
            "hb",
            1,
        )
        .unwrap();
        forget(&store, "claude", AssetKind::Rule, "a").unwrap();

        let ledger = load(&store, "claude").unwrap();
        assert!(ledger.get(AssetKind::Rule, "a").is_none());
        assert!(ledger.get(AssetKind::Rule, "b").is_some());
        // 记不住的路径 Forget 不该报错（幂等）
        forget(&store, "claude", AssetKind::Rule, "missing").unwrap();
    }

    #[test]
    fn a_corrupt_ledger_is_an_error_not_an_empty_history() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        let path = ledger_path(&store, "claude");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ 这不是 JSON").unwrap();

        let error = load(&store, "claude").unwrap_err();
        assert!(
            matches!(error, AppError::Json(_)),
            "把损坏的台账当空表 = 忘掉全部归属，之后会误删用户的文件：{error:?}"
        );
    }

    #[test]
    fn entry_json_uses_camel_case_keys() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        record(
            &store,
            "claude",
            AssetKind::Rule,
            "a",
            &dir.path().join("a"),
            "ha",
            3,
        )
        .unwrap();
        let text = std::fs::read_to_string(ledger_path(&store, "claude")).unwrap();
        for field in ["assetType", "assetId", "targetPath", "hash", "renderedBy"] {
            assert!(text.contains(field), "{field} 不在 {text}");
        }
    }

    #[test]
    fn ledger_asset_type_names_match_the_serialized_capability_kind() {
        // 前端拿到的 CapabilityStatus.kind 与台账里的 assetType 必须是同一个字符串，
        // 否则漂移报告会指着一类 UI 上不存在的能力。
        for kind in AssetKind::ALL {
            let serialized = serde_json::to_value(kind).unwrap();
            assert_eq!(
                serialized.as_str().unwrap(),
                kind.snake_name(),
                "{kind:?} 的 serde 名与 snake_name 不一致"
            );
        }
    }

    #[test]
    fn ledger_keys_are_namespaced_by_capability() {
        // key 里的能力名用 snake_name，与真源目录名（复数）刻意不同：
        // 台账记的是「能力 × 资产」，不是磁盘布局。
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        record(
            &store,
            "claude",
            AssetKind::Rule,
            "style",
            &dir.path().join("a"),
            "ha",
            1,
        )
        .unwrap();
        let text = std::fs::read_to_string(ledger_path(&store, "claude")).unwrap();
        assert!(text.contains("\"rule/style\""), "{text}");
    }
}
