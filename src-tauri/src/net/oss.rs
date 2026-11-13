//! 阿里云 OSS 传输（S3 兼容接口）。
//!
//! 传输保持同步（`reqwest::blocking`），由调用方放进 `spawn_blocking`。
//! 请求按 AWS Signature V4 签名；AccessKey Secret 只用于本地计算，
//! 只出现在 `Authorization` 头里，不写进 URL，也不落盘到设置文件。

use crate::error::AppError;
use crate::net::webdav::{
    download_to_file, remote_from_headers, transfer_timeout, DownloadOutcome, RemoteObject,
    BACKUP_OBJECT, MANIFEST_OBJECT, MAX_CLOUD_BYTES, MAX_MANIFEST_BYTES,
};
use crate::net::ProxyConfig;
use hmac::{Hmac, Mac};
use reqwest::blocking::{Body, Client, RequestBuilder};
use reqwest::header::{
    HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE,
};
use reqwest::{Method, StatusCode, Url};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;

/// 空请求体的 sha256；GET / HEAD / 删除类请求的签名都用它。
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// S3 兼容接口固定用 `s3` 作为签名服务名，与 OSS 原生 V4 的 `oss` 不同。
const SERVICE: &str = "s3";
/// 签名的请求头集合；Range 不参与签名，续传时可以直接改。
const SIGNED_HEADERS: &str = "host;x-amz-content-sha256;x-amz-date";
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct OssConfig {
    /// 地域 ID，如 `cn-hangzhou`。
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    /// 对象前缀，等价于 WebDAV 的远端目录。
    pub prefix: String,
    /// 自定义 Endpoint（可带协议）。留空时按地域拼阿里云公网 Endpoint；
    /// 测试替身也通过它指向本地服务器。
    pub endpoint: Option<String>,
}

pub struct OssClient {
    client: Client,
    config: OssConfig,
    base: Url,
    /// 自定义 Endpoint 走 path-style（`/bucket/key`）；阿里云默认域名走 virtual-hosted。
    path_style: bool,
}

impl OssClient {
    pub fn new(config: OssConfig, proxy: Option<&ProxyConfig>) -> Result<Self, AppError> {
        if config.region.trim().is_empty() {
            return Err(AppError::InvalidInput("OSS 地域不能为空".into()));
        }
        if config.bucket.trim().is_empty() {
            return Err(AppError::InvalidInput("OSS Bucket 不能为空".into()));
        }
        let custom = config
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let (base, path_style) = match custom {
            Some(endpoint) => {
                let raw = endpoint.trim_end_matches('/');
                let with_scheme = if raw.starts_with("http://") || raw.starts_with("https://") {
                    raw.to_string()
                } else {
                    format!("https://{raw}")
                };
                let url = Url::parse(&with_scheme)
                    .map_err(|error| AppError::InvalidInput(format!("OSS Endpoint 无效：{error}")))?;
                (url, true)
            }
            None => {
                let url = Url::parse(&format!(
                    "https://{}.s3.oss-{}.aliyuncs.com",
                    config.bucket.trim(),
                    config.region.trim()
                ))
                .map_err(|error| AppError::InvalidInput(format!("OSS Endpoint 无效：{error}")))?;
                (url, false)
            }
        };

        let mut builder = Client::builder()
            .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONTROL_TIMEOUT)
            .timeout(CONTROL_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = proxy {
            builder = proxy.apply_blocking(builder)?;
        }
        let client = builder
            .build()
            .map_err(|error| AppError::Cloud(format!("创建 OSS 客户端失败：{error}")))?;
        Ok(Self {
            client,
            config,
            base,
            path_style,
        })
    }

    pub fn head(&self) -> Result<RemoteObject, AppError> {
        self.head_object(BACKUP_OBJECT)
    }

    /// 读对象元数据。OSS 没有目录概念，也不需要 WebDAV 的 PROPFIND。
    fn head_object(&self, name: &str) -> Result<RemoteObject, AppError> {
        let response = self
            .signed(Method::HEAD, self.object_url(name)?, EMPTY_SHA256)?
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

    /// 写 manifest；包先落地、manifest 最后写，远端读到 manifest 就说明包是完整的。
    pub fn put_manifest(&self, bytes: &[u8]) -> Result<RemoteObject, AppError> {
        let length = bytes.len() as u64;
        if length > MAX_MANIFEST_BYTES {
            return Err(AppError::Cloud(format!(
                "manifest 超过上限（{MAX_MANIFEST_BYTES} 字节）"
            )));
        }
        let hash = sha256_hex(bytes);
        let response = self
            .signed(Method::PUT, self.object_url(MANIFEST_OBJECT)?, &hash)?
            .timeout(transfer_timeout(length))
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

    /// 读 manifest；远端没有时返回 None。
    pub fn get_manifest(&self) -> Result<Option<Vec<u8>>, AppError> {
        let response = self
            .signed(Method::GET, self.object_url(MANIFEST_OBJECT)?, EMPTY_SHA256)?
            .send()
            .map_err(transport_error("GET"))?;
        let status = response.status();
        match status.as_u16() {
            200 => {}
            404 => return Ok(None),
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

    /// 上传备份包到最终对象，PUT 成功后再 HEAD 回读校验大小。
    pub fn put(&self, source: &Path) -> Result<RemoteObject, AppError> {
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
        let payload_hash = sha256_file(source)?;
        let file = fs::File::open(source)?;
        let response = self
            .signed(Method::PUT, self.object_url(BACKUP_OBJECT)?, &payload_hash)?
            .timeout(transfer_timeout(length))
            .header(CONTENT_TYPE, "application/zip")
            .header(CONTENT_LENGTH, length)
            .body(Body::sized(file, length))
            .send()
            .map_err(transport_error("PUT"))?;
        let status = response.status();
        if !status.is_success() {
            if matches!(status.as_u16(), 401 | 403) {
                return Err(auth_error(status));
            }
            return Err(status_error("PUT", status));
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

    pub fn get_to_file(
        &self,
        dest: &Path,
        max_bytes: u64,
        on_progress: Option<&dyn Fn(u64, u64)>,
    ) -> Result<DownloadOutcome, AppError> {
        let url = self.object_url(BACKUP_OBJECT)?;
        let sign_url = url.clone();
        download_to_file(
            &self.client,
            url,
            dest,
            max_bytes,
            on_progress,
            // 每次（含续传）都重新签名：时间戳过期后旧签名会被 OSS 拒绝。
            move |request, _written| match self.signature_headers(
                &Method::GET,
                &sign_url,
                EMPTY_SHA256,
            ) {
                Ok(headers) => request.headers(headers),
                Err(_) => request,
            },
        )
    }

    /// 连接测试：HEAD 备份对象；对象不存在（404）视为连接可用。
    pub fn test(&self) -> Result<(), AppError> {
        let _ = self.head()?;
        Ok(())
    }

    fn object_key(&self, name: &str) -> String {
        let prefix = self.config.prefix.trim_matches('/');
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        }
    }

    fn object_url(&self, name: &str) -> Result<Url, AppError> {
        let key = self.object_key(name);
        let mut url = self.base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| AppError::InvalidInput("OSS Endpoint 格式不支持追加路径".into()))?;
            segments.pop_if_empty();
            if self.path_style {
                segments.push(self.config.bucket.trim());
            }
            for segment in key.split('/').filter(|segment| !segment.is_empty()) {
                segments.push(segment);
            }
        }
        Ok(url)
    }

    /// 构造一个已签名的请求；`payload_hash` 必须是实际发送正文的 sha256。
    fn signed(
        &self,
        method: Method,
        url: Url,
        payload_hash: &str,
    ) -> Result<RequestBuilder, AppError> {
        let headers = self.signature_headers(&method, &url, payload_hash)?;
        Ok(self.client.request(method, url).headers(headers))
    }

    /// 计算一次请求的签名头。续传时调用方会重新调用，时间戳始终是当时的。
    fn signature_headers(
        &self,
        method: &Method,
        url: &Url,
        payload_hash: &str,
    ) -> Result<HeaderMap, AppError> {
        let now = chrono::Utc::now();
        let datetime = now.format("%Y%m%dT%H%M%SZ").to_string();
        let host = host_header(url)?;
        let authorization = authorization_header(
            self.config.access_key_id.trim(),
            &self.config.access_key_secret,
            self.config.region.trim(),
            method.as_str(),
            url.path(),
            url.query().unwrap_or(""),
            &host,
            payload_hash,
            &datetime,
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-date",
            HeaderValue::from_str(&datetime)
                .map_err(|error| AppError::Cloud(format!("签名时间戳无效：{error}")))?,
        );
        headers.insert(
            "x-amz-content-sha256",
            HeaderValue::from_str(payload_hash)
                .map_err(|error| AppError::Cloud(format!("签名内容哈希无效：{error}")))?,
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&authorization)
                .map_err(|error| AppError::Cloud(format!("签名头无效：{error}")))?,
        );
        Ok(headers)
    }
}

fn auth_error(status: StatusCode) -> AppError {
    AppError::CloudAuth(format!(
        "OSS 认证失败（HTTP {status}），请检查 AccessKey ID 与 Secret"
    ))
}

fn status_error(operation: &str, status: StatusCode) -> AppError {
    AppError::Cloud(format!("OSS {operation} 失败：HTTP {status}"))
}

fn transport_error(operation: &'static str) -> impl Fn(reqwest::Error) -> AppError {
    move |error| AppError::Cloud(format!("OSS {operation} 请求失败：{error}"))
}

fn header_u64(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn sha256_file(path: &Path) -> Result<String, AppError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn signing_key(secret: &str, date: &str, region: &str) -> Vec<u8> {
    let date_key = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, SERVICE.as_bytes());
    hmac_sha256(&service_key, b"aws4_request")
}

fn host_header(url: &Url) -> Result<String, AppError> {
    let host = url
        .host_str()
        .ok_or_else(|| AppError::InvalidInput("OSS Endpoint 缺少主机名".into()))?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// 计算 SigV4 `Authorization` 头；`datetime` 形如 `20240101T000000Z`。
#[allow(clippy::too_many_arguments)]
fn authorization_header(
    access_key_id: &str,
    secret: &str,
    region: &str,
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    host: &str,
    payload_hash: &str,
    datetime: &str,
) -> String {
    let date = &datetime[..8];
    let scope = format!("{date}/{region}/{SERVICE}/aws4_request");
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{datetime}\n");
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{SIGNED_HEADERS}\n{payload_hash}"
    );
    let string_to_sign = format!(
        "{ALGORITHM}\n{datetime}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signature = hex(&hmac_sha256(
        &signing_key(secret, date, region),
        string_to_sign.as_bytes(),
    ));
    format!(
        "{ALGORITHM} Credential={access_key_id}/{scope}, SignedHeaders={SIGNED_HEADERS}, Signature={signature}"
    )
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Default)]
    struct FakeState {
        objects: HashMap<String, Vec<u8>>,
        requests: Vec<(String, String)>,
        reject_auth: bool,
    }

    /// 最小 OSS S3 兼容服务：只实现 HEAD / GET / PUT 与固定的 path-style 寻址，
    /// 用来验证签名头、对象读写与错误映射；不校验签名值本身。
    pub(crate) struct FakeOss {
        addr: SocketAddr,
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeOss {
        pub(crate) fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake oss");
            let addr = listener.local_addr().expect("fake oss addr");
            let state = Arc::new(Mutex::new(FakeState::default()));
            let shared = state.clone();
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let state = shared.clone();
                    thread::spawn(move || handle(stream, state));
                }
            });
            Self { addr, state }
        }

        pub(crate) fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        pub(crate) fn put_object(&self, key: &str, bytes: Vec<u8>) {
            self.state
                .lock()
                .unwrap()
                .objects
                .insert(key.to_string(), bytes);
        }

        pub(crate) fn object(&self, key: &str) -> Option<Vec<u8>> {
            self.state.lock().unwrap().objects.get(key).cloned()
        }

        pub(crate) fn reject_auth(&self) {
            self.state.lock().unwrap().reject_auth = true;
        }

        pub(crate) fn request_count(&self, method: &str) -> usize {
            self.state
                .lock()
                .unwrap()
                .requests
                .iter()
                .filter(|(recorded, _)| recorded == method)
                .count()
        }
    }

    fn handle(mut stream: TcpStream, state: Arc<Mutex<FakeState>>) {
        let Some((method, target, headers, body)) = read_request(&mut stream) else {
            return;
        };
        state
            .lock()
            .unwrap()
            .requests
            .push((method.clone(), target.clone()));

        let authorized = headers
            .get("authorization")
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256 "));
        if !authorized || state.lock().unwrap().reject_auth {
            respond(&mut stream, "403 Forbidden", &[], b"");
            return;
        }

        // path-style：/bucket/key；整段保留，测试里可以直观看到 bucket。
        let key = target.trim_start_matches('/').to_string();
        match method.as_str() {
            "HEAD" => {
                let found = state.lock().unwrap().objects.get(&key).cloned();
                match found {
                    Some(bytes) => respond_head(&mut stream, "200 OK", bytes.len() as u64),
                    None => respond_head(&mut stream, "404 Not Found", 0),
                }
            }
            "GET" => {
                let found = state.lock().unwrap().objects.get(&key).cloned();
                match found {
                    Some(bytes) => respond(&mut stream, "200 OK", &[], &bytes),
                    None => respond(&mut stream, "404 Not Found", &[], b""),
                }
            }
            "PUT" => {
                state.lock().unwrap().objects.insert(key, body);
                respond(&mut stream, "200 OK", &[("ETag", "\"fake\"".to_string())], b"");
            }
            _ => respond(&mut stream, "405 Method Not Allowed", &[], b""),
        }
    }

    fn read_request(
        stream: &mut TcpStream,
    ) -> Option<(String, String, HashMap<String, String>, Vec<u8>)> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 8192];
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

    fn respond(stream: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &[u8]) {
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

    fn respond_head(stream: &mut TcpStream, status: &str, length: u64) {
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::oss::test_support::FakeOss;
    use tempfile::tempdir;

    fn client_for(server: &FakeOss) -> OssClient {
        OssClient::new(
            OssConfig {
                region: "cn-hangzhou".into(),
                bucket: "test-bucket".into(),
                access_key_id: "AKID".into(),
                access_key_secret: "SECRET".into(),
                prefix: "ccswitchx".into(),
                endpoint: Some(server.base_url()),
            },
            None,
        )
        .unwrap()
    }

    #[test]
    fn head_missing_returns_exists_false() {
        let server = FakeOss::start();
        let client = client_for(&server);
        assert!(!client.head().unwrap().exists);
    }

    #[test]
    fn put_uploads_with_prefix_and_head_confirms_size() {
        let server = FakeOss::start();
        let client = client_for(&server);
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, b"zip-bytes").unwrap();

        let remote = client.put(file.path()).unwrap();
        assert!(remote.exists);
        assert_eq!(remote.size_bytes, Some(9));
        assert_eq!(
            server.object("test-bucket/ccswitchx/ccswitchx-backup.zip"),
            Some(b"zip-bytes".to_vec())
        );
    }

    #[test]
    fn manifest_roundtrip() {
        let server = FakeOss::start();
        let client = client_for(&server);
        client.put_manifest(br#"{"format":"ccswitchx-cloud"}"#).unwrap();
        let bytes = client.get_manifest().unwrap().unwrap();
        assert_eq!(bytes, br#"{"format":"ccswitchx-cloud"}"#.to_vec());
    }

    #[test]
    fn get_downloads_object_and_reports_hash() {
        let server = FakeOss::start();
        server.put_object("test-bucket/ccswitchx/ccswitchx-backup.zip", b"zip-bytes".to_vec());
        let client = client_for(&server);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("download.zip");
        let downloaded = client.get_to_file(&dest, MAX_CLOUD_BYTES, None).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"zip-bytes");
        assert_eq!(downloaded.sha256, sha256_hex(b"zip-bytes"));
    }

    #[test]
    fn auth_rejection_maps_to_cloud_auth() {
        let server = FakeOss::start();
        server.reject_auth();
        let client = client_for(&server);
        let error = client.head().unwrap_err();
        assert!(matches!(error, AppError::CloudAuth(_)), "{error}");
    }

    #[test]
    fn object_key_joins_prefix() {
        let server = FakeOss::start();
        let client = client_for(&server);
        assert_eq!(
            client.object_key("manifest.json"),
            "ccswitchx/manifest.json"
        );
    }


    #[test]
    fn authorization_header_has_expected_shape() {
        let header = authorization_header(
            "AKID",
            "SECRET",
            "cn-hangzhou",
            "GET",
            "/ccswitchx/manifest.json",
            "",
            "examplebucket.s3.oss-cn-hangzhou.aliyuncs.com",
            EMPTY_SHA256,
            "20240102T030405Z",
        );
        assert!(header.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKID/20240102/cn-hangzhou/s3/aws4_request"
        ));
        assert!(header.contains(
            "SignedHeaders=host;x-amz-content-sha256;x-amz-date"
        ));
        let signature = header.rsplit("Signature=").next().unwrap();
        assert_eq!(signature.len(), 64, "SigV4 签名是 64 位十六进制");
        assert!(signature.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn authorization_changes_with_region_and_payload() {
        let base = authorization_header(
            "AKID", "SECRET", "cn-hangzhou", "GET", "/a", "", "h", EMPTY_SHA256,
            "20240102T030405Z",
        );
        let other_region = authorization_header(
            "AKID", "SECRET", "cn-beijing", "GET", "/a", "", "h", EMPTY_SHA256,
            "20240102T030405Z",
        );
        let other_payload = authorization_header(
            "AKID", "SECRET", "cn-hangzhou", "GET", "/a", "", "h", "abc",
            "20240102T030405Z",
        );
        assert_ne!(base, other_region);
        assert_ne!(base, other_payload);
    }
}
