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
pub const MAX_CLOUD_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_REMOTE_DIR: &str = "ccswitchx";

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
        let response = self
            .request(Method::HEAD, self.object_url()?)
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
        if move_status.as_u16() == 405 || move_status.as_u16() == 501 {
            // 直写成功前不动临时对象：直写失败时它是唯一的完整副本。
            self.put_file(source, length, final_url)?;
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
        let segments: Vec<&str> = self.config.remote_dir.split('/').collect();
        let dir = self.dir_url(&segments)?;
        dir.join(name)
            .map_err(|error| AppError::Cloud(format!("拼接临时对象地址失败：{error}")))
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
        let segments: Vec<&str> = self.config.remote_dir.split('/').collect();
        let dir = self.dir_url(&segments)?;
        dir.join(BACKUP_OBJECT)
            .map_err(|error| AppError::Cloud(format!("拼接远端对象地址失败：{error}")))
    }
}

fn mkcol_method() -> Method {
    Method::from_bytes(b"MKCOL").expect("MKCOL is a valid method")
}

fn move_method() -> Method {
    Method::from_bytes(b"MOVE").expect("MOVE is a valid method")
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
        redirect: bool,
        fail_put: bool,
        fail_put_after: Option<usize>,
        put_attempts: usize,
        truncate_after_put: bool,
        head_size_override: Option<u64>,
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
                redirect: false,
                fail_put: false,
                fail_put_after: None,
                put_attempts: 0,
                truncate_after_put: false,
                head_size_override: None,
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
                None => respond(&mut stream, "404 Not Found", &[], b""),
            },
            "HEAD" => match state.objects.get(&path) {
                Some(bytes) => {
                    let length = state.head_size_override.unwrap_or(bytes.len() as u64);
                    respond_head(&mut stream, "200 OK", length, &object_headers());
                }
                None => respond_head(&mut stream, "404 Not Found", 0, &[]),
            },
            _ => respond(&mut stream, "405 Method Not Allowed", &[], b""),
        }
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
}
