//! WebDAV 地址校验与基础传输。
//!
//! 传输保持同步（`reqwest::blocking`），由调用方放进 `spawn_blocking`。
//! 密码只进 Basic Auth 头，不写进 URL；请求不跟随重定向。

use crate::error::AppError;
use crate::net::ProxyConfig;
use reqwest::blocking::{Body, Client};
use reqwest::header::{HeaderMap, CONTENT_LENGTH, CONTENT_TYPE, ETAG, LAST_MODIFIED};
use reqwest::{Method, StatusCode, Url};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

pub const BACKUP_OBJECT: &str = "ccswitchx-backup.zip";
/// 备份包旁边的描述文件：记录上传设备、版本与包的哈希，供换机时先看不用先下载。
pub const MANIFEST_OBJECT: &str = "manifest.json";
pub const MAX_CLOUD_BYTES: u64 = 512 * 1024 * 1024;
/// manifest 只描述一个包，正常只有几百字节；上限防止被别的应用塞进大文件。
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_REMOTE_DIR: &str = "ccswitchx";
/// PROPFIND 应答只用来取几个属性，正常几百字节；超长说明响应异常，截断后按解析失败处理。
const MAX_PROPINFO_BYTES: u64 = 64 * 1024;
/// 只问 PROPFIND 要用到的属性，避免服务器回整份死属性集。
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:getcontentlength/><d:getlastmodified/><d:getetag/></d:prop></d:propfind>"#;

#[derive(Clone)]
pub struct WebDavConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub remote_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteObject {
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub etag: Option<String>,
}

impl RemoteObject {
    fn missing() -> Self {
        Self {
            exists: false,
            size_bytes: None,
            modified_at: None,
            etag: None,
        }
    }
}

pub struct WebDavClient {
    client: Client,
    config: WebDavConfig,
    base: Url,
}

impl WebDavClient {
    pub fn new(config: WebDavConfig, proxy: Option<&ProxyConfig>) -> Result<Self, AppError> {
        let base_url = normalize_base_url(&config.base_url)?;
        if base_url.is_empty() {
            return Err(AppError::InvalidInput("WebDAV 地址不能为空".into()));
        }
        let remote_dir = validate_remote_dir(&config.remote_dir)?;
        let base = Url::parse(&base_url)
            .map_err(|error| AppError::InvalidInput(format!("WebDAV 地址无效：{error}")))?;
        let config = WebDavConfig {
            base_url,
            remote_dir,
            ..config
        };

        let mut builder = Client::builder()
            .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = proxy {
            builder = proxy.apply_blocking(builder)?;
        }
        let client = builder
            .build()
            .map_err(|error| AppError::Cloud(format!("创建 WebDAV 客户端失败：{error}")))?;
        Ok(Self {
            client,
            config,
            base,
        })
    }

    /// 逐级创建远端目录；405 表示目录已存在。
    pub fn ensure_dir(&self) -> Result<(), AppError> {
        let segments: Vec<&str> = self.config.remote_dir.split('/').collect();
        for count in 1..=segments.len() {
            let url = self.dir_url(&segments[..count])?;
            let response = self
                .request(mkcol_method(), url)
                .send()
                .map_err(transport_error("MKCOL"))?;
            let status = response.status();
            match status.as_u16() {
                200 | 201 | 204 | 405 => {}
                409 => {
                    return Err(AppError::Cloud(
                        "WebDAV 远端目录的父目录不存在（HTTP 409）".into(),
                    ))
                }
                401 | 403 => return Err(auth_error(status)),
                _ => return Err(status_error("MKCOL", status)),
            }
        }
        Ok(())
    }

    pub fn head(&self) -> Result<RemoteObject, AppError> {
        self.head_object(BACKUP_OBJECT)
    }

    /// 读对象元数据：先 PROPFIND，服务器不支持时退回 HEAD。
    ///
    /// 坚果云对 HEAD 一律回 `Content-Length: 0`（哪怕对象有 13 MB），拿 HEAD 判大小
    /// 会把正常上传误判成失败；`getcontentlength` 才是 WebDAV 规定的元数据来源。
    fn head_object(&self, name: &str) -> Result<RemoteObject, AppError> {
        match self.propfind_object(name)? {
            Some(remote) => Ok(remote),
            None => self.head_object_via_head(name),
        }
    }

    /// PROPFIND `Depth: 0`；`Ok(None)` 表示服务器不支持或应答无法解析，调用方回退 HEAD。
    fn propfind_object(&self, name: &str) -> Result<Option<RemoteObject>, AppError> {
        let response = self
            .request(propfind_method(), self.remote_url(name)?)
            .header("Depth", "0")
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND_BODY)
            .send()
            .map_err(transport_error("PROPFIND"))?;
        let status = response.status();
        match status.as_u16() {
            // 个别实现用 200 代替 207 返回单条 response，解析逻辑一致。
            200 | 207 => {}
            404 | 409 => return Ok(Some(RemoteObject::missing())),
            405 | 501 => return Ok(None),
            401 | 403 => return Err(auth_error(status)),
            _ => return Err(status_error("PROPFIND", status)),
        }
        let mut body = String::new();
        response
            .take(MAX_PROPINFO_BYTES)
            .read_to_string(&mut body)
            .map_err(|error| AppError::Cloud(format!("读取 PROPFIND 应答失败：{error}")))?;
        Ok(remote_from_propfind(&body))
    }

    fn head_object_via_head(&self, name: &str) -> Result<RemoteObject, AppError> {
        let response = self
            .request(Method::HEAD, self.remote_url(name)?)
            .send()
            .map_err(transport_error("HEAD"))?;
        let status = response.status();
        match status.as_u16() {
            200 => Ok(remote_from_headers(response.headers(), None, true)),
            404 => Ok(RemoteObject::missing()),
            401 | 403 => Err(auth_error(status)),
            _ => Err(status_error("HEAD", status)),
        }
    }

    /// 写描述文件。包先落地、manifest 最后写：远端读到 manifest 就说明它描述的包是完整的。
    pub fn put_manifest(&self, bytes: &[u8]) -> Result<RemoteObject, AppError> {
        let length = bytes.len() as u64;
        if length > MAX_MANIFEST_BYTES {
            return Err(AppError::Cloud(format!(
                "manifest 超过上限（{MAX_MANIFEST_BYTES} 字节）"
            )));
        }
        let response = self
            .request(Method::PUT, self.remote_url(MANIFEST_OBJECT)?)
            .header(CONTENT_TYPE, "application/json")
            .header(CONTENT_LENGTH, length)
            .body(Body::from(bytes.to_vec()))
            .send()
            .map_err(transport_error("PUT"))?;
        let status = response.status();
        if !status.is_success() {
            if matches!(status.as_u16(), 401 | 403) {
                return Err(auth_error(status));
            }
            return Err(status_error("PUT", status));
        }

        // 回读确认：manifest 是恢复端判断远端版本的唯一依据，写残缺比写失败更难排查。
        let remote = self.head_object(MANIFEST_OBJECT)?;
        if !remote.exists {
            return Err(AppError::Cloud("上传后远端 manifest 不存在".into()));
        }
        if let Some(size) = remote.size_bytes {
            if size != length {
                return Err(AppError::Cloud(format!(
                    "上传后远端 manifest 大小不一致：本地 {length} 字节，远端 {size} 字节"
                )));
            }
        }
        Ok(remote)
    }

    /// 读描述文件；远端没有（旧版上传的裸包）时返回 None。
    pub fn get_manifest(&self) -> Result<Option<Vec<u8>>, AppError> {
        let response = self
            .request(Method::GET, self.remote_url(MANIFEST_OBJECT)?)
            .send()
            .map_err(transport_error("GET"))?;
        let status = response.status();
        match status.as_u16() {
            200 => {}
            // 坚果云在父目录不存在时用 409 代替标准 404；这里只探测「有没有 manifest」，
            // 两种状态都按没有处理（真正缺包由 HEAD 备份对象判定）。
            404 | 409 => return Ok(None),
            401 | 403 => return Err(auth_error(status)),
            _ => return Err(status_error("GET", status)),
        }
        if let Some(length) = header_u64(response.headers(), CONTENT_LENGTH) {
            if length > MAX_MANIFEST_BYTES {
                return Err(AppError::Cloud(format!(
                    "远端 manifest 超过上限（{MAX_MANIFEST_BYTES} 字节）"
                )));
            }
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(AppError::Cloud(format!(
                "远端 manifest 超过上限（{MAX_MANIFEST_BYTES} 字节）"
            )));
        }
        Ok(Some(bytes))
    }

    pub fn get_to_file(&self, dest: &Path, max_bytes: u64) -> Result<RemoteObject, AppError> {
        let mut response = self
            .request(Method::GET, self.object_url()?)
            .send()
            .map_err(transport_error("GET"))?;
        let status = response.status();
        match status.as_u16() {
            200 => {}
            404 => {
                return Err(AppError::Cloud(
                    "云端还没有备份，请先上传".into(),
                ))
            }
            401 | 403 => return Err(auth_error(status)),
            _ => return Err(status_error("GET", status)),
        }

        let headers = response.headers().clone();
        if let Some(length) = header_u64(&headers, CONTENT_LENGTH) {
            if length > max_bytes {
                return Err(oversize_error(max_bytes));
            }
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(dest)?;
        let mut limited = response.by_ref().take(max_bytes + 1);
        let result = (|| -> Result<u64, AppError> {
            let copied = std::io::copy(&mut limited, &mut file)?;
            if copied > max_bytes {
                return Err(oversize_error(max_bytes));
            }
            file.flush()?;
            Ok(copied)
        })();
        match result {
            Ok(copied) => Ok(remote_from_headers(&headers, Some(copied), true)),
            Err(error) => {
                drop(file);
                let _ = fs::remove_file(dest);
                Err(error)
            }
        }
    }

    /// 连接测试：目录可创建 / 已存在，对象可 HEAD；404 视为成功。
    pub fn test(&self) -> Result<(), AppError> {
        self.ensure_dir()?;
        let _ = self.head()?;
        Ok(())
    }

    /// 上传到临时对象后 MOVE 覆盖最终对象；MOVE 不支持时回退直接 PUT。
    pub fn put(&self, source: &Path) -> Result<RemoteObject, AppError> {
        self.ensure_dir()?;
        let metadata = fs::metadata(source)?;
        if !metadata.is_file() {
            return Err(AppError::InvalidInput("上传源不是文件".into()));
        }
        let length = metadata.len();
        if length > MAX_CLOUD_BYTES {
            return Err(AppError::Cloud(format!(
                "备份包超过上传上限（{MAX_CLOUD_BYTES} 字节）"
            )));
        }

        let temp_name = format!(
            "{BACKUP_OBJECT}.tmp-{}",
            chrono::Utc::now().timestamp_millis()
        );
        let temp_url = self.temp_url(&temp_name)?;
        let final_url = self.object_url()?;

        if let Err(error) = self.put_file(source, length, temp_url.clone()) {
            let _ = self.delete_url(temp_url.clone());
            return Err(error);
        }

        let move_status = self
            .request(move_method(), temp_url.clone())
            .header("Destination", final_url.as_str())
            .header("Overwrite", "T")
            .send()
            .map_err(transport_error("MOVE"))?
            .status();
        if matches!(move_status.as_u16(), 405 | 409 | 501) {
            // 405/501：服务器不支持 MOVE；409：坚果云这类服务拒绝覆盖式 MOVE（目标已存在）。
            // 两者都退回直写最终对象；直写成功前不动临时对象，失败时它是唯一的完整副本。
            if let Err(error) = self.put_file(source, length, final_url) {
                return Err(AppError::Cloud(format!(
                    "WebDAV MOVE 被拒绝（HTTP {move_status}），直写最终对象也失败：{error}"
                )));
            }
            let _ = self.delete_url(temp_url);
        } else if !move_status.is_success() {
            let _ = self.delete_url(temp_url);
            return Err(status_error("MOVE", move_status));
        }

        match self.head() {
            Ok(remote) => {
                if !remote.exists {
                    return Err(AppError::Cloud("上传后远端对象不存在".into()));
                }
                if let Some(size) = remote.size_bytes {
                    if size != length {
                        return Err(AppError::Cloud(format!(
                            "上传后远端大小不一致：本地 {length} 字节，远端 {size} 字节"
                        )));
                    }
                }
                Ok(remote)
            }
            Err(error) => {
                eprintln!("上传后检查远端大小失败：{error}");
                Ok(RemoteObject {
                    exists: true,
                    size_bytes: Some(length),
                    modified_at: None,
                    etag: None,
                })
            }
        }
    }

    fn put_file(&self, source: &Path, length: u64, url: Url) -> Result<(), AppError> {
        let file = fs::File::open(source)?;
        let response = self
            .request(Method::PUT, url)
            .header(CONTENT_TYPE, "application/zip")
            .header(CONTENT_LENGTH, length)
            .body(Body::sized(file, length))
            .send()
            .map_err(transport_error("PUT"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error("PUT", status));
        }
        Ok(())
    }

    fn delete_url(&self, url: Url) -> Result<(), AppError> {
        let response = self
            .request(Method::DELETE, url)
            .send()
            .map_err(transport_error("DELETE"))?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 404 {
            Ok(())
        } else {
            Err(status_error("DELETE", status))
        }
    }

    fn temp_url(&self, name: &str) -> Result<Url, AppError> {
        self.remote_url(name)
    }

    fn request(&self, method: Method, url: Url) -> reqwest::blocking::RequestBuilder {
        self.client
            .request(method, url)
            .basic_auth(self.config.username.as_str(), Some(self.config.password.as_str()))
    }

    fn dir_url(&self, segments: &[&str]) -> Result<Url, AppError> {
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| AppError::InvalidInput("WebDAV 地址格式不支持追加路径".into()))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
            path.push("");
        }
        Ok(url)
    }

    fn object_url(&self) -> Result<Url, AppError> {
        self.remote_url(BACKUP_OBJECT)
    }

    fn remote_url(&self, name: &str) -> Result<Url, AppError> {
        let segments: Vec<&str> = self.config.remote_dir.split('/').collect();
        let dir = self.dir_url(&segments)?;
        dir.join(name)
            .map_err(|error| AppError::Cloud(format!("拼接远端对象地址失败：{error}")))
    }
}

fn mkcol_method() -> Method {
    Method::from_bytes(b"MKCOL").expect("MKCOL is a valid method")
}

fn move_method() -> Method {
    Method::from_bytes(b"MOVE").expect("MOVE is a valid method")
}

fn propfind_method() -> Method {
    Method::from_bytes(b"PROPFIND").expect("PROPFIND is a valid method")
}

fn transport_error(operation: &'static str) -> impl Fn(reqwest::Error) -> AppError {
    move |error| AppError::Cloud(format!("WebDAV {operation} 请求失败：{error}"))
}

fn status_error(operation: &str, status: StatusCode) -> AppError {
    AppError::Cloud(format!("WebDAV {operation} 失败：HTTP {status}"))
}

fn auth_error(status: StatusCode) -> AppError {
    AppError::Cloud(format!(
        "WebDAV 认证失败（HTTP {status}），请检查用户名和应用密码"
    ))
}

fn oversize_error(max_bytes: u64) -> AppError {
    AppError::Cloud(format!("云端备份超过下载上限（{max_bytes} 字节）"))
}

fn header_u64(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn remote_from_headers(
    headers: &HeaderMap,
    size_fallback: Option<u64>,
    exists: bool,
) -> RemoteObject {
    let size_bytes = header_u64(headers, CONTENT_LENGTH).or(size_fallback);
    let modified_at = headers
        .get(LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
        .map(|time| time.timestamp_millis());
    let etag = headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    RemoteObject {
        exists,
        size_bytes,
        modified_at,
        etag,
    }
}

/// 从 PROPFIND 的 multistatus 应答里取对象元数据；一个字段都取不到时返回 None，
/// 由调用方回退 HEAD（宁可少一次校验，也不能因为解析失败挡住上传）。
fn remote_from_propfind(xml: &str) -> Option<RemoteObject> {
    let size_bytes = element_text(xml, "getcontentlength").and_then(|text| text.parse().ok());
    let modified_at = element_text(xml, "getlastmodified")
        .and_then(|text| chrono::DateTime::parse_from_rfc2822(&text).ok())
        .map(|time| time.timestamp_millis());
    let etag = element_text(xml, "getetag").filter(|text| !text.is_empty());
    if size_bytes.is_none() && modified_at.is_none() && etag.is_none() {
        return None;
    }
    Some(RemoteObject {
        exists: true,
        size_bytes,
        modified_at,
        etag,
    })
}

/// 取元素内部文本（trim 后）。只服务 `Depth: 0` 的扁平属性，不做通用 XML 解析；
/// 命名空间前缀按 `:` 之后的部分比较，兼容 `d:` / `D:` / `lp1:` 等写法。
fn element_text(xml: &str, local: &str) -> Option<String> {
    let (inner, _) = find_element(xml, local)?;
    Some(inner.trim().to_string())
}

/// 找到第一个本地名匹配的元素，返回其内部文本与是否自闭合。
fn find_element<'a>(xml: &'a str, local: &str) -> Option<(&'a str, bool)> {
    let mut search_from = 0;
    while let Some(relative) = xml[search_from..].find('<') {
        let start = search_from + relative;
        let body_start = start + 1;
        let rest = &xml[body_start..];
        // 注释、处理指令与闭合标签都不是起始元素。
        if rest.starts_with('?') || rest.starts_with('!') || rest.starts_with('/') {
            search_from = body_start;
            continue;
        }
        let end = body_start + rest.find('>')?;
        let tag = &xml[body_start..end];
        let after = end + 1;
        let name = tag.split_whitespace().next().unwrap_or_default();
        if name.rsplit(':').next() != Some(local) {
            search_from = after;
            continue;
        }
        if tag.ends_with('/') {
            return Some(("", true));
        }
        let close = find_closing_tag(xml, after, local)?;
        return Some((&xml[after..close], false));
    }
    None
}

fn find_closing_tag(xml: &str, from: usize, local: &str) -> Option<usize> {
    let mut search_from = from;
    while let Some(relative) = xml[search_from..].find("</") {
        let start = search_from + relative;
        let name_start = start + 2;
        let end = name_start + xml[name_start..].find('>')?;
        if xml[name_start..end].trim().rsplit(':').next() == Some(local) {
            return Some(start);
        }
        search_from = end + 1;
    }
    None
}

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

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_support {
    use std::collections::{HashMap, HashSet};
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    struct FakeState {
        username: String,
        password: String,
        objects: HashMap<String, Vec<u8>>,
        collections: HashSet<String>,
        methods: HashMap<String, usize>,
        reject_move: bool,
        fail_move: bool,
        conflict_move: bool,
        redirect: bool,
        fail_put: bool,
        fail_put_after: Option<usize>,
        put_attempts: usize,
        truncate_after_put: bool,
        head_size_override: Option<u64>,
        /// 模拟坚果云：HEAD 一律回 `Content-Length: 0`，真实大小只能靠 PROPFIND。
        head_zero_length: bool,
        /// 模拟不支持 PROPFIND 的服务器：应答 405，客户端应回退 HEAD。
        unsupported_propfind: bool,
        /// 模拟坚果云：GET 不存在的对象时返回 409 而不是 404。
        missing_get_conflict: bool,
    }

    pub(crate) struct FakeWebDav {
        addr: SocketAddr,
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeWebDav {
        pub(crate) fn start(username: &str, password: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake webdav");
            let addr = listener.local_addr().expect("fake webdav addr");
            let state = Arc::new(Mutex::new(FakeState {
                username: username.to_string(),
                password: password.to_string(),
                objects: HashMap::new(),
                collections: HashSet::new(),
                methods: HashMap::new(),
                reject_move: false,
                fail_move: false,
                conflict_move: false,
                redirect: false,
                fail_put: false,
                fail_put_after: None,
                put_attempts: 0,
                truncate_after_put: false,
                head_size_override: None,
                head_zero_length: false,
                unsupported_propfind: false,
                missing_get_conflict: false,
            }));
            let thread_state = state.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if let Ok(stream) = stream {
                        handle_connection(stream, &thread_state);
                    }
                }
            });
            Self { addr, state }
        }

        pub(crate) fn base_url(&self) -> String {
            format!("http://{}/dav", self.addr)
        }

        pub(crate) fn put_object(&self, path: &str, bytes: Vec<u8>) {
            self.state
                .lock()
                .unwrap()
                .objects
                .insert(normalize_path(path), bytes);
        }

        pub(crate) fn object(&self, path: &str) -> Option<Vec<u8>> {
            self.state
                .lock()
                .unwrap()
                .objects
                .get(&normalize_path(path))
                .cloned()
        }

        pub(crate) fn paths(&self) -> Vec<String> {
            let mut paths = self
                .state
                .lock()
                .unwrap()
                .objects
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            paths.sort();
            paths
        }

        pub(crate) fn request_count(&self, method: &str) -> usize {
            self.state
                .lock()
                .unwrap()
                .methods
                .get(method)
                .copied()
                .unwrap_or(0)
        }

        pub(crate) fn set_reject_move(&self, reject: bool) {
            self.state.lock().unwrap().reject_move = reject;
        }

        pub(crate) fn set_fail_move(&self, fail: bool) {
            self.state.lock().unwrap().fail_move = fail;
        }

        /// 目标已存在时 MOVE 返回 409 —— 坚果云等服务的实际行为。
        pub(crate) fn set_move_conflict(&self, conflict: bool) {
            self.state.lock().unwrap().conflict_move = conflict;
        }

        pub(crate) fn set_redirect(&self, redirect: bool) {
            self.state.lock().unwrap().redirect = redirect;
        }

        pub(crate) fn set_fail_put(&self, fail: bool) {
            self.state.lock().unwrap().fail_put = fail;
        }

        /// 前 `n` 次 PUT 成功，之后失败；用于覆盖「临时对象已传、直写最终对象失败」。
        pub(crate) fn set_fail_put_after(&self, attempts: usize) {
            self.state.lock().unwrap().fail_put_after = Some(attempts);
        }

        pub(crate) fn set_truncate_after_put(&self, truncate: bool) {
            self.state.lock().unwrap().truncate_after_put = truncate;
        }

        pub(crate) fn set_head_size_override(&self, size: Option<u64>) {
            self.state.lock().unwrap().head_size_override = size;
        }

        pub(crate) fn set_head_zero_length(&self, zero: bool) {
            self.state.lock().unwrap().head_zero_length = zero;
        }

        pub(crate) fn set_unsupported_propfind(&self, unsupported: bool) {
            self.state.lock().unwrap().unsupported_propfind = unsupported;
        }

        pub(crate) fn set_missing_get_conflict(&self, conflict: bool) {
            self.state.lock().unwrap().missing_get_conflict = conflict;
        }
    }

    fn normalize_path(path: &str) -> String {
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    }

    fn handle_connection(mut stream: TcpStream, state: &Arc<Mutex<FakeState>>) {
        let Some((method, target, headers, body)) = read_request(&mut stream) else {
            return;
        };
        let path = normalize_path(target.split('?').next().unwrap_or("/"));
        let mut state = state.lock().unwrap();
        *state.methods.entry(method.clone()).or_insert(0) += 1;

        let expected = format!(
            "Basic {}",
            base64_encode(format!("{}:{}", state.username, state.password).as_bytes())
        );
        if headers.get("authorization") != Some(&expected) {
            respond(&mut stream, "401 Unauthorized", &[], b"");
            return;
        }
        if state.redirect {
            respond(
                &mut stream,
                "302 Found",
                &[("Location", "/dav/ccswitchx/".to_string())],
                b"",
            );
            return;
        }

        match method.as_str() {
            "MKCOL" => {
                if state.collections.contains(&path) {
                    respond(&mut stream, "405 Method Not Allowed", &[], b"");
                } else {
                    state.collections.insert(path);
                    respond(&mut stream, "201 Created", &[], b"");
                }
            }
            "PUT" => {
                state.put_attempts += 1;
                let should_fail = state.fail_put
                    || state
                        .fail_put_after
                        .is_some_and(|limit| state.put_attempts > limit);
                if should_fail {
                    respond(&mut stream, "500 Internal Server Error", &[], b"");
                } else {
                    let mut body = body;
                    if state.truncate_after_put {
                        body.truncate(3);
                    }
                    state.objects.insert(path, body);
                    respond(&mut stream, "201 Created", &[], b"");
                }
            }
            "MOVE" => {
                if state.reject_move {
                    respond(&mut stream, "405 Method Not Allowed", &[], b"");
                } else if state.fail_move {
                    respond(&mut stream, "500 Internal Server Error", &[], b"");
                } else if state.conflict_move {
                    respond(&mut stream, "409 Conflict", &[], b"");
                } else {
                    let destination = headers
                        .get("destination")
                        .map(|value| destination_path(value))
                        .unwrap_or_else(|| path.clone());
                    match state.objects.remove(&path) {
                        Some(bytes) => {
                            state.objects.insert(destination, bytes);
                            respond(&mut stream, "201 Created", &[], b"");
                        }
                        None => respond(&mut stream, "404 Not Found", &[], b""),
                    }
                }
            }
            "DELETE" => {
                state.objects.remove(&path);
                respond(&mut stream, "204 No Content", &[], b"");
            }
            "GET" => match state.objects.get(&path) {
                Some(bytes) => {
                    let headers = object_headers();
                    respond(&mut stream, "200 OK", &headers, bytes);
                }
                None if state.missing_get_conflict => {
                    respond(&mut stream, "409 Conflict", &[], b"")
                }
                None => respond(&mut stream, "404 Not Found", &[], b""),
            },
            "HEAD" => match state.objects.get(&path) {
                Some(bytes) => {
                    let length = if state.head_zero_length {
                        0
                    } else {
                        state.head_size_override.unwrap_or(bytes.len() as u64)
                    };
                    respond_head(&mut stream, "200 OK", length, &object_headers());
                }
                None => respond_head(&mut stream, "404 Not Found", 0, &[]),
            },
            "PROPFIND" => {
                if state.unsupported_propfind {
                    respond(&mut stream, "405 Method Not Allowed", &[], b"");
                } else {
                    match state.objects.get(&path) {
                        Some(bytes) => {
                            let length = state.head_size_override.unwrap_or(bytes.len() as u64);
                            let xml = propfind_document(&path, length);
                            respond(
                                &mut stream,
                                "207 Multi-Status",
                                &[("Content-Type", "application/xml; charset=utf-8".to_string())],
                                xml.as_bytes(),
                            );
                        }
                        None => respond(&mut stream, "404 Not Found", &[], b""),
                    }
                }
            }
            _ => respond(&mut stream, "405 Method Not Allowed", &[], b""),
        }
    }

    /// 形如真实服务器的单条 multistatus 应答，属性顺序故意与坚果云保持一致。
    fn propfind_document(path: &str, content_length: u64) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>{path}</d:href><d:propstat><d:prop><d:getetag>fake-etag</d:getetag><d:getcontentlength>{content_length}</d:getcontentlength><d:getlastmodified>Wed, 21 Oct 2015 07:28:00 GMT</d:getlastmodified><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
        )
    }

    fn object_headers() -> Vec<(&'static str, String)> {
        vec![
            ("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT".to_string()),
            ("ETag", "\"fake-etag\"".to_string()),
        ]
    }

    fn destination_path(value: &str) -> String {
        reqwest::Url::parse(value)
            .map(|url| url.path().to_string())
            .unwrap_or_else(|_| value.to_string())
    }

    fn read_request(
        stream: &mut TcpStream,
    ) -> Option<(String, String, HashMap<String, String>, Vec<u8>)> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
            if buffer.len() > 128 * 1024 {
                return None;
            }
        };

        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let mut lines = head.split("\r\n");
        let request_line = lines.next()?;
        let mut parts = request_line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();
        let mut headers = HashMap::new();
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let length = headers
            .get("content-length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let mut body = buffer[header_end..].to_vec();
        while body.len() < length {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
        }
        body.truncate(length);
        Some((method, target, headers, body))
    }

    fn respond(
        stream: &mut TcpStream,
        status: &str,
        headers: &[(&str, String)],
        body: &[u8],
    ) {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (key, value) in headers {
            response.push_str(&format!("{key}: {value}\r\n"));
        }
        response.push_str("\r\n");
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    fn respond_head(
        stream: &mut TcpStream,
        status: &str,
        content_length: u64,
        headers: &[(&str, String)],
    ) {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\nConnection: close\r\n"
        );
        for (key, value) in headers {
            response.push_str(&format!("{key}: {value}\r\n"));
        }
        response.push_str("\r\n");
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn base64_encode(input: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let first = chunk[0] as u32;
            let second = *chunk.get(1).unwrap_or(&0) as u32;
            let third = *chunk.get(2).unwrap_or(&0) as u32;
            let combined = (first << 16) | (second << 8) | third;
            out.push(TABLE[((combined >> 18) & 63) as usize] as char);
            out.push(TABLE[((combined >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                TABLE[((combined >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                TABLE[(combined & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::FakeWebDav;
    use super::*;
    use tempfile::tempdir;

    fn config_for(server: &FakeWebDav) -> WebDavConfig {
        WebDavConfig {
            base_url: server.base_url(),
            username: "me".into(),
            password: "pw".into(),
            remote_dir: "ccswitchx".into(),
        }
    }

    fn client_for(server: &FakeWebDav) -> WebDavClient {
        WebDavClient::new(config_for(server), None).unwrap()
    }

    fn temp_file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, bytes).unwrap();
        file
    }

    #[test]
    fn mkcol_is_ok_when_directory_exists() {
        let server = FakeWebDav::start("me", "pw");
        let client = client_for(&server);
        client.ensure_dir().unwrap();
        client.ensure_dir().unwrap();
        assert!(server.request_count("MKCOL") >= 2);
    }

    #[test]
    fn head_missing_returns_exists_false() {
        let server = FakeWebDav::start("me", "pw");
        let client = client_for(&server);
        let remote = client.head().unwrap();
        assert!(!remote.exists);
    }

    #[test]
    fn get_downloads_object() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object(
            "/dav/ccswitchx/ccswitchx-backup.zip",
            b"zip-bytes".to_vec(),
        );
        let client = client_for(&server);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("download.zip");
        let remote = client.get_to_file(&dest, MAX_CLOUD_BYTES).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"zip-bytes");
        assert_eq!(remote.size_bytes, Some(9));
    }

    #[test]
    fn get_rejects_oversize_before_writing() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", vec![0; 32]);
        let client = client_for(&server);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("download.zip");
        assert!(client.get_to_file(&dest, 8).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn wrong_password_returns_auth_error() {
        let server = FakeWebDav::start("me", "pw");
        let mut config = config_for(&server);
        config.password = "wrong".into();
        let client = WebDavClient::new(config, None).unwrap();
        let error = client.test().unwrap_err().to_string();
        assert!(error.contains("认证失败"));
        assert!(!error.contains("wrong"));
    }

    #[test]
    fn redirect_not_followed() {
        let server = FakeWebDav::start("me", "pw");
        server.set_redirect(true);
        let client = client_for(&server);
        assert!(client.test().is_err());
    }

    #[test]
    fn put_replaces_object_via_move() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"old".to_vec());
        let client = client_for(&server);
        let file = temp_file(b"new-zip");
        let remote = client.put(file.path()).unwrap();
        assert_eq!(
            server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(),
            b"new-zip"
        );
        assert_eq!(remote.size_bytes, Some(7));
        assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
    }

    #[test]
    fn move_unsupported_falls_back_to_put() {
        let server = FakeWebDav::start("me", "pw");
        server.set_reject_move(true);
        let client = client_for(&server);
        let file = temp_file(b"fallback");
        client.put(file.path()).unwrap();
        assert_eq!(
            server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(),
            b"fallback"
        );
        assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
    }

    #[test]
    fn move_conflict_falls_back_to_put() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"old".to_vec());
        server.set_move_conflict(true);
        let client = client_for(&server);
        let file = temp_file(b"new-zip");
        let remote = client.put(file.path()).unwrap();
        assert_eq!(
            server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(),
            b"new-zip"
        );
        assert_eq!(remote.size_bytes, Some(7));
        assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
    }

    #[test]
    fn put_and_get_manifest_roundtrip() {
        let server = FakeWebDav::start("me", "pw");
        let client = client_for(&server);
        client.put_manifest(br#"{"format":"ccswitchx-cloud"}"#).unwrap();
        assert_eq!(
            client.get_manifest().unwrap().unwrap(),
            br#"{"format":"ccswitchx-cloud"}"#
        );
    }

    #[test]
    fn missing_manifest_conflict_reads_as_absent() {
        let server = FakeWebDav::start("me", "pw");
        server.set_missing_get_conflict(true);
        let client = client_for(&server);
        assert!(client.get_manifest().unwrap().is_none());
    }

    #[test]
    fn failed_move_cleans_temp_object() {
        let server = FakeWebDav::start("me", "pw");
        server.set_fail_move(true);
        let client = client_for(&server);
        let file = temp_file(b"data");
        assert!(client.put(file.path()).is_err());
        assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
    }

    #[test]
    fn failed_direct_put_keeps_complete_temp_object() {
        let server = FakeWebDav::start("me", "pw");
        server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"old".to_vec());
        server.set_reject_move(true);
        server.set_fail_put_after(1);
        let client = client_for(&server);
        let file = temp_file(b"new-zip");
        assert!(client.put(file.path()).is_err());
        assert_eq!(
            server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(),
            b"old",
            "直写失败不应改动旧副本"
        );
        assert!(
            server.paths().iter().any(|path| path.contains(".tmp-")),
            "回退直写失败时应保留完整临时对象供重试"
        );
    }

    #[test]
    fn put_verifies_remote_size() {
        let server = FakeWebDav::start("me", "pw");
        server.set_truncate_after_put(true);
        let client = client_for(&server);
        let file = temp_file(b"0123456789");
        let error = client.put(file.path()).unwrap_err().to_string();
        assert!(error.contains("大小不一致"));
    }

    #[test]
    fn put_succeeds_when_head_reports_zero_like_jianguoyun() {
        // 坚果云对 HEAD 一律回 Content-Length: 0；真实大小只能从 PROPFIND 拿到。
        let server = FakeWebDav::start("me", "pw");
        server.set_head_zero_length(true);
        let client = client_for(&server);
        let file = temp_file(b"0123456789");
        let remote = client.put(file.path()).unwrap();
        assert_eq!(remote.size_bytes, Some(10));
        assert_eq!(client.head().unwrap().size_bytes, Some(10));
    }

    #[test]
    fn put_still_verifies_size_when_propfind_unsupported() {
        let server = FakeWebDav::start("me", "pw");
        server.set_unsupported_propfind(true);
        server.set_truncate_after_put(true);
        let client = client_for(&server);
        let file = temp_file(b"0123456789");
        let error = client.put(file.path()).unwrap_err().to_string();
        assert!(error.contains("大小不一致"));
    }

    #[test]
    fn propfind_document_from_real_server_is_parsed() {
        // 坚果云 dav.jianguoyun.com 的 Depth: 0 原样应答。
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?><d:multistatus xmlns:d="DAV:" xmlns:s="http://ns.jianguoyun.com"><d:response><d:href>/dav/ccswitchx/ccswitchx-backup.zip</d:href><d:propstat><d:prop><d:getetag>7vnBCItOLq839ZdlmYBpLQ</d:getetag><d:getcontenttype>application/zip</d:getcontenttype><d:displayname>ccswitchx-backup.zip</d:displayname><d:getcontentlength>13676741</d:getcontentlength><d:getlastmodified>Thu, 29 Oct 2026 02:47:32 GMT</d:getlastmodified><d:resourcetype/></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let remote = remote_from_propfind(xml).expect("应解析出元数据");
        assert_eq!(remote.size_bytes, Some(13_676_741));
        assert_eq!(remote.etag.as_deref(), Some("7vnBCItOLq839ZdlmYBpLQ"));
        assert!(remote.modified_at.is_some());
    }

    #[test]
    fn propfind_parsing_tolerates_other_name_prefixes() {
        let xml = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:propstat><D:prop><lp1:getcontentlength xmlns:lp1="DAV:">42</lp1:getcontentlength></D:prop></D:propstat></D:response></D:multistatus>"#;
        assert_eq!(remote_from_propfind(xml).unwrap().size_bytes, Some(42));
    }

    #[test]
    fn propfind_without_known_properties_falls_back_to_head() {
        assert!(remote_from_propfind("<d:multistatus xmlns:d=\"DAV:\"/>").is_none());
    }
}
