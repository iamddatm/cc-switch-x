//! 真机端到端验证：**临时 store + 临时项目**跑通
//! 注册 → 扫描 → 基线 → 能力闸 → 勾选目标 → 同步 → 删除传播 + 备份 的完整链路。
//!
//! 只在 `cargo test -- --ignored` 时运行。项目根与 store 都在 `tempfile` 目录里，
//! 随 tempdir 一起回收，绝不把本仓库当作项目根；真实 `~/.claude`、`~/.qoder-cn` 等
//! 用户级配置根只读对照，绝不写入。
//!
//! 用法（仓库根目录）：
//! ```bash
//! pnpm build            # tauri 的 generate_context 需要 dist/
//! cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
//! ```

use cc_switch_x_lib::domain::agent::builtin_agents;
use cc_switch_x_lib::domain::project::ProjectLedger;
use cc_switch_x_lib::services::agents::AgentStatus;
use cc_switch_x_lib::services::project_registry;
use cc_switch_x_lib::services::project_scan;
use cc_switch_x_lib::services::project_sync;
use cc_switch_x_lib::store::Store;
use std::path::{Path, PathBuf};

const PROBE: &str = "zz-ccswitchx-probe";
/// 已实测的两个项目级目录；未实测的 `.agents/skills` 不在其中。
const ENABLED_AGENTS: [&str; 2] = ["claude", "qoder-cn"];

fn probe_dir(root: &Path, dir: &str) -> PathBuf {
    root.join(dir).join(PROBE)
}

fn write_probe(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: zz-ccswitchx-probe\ndescription: CC Switch X 项目级落点真机探针\n---\n\n探针。\n",
    )
    .unwrap();
}

fn names_in(dir: &Path, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|read| {
            read.flatten()
                .map(|item| item.file_name().to_string_lossy().to_string())
                .filter(|name| name.starts_with(prefix))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// 真实 home 里用户级的配置与 skills 目录清单。项目级同步只许写 `<项目>/…`，
/// 这些路径必须逐字不变 —— 与 `asset_chain_e2e` 的 real_agent_config_roots_are_untouched 同一手法：
/// 只记内容不记 mtime，别的工具顺手 touch 一次不算我们改了。
fn user_state(home: &Path) -> Vec<(PathBuf, Snapshot)> {
    [
        ".claude.json",
        ".qoder-cn/settings.json",
        ".codex/config.toml",
        ".claude/skills",
        ".qoder-cn/skills",
        ".agents/skills",
    ]
    .into_iter()
    .map(|relative| home.join(relative))
    .map(|path| (path.clone(), record(&path)))
    .collect()
}

/// 一条路径的当前内容：文件留字节，目录用项目自己的内容哈希，不存在就记 Missing。
#[derive(Debug, PartialEq, Eq)]
enum Snapshot {
    Missing,
    Bytes(Vec<u8>),
    Dir(String),
}

fn record(path: &Path) -> Snapshot {
    if path.is_dir() {
        return Snapshot::Dir(
            cc_switch_x_lib::net::archive::compute_dir_hash(path)
                .unwrap_or_else(|error| format!("哈希失败：{error}")),
        );
    }
    match std::fs::read(path) {
        Ok(bytes) => Snapshot::Bytes(bytes),
        Err(_) => Snapshot::Missing,
    }
}

/// 用代码里的静态 agent 表造一份现状，只改启用位；不碰真实的 `~/.claude` 等配置根。
fn enabled_agents() -> Vec<AgentStatus> {
    builtin_agents()
        .into_iter()
        .map(|descriptor| AgentStatus {
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
            enabled: ENABLED_AGENTS.contains(&descriptor.id.as_str()),
            enabled_mcp: Vec::new(),
            enabled_skills: Vec::new(),
            enabled_rules: Vec::new(),
            enabled_subagents: Vec::new(),
            enabled_hooks: Vec::new(),
            active_prompt: None,
        })
        .collect()
}

#[test]
#[ignore = "真机验证：需要能访问这台机器的真实 agent 用户级配置根做只读对照"]
fn project_skills_render_from_the_project_canonical() {
    // v3：真源与台账住在 store 里，渲染产物仍写在项目里。
    // 项目是临时目录，整个现场随 tempdir 一起回收，绝不碰本仓库。
    let temp = tempfile::tempdir().unwrap();
    let store = Store::init(temp.path().join("store")).unwrap();
    let root = temp.path().join("work/app");
    std::fs::create_dir_all(&root).unwrap();
    let paths = cc_switch_x_lib::domain::project::ProjectPaths::resolve(
        store.root(),
        &project_registry::project_id(&root),
        &root,
    )
    .unwrap();
    let claude_dir = probe_dir(&root, ".claude/skills");
    let qoder_dir = probe_dir(&root, ".qoder/skills");
    let qoder_skills = root.join(".qoder/skills");
    let canonical_dir = paths.canonical_skill_dir(PROBE).unwrap();
    let canonical_root = paths.canonical_root().to_path_buf();
    let ledger_path = paths.ledger_path().to_path_buf();

    write_probe(&claude_dir);

    let home = dirs::home_dir().expect("需要真实 home 才能校验用户级配置根");
    let user_before = user_state(&home);
    assert!(
        user_before.iter().any(|(path, _)| path.exists()),
        "这台机器上没有可对照的真实 agent 用户级配置，本次验证证明不了「不碰用户级配置」"
    );
    let enabled_ids: Vec<String> = ENABLED_AGENTS.iter().map(|id| id.to_string()).collect();
    let agents = enabled_agents();
    let selectable = project_registry::selectable_dir_keys(&agents);
    // 1. 注册项目：项目根在临时目录里；`~/.claude` 与 store 都在它之外，路径校验必须放行。
    let view = project_registry::add_project(&store, &home, &root, &enabled_ids)
        .expect("添加项目失败");
    assert!(view.exists);
    let view_id = view.id.clone();
    let sync = |resolutions: &[project_sync::Resolution]| {
        project_sync::sync_project(&store, &paths, &enabled_ids, &selectable, resolutions)
    };

    assert!(
        ledger_path.is_file(),
        "添加项目必须在 store 里写出台账：{}",
        ledger_path.display()
    );
    let ledger: ProjectLedger = project_scan::load_ledger(&paths).unwrap();
    assert_eq!(
        ledger.version,
        cc_switch_x_lib::domain::project::PROJECT_LEDGER_VERSION
    );
    let entry = ledger.skills.get(PROBE).unwrap_or_else(|| {
        panic!(
            "账本里应记录探针 {PROBE}，实际键：{:?}",
            ledger.skills.keys()
        )
    });
    assert!(entry.targets.is_empty(), "首次添加不得写入任何 target");
    assert!(
        !entry.canonical.present,
        "首次添加时真源还没落地，等用户收进真源"
    );
    assert!(
        entry.dirs.is_empty(),
        "dirs 只记渲染基线，观察到的现状不能写进去"
    );
    assert!(!qoder_dir.exists(), "首次添加绝不能复制内容");
    assert!(!canonical_dir.exists(), "首次添加不该建出真源目录");

    // 2. 能力闸按真实 agent 现状判定：已实测的两个目录可勾选，未取证的 `.agents/skills` 不行。
    assert!(
        selectable.contains(&"claude".to_string()) && selectable.contains(&"qoder".to_string()),
        "已实测目录应放行，实际：{selectable:?}"
    );
    assert!(
        !selectable.contains(&"agents".to_string()),
        "`.agents/skills` 未取证，必须被挡在可勾选列表外"
    );

    // 3. 收进真源：把 Claude 侧的探针变成项目真源，原目录一个字节都不动。
    let before = std::fs::read(claude_dir.join("SKILL.md")).unwrap();
    project_sync::adopt(&store, &paths, PROBE, "claude").expect("收进真源失败");
    assert_eq!(
        std::fs::read(claude_dir.join("SKILL.md")).unwrap(),
        before,
        "收进真源不得改动来源目录"
    );
    assert!(
        canonical_dir.join("SKILL.md").is_file(),
        "真源必须落在 store 的 local/projects/<id>/skills/{PROBE}"
    );

    // 4. 勾选目标：只记账本，不写盘。
    project_sync::set_targets(
        &paths,
        PROBE,
        &["claude".to_string(), "qoder".to_string()],
        &selectable,
    )
    .expect("勾选目标失败");
    assert!(
        !qoder_dir.exists(),
        "勾选本身不该复制（设计 5.3 的两步流程）"
    );

    // 5. 同步：把真源渲染到还没有副本的 qoder。
    let report = sync(&[]).expect("同步失败");
    assert!(report.conflicts.is_empty(), "不该有冲突：{:?}", report.conflicts);
    assert!(
        report
            .outcomes
            .iter()
            .any(|item| item.skill_id == PROBE && item.dir == "qoder" && item.action == "render"),
        "应有一条渲染到 qoder 的结果，实际：{:?}",
        report.outcomes
    );
    assert!(
        qoder_dir.join("SKILL.md").is_file(),
        "同步后 Qoder 项目目录里必须有 {PROBE}"
    );
    let after: ProjectLedger = project_scan::load_ledger(&paths).unwrap();
    let after_entry = after.skills.get(PROBE).unwrap();
    assert!(after_entry.dirs["qoder"].present, "qoder 基线应已写入");
    assert_eq!(
        after_entry.dirs["qoder"].tree_hash, after_entry.canonical.tree_hash,
        "渲染成功后基线必须等于真源"
    );
    assert!(after_entry.last_op.is_some(), "应记录最近一次操作");

    // 6. 幂等：无事可做时不得有写入，也不得重写仓库里要提交的 JSON。
    let ledger_bytes = std::fs::read(&ledger_path).unwrap();
    let second = sync(&[]).expect("第二次同步失败");
    assert!(
        second.outcomes.is_empty(),
        "无事可做时不该有写入：{:?}",
        second.outcomes
    );
    assert_eq!(
        std::fs::read(&ledger_path).unwrap(),
        ledger_bytes,
        "空跑不得重写项目里要提交的 JSON"
    );

    // 7a. 外部改副本：真源与副本都变了，只能报冲突，不许覆盖用户的手改。
    std::fs::write(qoder_dir.join("SKILL.md"), "用户的手改\n").unwrap();
    std::fs::write(canonical_dir.join("SKILL.md"), "---\nname: x\n---\n真源 v2\n").unwrap();
    let third = sync(&[]).expect("冲突轮同步失败");
    assert!(third.outcomes.is_empty(), "冲突时不写盘：{:?}", third.outcomes);
    assert_eq!(third.conflicts.len(), 1, "{:?}", third.conflicts);
    assert_eq!(third.conflicts[0].kind, "targetVsCanonical");
    assert_eq!(
        std::fs::read(qoder_dir.join("SKILL.md")).unwrap(),
        "用户的手改\n".as_bytes()
    );

    // 7b. 裁决以真源为准（keepBoth 保住手改）后，副本跟上真源。
    sync(&[project_sync::Resolution {
        skill_id: PROBE.to_string(),
        dir: "qoder".to_string(),
        action: "keepBoth".to_string(),
    }])
    .expect("裁决后同步失败");
    assert_eq!(
        std::fs::read(qoder_dir.join("SKILL.md")).unwrap(),
        std::fs::read(canonical_dir.join("SKILL.md")).unwrap(),
        "被改副本要按真源重渲染"
    );
    let copies = names_in(&qoder_skills, &format!("{PROBE}-copy"));
    assert!(
        copies
            .iter()
            .any(|name| qoder_skills.join(name).join("SKILL.md").is_file()),
        "落败的手改必须另存为未跟踪副本，实际：{copies:?}"
    );

    // 8. 卸载：真源与全部副本一起删除，且都先留了备份；条目随之移除。
    project_sync::uninstall(&store, &paths, PROBE).expect("卸载失败");
    assert!(!canonical_dir.exists(), "卸载必须删除项目真源");
    assert!(!claude_dir.exists(), "Claude 侧副本应被删除");
    assert!(!qoder_dir.exists(), "Qoder 侧副本应被删除");
    assert!(
        store
            .root()
            .join("backups")
            .join("projects")
            .join(&view_id)
            .is_dir(),
        "删除前必须留下备份"
    );
    assert!(
        !root.join(".agents/skills").exists(),
        "未实测的 .agents/skills 一个文件都不该被写进去"
    );

    // 8b. 未跟踪的落败副本不归本项目管，卸载不得顺手删掉它。
    assert_eq!(
        names_in(&qoder_skills, &format!("{PROBE}-copy")),
        copies,
        "卸载只清 tracked 副本，未跟踪副本必须原样留着"
    );

    let final_ledger = project_scan::load_ledger(&paths).unwrap();
    assert!(
        !final_ledger.skills.contains_key(PROBE),
        "真源与所有 target 都不存在时应移除条目"
    );
    // 9. 项目里不该留下任何属于我们自己的文件（只剩 agent 渲染目录的痕迹）。
    assert!(
        !root.join(".ccswitchx").exists(),
        "v3 不该在项目里留下 .ccswitchx"
    );
    assert!(!canonical_root.starts_with(&root), "真源必须住在 store 里");

    // 10. 用户级配置一个字节都没碰：真实 home 下的 agent 配置与 skills 目录内容必须逐字相同。
    for (path, before) in &user_before {
        assert_eq!(before, &record(path), "项目级同步改动了用户级配置：{path:?}");
    }
}
