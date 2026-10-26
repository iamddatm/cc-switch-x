//! WebDAV 地址与远端目录校验；传输实现由后续任务补上。
//!
//! 密码永远不写进 URL：地址里出现 userinfo 直接拒绝。

use crate::error::AppError;
use reqwest::Url;

pub const BACKUP_OBJECT: &str = "ccswitchx-backup.zip";
pub const MAX_CLOUD_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_REMOTE_DIR: &str = "ccswitchx";

pub fn normalize_base_url(raw: &str) -> Result<String, AppError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let url = Url::parse(raw)
        .map_err(|error| AppError::InvalidInput(format!("WebDAV 地址无效：{error}")))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(AppError::InvalidInput(
                "WebDAV 地址只支持 http:// 或 https://".into(),
            ))
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::InvalidInput(
            "不要把用户名或密码写进 WebDAV 地址".into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::InvalidInput(
            "WebDAV 地址不能包含查询串或 fragment".into(),
        ));
    }
    if url.host_str().is_none() {
        return Err(AppError::InvalidInput("WebDAV 地址缺少主机名".into()));
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn validate_remote_dir(raw: &str) -> Result<String, AppError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(DEFAULT_REMOTE_DIR.to_string());
    }
    if raw.len() > 128 {
        return Err(AppError::InvalidInput("远端目录不能超过 128 个字符".into()));
    }
    if raw.starts_with('/') || raw.ends_with('/') || raw.contains('\\') {
        return Err(AppError::InvalidInput(
            "远端目录必须是相对路径，且不能包含反斜杠".into(),
        ));
    }
    for segment in raw.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(AppError::InvalidInput(
                "远端目录不能包含空段、`.` 或 `..`".into(),
            ));
        }
        if !segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        {
            return Err(AppError::InvalidInput(format!(
                "远端目录只能包含字母、数字、`.`、`_`、`-`：{segment}"
            )));
        }
    }
    Ok(raw.to_string())
}

pub fn remote_url(base_url: &str, remote_dir: &str, object: &str) -> Result<Url, AppError> {
    let base_url = normalize_base_url(base_url)?;
    if base_url.is_empty() {
        return Err(AppError::InvalidInput("WebDAV 地址不能为空".into()));
    }
    let remote_dir = validate_remote_dir(remote_dir)?;
    let object = object.trim();
    if object.is_empty() || object.contains('/') || object.contains('\\') || object.contains("..")
    {
        return Err(AppError::InvalidInput("远端对象名非法".into()));
    }

    let mut url = Url::parse(&base_url)
        .map_err(|error| AppError::InvalidInput(format!("WebDAV 地址无效：{error}")))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| AppError::InvalidInput("WebDAV 地址格式不支持追加路径".into()))?;
        segments.pop_if_empty();
        for segment in remote_dir.split('/') {
            segments.push(segment);
        }
        segments.push(object);
    }
    Ok(url)
}
