//! 上游 Skill 仓库的获取：GitHub 输入解析与归档下载。
//!
//! 网络层刻意保持同步（`reqwest::blocking`），由调用方放进 `spawn_blocking`；
//! 测试通过 `RepoFetcher` trait 注入本地 fixture，不依赖真实网络。

pub mod archive;
pub mod webdav;

use crate::error::AppError;
use std::io::{Read, Write};
use std::time::Duration;
use tempfile::TempDir;

/// 单个仓库归档的下载上限（与原版 cc-switch 一致）。
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

/// 全局出站代理。密码由调用方从 secrets 取出，不进设置文件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    pub url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl ProxyConfig {
    pub fn new(url: Option<String>, username: Option<String>, password: Option<String>) -> Self {
        Self {
            url,
            username,
            password,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty())
    }

    /// 校验用户输入；密码可以只在 secrets 里，因此不要求出现在这里。
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.is_enabled() {
            if self
                .username
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
                || self.password.as_deref().is_some_and(|v| !v.is_empty())
            {
                return Err(AppError::InvalidInput(
                    "填写代理用户名或密码前，请先填写代理地址".into(),
                ));
            }
            return Ok(());
        }
        let url = self.url_with_auth()?;
        reqwest::Proxy::all(url.as_str())
            .map_err(|error| AppError::InvalidInput(format!("代理地址不可用：{error}")))?;
        Ok(())
    }

    /// 把认证信息并进 URL。用户名或密码为空时不写入 userinfo。
    pub fn url_with_auth(&self) -> Result<reqwest::Url, AppError> {
        let raw = self
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::InvalidInput("代理地址不能为空".into()))?;
        let mut parsed = reqwest::Url::parse(raw)
            .map_err(|error| AppError::InvalidInput(format!("代理地址格式错误：{error}")))?;
        if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
            return Err(AppError::InvalidInput(
                "代理只支持 http / https / socks5 / socks5h".into(),
            ));
        }
        if parsed.host_str().is_none() {
            return Err(AppError::InvalidInput("代理地址缺少主机名".into()));
        }
        let username = self.username.as_deref().unwrap_or_default().trim();
        let password = self.password.as_deref().unwrap_or_default();
        if !username.is_empty() {
            parsed
                .set_username(username)
                .map_err(|_| AppError::InvalidInput("代理用户名无法写入地址".into()))?;
            if !password.is_empty() {
                parsed
                    .set_password(Some(password))
                    .map_err(|_| AppError::InvalidInput("代理密码无法写入地址".into()))?;
            }
        } else if !password.is_empty() {
            return Err(AppError::InvalidInput(
                "填写代理密码前，请先填写代理用户名".into(),
            ));
        }
        Ok(parsed)
    }

    /// 返回可直接挂到 reqwest 客户端上的代理；直连时为 None。
    pub fn proxy(&self) -> Result<Option<reqwest::Proxy>, AppError> {
        if !self.is_enabled() {
            return Ok(None);
        }
        let url = self.url_with_auth()?;
        let proxy = reqwest::Proxy::all(url.as_str())
            .map_err(|error| AppError::InvalidInput(format!("代理地址不可用：{error}")))?;
        Ok(Some(proxy))
    }

    /// 把配置应用到阻塞客户端。留空时显式关闭系统代理，保证「留空 = 直连」。
    pub fn apply_blocking(
        &self,
        mut builder: reqwest::blocking::ClientBuilder,
    ) -> Result<reqwest::blocking::ClientBuilder, AppError> {
        match self.proxy()? {
            Some(proxy) => builder = builder.proxy(proxy),
            None => builder = builder.no_proxy(),
        }
        Ok(builder)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
    /// None 表示按 main → master 探测。
    pub branch: Option<String>,
}

impl RepoRef {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// 接受 `owner/repo` 或 `https://github.com/owner/repo[.git]`。
/// 分支单独输入；带路径的 tree URL 暂不支持，避免猜错 skill 子目录。
pub fn parse_github_input(input: &str, branch: Option<String>) -> Result<RepoRef, AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::Source("仓库不能为空".into()));
    }
    let path = if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        rest
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(AppError::Source(
            "只支持 https://github.com/<owner>/<repo>".into(),
        ));
    } else {
        trimmed
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.contains(['?', '#', '%', '\\']) {
        return Err(AppError::Source("仓库地址里不能带查询串或转义字符".into()));
    }
    let mut parts = path.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return Err(AppError::Source(
            "仓库格式应为 owner/repo 或 https://github.com/owner/repo".into(),
        ));
    }
    validate_owner(owner)?;
    validate_repo_name(name)?;
    let branch = branch
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(value) = branch.as_deref() {
        validate_branch(value)?;
    }
    Ok(RepoRef {
        owner: owner.to_string(),
        name: name.to_string(),
        branch,
    })
}

fn validate_owner(owner: &str) -> Result<(), AppError> {
    if owner.len() > 39
        || !owner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err(AppError::Source(format!("非法的仓库 owner：{owner}")));
    }
    Ok(())
}

fn validate_repo_name(name: &str) -> Result<(), AppError> {
    if name.len() > 100
        || name == "."
        || name == ".."
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(AppError::Source(format!("非法的仓库名：{name}")));
    }
    Ok(())
}

fn validate_branch(branch: &str) -> Result<(), AppError> {
    if branch.len() > 255 || branch.starts_with('/') || branch.ends_with('/') {
        return Err(AppError::Source(format!("非法的分支名：{branch}")));
    }
    for segment in branch.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(AppError::Source(format!("非法的分支名：{branch}")));
        }
        if !segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        {
            return Err(AppError::Source(format!("非法的分支名：{branch}")));
        }
    }
    Ok(())
}

pub struct FetchedRepo {
    pub dir: TempDir,
    pub branch: String,
}

pub trait RepoFetcher {
    /// 下载并解压仓库归档；返回的临时目录必须活到比对/安装结束。
    fn fetch(&self, repo: &RepoRef) -> Result<FetchedRepo, AppError>;
}

pub struct HttpFetcher {
    client: reqwest::blocking::Client,
}

impl HttpFetcher {
    pub fn new(proxy: Option<&ProxyConfig>) -> Result<Self, AppError> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60));
        if let Some(proxy) = proxy {
            builder = proxy.apply_blocking(builder)?;
        }
        let client = builder
            .build()
            .map_err(|error| AppError::Network(format!("创建 HTTP 客户端失败：{error}")))?;
        Ok(Self { client })
    }
}

impl RepoFetcher for HttpFetcher {
    fn fetch(&self, repo: &RepoRef) -> Result<FetchedRepo, AppError> {
        let branches = branch_candidates(repo);
        for branch in &branches {
            match self.fetch_branch(repo, branch) {
                Ok(dir) => {
                    return Ok(FetchedRepo {
                        dir,
                        branch: branch.clone(),
                    })
                }
                Err(FetchFailure::NotFound) => continue,
                Err(FetchFailure::Fatal(error)) => return Err(error),
            }
        }
        Err(AppError::Network(format!(
            "{} 在分支 {} 上都没有归档",
            repo.slug(),
            branches.join(" / ")
        )))
    }
}

enum FetchFailure {
    NotFound,
    Fatal(AppError),
}

fn branch_candidates(repo: &RepoRef) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(branch) = repo.branch.as_deref() {
        out.push(branch.to_string());
    }
    for fallback in ["main", "master"] {
        if !out.iter().any(|existing| existing == fallback) {
            out.push(fallback.to_string());
        }
    }
    out
}

fn archive_url(repo: &RepoRef, branch: &str) -> String {
    format!(
        "https://github.com/{}/{}/archive/refs/heads/{branch}.zip",
        repo.owner, repo.name
    )
}

impl HttpFetcher {
    fn fetch_branch(&self, repo: &RepoRef, branch: &str) -> Result<TempDir, FetchFailure> {
        let url = archive_url(repo, branch);
        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|error| FetchFailure::Fatal(network_error(repo, branch, &error)))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(FetchFailure::NotFound);
        }
        if !status.is_success() {
            return Err(FetchFailure::Fatal(AppError::Network(format!(
                "{}@{branch} 返回 HTTP {status}",
                repo.slug()
            ))));
        }
        if let Some(length) = response.content_length() {
            if length > MAX_ARCHIVE_BYTES {
                return Err(FetchFailure::Fatal(AppError::Archive(format!(
                    "{}@{branch} 归档大小 {length} 字节，超过上限 {MAX_ARCHIVE_BYTES}",
                    repo.slug()
                ))));
            }
        }

        let mut archive = tempfile::Builder::new()
            .prefix("repo-")
            .suffix(".zip")
            .tempfile()
            .map_err(|error| FetchFailure::Fatal(AppError::Io(error)))?;
        let mut limited = response.take(MAX_ARCHIVE_BYTES + 1);
        let mut buffer = [0u8; 64 * 1024];
        let mut written = 0u64;
        loop {
            let read = limited.read(&mut buffer).map_err(|error| {
                FetchFailure::Fatal(AppError::Network(format!(
                    "{}@{branch} 下载中断：{error}",
                    repo.slug()
                )))
            })?;
            if read == 0 {
                break;
            }
            written += read as u64;
            if written > MAX_ARCHIVE_BYTES {
                return Err(FetchFailure::Fatal(AppError::Archive(format!(
                    "{}@{branch} 归档超过上限 {MAX_ARCHIVE_BYTES} 字节",
                    repo.slug()
                ))));
            }
            archive
                .write_all(&buffer[..read])
                .map_err(|error| FetchFailure::Fatal(AppError::Io(error)))?;
        }
        archive
            .flush()
            .map_err(|error| FetchFailure::Fatal(AppError::Io(error)))?;

        let dir = tempfile::Builder::new()
            .prefix("repo-")
            .tempdir()
            .map_err(|error| FetchFailure::Fatal(AppError::Io(error)))?;
        archive::extract_archive(
            archive.path(),
            dir.path(),
            &archive::ArchiveLimits::default(),
        )
        .map_err(FetchFailure::Fatal)?;
        Ok(dir)
    }
}

fn network_error(repo: &RepoRef, branch: &str, error: &reqwest::Error) -> AppError {
    AppError::Network(format!("{}@{branch} 下载失败：{error}", repo.slug()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_repo_and_https_forms() {
        let repo = parse_github_input("acme/demo", None).unwrap();
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "demo");
        assert_eq!(repo.branch, None);

        let repo =
            parse_github_input("https://github.com/acme/demo.git/", Some(" dev ".into())).unwrap();
        assert_eq!(repo.slug(), "acme/demo");
        assert_eq!(repo.branch.as_deref(), Some("dev"));
    }

    #[test]
    fn rejects_non_github_and_path_traversal() {
        for input in [
            "http://github.com/acme/demo",
            "https://gitlab.com/acme/demo",
            "https://github.com/acme/demo/tree/main/skills",
            "acme/../demo",
            "acme",
            "acme/demo?x=1",
        ] {
            assert!(parse_github_input(input, None).is_err(), "{input}");
        }
        for branch in ["../main", "a//b", "a b", "main#x"] {
            assert!(
                parse_github_input("acme/demo", Some(branch.into())).is_err(),
                "{branch}"
            );
        }
    }

    #[test]
    fn proxy_config_rejects_bad_schemes_and_orphan_credentials() {
        let direct = ProxyConfig::default();
        assert!(!direct.is_enabled());
        assert!(direct.validate().is_ok());

        let auth = ProxyConfig::new(
            Some("http://127.0.0.1:7890".into()),
            Some("me".into()),
            Some("p@ss".into()),
        );
        assert!(auth.validate().is_ok());
        let url = auth.url_with_auth().unwrap();
        assert_eq!(url.username(), "me");
        // Url 保存的是 percent-encoded userinfo，reqwest 解析代理时会还原。
        assert_eq!(url.password(), Some("p%40ss"));
        assert_eq!(url.host_str(), Some("127.0.0.1"));

        for bad in [
            ProxyConfig::new(Some("ftp://127.0.0.1:21".into()), None, None),
            ProxyConfig::new(Some("not a url".into()), None, None),
            ProxyConfig::new(
                Some("http://127.0.0.1:7890".into()),
                None,
                Some("pw".into()),
            ),
            ProxyConfig::new(None, Some("me".into()), None),
        ] {
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn branch_candidates_put_the_requested_branch_first() {
        let repo = RepoRef {
            owner: "a".into(),
            name: "b".into(),
            branch: Some("dev".into()),
        };
        assert_eq!(branch_candidates(&repo), vec!["dev", "main", "master"]);

        let repo = RepoRef {
            owner: "a".into(),
            name: "b".into(),
            branch: None,
        };
        assert_eq!(branch_candidates(&repo), vec!["main", "master"]);
        assert_eq!(
            archive_url(&repo, "main"),
            "https://github.com/a/b/archive/refs/heads/main.zip"
        );
    }
}
