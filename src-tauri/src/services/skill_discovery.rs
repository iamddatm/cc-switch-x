//! Skills 发现：读取配置的 GitHub 仓库并递归扫描 `SKILL.md`。
//!
//! 仓库归档下载很慢，多个仓库按小块并行；单个仓库失败只进 `failures`，
//! 不影响其他仓库的结果。skills.sh 作为补充目录，只做搜索，不落任何缓存。

use crate::domain::skill_meta::{read_meta, SkillMeta};
use crate::domain::AssetType;
use crate::error::AppError;
use crate::net::archive::scan_skills;
use crate::net::{parse_github_input, ProxyConfig, RepoFetcher};
use crate::services::skill_source::github_candidate_id;
use crate::settings::SkillRepo;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
    /// 本次发现使用的仓库：配置仓库 + 已安装 Skill 的来源仓库（含已停用项）。
    pub repos: Vec<SkillRepo>,
}

/// 读取全部已安装 Skill 的来源元数据。
///
/// 索引只是缓存，来源信息以真源的 `.meta.json` 为准；读不到的 Skill 直接跳过。
/// 发现流程每次只扫一遍磁盘，避免按仓库重复读取。
fn installed_metas(store: &Store) -> Vec<SkillMeta> {
    let Ok(ids) = store.list_ids(AssetType::Skill) else {
        return Vec::new();
    };
    ids.into_iter()
        .filter_map(|id| store.entry_path(AssetType::Skill, &id).ok())
        .filter_map(|dir| read_meta(&dir))
        .collect()
}

/// 把已安装 Skill 的来源仓库并入发现列表，让从 skills.sh 等渠道装过的仓库
/// 也能继续浏览；已配置的同名仓库优先，保留用户的启用状态。
pub fn merge_installed_repos(store: &Store, configured: &[SkillRepo]) -> Vec<SkillRepo> {
    merge_repos(&installed_metas(store), configured)
}

fn merge_repos(installed: &[SkillMeta], configured: &[SkillRepo]) -> Vec<SkillRepo> {
    let mut repos = configured.to_vec();
    let mut seen: HashSet<String> = repos
        .iter()
        .map(|repo| repo.slug().to_lowercase())
        .collect();
    for meta in installed {
        let Some(slug) = meta.repo_slug() else {
            continue;
        };
        let Some((owner, name)) = slug.split_once('/') else {
            continue;
        };
        if !seen.insert(slug.to_lowercase()) {
            continue;
        }
        let branch = meta
            .repo_branch
            .as_deref()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .unwrap_or("main")
            .to_string();
        repos.push(SkillRepo {
            owner: owner.to_string(),
            name: name.to_string(),
            branch,
            enabled: true,
        });
    }
    repos
}

/// 发现入口：自动带上已安装 Skill 的来源仓库，并在报告里回传实际仓库列表。
pub fn discover_with_installed<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    configured: &[SkillRepo],
) -> SkillDiscoveryReport {
    let installed = installed_metas(store);
    let repos = merge_repos(&installed, configured);
    discover_repos(store, fetcher, &repos, &installed)
}

/// 停用的仓库不读也不报。
pub fn discover<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    repos: &[SkillRepo],
) -> SkillDiscoveryReport {
    discover_repos(store, fetcher, repos, &installed_metas(store))
}

fn discover_repos<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    repos: &[SkillRepo],
    installed: &[SkillMeta],
) -> SkillDiscoveryReport {
    let enabled: Vec<&SkillRepo> = repos.iter().filter(|repo| repo.enabled).collect();
    let mut report = SkillDiscoveryReport {
        repos: repos.to_vec(),
        ..Default::default()
    };

    for chunk in enabled.chunks(DISCOVERY_CONCURRENCY) {
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|repo| scope.spawn(move || discover_repo(store, fetcher, repo, installed)))
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
    installed: &[SkillMeta],
) -> Result<Vec<DiscoverableSkill>, AppError> {
    let parsed = parse_github_input(&repo.slug(), Some(repo.branch.clone()))?;
    let fetched = fetcher.fetch(&parsed)?;
    build_from_dir_with_installed(store, fetched.dir.path(), repo, &fetched.branch, installed)
}

/// 已安装 Skill 在这个仓库里的路径集合（小写）：新旧两种命名规则都能命中。
fn installed_paths(installed: &[SkillMeta], repo_slug: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for meta in installed {
        let Some(slug) = meta.repo_slug() else {
            continue;
        };
        if !slug.eq_ignore_ascii_case(repo_slug) {
            continue;
        }
        for value in [&meta.repo_path, &meta.source_directory] {
            if let Some(path) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                set.insert(path.to_lowercase());
            }
        }
    }
    set
}

/// 扫描一个已经解压的仓库目录；单独拆出来便于用本地 fixture 测试。
pub fn build_from_dir(
    store: &Store,
    root: &std::path::Path,
    repo: &SkillRepo,
    branch: &str,
) -> Result<Vec<DiscoverableSkill>, AppError> {
    build_from_dir_with_installed(store, root, repo, branch, &installed_metas(store))
}

fn build_from_dir_with_installed(
    store: &Store,
    root: &std::path::Path,
    repo: &SkillRepo,
    branch: &str,
    installed: &[SkillMeta],
) -> Result<Vec<DiscoverableSkill>, AppError> {
    let scanned = scan_skills(root, MAX_SKILLS_PER_REPO)?;
    let known_paths = installed_paths(installed, &repo.slug());
    Ok(scanned
        .into_iter()
        .map(|skill| {
            let suggested_id = github_candidate_id(&skill.relative_path, &repo.owner, &repo.name);
            let is_installed = store
                .entry_path(AssetType::Skill, &suggested_id)
                .map(|path| path.exists())
                .unwrap_or(false)
                || path_is_installed(&skill.relative_path, &known_paths);
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
                installed: is_installed,
            }
        })
        .collect())
}

/// 旧版 cc-switch 导入的 ID 是 owner-repo-skillId，和新命名对不上；
/// 以元数据记录的仓库内路径兜底，避免把已安装项显示成未安装。
fn path_is_installed(relative_path: &str, known_paths: &HashSet<String>) -> bool {
    if known_paths.is_empty() {
        return false;
    }
    let relative = relative_path.to_lowercase();
    let leaf = relative.rsplit('/').next().unwrap_or(&relative);
    known_paths.contains(&relative) || known_paths.contains(leaf)
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

/// skills.sh 趋势榜：站点按固定 200 条一页提供，`page` 从 0 开始。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsShTrendingResult {
    pub skills: Vec<SkillsShSkill>,
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsShTrendingResponse {
    #[serde(default)]
    skills: Vec<SkillsShApiSkill>,
    #[serde(default)]
    has_more: bool,
}

fn skills_sh_client(proxy: Option<&ProxyConfig>) -> Result<reqwest::blocking::Client, AppError> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15));
    if let Some(proxy) = proxy {
        builder = proxy.apply_blocking(builder)?;
    }
    builder
        .build()
        .map_err(|error| AppError::Network(format!("创建 skills.sh 客户端失败：{error}")))
}

fn get_skills_sh_json<T: serde::de::DeserializeOwned>(
    request: reqwest::blocking::RequestBuilder,
    action: &str,
) -> Result<T, AppError> {
    let response = request
        .send()
        .map_err(|error| AppError::Network(format!("skills.sh {action}失败：{error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::Network(format!("skills.sh 返回 HTTP {status}")));
    }
    response
        .json()
        .map_err(|error| AppError::Network(format!("skills.sh 响应无法解析：{error}")))
}

fn map_skills_sh_skill(item: SkillsShApiSkill) -> Option<SkillsShSkill> {
    // 只接受能落到 GitHub 归档上的坐标，同时挡掉非 GitHub 来源。
    let parsed = parse_github_input(&item.source, Some("main".into())).ok()?;
    let name = if item.name.is_empty() {
        item.skill_id.clone()
    } else {
        item.name.clone()
    };
    if name.is_empty() {
        return None;
    }
    let key = if item.id.is_empty() {
        format!("{}:{}", parsed.slug(), item.skill_id)
    } else {
        item.id
    };
    Some(SkillsShSkill {
        key,
        name,
        directory: item.skill_id,
        readme_url: format!("https://github.com/{}", parsed.slug()),
        repo_owner: parsed.owner,
        repo_name: parsed.name,
        repo_branch: "main".into(),
        installs: item.installs,
    })
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

    let client = skills_sh_client(proxy)?;
    let body: SkillsShResponse = get_skills_sh_json(
        client.get("https://skills.sh/api/search").query(&[
            ("q", query),
            ("limit", &limit.to_string()),
            ("offset", &offset.to_string()),
        ]),
        "搜索",
    )?;

    let skills: Vec<SkillsShSkill> = body
        .skills
        .into_iter()
        .filter_map(map_skills_sh_skill)
        .collect();
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

/// 发现页切到 skills.sh 时的默认内容：站点「Trending (24h)」榜单。
pub fn trending_skills_sh(
    proxy: Option<&ProxyConfig>,
    page: usize,
) -> Result<SkillsShTrendingResult, AppError> {
    // 站点总量约一万条、每页 200 条；再往后翻已经没有意义。
    if page > 50 {
        return Err(AppError::InvalidInput("趋势榜翻页过深".into()));
    }

    let client = skills_sh_client(proxy)?;
    let body: SkillsShTrendingResponse = get_skills_sh_json(
        client.get(format!("https://skills.sh/api/skills/trending/{page}")),
        "趋势榜读取",
    )?;

    let skills: Vec<SkillsShSkill> = body
        .skills
        .into_iter()
        .filter_map(map_skills_sh_skill)
        .collect();
    Ok(SkillsShTrendingResult {
        has_more: body.has_more,
        skills,
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
        std::fs::create_dir_all(root.join("skills/legacy")).unwrap();
        std::fs::write(
            root.join("skills/legacy/SKILL.md"),
            "---\nname: legacy\n---\n",
        )
        .unwrap();

        let store_root = dir.path().join("store");
        let store = Store::init(store_root).unwrap();
        // 预置一个同 ID 的 Skill，验证发现页会显示「已安装」。
        let existing = github_candidate_id("skills/pdf", "acme", "demo");
        std::fs::create_dir_all(store.entry_path(AssetType::Skill, &existing).unwrap()).unwrap();
        // 旧版 cc-switch 的目录名与新建议 ID 不同，靠元数据里的仓库路径识别。
        let legacy = store
            .entry_path(AssetType::Skill, "acme-demo-legacy")
            .unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join(".meta.json"),
            r#"{"repoOwner":"acme","repoName":"demo","sourceDirectory":"legacy","repoPath":"skills/legacy"}"#,
        )
        .unwrap();

        let found = build_from_dir(&store, &root, &repo(), "main").unwrap();
        assert_eq!(found.len(), 3);
        let pdf = found.iter().find(|item| item.name == "pdf").unwrap();
        assert_eq!(pdf.relative_path, "skills/pdf");
        assert_eq!(pdf.description.as_deref(), Some("PDF 处理"));
        assert!(pdf.installed);
        assert!(pdf.key.contains("@main:skills/pdf"));

        let writing = found.iter().find(|item| item.name == "writing").unwrap();
        assert!(!writing.installed);

        let legacy = found.iter().find(|item| item.name == "legacy").unwrap();
        assert!(legacy.installed);
    }

    #[test]
    fn maps_trending_items_to_github_coordinates() {
        // 趋势榜条目没有 `id` 字段，key 要用仓库 + skillId 兜底。
        let item: SkillsShApiSkill = serde_json::from_str(
            r#"{"source":"acme/skills","skillId":"pdf","name":"pdf","installs":42}"#,
        )
        .unwrap();
        let mapped = map_skills_sh_skill(item).unwrap();
        assert_eq!(mapped.key, "acme/skills:pdf");
        assert_eq!(mapped.name, "pdf");
        assert_eq!(mapped.directory, "pdf");
        assert_eq!(mapped.repo_owner, "acme");
        assert_eq!(mapped.repo_name, "skills");
        assert_eq!(mapped.installs, 42);
        assert_eq!(mapped.readme_url, "https://github.com/acme/skills");
    }

    fn installed_skill(store: &Store, id: &str, meta: &str) {
        let dir = store.entry_path(AssetType::Skill, id).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".meta.json"), meta).unwrap();
    }

    #[test]
    fn merges_installed_skill_repos_and_keeps_configured_state() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        installed_skill(
            &store,
            "ask-ui",
            r#"{"repoOwner":"iamddatm","repoName":"skills","repoBranch":"main"}"#,
        );
        installed_skill(
            &store,
            "custom",
            r#"{"repoOwner":"acme","repoName":"extra","repoBranch":"dev"}"#,
        );
        // 本地目录没有仓库坐标，不应该冒充一个仓库。
        installed_skill(
            &store,
            "local-one",
            r#"{"sourceKind":"local","sourceUrl":"D:/skills/local-one"}"#,
        );

        let configured = vec![SkillRepo {
            owner: "IAMDDATM".into(),
            name: "Skills".into(),
            branch: "release".into(),
            enabled: false,
        }];
        let merged = merge_installed_repos(&store, &configured);
        assert_eq!(merged.len(), 2, "{merged:?}");
        // 同名配置优先：用户显式停用的仓库不能被隐式打开或改分支。
        assert_eq!(merged[0].slug(), "IAMDDATM/Skills");
        assert_eq!(merged[0].branch, "release");
        assert!(!merged[0].enabled);
        assert_eq!(merged[1].slug(), "acme/extra");
        assert_eq!(merged[1].branch, "dev");
        assert!(merged[1].enabled);
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
        // 报告要回传本次使用的仓库（含停用项），前端据此渲染筛选区。
        assert_eq!(report.repos.len(), 2);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].repo, "acme/demo");
        assert!(report.failures[0].error.contains("无法读取"));
    }
}
