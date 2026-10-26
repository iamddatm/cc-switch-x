use crate::adapters::paths::expand_root;
use crate::domain::agent::builtin_agents;
use crate::domain::project::{
    dir_spec, ProjectDirState, ProjectEntry, ProjectSkillDirKey, ProjectSkillOp, ProjectSkillSource,
    PROJECT_SKILL_DIRS,
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
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirView {
    pub key: ProjectSkillDirKey,
    pub label: String,
    pub relative_path: String,
    pub agents: Vec<String>,
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
    let scan = project_scan::scan(&canonical, &keys)?;
    project_scan::ensure_baseline(&canonical, &scan)?;

    registry.projects.push(entry.clone());
    crate::store::projects::save(store, &registry)?;
    Ok(project_view(&entry))
}

/// 只从客户端移除项目，不删除项目文件，也不删除 `.ccswitchx/skills.json`。
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
    let names: Vec<String> = spec
        .agent_ids
        .iter()
        .filter_map(|id| agents.iter().find(|agent| agent.id == *id))
        .map(|agent| agent.display_name.clone())
        .collect();
    ProjectDirMeta {
        key,
        label: names.join(" / "),
        relative_path: spec.relative_path.to_string(),
        agents: names,
        verified: selectable_dir_keys(agents).iter().any(|k| k == spec.key.as_str()),
    }
}

/// 列表展示的名称与描述：优先读项目真源，其次按启用目录的顺序读渲染产物。
/// 真源是唯一可编辑副本，所以它一旦存在就以它为准；未迁移/未采纳时才回落到副本。
fn describe_skill(
    project_root: &Path,
    id: &str,
    scan: &ProjectScan,
    keys: &[ProjectSkillDirKey],
) -> (String, Option<String>) {
    if scan
        .canonical
        .get(id)
        .is_some_and(|state| state.present && !scan.blocks_canonical(id))
    {
        let path = crate::domain::project::canonical_root(project_root)
            .join(id)
            .join("SKILL.md");
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
        let path = project_root
            .join(dir_spec(*key).relative_path)
            .join(id)
            .join("SKILL.md");
        if let Ok(text) = std::fs::read_to_string(path) {
            let meta = crate::domain::skill::parse_metadata(&text);
            return (meta.name.unwrap_or_else(|| id.to_string()), meta.description);
        }
    }
    (id.to_string(), None)
}

fn state_for(
    key: ProjectSkillDirKey,
    entry: Option<&crate::domain::project::ProjectSkillEntry>,
    current: &ProjectDirState,
    enabled_keys: &[ProjectSkillDirKey],
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
    let baseline = entry.and_then(|entry| entry.dirs.get(key.as_str()));
    match baseline {
        Some(baseline) if baseline.present && baseline.tree_hash != current.tree_hash => "different",
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
        });
    }

    let enabled_ids: Vec<String> = agents
        .iter()
        .filter(|agent| agent.enabled)
        .map(|agent| agent.id.clone())
        .collect();
    let keys = enabled_dir_keys(&enabled_ids);
    let scan = project_scan::scan(&entry.path, &keys)?;
    let ledger = project_scan::load_ledger(&entry.path)?;

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

    let mut ids: BTreeSet<String> = scan.skills.keys().cloned().collect();
    ids.extend(ledger.skills.keys().cloned());

    let mut skills = Vec::new();
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
            let state = state_for(key, ledger_entry, &current, &keys);
            dirs.insert(
                key.as_str().to_string(),
                ProjectDirView {
                    key,
                    label: meta.label,
                    relative_path: meta.relative_path,
                    agents: meta.agents,
                    verified: meta.verified,
                    state: state.to_string(),
                },
            );
        }

        let (name, description) = describe_skill(&entry.path, &id, &scan, &keys);
        skills.push(ProjectSkillView {
            id,
            name,
            description,
            targets,
            dirs,
            source: ledger_entry.and_then(|entry| entry.source.clone()),
            last_op: ledger_entry.and_then(|entry| entry.last_op.clone()),
        });
    }

    // v1 台账要先给出迁移计划：界面据此提示「会在你仓库里新建哪些文件」。
    let migration = if project_scan::ledger_needs_migration(&ledger) {
        let plan = crate::services::project_migrate::plan_migration(&ledger, &scan);
        (!plan.is_empty()).then_some(plan)
    } else {
        None
    };
    let conflict_count = migration
        .as_ref()
        .map_or(0, |plan| plan.conflicts.len());

    Ok(ProjectSkillsView {
        project,
        dirs: dir_metas,
        skills,
        conflict_count,
        migration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(project.join(".ccswitchx/skills.json").is_file());
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
        let root = dir.path();
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

        let scan = project_scan::scan(root, &keys).unwrap();
        assert_eq!(describe_skill(root, "a", &scan, &keys).0, "副本名");

        write_skill(crate::domain::project::canonical_skill_dir(root, "a").unwrap(), "真源名");
        let scan = project_scan::scan(root, &keys).unwrap();
        let (name, description) = describe_skill(root, "a", &scan, &keys);
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
        let agents = vec![agent_status("claude", true), agent_status("qoder", true)];
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

        let mut ledger = project_scan::load_ledger(&project).unwrap();
        ledger.skills.get_mut("a").unwrap().targets =
            vec!["claude".to_string(), "qoder".to_string()];
        project_scan::save_ledger(&project, &ledger).unwrap();

        let agents = vec![agent_status("claude", true), agent_status("qoder", false)];
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
            list_project_skills(&store, &view.id, &[agent_status("claude", true)]).unwrap();
        assert!(!skills.project.exists);
        assert!(skills.skills.is_empty());
    }
}
