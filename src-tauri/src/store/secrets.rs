use crate::domain::secret::{parse_reference_body, scan_candidates, SecretRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("缺少密钥：{0}")]
    Missing(SecretRef),
    #[error("密钥引用格式非法：{0}")]
    BadReference(String),
}

/// `secrets.local.json` 的内容：scope -> key -> 明文值。永不参与同步与导出。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    values: BTreeMap<String, BTreeMap<String, String>>,
}

impl Secrets {
    pub fn load(path: &Path) -> Result<Self, SecretsError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), SecretsError> {
        super::atomic::write_atomic(path, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, secret_ref: &SecretRef) -> Option<&str> {
        self.values
            .get(&secret_ref.scope)?
            .get(&secret_ref.key)
            .map(String::as_str)
    }

    pub fn insert(&mut self, secret_ref: &SecretRef, value: &str) {
        self.values
            .entry(secret_ref.scope.clone())
            .or_default()
            .insert(secret_ref.key.clone(), value.to_string());
    }

    pub fn remove(&mut self, secret_ref: &SecretRef) {
        if let Some(scope) = self.values.get_mut(&secret_ref.scope) {
            scope.remove(&secret_ref.key);
            if scope.is_empty() {
                self.values.remove(&secret_ref.scope);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.values().all(|scope| scope.is_empty())
    }

    /// 把文本里的引用换成明文。任一引用坏掉或缺失都整体失败——
    /// 绝不能把残缺引用或空值写进 agent 配置。
    pub fn resolve_text(&self, text: &str) -> Result<String, SecretsError> {
        let candidates = scan_candidates(text);
        let mut out = text.to_string();
        for candidate in candidates.into_iter().rev() {
            let secret_ref = match parse_reference_body(candidate.body) {
                Some(parsed) => parsed,
                None => return Err(SecretsError::BadReference(candidate.body.to_string())),
            };
            let value = self
                .get(&secret_ref)
                .ok_or(SecretsError::Missing(secret_ref))?;
            out.replace_range(candidate.span, value);
        }
        Ok(out)
    }
}

pub fn secrets_path(store_root: &Path) -> std::path::PathBuf {
    store_root.join("secrets.local.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::secret::reference;
    use tempfile::tempdir;

    #[test]
    fn missing_reference_is_an_error_not_an_empty_value() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secrets.local.json");
        let mut secrets = Secrets::load(&path).unwrap();
        secrets.insert(
            &SecretRef {
                scope: "mcp.pw".into(),
                key: "TOKEN".into(),
            },
            "abc",
        );
        secrets.save(&path).unwrap();

        let loaded = Secrets::load(&path).unwrap();
        assert_eq!(
            loaded.resolve_text(&reference("mcp.pw", "TOKEN")).unwrap(),
            "abc"
        );
        assert!(matches!(
            loaded.resolve_text(&reference("mcp.pw", "NOPE")),
            Err(SecretsError::Missing(_))
        ));
    }

    #[test]
    fn broken_reference_is_rejected_before_writing() {
        let dir = tempdir().unwrap();
        let secrets = Secrets::load(&dir.path().join("secrets.local.json")).unwrap();
        assert!(matches!(
            secrets.resolve_text(r#"{"url":"https://x?a=${secret:broken}"}"#),
            Err(SecretsError::BadReference(_))
        ));
    }

    #[test]
    fn resolving_inside_a_bigger_document() {
        let dir = tempdir().unwrap();
        let mut secrets = Secrets::load(&dir.path().join("secrets.local.json")).unwrap();
        secrets.insert(
            &SecretRef {
                scope: "mcp.tavily".into(),
                key: "tavilyApiKey".into(),
            },
            "tvly-abc",
        );
        let text = format!(
            r#"{{"url":"https://api.tavily.com/mcp?tavilyApiKey={}"}}"#,
            reference("mcp.tavily", "tavilyApiKey")
        );
        assert_eq!(
            secrets.resolve_text(&text).unwrap(),
            r#"{"url":"https://api.tavily.com/mcp?tavilyApiKey=tvly-abc"}"#
        );
    }

    #[test]
    fn load_of_missing_file_is_empty_not_fatal() {
        let dir = tempdir().unwrap();
        let secrets = Secrets::load(&dir.path().join("nope.json")).unwrap();
        assert!(secrets.is_empty());
    }
}
