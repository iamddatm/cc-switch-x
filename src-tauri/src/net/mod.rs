//! 上游 Skill 仓库的获取：GitHub 输入解析与归档下载。
//!
//! 网络层刻意保持同步（`reqwest::blocking`），由调用方放进 `spawn_blocking`；
//! 测试通过 `RepoFetcher` trait 注入本地 fixture，不依赖真实网络。

pub mod archive;

use crate::error::AppError;
use std::io::{Read, Write};
use std::time::Duration;
use tempfile::TempDir;

/// 单个仓库归档的下载上限（与原版 cc-switch 一致）。
pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

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
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
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
