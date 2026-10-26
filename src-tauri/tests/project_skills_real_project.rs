//! 真机端到端验证：用**临时 store** 把本仓库当作一个真实项目，跑通
//! 注册 → 扫描 → 基线 → 能力闸 → 勾选目标 → 同步 → 删除传播 + 备份 的完整链路。
//!
//! 只在 `cargo test -- --ignored` 时运行；它会在这台机器上读写
//! `.claude/skills/`、`.qoder/skills/` 与项目里的 `.ccswitchx/skills.json`，
//! 并在结束时清掉本次运行创建的文件。
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

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri 的上一级就是仓库根")
        .to_path_buf()
}

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

/// 只删除「本次运行前不存在」的路径，避免把用户原有的东西一起清掉。
struct Restore {
    path: PathBuf,
    existed_before: bool,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if self.existed_before || !self.path.exists() {
            return;
        }
        let result = if self.path.is_dir() {
            std::fs::remove_dir_all(&self.path)
        } else {
            std::fs::remove_file(&self.path)
        };
        if result.is_err() {
            eprintln!("清理探针失败，请手动删除：{}", self.path.display());
        }
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
#[ignore = "真机验证：会临时写本仓库的项目目录，需要能访问这台机器的 agent 配置根"]
fn project_skills_sync_on_real_project() {
    let root = project_root();
    let claude_dir = probe_dir(&root, ".claude/skills");
    let qoder_dir = probe_dir(&root, ".qoder/skills");
    let ledger_path = project_scan::ledger_path(&root);

    let qoder_restore = Restore {
        path: qoder_dir.clone(),
        existed_before: qoder_dir.exists(),
    };
    let ledger_restore = Restore {
        path: ledger_path.clone(),
        existed_before: ledger_path.exists(),
    };
    assert!(
        !qoder_restore.existed_before,
        "`.qoder/skills/{PROBE}` 已存在，无法判断它是不是本次同步生成的；请先手动删除"
    );

    write_probe(&claude_dir);

    let temp = tempfile::tempdir().unwrap();
    let store = Store::init(temp.path().join("store")).unwrap();
    let home = dirs::home_dir().expect("需要真实 home 才能校验用户级配置根");
    let enabled_ids: Vec<String> = ENABLED_AGENTS.iter().map(|id| id.to_string()).collect();

    // 1. 注册项目：项目根就是本仓库；`~/.claude` 与 store 都在它之外，路径校验必须放行。
    let view = project_registry::add_project(&store, &home, &root, &enabled_ids)
        .expect("添加项目失败");
    assert!(view.exists);
    assert!(ledger_path.is_file(), "添加项目必须写入 .ccswitchx/skills.json");

    let ledger: ProjectLedger = project_scan::load_ledger(&root).unwrap();
    let entry = ledger.skills.get(PROBE).unwrap_or_else(|| {
        panic!(
            "账本里应记录探针 {PROBE}，实际键：{:?}",
            ledger.skills.keys()
        )
    });
    assert!(entry.dirs["claude"].present, "Claude 侧应扫描到探针目录");
    assert!(entry.targets.is_empty(), "首次添加不得写入任何 target");
    assert!(
        !qoder_dir.exists(),
        "首次添加只写 JSON，绝不能复制内容"
    );

    // 2. 能力闸按真实 agent 现状判定：已实测的两个目录可勾选，未取证的 `.agents/skills` 不行。
    let selectable = project_registry::selectable_dir_keys(&enabled_agents());
    assert!(
        selectable.contains(&"claude".to_string()) && selectable.contains(&"qoder".to_string()),
        "已实测目录应放行，实际：{selectable:?}"
    );
    assert!(
        !selectable.contains(&"agents".to_string()),
        "`.agents/skills` 未取证，必须被挡在可勾选列表外"
    );

    // 3. 勾选目标：只记账本，不写盘。
    project_sync::set_targets(
        &root,
        PROBE,
        &["claude".to_string(), "qoder".to_string()],
        &selectable,
    )
    .expect("勾选目标失败");
    assert!(
        !qoder_dir.exists(),
        "勾选本身不该复制（设计 5.3 的两步流程）"
    );

    // 4. 同步：把 Claude 侧的探针复制到 Qoder 侧。
    let report = project_sync::sync_project(&store, &view.id, &root, &enabled_ids, &[])
        .expect("同步失败");
    assert!(report.conflicts.is_empty(), "不该有冲突：{:?}", report.conflicts);
    assert!(
        report
            .outcomes
            .iter()
            .any(|item| item.skill_id == PROBE && item.dir == "qoder" && item.action == "copy"),
        "应有一条复制到 qoder 的结果，实际：{:?}",
        report.outcomes
    );
    assert!(
        qoder_dir.join("SKILL.md").is_file(),
        "同步后 Qoder 项目目录里必须有 {PROBE}"
    );

    let after: ProjectLedger = project_scan::load_ledger(&root).unwrap();
    let after_entry = after.skills.get(PROBE).unwrap();
    assert!(after_entry.dirs["qoder"].present, "qoder 基线应已写入");
    assert!(after_entry.last_op.is_some(), "应记录最近一次操作");

    // 5. 幂等：无事可做时不得有写入，也不得重写仓库里要提交的 JSON。
    let ledger_bytes = std::fs::read(&ledger_path).unwrap();
    let second = project_sync::sync_project(&store, &view.id, &root, &enabled_ids, &[])
        .expect("第二次同步失败");
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

    // 6. 删除传播：删掉 Claude 侧副本后，同步必须把 Qoder 侧一起删掉，且先留备份。
    std::fs::remove_dir_all(&claude_dir).unwrap();
    let third = project_sync::sync_project(&store, &view.id, &root, &enabled_ids, &[])
        .expect("删除传播失败");
    assert!(
        !qoder_dir.exists(),
        "删除必须传播到其他 target；outcomes={:?}",
        third.outcomes
    );
    assert!(
        store
            .root()
            .join("backups")
            .join("projects")
            .join(&view.id)
            .is_dir(),
        "删除前必须留下备份"
    );

    let final_ledger = project_scan::load_ledger(&root).unwrap();
    assert!(
        !final_ledger.skills.contains_key(PROBE),
        "所有 target 都不存在时应移除条目"
    );

    std::mem::drop(qoder_restore);
    std::mem::drop(ledger_restore);
}
