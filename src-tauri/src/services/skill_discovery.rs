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
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_SKILLS_PER_REPO: usize = 2000;
const DISCOVERY_CONCURRENCY: usize = 4;
/// 仓库归档很贵（GitHub 整包 + 解压扫描），一次发现的结果在本地缓存里放一段时间；
/// 手动刷新（force）始终重新下载。
const DISCOVERY_CACHE_TTL_SECS: u64 = 600;
/// 派生缓存版本；扫描结果的字段变化时递增，旧文件自然失效。
const DISCOVERY_CACHE_VERSION: u32 = 1;
pub const SKILLS_SH_MAX_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// 本次实际参与发现的仓库：配置为启用的仓库 + 前端显式勾选的已安装来源仓库。
    pub repos: Vec<SkillRepo>,
    /// 只在已安装 Skill 的元数据里出现、尚未配置的仓库。
    /// 默认不下载：放进发现流程会让每次刷新多拉十几个归档；由前端按需勾选后再传 `extra_repos`。
    pub installed_repos: Vec<SkillRepo>,
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

/// 已安装 Skill 里出现、但配置里没有的仓库。
/// 只作为「可选来源」交给前端展示，默认不下载：每次发现都拉它们会让刷新慢一个数量级。
pub fn merge_installed_repos(store: &Store, configured: &[SkillRepo]) -> Vec<SkillRepo> {
    suggested_repos(&installed_metas(store), configured)
}

fn suggested_repos(installed: &[SkillMeta], configured: &[SkillRepo]) -> Vec<SkillRepo> {
    let mut repos = Vec::new();
    let mut seen: HashSet<String> = configured
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

/// 发现入口：只读配置为启用的仓库 + 前端显式勾选的额外仓库；
/// 已安装 Skill 的来源仓库只回传为 `installed_repos` 备选，不自动抓取。
pub fn discover_with_installed<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    configured: &[SkillRepo],
    extra_repos: &[SkillRepo],
    force: bool,
) -> SkillDiscoveryReport {
    let installed = installed_metas(store);
    let installed_repos = suggested_repos(&installed, configured);
    let mut fetch_repos: Vec<SkillRepo> = configured
        .iter()
        .filter(|repo| repo.enabled)
        .cloned()
        .collect();
    for extra in extra_repos {
        if fetch_repos
            .iter()
            .any(|repo| repo.slug().eq_ignore_ascii_case(&extra.slug()))
        {
            continue;
        }
        fetch_repos.push(SkillRepo {
            enabled: true,
            ..extra.clone()
        });
    }
    let mut report = discover_repos(store, fetcher, &fetch_repos, &installed, force);
    report.installed_repos = installed_repos;
    report
}

/// 停用的仓库不读也不报。
pub fn discover<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    repos: &[SkillRepo],
) -> SkillDiscoveryReport {
    discover_repos(store, fetcher, repos, &installed_metas(store), true)
}

fn discover_repos<F: RepoFetcher + Sync>(
    store: &Store,
    fetcher: &F,
    repos: &[SkillRepo],
    installed: &[SkillMeta],
    force: bool,
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
                .map(|repo| scope.spawn(move || discover_repo(store, fetcher, repo, installed, force)))
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryCache {
    version: u32,
    fetched_at: u64,
    /// 实际命中的分支：配置写 main 但上游只有 master 时，缓存要记住真正读的那条。
    branch: String,
    skills: Vec<DiscoverableSkill>,
}

fn cache_path(store: &Store, repo: &SkillRepo) -> PathBuf {
    // owner/name/branch 都经过 GitHub 输入校验，这里只把分隔符换掉，避免生成子目录。
    let name = format!("{}_{}@{}", repo.owner, repo.name, repo.branch)
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>();
    store
        .root()
        .join("local")
        .join("skill-discovery")
        .join(format!("{name}.json"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn read_cache(store: &Store, repo: &SkillRepo, force: bool) -> Option<DiscoveryCache> {
    if force {
        return None;
    }
    let bytes = std::fs::read(cache_path(store, repo)).ok()?;
    let cache: DiscoveryCache = serde_json::from_slice(&bytes).ok()?;
    if cache.version != DISCOVERY_CACHE_VERSION {
        return None;
    }
    if unix_now().saturating_sub(cache.fetched_at) > DISCOVERY_CACHE_TTL_SECS {
        return None;
    }
    Some(cache)
}

fn write_cache(store: &Store, repo: &SkillRepo, branch: &str, skills: &[DiscoverableSkill]) {
    let path = cache_path(store, repo);
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let entry = DiscoveryCache {
        version: DISCOVERY_CACHE_VERSION,
        fetched_at: unix_now(),
        branch: branch.to_string(),
        skills: skills.to_vec(),
    };
    if let Ok(bytes) = serde_json::to_vec(&entry) {
        // 派生缓存：写失败不影响本次结果，下次重新下载即可。
        let _ = write_atomic(&path, &bytes);
    }
}

fn discover_repo<F: RepoFetcher>(
    store: &Store,
    fetcher: &F,
    repo: &SkillRepo,
    installed: &[SkillMeta],
    force: bool,
) -> Result<Vec<DiscoverableSkill>, AppError> {
    if let Some(cache) = read_cache(store, repo, force) {
        // 安装标记易变，缓存只用扫描结果，每次按当前真源重算。
        return Ok(apply_installed_flags(store, repo, cache.skills, installed));
    }
    let parsed = parse_github_input(&repo.slug(), Some(repo.branch.clone()))?;
    let fetched = fetcher.fetch(&parsed)?;
    let skills =
        build_from_dir_with_installed(store, fetched.dir.path(), repo, &fetched.branch, installed)?;
    write_cache(store, repo, &fetched.branch, &skills);
    Ok(skills)
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
            let is_installed =
                skill_installed(store, &suggested_id, &skill.relative_path, &known_paths);
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

/// 缓存里的扫描结果复用同一套「已安装」判定，避免安装/卸载后标记停留在旧状态。
fn apply_installed_flags(
    store: &Store,
    repo: &SkillRepo,
    skills: Vec<DiscoverableSkill>,
    installed: &[SkillMeta],
) -> Vec<DiscoverableSkill> {
    let known_paths = installed_paths(installed, &repo.slug());
    skills
        .into_iter()
        .map(|mut skill| {
            skill.installed = skill_installed(
                store,
                &skill.suggested_id,
                &skill.relative_path,
                &known_paths,
            );
            skill
        })
        .collect()
}

/// 候选是否已安装：先看建议 ID 对应目录，再退回元数据里的仓库内路径。
fn skill_installed(
    store: &Store,
    suggested_id: &str,
    relative_path: &str,
    known_paths: &HashSet<String>,
) -> bool {
    store
        .entry_path(AssetType::Skill, suggested_id)
        .map(|path| path.exists())
        .unwrap_or(false)
        || path_is_installed(relative_path, known_paths)
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
    fn suggests_installed_skill_repos_without_duplicating_configured() {
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
        let suggested = merge_installed_repos(&store, &configured);
        // 建议列表不含已配置仓库（即使分支不同），避免同一个仓库出现两次。
        assert_eq!(suggested.len(), 1, "{suggested:?}");
        assert_eq!(suggested[0].slug(), "acme/extra");
        assert_eq!(suggested[0].branch, "dev");
        assert!(suggested[0].enabled);
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

    #[test]
    fn cached_discovery_skips_the_archive_until_forced() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingFetcher {
            calls: AtomicUsize,
        }
        impl RepoFetcher for CountingFetcher {
            fn fetch(
                &self,
                _repo: &crate::net::RepoRef,
            ) -> Result<crate::net::FetchedRepo, AppError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let dir = tempfile::tempdir().unwrap();
                std::fs::create_dir_all(dir.path().join("pdf")).unwrap();
                std::fs::write(
                    dir.path().join("pdf/SKILL.md"),
                    "---\nname: pdf\ndescription: PDF 处理\n---\n",
                )
                .unwrap();
                Ok(crate::net::FetchedRepo {
                    dir,
                    branch: "main".into(),
                })
            }
        }

        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let fetcher = CountingFetcher {
            calls: AtomicUsize::new(0),
        };

        let first = discover_with_installed(&store, &fetcher, &[repo()], &[], false);
        assert_eq!(first.skills.len(), 1);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);

        // 缓存命中：不再下载，但结果字段完整。
        let second = discover_with_installed(&store, &fetcher, &[repo()], &[], false);
        assert_eq!(second.skills.len(), 1);
        assert_eq!(second.skills[0].suggested_id, "acme-demo-pdf");
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);

        // 手动刷新必须绕过缓存。
        let _ = discover_with_installed(&store, &fetcher, &[repo()], &[], true);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn installed_source_repos_are_suggested_but_not_fetched() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingFetcher {
            calls: AtomicUsize,
        }
        impl RepoFetcher for CountingFetcher {
            fn fetch(
                &self,
                _repo: &crate::net::RepoRef,
            ) -> Result<crate::net::FetchedRepo, AppError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let dir = tempfile::tempdir().unwrap();
                std::fs::create_dir_all(dir.path().join("demo")).unwrap();
                std::fs::write(dir.path().join("demo/SKILL.md"), "---\nname: demo\n---\n")
                    .unwrap();
                Ok(crate::net::FetchedRepo {
                    dir,
                    branch: "main".into(),
                })
            }
        }

        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        installed_skill(
            &store,
            "ask-ui",
            r#"{"repoOwner":"iamddatm","repoName":"skills","repoBranch":"main"}"#,
        );
        let fetcher = CountingFetcher {
            calls: AtomicUsize::new(0),
        };

        let report = discover_with_installed(&store, &fetcher, &[repo()], &[], false);

        // 只下载配置的仓库；已安装来源只作为 suggestion 回传。
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
        assert_eq!(report.repos.len(), 1);
        assert_eq!(report.installed_repos.len(), 1);
        assert_eq!(report.installed_repos[0].slug(), "iamddatm/skills");
    }
}
