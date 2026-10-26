//! 把 skill 装进**项目真源**：安装（GitHub/ZIP/文件夹）、导入、新建。
//!
//! 与中央库的差别只有目的端；下载、解压、校验、暂存、换目录全部复用
//! `skill_source`（`SkillSink::Project`）。装完之后：
//! 1. 先校验所有渲染目标都过了能力闸，**再写任何东西** —— 不能装出一半才发现某列不能写；
//! 2. 写台账条目（`targets`、`source` 来源、`canonical` 基线）；
//! 3. 渲染交给 `project_sync::sync_project`，本模块不自己复制目录 —— 部署的唯一入口永远是同步。

use crate::domain::project::{
    canonical_skill_dir, ProjectLedger, ProjectSkillOp, ProjectSkillSource,
};
use crate::error::AppError;
use crate::import::mcp::ConflictPolicy;
use crate::net::archive;
use crate::services::project_scan;
use crate::services::project_sync::{self, Resolution, SyncReport};
use crate::services::skill_source::{
    self, InstallSelection, SkillInstallOutcome, SkillSink, SkillSourceInput,
};
use crate::store::{copy_dir, Store};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInstallReport {
    pub installed: Vec<SkillInstallOutcome>,
    pub render: SyncReport,
    pub warnings: Vec<String>,
}

fn assert_targets_allowed(targets: &[String], allowed_dirs: &[String]) -> Result<(), AppError> {
    for dir in targets {
        if !allowed_dirs.iter().any(|allowed| allowed == dir) {
            return Err(AppError::Project(format!(
                "渲染目标 {dir} 未实测或未启用，不能写入；本次未改动任何文件"
            )));
        }
    }
    Ok(())
}

/// 安装后写台账：真源基线取磁盘现状，`dirs` 留给同步刷新。
fn upsert(
    ledger: &mut ProjectLedger,
    project_root: &Path,
    id: &str,
    targets: &[String],
    source: Option<ProjectSkillSource>,
    action: &str,
) -> Result<(), AppError> {
    let canonical = project_scan::tree_state(&canonical_skill_dir(project_root, id)?)?;
    if !canonical.present {
        return Err(AppError::Project(format!(
            "{id} 的真源写入后找不到 SKILL.md"
        )));
    }
    let entry = ledger.skills.entry(id.to_string()).or_default();
    entry.targets = targets.to_vec();
    if source.is_some() {
        entry.source = source;
    }
    entry.canonical = canonical;
    entry.last_op = Some(ProjectSkillOp {
        dir: "canonical".to_string(),
        action: action.to_string(),
        at: chrono::Utc::now().to_rfc3339(),
    });
    Ok(())
}

/// 来源信息：GitHub 才带 repo/branch/subpath，`treeHash` 统一用与全局同一算法的
/// `compute_dir_hash`（**不能**与台账里 `sha256:` 前缀的目录指纹互比）。
fn provenance(
    input: &SkillSourceInput,
    relative_path: &str,
    canonical: &Path,
) -> Option<ProjectSkillSource> {
    let tree_hash = archive::compute_dir_hash(canonical).ok();
    match input {
        SkillSourceInput::Github { repo, branch } => Some(ProjectSkillSource {
            kind: "github".to_string(),
            repo: Some(repo.clone()),
            branch: branch.clone(),
            subpath: Some(relative_path.to_string()),
            tree_hash,
        }),
        SkillSourceInput::Zip { .. } => Some(ProjectSkillSource {
            kind: "zip".to_string(),
            repo: None,
            branch: None,
            subpath: None,
            tree_hash,
        }),
        SkillSourceInput::Directory { .. } => Some(ProjectSkillSource {
            kind: "local".to_string(),
            repo: None,
            branch: None,
            subpath: None,
            tree_hash,
        }),
    }
}

/// 从来源安装到项目真源，并渲染到勾选的目标目录。
/// 预览（下载/解压/列候选）由界面先调 `preview_project_skill_source` 拿到 `staging_id`，
/// 这里只做「按选择落盘」，与中央库的安装流程一致。
pub fn install_project_skill(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    input: &SkillSourceInput,
    staging_id: Option<&str>,
    selections: &[InstallSelection],
    policy: ConflictPolicy,
    targets: &[String],
    allowed_dirs: &[String],
    enabled_agent_ids: &[String],
    resolutions: &[Resolution],
) -> Result<ProjectInstallReport, AppError> {
    // 先校验目标，避免装完真源却写不进任何目录的半截状态。
    assert_targets_allowed(targets, allowed_dirs)?;

    let timestamp = chrono::Utc::now().timestamp_millis();
    let report = skill_source::install(
        SkillSink::Project {
            store,
            project_root,
            project_id,
            timestamp,
        },
        input,
        staging_id,
        selections,
        policy,
    )?;

    let mut ledger = project_scan::load_ledger(project_root)?;
    let mut installed = Vec::new();
    for outcome in report.outcomes {
        if outcome.status != "installed" && outcome.status != "overwritten" {
            installed.push(outcome);
            continue;
        }
        let canonical = canonical_skill_dir(project_root, &outcome.id)?;
        let source = provenance(input, &outcome.name, &canonical);
        upsert(
            &mut ledger,
            project_root,
            &outcome.id,
            targets,
            source,
            "render",
        )?;
        installed.push(outcome);
    }
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    project_scan::save_ledger(project_root, &ledger)?;

    let render = render_now(
        store,
        project_id,
        project_root,
        enabled_agent_ids,
        allowed_dirs,
        resolutions,
    )?;
    Ok(ProjectInstallReport {
        installed,
        render,
        warnings: Vec::new(),
    })
}

/// 从本地文件夹导入一个 skill 目录到项目真源（已存在同名则拒绝，不覆盖）。
pub fn import_project_skill_dir(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    source: &Path,
    targets: &[String],
    allowed_dirs: &[String],
    enabled_agent_ids: &[String],
) -> Result<ProjectInstallReport, AppError> {
    assert_targets_allowed(targets, allowed_dirs)?;
    if !source.join("SKILL.md").is_file() {
        return Err(AppError::InvalidInput(
            "选择的目录里没有 SKILL.md，不是一个 skill".into(),
        ));
    }
    let id = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::InvalidInput("无法从路径得到 skill 名称".into()))?
        .to_string();
    let canonical = canonical_skill_dir(project_root, &id)?;
    if canonical.exists() {
        return Err(AppError::Project(format!(
            "项目真源里已经有 {id}；要更新请走更新，或先卸载"
        )));
    }
    if let Some(parent) = canonical.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_dir(source, &canonical)?;
    if project_scan::tree_state(&canonical)?.tree_hash != project_scan::tree_state(source)?.tree_hash
    {
        let _ = std::fs::remove_dir_all(&canonical);
        return Err(AppError::Project("导入后回读校验与来源不一致".into()));
    }

    let mut ledger = project_scan::load_ledger(project_root)?;
    let source_info = provenance(&SkillSourceInput::Directory { path: source.into() }, ".", &canonical);
    upsert(
        &mut ledger,
        project_root,
        &id,
        targets,
        source_info,
        "render",
    )?;
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    project_scan::save_ledger(project_root, &ledger)?;

    let render = render_now(
        store,
        project_id,
        project_root,
        enabled_agent_ids,
        allowed_dirs,
        &[],
    )?;
    Ok(ProjectInstallReport {
        installed: vec![SkillInstallOutcome {
            id,
            name: String::new(),
            status: "installed".into(),
            backup_path: None,
            error: None,
            warnings: Vec::new(),
        }],
        render,
        warnings: Vec::new(),
    })
}

/// 在项目真源里新建一个 skill（正文由表单渲染好传进来）。
pub fn create_project_skill(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    id: &str,
    content: &str,
    targets: &[String],
    allowed_dirs: &[String],
    enabled_agent_ids: &[String],
) -> Result<ProjectInstallReport, AppError> {
    assert_targets_allowed(targets, allowed_dirs)?;
    let canonical = canonical_skill_dir(project_root, id)?;
    if canonical.exists() {
        return Err(AppError::Project(format!("项目真源里已经有 {id}")));
    }
    std::fs::create_dir_all(&canonical)?;
    let path = canonical.join("SKILL.md");
    crate::store::atomic::write_atomic(&path, content.as_bytes())
        .map_err(|error| AppError::Project(format!("写入真源失败：{error}")))?;
    if std::fs::read(&path)? != content.as_bytes() {
        let _ = std::fs::remove_dir_all(&canonical);
        return Err(AppError::Project("新建真源的回读与提交内容不一致".into()));
    }

    let mut ledger = project_scan::load_ledger(project_root)?;
    upsert(&mut ledger, project_root, id, targets, None, "render")?;
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    project_scan::save_ledger(project_root, &ledger)?;

    let render = render_now(
        store,
        project_id,
        project_root,
        enabled_agent_ids,
        allowed_dirs,
        &[],
    )?;
    Ok(ProjectInstallReport {
        installed: vec![SkillInstallOutcome {
            id: id.to_string(),
            name: String::new(),
            status: "installed".into(),
            backup_path: None,
            error: None,
            warnings: Vec::new(),
        }],
        render,
        warnings: Vec::new(),
    })
}

/// 渲染只走同步引擎；`resolutions` 留给界面在冲突时再调一次同步用。
fn render_now(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    enabled_agent_ids: &[String],
    allowed_dirs: &[String],
    resolutions: &[Resolution],
) -> Result<SyncReport, AppError> {
    project_sync::sync_project(
        store,
        project_id,
        project_root,
        enabled_agent_ids,
        allowed_dirs,
        resolutions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::project_registry::selectable_dir_keys;
    use crate::services::agents::AgentStatus;
    use std::path::PathBuf;

    struct Fixture {
        _temp: tempfile::TempDir,
        store: Store,
        root: PathBuf,
        allowed: Vec<String>,
        enabled: Vec<String>,
    }

    /// 只给能力表里「已实测」的 agent 造现状：`.agents/skills` 必须继续被挡在外面。
    fn agent(id: &str) -> AgentStatus {
        let descriptor = crate::domain::agent::builtin_agents()
            .into_iter()
            .find(|agent| agent.id == id)
            .unwrap();
        AgentStatus {
            id: descriptor.id.clone(),
            display_name: descriptor.display_name.clone(),
            detected: true,
            cli_available: true,
            cli_program: descriptor.cli_program.to_string(),
            capabilities: descriptor.capabilities(),
            mcp_needs_restart: None,
            mcp_path: PathBuf::new(),
            prompt_path: PathBuf::new(),
            skills_dir: PathBuf::new(),
            enabled: true,
            enabled_mcp: Vec::new(),
            enabled_skills: Vec::new(),
            enabled_rules: Vec::new(),
            enabled_subagents: Vec::new(),
            enabled_hooks: Vec::new(),
            active_prompt: None,
        }
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::init(temp.path().join("store")).unwrap();
        let root = temp.path().join("home/work/app");
        std::fs::create_dir_all(&root).unwrap();
        let agents = vec![agent("claude"), agent("qoder-cn"), agent("codex")];
        Fixture {
            allowed: selectable_dir_keys(&agents),
            enabled: vec!["claude".to_string(), "qoder-cn".to_string(), "codex".to_string()],
            _temp: temp,
            store,
            root,
        }
    }

    /// 一个本地目录来源（`install` 对它不需要下载与暂存）。
    fn source_dir(fx: &Fixture, id: &str, body: &str) -> PathBuf {
        let path = fx._temp.path().join("incoming").join(id);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), body).unwrap();
        std::fs::create_dir_all(path.join("references")).unwrap();
        std::fs::write(path.join("references/a.md"), "附件").unwrap();
        path
    }

    fn install_dir(
        fx: &Fixture,
        source: &Path,
        id: &str,
        targets: &[&str],
        allowed: &[String],
    ) -> Result<ProjectInstallReport, AppError> {
        install_project_skill(
            &fx.store,
            "app-1",
            &fx.root,
            &SkillSourceInput::Directory { path: source.to_path_buf() },
            None,
            &[InstallSelection {
                relative_path: ".".into(),
                id: id.to_string(),
            }],
            ConflictPolicy::Skip,
            &targets.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            allowed,
            &fx.enabled,
            &[],
        )
    }

    #[test]
    fn project_sink_installs_into_the_canonical_and_renders_the_targets() {
        let fx = fixture();
        let source = source_dir(&fx, "demo", "---\nname: Demo\n---\n内容\n");

        let report = install_dir(&fx, &source, "demo", &["claude", "qoder"], &fx.allowed)
            .expect("安装失败");
        assert_eq!(report.installed[0].status, "installed");
        assert!(
            report
                .render
                .outcomes
                .iter()
                .all(|item| item.status == "ok" && item.action == "render")
        );
        assert_eq!(report.render.outcomes.len(), 2, "两个目标都该被渲染");

        let canonical = canonical_skill_dir(&fx.root, "demo").unwrap();
        assert!(canonical.join("SKILL.md").is_file());
        assert!(canonical.join("references/a.md").is_file(), "整目录复制");
        for dir in [".claude", ".qoder"] {
            assert_eq!(
                std::fs::read_to_string(fx.root.join(dir).join("skills/demo/SKILL.md")).unwrap(),
                "---\nname: Demo\n---\n内容\n"
            );
        }

        let ledger = project_scan::load_ledger(&fx.root).unwrap();
        let entry = &ledger.skills["demo"];
        assert!(entry.canonical.present);
        assert_eq!(entry.targets, vec!["claude".to_string(), "qoder".to_string()]);
        assert_eq!(entry.source.as_ref().unwrap().kind, "local");
        assert_eq!(
            entry.dirs["claude"].tree_hash, entry.canonical.tree_hash,
            "渲染成功后基线等于真源"
        );
    }

    #[test]
    fn project_canonical_never_gets_a_meta_json() {
        let fx = fixture();
        let source = source_dir(&fx, "demo", "---\nname: Demo\n---\n内容\n");
        install_dir(&fx, &source, "demo", &["claude"], &fx.allowed).unwrap();

        let canonical = canonical_skill_dir(&fx.root, "demo").unwrap();
        assert!(
            !canonical.join(".meta.json").exists(),
            "项目真源不写 .meta.json：它随仓库提交，且会让每次检查更新都改变目录指纹"
        );
        // 中央库那边照旧带 meta，说明差别只在 sink，不在流程。
        assert!(canonical.join("SKILL.md").is_file());
    }

    #[test]
    fn project_staging_lives_inside_the_canonical_root() {
        let fx = fixture();
        let sink = SkillSink::Project {
            store: &fx.store,
            project_root: &fx.root,
            project_id: "app-1",
            timestamp: 1234,
        };
        let staging = sink.staging("install-0").unwrap();
        assert!(
            staging.starts_with(crate::domain::project::canonical_root(&fx.root)),
            "暂存必须与目的同卷，否则项目在另一块盘时 rename 会失败：{staging:?}"
        );
        assert!(staging.file_name().unwrap().to_string_lossy().starts_with(".staging-"));
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn unselectable_targets_are_refused_before_anything_is_written() {
        let fx = fixture();
        let source = source_dir(&fx, "demo", "---\nname: Demo\n---\n内容\n");

        // `agents` 未实测：不能因为它把整次安装变成「装了但渲染不进」。
        let error = install_dir(&fx, &source, "demo", &["claude", "agents"], &fx.allowed).unwrap_err();
        assert!(error.to_string().contains("未改动任何文件"), "{error}");
        assert!(
            !canonical_skill_dir(&fx.root, "demo").unwrap().exists(),
            "校验发生在写入之前"
        );
        assert!(!fx.root.join(".claude/skills/demo").exists());
    }

    #[test]
    fn installing_with_no_targets_only_lands_the_canonical() {
        let fx = fixture();
        let source = source_dir(&fx, "demo", "---\nname: Demo\n---\n内容\n");

        let report = install_dir(&fx, &source, "demo", &[], &[]).unwrap();
        assert!(report.render.outcomes.is_empty());
        assert!(canonical_skill_dir(&fx.root, "demo").unwrap().is_dir());
        assert!(!fx.root.join(".claude/skills/demo").exists());
        let ledger = project_scan::load_ledger(&fx.root).unwrap();
        assert!(ledger.skills["demo"].canonical.present);
        assert!(ledger.skills["demo"].targets.is_empty());
    }

    #[test]
    fn create_and_import_refuse_to_shadow_an_existing_canonical() {
        let fx = fixture();
        create_project_skill(
            &fx.store,
            "app-1",
            &fx.root,
            "demo",
            "---\nname: Demo\n---\n",
            &["claude".to_string()],
            &["claude".to_string()],
            &fx.enabled,
        )
        .unwrap();

        let again = create_project_skill(
            &fx.store,
            "app-1",
            &fx.root,
            "demo",
            "---\nname: Other\n---\n",
            &["claude".to_string()],
            &["claude".to_string()],
            &fx.enabled,
        );
        assert!(again.is_err(), "同名真源已存在时不覆盖");
        assert_eq!(
            std::fs::read_to_string(
                canonical_skill_dir(&fx.root, "demo").unwrap().join("SKILL.md")
            )
            .unwrap(),
            "---\nname: Demo\n---\n"
        );

        let source = source_dir(&fx, "demo", "外来内容");
        let imported = import_project_skill_dir(
            &fx.store,
            "app-1",
            &fx.root,
            &source,
            &["claude".to_string()],
            &["claude".to_string()],
            &fx.enabled,
        );
        assert!(imported.is_err(), "导入同样不覆盖已有真源");
    }

    #[test]
    fn github_provenance_keeps_repo_branch_and_upstream_hash() {
        let fx = fixture();
        let canonical = source_dir(&fx, "demo", "---\nname: Demo\n---\n");
        let source = SkillSourceInput::Github {
            repo: "owner/repo".into(),
            branch: Some("main".into()),
        };
        let meta = provenance(&source, "skills/demo", &canonical).unwrap();
        assert_eq!(meta.kind, "github");
        assert_eq!(meta.repo.as_deref(), Some("owner/repo"));
        assert_eq!(meta.branch.as_deref(), Some("main"));
        assert_eq!(meta.subpath.as_deref(), Some("skills/demo"));
        assert!(meta.tree_hash.is_some());
        assert!(
            !meta.tree_hash.unwrap().starts_with("sha256:"),
            "上游哈希用 compute_dir_hash（裸 hex），不能与台账里的 sha256: 指纹互比"
        );
    }
}
