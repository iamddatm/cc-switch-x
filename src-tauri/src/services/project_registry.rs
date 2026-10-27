use crate::adapters::paths::expand_root;
use crate::domain::agent::builtin_agents;
use crate::domain::project::{
    dir_spec, ProjectDirState, ProjectEntry, ProjectLayoutStatus, ProjectPaths, ProjectSkillDirKey,
    ProjectSkillOp, ProjectSkillSource, PROJECT_SKILL_DIRS,
};
use crate::error::AppError;
use crate::services::agents::AgentStatus;
use crate::services::project_scan::{self, enabled_dir_keys, ProjectScan};
use crate::store::Store;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub added_at: String,
    pub last_opened_at: String,
    pub auto_sync: bool,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirMeta {
    pub key: ProjectSkillDirKey,
    pub label: String,
    pub relative_path: String,
    pub agents: Vec<String>,
    /// 映射到这个物理目录的 agent id。前端按 agent 展开矩阵列，需要从目录反查 id。
    pub agent_ids: Vec<String>,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirView {
    pub key: ProjectSkillDirKey,
    pub label: String,
    pub relative_path: String,
    pub agents: Vec<String>,
    pub agent_ids: Vec<String>,
    pub verified: bool,
    /// present | absent | different | untracked | unavailable
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillView {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub targets: Vec<String>,
    pub dirs: BTreeMap<String, ProjectDirView>,
    pub source: Option<ProjectSkillSource>,
    pub last_op: Option<ProjectSkillOp>,
    /// 项目真源是否已落地（客户端数据目录里的 `skills/<id>/` 有 SKILL.md）。
    pub canonical: bool,
    /// 账本记着有真源、磁盘上却没有：只能由显式 purge 处置。
    pub canonical_missing: bool,
    /// 上次检查更新的结果。项目真源不写 `.meta.json`，「可更新」与检查失败原因
    /// 只能来自本机 `local/project-skill-checks/<pid>.json`。
    pub check: Option<crate::store::project_checks::ProjectSkillCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillsView {
    pub project: ProjectView,
    pub dirs: Vec<ProjectDirMeta>,
    pub skills: Vec<ProjectSkillView>,
    pub conflict_count: usize,
    /// v1 台账的迁移计划；已经是 v2 或无事可做时为 None。只读计算，不落盘。
    pub migration: Option<crate::services::project_migrate::ProjectMigrationPlan>,
    /// 项目数据布局。`Hybrid` / `Foreign` 时面板顶部要给出显式提示，
    /// `Unsupported` 时所有命令都会先报错，面板根本走不到这里。
    pub layout: ProjectLayoutStatus,
}

pub fn project_id(path: &Path) -> String {
    let canonical =
        crate::domain::project::strip_verbatim(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()));
    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let slug = crate::import::slug(name).unwrap_or_else(|| "project".into());
    let hash = crate::index::sha256_hex(canonical.to_string_lossy().as_bytes());
    format!("{slug}-{}", &hash[..12])
}

fn project_view(entry: &ProjectEntry) -> ProjectView {
    ProjectView {
        id: entry.id.clone(),
        path: entry.path.clone(),
        name: entry.name.clone(),
        added_at: entry.added_at.clone(),
        last_opened_at: entry.last_opened_at.clone(),
        auto_sync: entry.auto_sync,
        exists: entry.path.is_dir(),
    }
}

pub fn list_projects(store: &Store) -> Result<Vec<ProjectView>, AppError> {
    let registry = crate::store::projects::load(store)?;
    Ok(registry.projects.iter().map(project_view).collect())
}

pub fn add_project(
    store: &Store,
    home: &Path,
    path: &Path,
    enabled_agent_ids: &[String],
) -> Result<ProjectView, AppError> {
    let mut registry = crate::store::projects::load(store)?;
    let user_level_roots: Vec<PathBuf> = builtin_agents()
        .iter()
        .map(|agent| expand_root(&agent.config_root, None, home))
        .collect();
    let canonical = crate::domain::project::validate_project_root(
        path,
        home,
        store.root(),
        &user_level_roots,
    )?;
    let now = chrono::Utc::now().to_rfc3339();

    if let Some(index) = registry
        .projects
        .iter()
        .position(|entry| entry.path == canonical)
    {
        registry.projects[index].last_opened_at = now;
        // 重新打开也要先处理旧布局：从 v2 升级回来的项目第一次被打开时正是这里。
        let paths = ProjectPaths::resolve(
            store.root(),
            &registry.projects[index].id,
            &registry.projects[index].path,
        )?;
        crate::services::project_relocate::auto(store, &paths)?;
        let view = project_view(&registry.projects[index]);
        crate::store::projects::save(store, &registry)?;
        return Ok(view);
    }

    let entry = ProjectEntry {
        id: project_id(&canonical),
        name: canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_string(),
        path: canonical.clone(),
        added_at: now.clone(),
        last_opened_at: now,
        auto_sync: true,
    };

    let keys = enabled_dir_keys(enabled_agent_ids);
    let paths = ProjectPaths::resolve(store.root(), &entry.id, &canonical)?;
    // 存量项目（v2 把真源放在仓库里）在登记时先搬进 store，删旧目录的三重闸在搬迁模块里。
    let relocation = crate::services::project_relocate::auto(store, &paths)?;
    // 两边都有数据且内容分歧：先登记，由面板横幅让人裁决；此时写空台账会丢归属。
    let disputed = matches!(relocation.status, ProjectLayoutStatus::Hybrid { .. })
        && !paths.ledger_path().exists();
    if !disputed {
        let scan = project_scan::scan(&paths, &keys)?;
        project_scan::ensure_baseline(&paths, &scan)?;
    }

    registry.projects.push(entry.clone());
    crate::store::projects::save(store, &registry)?;
    Ok(project_view(&entry))
}

/// 只从客户端注册表移除项目：不动用户仓库里的任何文件，也不删 `local/projects/<id>/`
/// 里的真源与台账（留着，重新添加同一个项目就能接上；要清理请显式删目录）。
pub fn remove_project(store: &Store, id: &str) -> Result<(), AppError> {
    let mut registry = crate::store::projects::load(store)?;
    registry.projects.retain(|entry| entry.id != id);
    crate::store::projects::save(store, &registry)
}

pub fn touch_project(store: &Store, id: &str) -> Result<ProjectView, AppError> {
    let mut registry = crate::store::projects::load(store)?;
    let entry = registry
        .projects
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| AppError::NotFound(format!("项目不存在：{id}")))?;
    entry.last_opened_at = chrono::Utc::now().to_rfc3339();
    let view = project_view(entry);
    crate::store::projects::save(store, &registry)?;
    Ok(view)
}

pub fn set_auto_sync(store: &Store, id: &str, enabled: bool) -> Result<ProjectView, AppError> {
    let mut registry = crate::store::projects::load(store)?;
    let entry = registry
        .projects
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| AppError::NotFound(format!("项目不存在：{id}")))?;
    entry.auto_sync = enabled;
    let view = project_view(entry);
    crate::store::projects::save(store, &registry)?;
    Ok(view)
}

fn key_from_str(value: &str) -> Option<ProjectSkillDirKey> {
    PROJECT_SKILL_DIRS
        .iter()
        .find(|spec| spec.key.as_str() == value)
        .map(|spec| spec.key)
}

/// 项目级渲染只认「启用且 Skill 能力成立」的 agent：档位 OFF 的 agent
/// 不该造出一个永远没有列的目录，扫描与基线也就能少跑一遍。
pub fn skill_enabled_ids(agents: &[AgentStatus]) -> Vec<String> {
    agents
        .iter()
        .filter(|agent| {
            agent.enabled
                && agent.capabilities.iter().any(|capability| {
                    capability.kind == crate::domain::AssetKind::Skill && capability.supported
                })
        })
        .map(|agent| agent.id.clone())
        .collect()
}

/// 该物理目录能否作为同步目标（能力闸）：项目级落点自己已实测，
/// 且映射到它并启用的每个 agent 都已实测过 skill 落点。未通过的列 UI 显示「待实测」，
/// `set_project_skill_targets` 也会直接拒绝。
pub fn selectable_dir_keys(agents: &[AgentStatus]) -> Vec<String> {
    PROJECT_SKILL_DIRS
        .iter()
        .filter(|spec| spec.verified)
        .filter(|spec| {
            let mapped: Vec<&AgentStatus> = agents
                .iter()
                .filter(|agent| agent.enabled && spec.agent_ids.contains(&agent.id.as_str()))
                .collect();
            !mapped.is_empty()
                && mapped.iter().all(|agent| {
                    agent.capabilities.iter().any(|capability| {
                        capability.kind == crate::domain::AssetKind::Skill
                            && capability.supported
                            && capability.verified
                    })
                })
        })
        .map(|spec| spec.key.as_str().to_string())
        .collect()
}

fn dir_meta(key: ProjectSkillDirKey, agents: &[AgentStatus]) -> ProjectDirMeta {
    let spec = dir_spec(key);
    // 列名带上所有映射到该目录的 agent（含已停用的）：已停用的列也要显示名字，用户才认得出那是自己勾过的目标。
    let mapped: Vec<&AgentStatus> = spec
        .agent_ids
        .iter()
        .filter_map(|id| agents.iter().find(|agent| agent.id == *id))
        .collect();
    let names: Vec<String> = mapped
        .iter()
        .map(|agent| agent.display_name.clone())
        .collect();
    ProjectDirMeta {
        key,
        label: names.join(" / "),
        relative_path: spec.relative_path.to_string(),
        agents: names,
        agent_ids: mapped.iter().map(|agent| agent.id.clone()).collect(),
        verified: selectable_dir_keys(agents).iter().any(|k| k == spec.key.as_str()),
    }
}

/// 列表展示的名称与描述：优先读项目真源，其次按启用目录的顺序读渲染产物。
/// 真源是唯一可编辑副本，所以它一旦存在就以它为准；未迁移/未采纳时才回落到副本。
fn describe_skill(
    paths: &ProjectPaths,
    id: &str,
    scan: &ProjectScan,
    keys: &[ProjectSkillDirKey],
) -> (String, Option<String>) {
    if scan
        .canonical
        .get(id)
        .is_some_and(|state| state.present && !scan.blocks_canonical(id))
    {
        let path = paths.canonical_root().join(id).join("SKILL.md");
        if let Ok(text) = std::fs::read_to_string(path) {
            let meta = crate::domain::skill::parse_metadata(&text);
            return (meta.name.unwrap_or_else(|| id.to_string()), meta.description);
        }
    }
    for key in keys {
        let state = scan
            .skills
            .get(id)
            .and_then(|snapshot| snapshot.dirs.get(key.as_str()));
        if !state.is_some_and(|state| state.present) {
            continue;
        }
        let path = paths.render_dir(*key).join(id).join("SKILL.md");
        if let Ok(text) = std::fs::read_to_string(path) {
            let meta = crate::domain::skill::parse_metadata(&text);
            return (meta.name.unwrap_or_else(|| id.to_string()), meta.description);
        }
    }
    (id.to_string(), None)
}

/// 一格的状态。两轴：与基线的差 = 是否被外部改过；与真源的差 = 是否落后。
/// `canonical` 是该项目当前的真源状态（None = 还没有真源）。
fn state_for(
    key: ProjectSkillDirKey,
    entry: Option<&crate::domain::project::ProjectSkillEntry>,
    current: &ProjectDirState,
    enabled_keys: &[ProjectSkillDirKey],
    allowed_dirs: &[String],
    canonical: Option<&ProjectDirState>,
) -> &'static str {
    if !enabled_keys.contains(&key) {
        return "unavailable";
    }
    let tracked = entry
        .map(|entry| entry.targets.iter().any(|target| target == key.as_str()))
        .unwrap_or(false);
    if !tracked {
        return if current.present { "untracked" } else { "absent" };
    }
    if !current.present {
        return "absent";
    }
    // 未实测的落点我们从不写入，所以它的副本一律按「外部内容」处理，不能显示成已同步。
    if !allowed_dirs.iter().any(|dir| *dir == key.as_str()) {
        return "unavailable";
    }
    let baseline = entry.and_then(|entry| entry.dirs.get(key.as_str()));
    let matches_baseline = baseline.is_some_and(|baseline| {
        baseline.present && baseline.tree_hash.as_deref() == current.tree_hash.as_deref()
    });
    if !matches_baseline {
        return "edited";
    }
    match canonical {
        Some(canonical) if canonical.tree_hash != current.tree_hash => "stale",
        _ => "present",
    }
}

pub fn list_project_skills(
    store: &Store,
    project_id: &str,
    agents: &[AgentStatus],
) -> Result<ProjectSkillsView, AppError> {
    let registry = crate::store::projects::load(store)?;
    let entry = registry
        .projects
        .iter()
        .find(|entry| entry.id == project_id)
        .ok_or_else(|| AppError::NotFound(format!("项目不存在：{project_id}")))?;
    let project = project_view(entry);
    if !project.exists {
        return Ok(ProjectSkillsView {
            project,
            dirs: Vec::new(),
            skills: Vec::new(),
            conflict_count: 0,
            migration: None,
            layout: ProjectLayoutStatus::Native,
        });
    }

    let enabled_ids = skill_enabled_ids(agents);
    let keys = enabled_dir_keys(&enabled_ids);
    let paths = ProjectPaths::resolve(store.root(), &entry.id, &entry.path)?;
    // 打开面板时补完/清理旧布局：否则 v2 项目在这里会显示成空表。
    let relocation = crate::services::project_relocate::auto(store, &paths)?;
    let scan = project_scan::scan(&paths, &keys)?;
    let ledger = project_scan::load_ledger(&paths)?;
    // 能力闸允许的渲染目标：格子的「不可用」状态与写入侧校验共用同一份判定。
    let allowed = selectable_dir_keys(agents);
    let checks = crate::store::project_checks::ProjectCheckCache::load(store, project_id);

    // 列 = 本次启用的目录 + 账本里已勾选但当前停用的目录（后者标 unavailable，让用户能取消）。
    let mut column_keys = keys.clone();
    for entry in ledger.skills.values() {
        for target in &entry.targets {
            if let Some(key) = key_from_str(target) {
                if !column_keys.contains(&key) {
                    column_keys.push(key);
                }
            }
        }
    }
    let dir_metas: Vec<ProjectDirMeta> = column_keys
        .iter()
        .map(|key| dir_meta(*key, agents))
        .collect();
    let meta_by_key: BTreeMap<String, ProjectDirMeta> = dir_metas
        .iter()
        .map(|meta| (meta.key.as_str().to_string(), meta.clone()))
        .collect();

    // 行 = 真源 ∪ 各目录 ∪ 账本：只装进真源、还没渲染到任何目录的 skill 也必须出现。
    let mut ids: BTreeSet<String> = scan.skills.keys().cloned().collect();
    ids.extend(ledger.skills.keys().cloned());
    ids.extend(scan.canonical.keys().cloned());

    let mut skills = Vec::new();
    // 需要人工裁决的格子数（edited / canonicalMissing），与迁移冲突一起构成待处理数。
    let mut conflicts = 0usize;
    for id in ids {
        let ledger_entry = ledger.skills.get(&id);
        let targets = ledger_entry
            .map(|entry| entry.targets.clone())
            .unwrap_or_default();
        let mut dir_keys = keys.clone();
        for target in &targets {
            if let Some(key) = key_from_str(target) {
                if !dir_keys.contains(&key) {
                    dir_keys.push(key);
                }
            }
        }

        let canonical_state = scan.canonical.get(&id);
        let mut dirs = BTreeMap::new();
        for key in dir_keys {
            let meta = meta_by_key
                .get(key.as_str())
                .cloned()
                .unwrap_or_else(|| dir_meta(key, agents));
            let current = scan
                .skills
                .get(&id)
                .and_then(|snapshot| snapshot.dirs.get(key.as_str()))
                .cloned()
                .unwrap_or_default();
            let state = state_for(key, ledger_entry, &current, &keys, &allowed, canonical_state);
            if state == "edited" {
                conflicts += 1;
            }
            dirs.insert(
                key.as_str().to_string(),
                ProjectDirView {
                    key,
                    label: meta.label,
                    relative_path: meta.relative_path,
                    agents: meta.agents,
                    agent_ids: meta.agent_ids,
                    verified: meta.verified,
                    state: state.to_string(),
                },
            );
        }

        // 账本记着有真源、磁盘上却没有：整条等待用户处置，也要计入待处理数。
        let ledger_says_canonical = ledger_entry.is_some_and(|entry| entry.canonical.present);
        if ledger_says_canonical && canonical_state.is_none() {
            conflicts += 1;
        }

        let (name, description) = describe_skill(&paths, &id, &scan, &keys);
        let check = checks.get(&id).cloned();
        skills.push(ProjectSkillView {
            id,
            name,
            description,
            targets,
            dirs,
            source: ledger_entry.and_then(|entry| entry.source.clone()),
            last_op: ledger_entry.and_then(|entry| entry.last_op.clone()),
            canonical: canonical_state.is_some(),
            canonical_missing: ledger_says_canonical && canonical_state.is_none(),
            check,
        });
    }

    // v1 台账要先给出迁移计划：界面据此提示「会在你仓库里新建哪些文件」。
    let migration = if project_scan::ledger_needs_migration(&ledger) {
        let plan = crate::services::project_migrate::plan_migration(&ledger, &scan);
        (!plan.is_empty()).then_some(plan)
    } else {
        None
    };
    let conflict_count =
        conflicts + migration.as_ref().map_or(0, |plan| plan.conflicts.len());

    Ok(ProjectSkillsView {
        project,
        dirs: dir_metas,
        skills,
        conflict_count,
        migration,
        layout: relocation.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{
        ProjectLedger, ProjectSkillEntry, PROJECT_LEDGER_VERSION,
    };
    use crate::services::agents::AgentStatus;
    use crate::store::Store;
    use std::path::{Path, PathBuf};

    fn seed_skill(root: &Path, id: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: 演示\n---\n",
        )
        .unwrap();
    }

    fn agent_status(id: &str, enabled: bool) -> AgentStatus {
        AgentStatus {
            id: id.into(),
            display_name: id.into(),
            detected: true,
            cli_available: true,
            cli_program: id.into(),
            capabilities: Vec::new(),
            mcp_needs_restart: None,
            mcp_path: PathBuf::new(),
            prompt_path: PathBuf::new(),
            skills_dir: PathBuf::new(),
            enabled,
            enabled_mcp: Vec::new(),
            enabled_skills: Vec::new(),
            enabled_rules: Vec::new(),
            enabled_subagents: Vec::new(),
            enabled_hooks: Vec::new(),
            active_prompt: None,
        }
    }

    /// 用真实能力表造一份 agent 现状，只改启用位；能力闸的判定必须按代码里的实测档位走。
    fn agent_with_capabilities(id: &str, enabled: bool) -> AgentStatus {
        let descriptor = crate::domain::agent::builtin_agents()
            .into_iter()
            .find(|agent| agent.id == id)
            .unwrap();
        AgentStatus {
            display_name: descriptor.display_name.clone(),
            capabilities: descriptor.capabilities(),
            ..agent_status(id, enabled)
        }
    }

    #[test]
    fn selectable_dirs_require_verified_project_dir_and_verified_agents() {
        let agents = vec![
            agent_with_capabilities("claude", true),
            agent_with_capabilities("qoder-cn", true),
            agent_with_capabilities("codex", true),
        ];
        let selectable = selectable_dir_keys(&agents);
        assert!(
            selectable.contains(&"claude".to_string()),
            "`.claude/skills` 已在真机项目里实测：探针放进去 Claude 就报出 skill 名，移走就报 NONE"
        );
        assert!(
            selectable.contains(&"qoder".to_string()),
            "`.qoder/skills` 已在真机项目里实测：qoderclicn skills list 直接报出项目内的 SKILL.md 路径"
        );
        assert!(
            !selectable.contains(&"agents".to_string()),
            "`.agents/skills` 尚未取证（codex 无 skills 概念、gemini 未安装、pi 无凭据），不能开放勾选"
        );
    }

    #[test]
    fn shared_dir_is_blocked_when_any_enabled_agent_lacks_verified_skills() {
        // 同一物理目录映射到 qoder（Skill 未实测）与 qoder-cn（已实测）：
        // 只要启用的 agent 里有一个没实测过，这个目录就不能作为同步目标。
        let agents = vec![
            agent_with_capabilities("qoder", true),
            agent_with_capabilities("qoder-cn", true),
        ];
        assert!(!selectable_dir_keys(&agents).contains(&"qoder".to_string()));
    }

    #[test]
    fn add_project_writes_baseline_and_deduplicates_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let project = home.join("work/app");
        std::fs::create_dir_all(&project).unwrap();
        seed_skill(&project.join(".claude/skills"), "a");

        let first = add_project(&store, &home, &project, &["claude".into()]).unwrap();
        assert!(first.exists);
        assert!(
            !first.path.to_string_lossy().starts_with(r"\\?\"),
            "注册表与 UI 必须是可读路径：{:?}",
            first.path
        );
        assert!(
            !project.join(".ccswitchx").exists(),
            "v3 起项目里不再留任何属于我们的文件"
        );
        assert!(store
            .project_dir(&first.id)
            .unwrap()
            .join("skills.json")
            .is_file());
        let ids_before = list_projects(&store).unwrap().len();

        let second = add_project(&store, &home, &project, &["claude".into()]).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(list_projects(&store).unwrap().len(), ids_before);

        let third = add_project(&store, &home, &first.path, &["claude".into()]).unwrap();
        assert_eq!(
            third.id, first.id,
            "用注册表里的可读路径再添加一次不能产生第二条"
        );
        assert_eq!(list_projects(&store).unwrap().len(), ids_before);
    }

    #[test]
    fn project_id_is_stable_and_safe_for_file_names() {
        let dir = tempfile::tempdir().unwrap();
        let id = project_id(dir.path());
        assert!(id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')));
        assert_eq!(id, project_id(dir.path()));
    }

    /// 真源是唯一可编辑副本，一旦存在，列表的名称/描述必须以它为准。
    #[test]
    fn describe_skill_prefers_the_canonical_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store_root = dir.path().join("store");
        let root = dir.path().join("home/work/app");
        std::fs::create_dir_all(&root).unwrap();
        let paths = ProjectPaths::resolve(&store_root, "app-1", &root).unwrap();
        let write_skill = |path: std::path::PathBuf, name: &str| {
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} 的描述\n---\n"),
            )
            .unwrap();
        };
        write_skill(root.join(".claude/skills/a"), "副本名");
        let keys = [ProjectSkillDirKey::Claude];

        let scan = project_scan::scan(&paths, &keys).unwrap();
        assert_eq!(describe_skill(&paths, "a", &scan, &keys).0, "副本名");

        write_skill(paths.canonical_skill_dir("a").unwrap(), "真源名");
        let scan = project_scan::scan(&paths, &keys).unwrap();
        let (name, description) = describe_skill(&paths, "a", &scan, &keys);
        assert_eq!(name, "真源名");
        assert_eq!(description.as_deref(), Some("真源名 的描述"));
    }

    #[test]
    fn list_project_skills_marks_presence_and_untracked() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let project = home.join("work/app");
        std::fs::create_dir_all(&project).unwrap();
        seed_skill(&project.join(".claude/skills"), "a");
        let view = add_project(&store, &home, &project, &["claude".into()]).unwrap();
        let agents = vec![agent_with_capabilities("claude", true), agent_with_capabilities("qoder", true)];
        let skills = list_project_skills(&store, &view.id, &agents).unwrap();
        assert_eq!(skills.skills[0].id, "a");
        assert!(skills.skills[0].targets.is_empty());
        assert_eq!(skills.skills[0].dirs["claude"].state, "untracked");
        assert_eq!(skills.skills[0].dirs["qoder"].state, "absent");
    }

    /// 焦点 1 的接线：拒绝逻辑在 domain 里，但真正调用它的是 add_project 构造的 roots。
    /// 这两条测试钉住「roots 必须传进来」，漏传就会静默把用户级目录当项目扫。
    #[test]
    fn add_project_refuses_a_path_inside_a_user_level_agent_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let inside = home.join(".claude").join("skills").join("demo");
        std::fs::create_dir_all(&inside).unwrap();

        let error = add_project(&store, &home, &inside, &["claude".into()]).unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)), "{error:?}");
        assert!(list_projects(&store).unwrap().is_empty());
    }

    #[test]
    fn add_project_refuses_a_path_inside_the_store_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let inside = store.root().join("assets").join("skills").join("demo");
        std::fs::create_dir_all(&inside).unwrap();

        let error = add_project(&store, &home, &inside, &["claude".into()]).unwrap_err();
        assert!(matches!(error, AppError::InvalidInput(_)), "{error:?}");
        assert!(list_projects(&store).unwrap().is_empty());
    }

    #[test]
    fn a_disabled_agents_target_still_shows_as_a_column() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let project = home.join("work/app");
        std::fs::create_dir_all(&project).unwrap();
        seed_skill(&project.join(".claude/skills"), "a");
        let view = add_project(&store, &home, &project, &["claude".into()]).unwrap();

        let paths = ProjectPaths::resolve(store.root(), &view.id, &project).unwrap();
        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        ledger.skills.get_mut("a").unwrap().targets =
            vec!["claude".to_string(), "qoder".to_string()];
        project_scan::save_ledger(&paths, &ledger).unwrap();

        let agents = vec![agent_with_capabilities("claude", true), agent_with_capabilities("qoder", false)];
        let skills = list_project_skills(&store, &view.id, &agents).unwrap();
        assert!(skills
            .dirs
            .iter()
            .any(|meta| meta.key == ProjectSkillDirKey::Qoder));
        assert_eq!(skills.skills[0].dirs["qoder"].state, "unavailable");
    }

    #[test]
    fn missing_project_path_returns_empty_skills_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let project = home.join("gone");
        std::fs::create_dir_all(&project).unwrap();
        let view = add_project(&store, &home, &project, &["claude".into()]).unwrap();
        std::fs::remove_dir_all(&project).unwrap();

        let skills =
            list_project_skills(&store, &view.id, &[agent_with_capabilities("claude", true)]).unwrap();
        assert!(!skills.project.exists);
        assert!(skills.skills.is_empty());
    }

    /// 项目真源没有 `.meta.json`，「可更新」徽标与检查失败原因只能来自本机缓存，
    /// 所以缓存必须跟着行进视图，否则界面永远显示不出有更新可装。
    #[test]
    fn rows_carry_the_local_check_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let project = home.join("work/app");
        std::fs::create_dir_all(&project).unwrap();
        let view = add_project(&store, &home, &project, &["claude".into()]).unwrap();
        let paths = ProjectPaths::resolve(store.root(), &view.id, &project).unwrap();
        seed_skill(paths.canonical_root(), "demo");

        // 台账条目由安装/迁移/收进真源写入；这里直接造出「已跟踪且有 GitHub 来源」的现状。
        let mut ledger = ProjectLedger {
            version: PROJECT_LEDGER_VERSION,
            updated_at: String::new(),
            skills: Default::default(),
        };
        ledger.skills.insert(
            "demo".into(),
            ProjectSkillEntry {
                targets: vec!["claude".into()],
                source: Some(ProjectSkillSource {
                    kind: "github".into(),
                    repo: Some("acme/demo".into()),
                    branch: Some("main".into()),
                    subpath: Some("skills/demo".into()),
                    tree_hash: None,
                }),
                ..Default::default()
            },
        );
        project_scan::save_ledger(&paths, &ledger).unwrap();

        let mut cache = crate::store::project_checks::ProjectCheckCache::default();
        cache.record(
            "demo",
            crate::store::project_checks::ProjectSkillCheck {
                update_available: true,
                latest_hash: Some("remote-hash".into()),
                last_checked_at: Some(5),
                last_check_error: None,
                updatable: true,
            },
        );
        cache.save(&store, &view.id).unwrap();

        let skills = list_project_skills(&store, &view.id, &[agent_with_capabilities("claude", true)]).unwrap();
        let check = skills.skills[0]
            .check
            .as_ref()
            .expect("检查状态必须带到行上");
        assert!(check.update_available);
        assert_eq!(check.latest_hash.as_deref(), Some("remote-hash"));
        assert_eq!(skills.skills[0].source.as_ref().unwrap().repo.as_deref(), Some("acme/demo"));

        // 没检查过的行不带检查状态，界面据此显示「未检查」而不是「已最新」。
        seed_skill(paths.canonical_root(), "never-checked");
        let refreshed =
            list_project_skills(&store, &view.id, &[agent_with_capabilities("claude", true)]).unwrap();
        assert!(refreshed
            .skills
            .iter()
            .find(|skill| skill.id == "never-checked")
            .expect("新真源目录要出现在行里")
            .check
            .is_none());
    }
}
