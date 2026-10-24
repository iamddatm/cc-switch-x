//! Skills 发现：读取配置的 GitHub 仓库并递归扫描 `SKILL.md`。
//!
//! 仓库归档下载很慢，多个仓库按小块并行；单个仓库失败只进 `failures`，
//! 不影响其他仓库的结果。skills.sh 作为补充目录，只做搜索，不落任何缓存。

use crate::domain::AssetType;
use crate::error::AppError;
use crate::net::archive::scan_skills;
use crate::net::{parse_github_input, ProxyConfig, RepoFetcher};
use crate::services::skill_source::github_candidate_id;
use crate::settings::SkillRepo;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const MAX_SKILLS_PER_REPO: usize = 2000;
const DISCOVERY_CONCURRENCY: usize = 4;
pub const SKILLS_SH_MAX_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverableSkill {
    /// 仓库 + 分支 + 路径，列表行的稳定标识。
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub relative_path: String,
    pub suggested_id: String,
    pub repo: String,
    pub branch: String,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoFailure {
    pub repo: String,
    pub branch: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDiscoveryReport {
    pub skills: Vec<DiscoverableSkill>,
    pub failures: Vec<SkillRepoFailure>,
}

/// 停用的仓库不读也不报。
pub fn discover<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    repos: &[SkillRepo],
) -> SkillDiscoveryReport {
    let enabled: Vec<&SkillRepo> = repos.iter().filter(|repo| repo.enabled).collect();
    let mut report = SkillDiscoveryReport::default();

    for chunk in enabled.chunks(DISCOVERY_CONCURRENCY) {
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|repo| scope.spawn(move || discover_repo(store, fetcher, repo)))
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join())
                .collect::<Vec<_>>()
        });
        for (repo, result) in chunk.iter().zip(results) {
            match result {
                Ok(Ok(skills)) => report.skills.extend(skills),
                Ok(Err(error)) => report.failures.push(SkillRepoFailure {
                    repo: repo.slug(),
                    branch: repo.branch.clone(),
                    error: error.to_string(),
                }),
                Err(_) => report.failures.push(SkillRepoFailure {
                    repo: repo.slug(),
                    branch: repo.branch.clone(),
                    error: "仓库读取任务异常退出".into(),
                }),
            }
        }
    }

    report.skills.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.key.cmp(&right.key))
    });
    report
}

fn discover_repo<F: RepoFetcher>(
    store: &Store,
    fetcher: &F,
    repo: &SkillRepo,
) -> Result<Vec<DiscoverableSkill>, AppError> {
    let parsed = parse_github_input(&repo.slug(), Some(repo.branch.clone()))?;
    let fetched = fetcher.fetch(&parsed)?;
    build_from_dir(store, fetched.dir.path(), repo, &fetched.branch)
}

/// 扫描一个已经解压的仓库目录；单独拆出来便于用本地 fixture 测试。
pub fn build_from_dir(
    store: &Store,
    root: &std::path::Path,
    repo: &SkillRepo,
    branch: &str,
) -> Result<Vec<DiscoverableSkill>, AppError> {
    let scanned = scan_skills(root, MAX_SKILLS_PER_REPO)?;
    Ok(scanned
        .into_iter()
        .map(|skill| {
            let suggested_id = github_candidate_id(&skill.relative_path, &repo.owner, &repo.name);
            let installed = store
                .entry_path(AssetType::Skill, &suggested_id)
                .map(|path| path.exists())
                .unwrap_or(false);
            DiscoverableSkill {
                key: format!(
                    "{}/{}@{branch}:{}",
                    repo.owner, repo.name, skill.relative_path
                ),
                name: skill.name,
                description: skill.description,
                relative_path: skill.relative_path,
                suggested_id,
                repo: repo.slug(),
                branch: branch.to_string(),
                installed,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsShSkill {
    pub key: String,
    pub name: String,
    /// skills.sh 的 skillId，可能只是目录名的一部分，安装时再做候选匹配。
    pub directory: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub repo_branch: String,
    pub installs: u64,
    pub readme_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsShSearchResult {
    pub skills: Vec<SkillsShSkill>,
    pub total_count: usize,
    pub query: String,
}

#[derive(Debug, Deserialize)]
struct SkillsShResponse {
    #[serde(default)]
    skills: Vec<SkillsShApiSkill>,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsShApiSkill {
    #[serde(default)]
    id: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    skill_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    installs: u64,
}

pub fn search_skills_sh(
    proxy: Option<&ProxyConfig>,
    query: &str,
    limit: usize,
    offset: usize,
) -> Result<SkillsShSearchResult, AppError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(AppError::InvalidInput("请输入要搜索的 Skill 关键词".into()));
    }
    if query.chars().count() > 100 {
        return Err(AppError::InvalidInput("搜索关键词过长".into()));
    }
    let limit = limit.clamp(1, SKILLS_SH_MAX_LIMIT);
    if offset > 10_000 {
        return Err(AppError::InvalidInput("搜索结果翻页过深".into()));
    }

    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15));
    if let Some(proxy) = proxy {
        builder = proxy.apply_blocking(builder)?;
    }
    let client = builder
        .build()
        .map_err(|error| AppError::Network(format!("创建搜索客户端失败：{error}")))?;

    let response = client
        .get("https://skills.sh/api/search")
        .query(&[
            ("q", query),
            ("limit", &limit.to_string()),
            ("offset", &offset.to_string()),
        ])
        .send()
        .map_err(|error| AppError::Network(format!("skills.sh 搜索失败：{error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::Network(format!("skills.sh 返回 HTTP {status}")));
    }
    let body: SkillsShResponse = response
        .json()
        .map_err(|error| AppError::Network(format!("skills.sh 响应无法解析：{error}")))?;

    let mut skills = Vec::new();
    for item in body.skills {
        // 只接受能落到 GitHub 归档上的坐标，同时挡掉非 GitHub 来源。
        let Ok(parsed) = parse_github_input(&item.source, Some("main".into())) else {
            continue;
        };
        let name = if item.name.is_empty() {
            item.skill_id.clone()
        } else {
            item.name.clone()
        };
        if name.is_empty() {
            continue;
        }
        let key = if item.id.is_empty() {
            format!("{}:{}", parsed.slug(), item.skill_id)
        } else {
            item.id
        };
        skills.push(SkillsShSkill {
            key,
            name,
            directory: item.skill_id,
            readme_url: format!("https://github.com/{}", parsed.slug()),
            repo_owner: parsed.owner,
            repo_name: parsed.name,
            repo_branch: "main".into(),
            installs: item.installs,
        });
    }
    let total_count = body.count.max(skills.len());
    Ok(SkillsShSearchResult {
        skills,
        total_count,
        query: if body.query.is_empty() {
            query.to_string()
        } else {
            body.query
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn repo() -> SkillRepo {
        SkillRepo {
            owner: "acme".into(),
            name: "demo".into(),
            branch: "main".into(),
            enabled: true,
        }
    }

    #[test]
    fn scans_nested_skills_and_reports_installed_by_candidate_id() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::write(root.join("README.md"), "x").ok();
        std::fs::create_dir_all(root.join("skills/pdf")).unwrap();
        std::fs::write(
            root.join("skills/pdf/SKILL.md"),
            "---\nname: pdf\ndescription: PDF 处理\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("writing")).unwrap();
        std::fs::write(root.join("writing/SKILL.md"), "---\nname: writing\n---\n").unwrap();

        let store_root = dir.path().join("store");
        let store = Store::init(store_root).unwrap();
        // 预置一个同 ID 的 Skill，验证发现页会显示「已安装」。
        let existing = github_candidate_id("skills/pdf", "acme", "demo");
        std::fs::create_dir_all(store.entry_path(AssetType::Skill, &existing).unwrap()).unwrap();

        let found = build_from_dir(&store, &root, &repo(), "main").unwrap();
        assert_eq!(found.len(), 2);
        let pdf = found.iter().find(|item| item.name == "pdf").unwrap();
        assert_eq!(pdf.relative_path, "skills/pdf");
        assert_eq!(pdf.description.as_deref(), Some("PDF 处理"));
        assert!(pdf.installed);
        assert!(pdf.key.contains("@main:skills/pdf"));

        let writing = found.iter().find(|item| item.name == "writing").unwrap();
        assert!(!writing.installed);
    }

    #[test]
    fn skips_disabled_repos_and_reports_fetch_failures() {
        struct FailingFetcher;
        impl RepoFetcher for FailingFetcher {
            fn fetch(
                &self,
                repo: &crate::net::RepoRef,
            ) -> Result<crate::net::FetchedRepo, AppError> {
                Err(AppError::Network(format!("{} 无法读取", repo.slug())))
            }
        }

        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let disabled = SkillRepo {
            enabled: false,
            ..repo()
        };
        let enabled = repo();
        let report = discover(&store, &FailingFetcher, &[disabled, enabled]);
        assert!(report.skills.is_empty());
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].repo, "acme/demo");
        assert!(report.failures[0].error.contains("无法读取"));
    }
}
