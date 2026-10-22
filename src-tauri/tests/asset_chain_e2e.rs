//! 沙箱真机：三类文件型资产 + 漂移处置的全链路。
//!
//! 全部写在临时目录里：store 在 `temp/store`，agent 配置根通过 `local/agents/<id>.json`
//! 的 `baseOverride` 指到 `temp/sandbox`。**不允许**碰真实的 `~/.claude`、`~/.qoder-cn`、
//! `~/.codex`，所以最后一步会核对真实文件的哈希前后一致。

use cc_switch_x_lib::adapters::rules;
use cc_switch_x_lib::adapters::subagents;
use cc_switch_x_lib::domain::agent::{builtin_agents, AgentDescriptor};
use cc_switch_x_lib::domain::rule::{
    parse_rule, render_canonical as render_rule, Rule, RuleFrontmatter,
};
use cc_switch_x_lib::domain::subagent::{render_canonical as render_subagent, Subagent};
use cc_switch_x_lib::domain::{AssetType, DriftKind};
use cc_switch_x_lib::index::Index;
use cc_switch_x_lib::services::drift::{self, ResolveAction};
use cc_switch_x_lib::services::scan;
use cc_switch_x_lib::store::Store;
use serde_json::{Map, Value as JsonValue};
use std::path::PathBuf;

fn agent(id: &str) -> AgentDescriptor {
    builtin_agents().into_iter().find(|a| a.id == id).unwrap()
}

struct Sandbox {
    /// 只在 Drop 时保住临时目录。
    _dir: tempfile::TempDir,
    store: Store,
    index: Index,
    home: PathBuf,
    sandbox_root: PathBuf,
}

fn setup() -> Sandbox {
    let dir = tempfile::tempdir().expect("临时目录");
    let store = Store::init(dir.path().join("store")).expect("store");
    let index = Index::open(&dir.path().join("index.db")).expect("index");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let sandbox_root = dir.path().join("sandbox");
    std::fs::create_dir_all(&sandbox_root).unwrap();
    Sandbox {
        _dir: dir,
        store,
        index,
        home,
        sandbox_root,
    }
}

fn seed_rule(store: &Store) {
    let rule = Rule {
        id: "style".into(),
        meta: RuleFrontmatter {
            name: "style".into(),
            description: "写法约定".into(),
            paths: vec!["src/**".into(), "docs/{a,b}/*.md".into()],
            trigger: None,
        },
        body: "改文件前先读邻近实现。\n".into(),
    };
    store
        .write(
            AssetType::Rule,
            "style",
            render_rule(&rule).unwrap().as_bytes(),
        )
        .unwrap();
}

fn seed_subagent(store: &Store) {
    let mut extras = Map::new();
    extras.insert("when-to-use".into(), JsonValue::String("需要审查时".into()));
    let mut tool_extras = Map::new();
    tool_extras.insert("qoder-cn".into(), JsonValue::Object(extras));
    let sub = Subagent {
        name: "reviewer".into(),
        description: "代码审查".into(),
        instructions: "先看 diff，再看测试。\n第二行指令。".into(),
        model: None,
        tools: vec!["Read".into(), "Grep".into()],
        tool_extras: JsonValue::Object(tool_extras),
        targets: vec!["qoder-cn".into()],
    };
    store
        .write(
            AssetType::Subagent,
            "reviewer",
            render_subagent(&sub).unwrap().as_bytes(),
        )
        .unwrap();
}

fn deployed(sandbox: &Sandbox, agent: &AgentDescriptor, kind: AssetType) -> PathBuf {
    let dir = match kind {
        AssetType::Rule => agent.paths.rules_dir.as_ref().unwrap(),
        AssetType::Subagent => agent.paths.agents_dir.as_ref().unwrap(),
        other => unreachable!("只测文件型资产：{other:?}"),
    };
    let rest = dir.strip_prefix("~/").unwrap_or(dir.as_str());
    let name = match kind {
        AssetType::Rule => "style.md",
        _ => "reviewer.md",
    };
    sandbox.sandbox_root.join(rest).join(name)
}

#[test]
fn rules_and_subagents_deploy_read_back_and_are_adopted_on_drift() {
    let mut sandbox = setup();
    let qoder = agent("qoder-cn");
    seed_rule(&sandbox.store);
    seed_subagent(&sandbox.store);
    let base = Some(sandbox.sandbox_root.as_path());

    // ── 落盘 ────────────────────────────────────────────
    let rule_target = deployed(&sandbox, &qoder, AssetType::Rule);
    let subagent_target = deployed(&sandbox, &qoder, AssetType::Subagent);
    assert!(
        !rule_target.parent().unwrap().exists(),
        "沙箱里原本没有 rules 目录"
    );

    match rules::deploy(
        &sandbox.store,
        &mut sandbox.index,
        &qoder,
        "style",
        &sandbox.home,
        base,
    )
    .unwrap()
    {
        cc_switch_x_lib::adapters::Deployed::Written(report) => assert_eq!(report.written, 1),
        other => panic!("空目录里落盘不该被判外来：{other:?}"),
    }
    subagents::deploy(
        &sandbox.store,
        &mut sandbox.index,
        &qoder,
        "reviewer",
        &sandbox.home,
        base,
    )
    .unwrap();

    let rule_bytes = std::fs::read_to_string(&rule_target).unwrap();
    println!("── rules/style.md ──\n{rule_bytes}");
    assert_eq!(
        rule_bytes,
        "---\ntrigger: glob\nglob: src/**, docs/a/*.md, docs/b/*.md\n---\n\n改文件前先读邻近实现。\n",
        "brace 要展开成独立 glob 并去掉引号"
    );
    let subagent_bytes = std::fs::read_to_string(&subagent_target).unwrap();
    println!("── agents/reviewer.md ──\n{subagent_bytes}");
    assert!(subagent_bytes.contains("name: reviewer"));
    assert!(subagent_bytes.contains("when-to-use: 需要审查时"));
    assert!(subagent_bytes.ends_with("第二行指令。\n"));

    // ── 幂等 ────────────────────────────────────────────
    let again = rules::deploy(
        &sandbox.store,
        &mut sandbox.index,
        &qoder,
        "style",
        &sandbox.home,
        base,
    )
    .unwrap();
    match again {
        cc_switch_x_lib::adapters::Deployed::Written(report) => {
            assert_eq!(report.skipped, 1, "重复落盘必须一个字节都不写")
        }
        other => panic!("{other:?}"),
    }

    // ── 扫描导入幂等 ────────────────────────────────────
    let scanned = scan::rules(
        &sandbox.store,
        &mut sandbox.index,
        &qoder,
        &sandbox.home,
        base,
    )
    .unwrap();
    assert_eq!(
        (scanned.imported, scanned.updated, scanned.unchanged),
        (0, 0, 1),
        "{scanned:?}"
    );

    // ── 手改 → rescan → 漂移 ───────────────────────────
    std::fs::write(
        &rule_target,
        "---\ntrigger: always_on\n---\n\n用户在 Qoder 那边补了一条\n",
    )
    .unwrap();
    let drifts = drift::rescan(&sandbox.store, &mut sandbox.index).unwrap();
    println!("检出漂移 {} 条：{drifts:?}", drifts.len());
    assert_eq!(
        drifts.len(),
        1,
        "只有被手改的那条该被检出，没动的不能报漂移：{drifts:?}"
    );
    let rule_drift = drifts.first().unwrap();
    assert_eq!(rule_drift.asset_type, AssetType::Rule);
    assert_eq!(rule_drift.kind, DriftKind::Modified);
    assert_eq!(rule_drift.target_path, rule_target);

    // 有未解决漂移时，写命令必须被闸拦下
    assert!(drift::ensure_no_open_drift(
        &sandbox.index,
        cc_switch_x_lib::domain::AssetKind::Rule,
        "style",
        "qoder-cn"
    )
    .is_err());

    // ── 采纳磁盘 ───────────────────────────────────────
    let before = std::fs::read(&rule_target).unwrap();
    drift::resolve(
        &sandbox.store,
        &mut sandbox.index,
        &sandbox.home,
        base,
        rule_drift.id,
        ResolveAction::AdoptDisk,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&rule_target).unwrap(),
        before,
        "采纳磁盘不许动盘"
    );
    let canonical =
        String::from_utf8(sandbox.store.read(AssetType::Rule, "style").unwrap()).unwrap();
    let parsed = parse_rule("style", &canonical).unwrap();
    assert_eq!(parsed.body, "用户在 Qoder 那边补了一条\n");
    assert_eq!(
        parsed.meta.paths,
        vec!["src/**".to_string(), "docs/{a,b}/*.md".to_string()],
        "真源自己的生效范围不能被工具侧派生字段换掉：{canonical}"
    );
    assert!(drift::rescan(&sandbox.store, &mut sandbox.index)
        .unwrap()
        .iter()
        .all(|record| record.asset_id != "style"));

    // ── 重刷真源 ───────────────────────────────────────
    std::fs::write(
        &subagent_target,
        "---
name: reviewer
description: 代码审查
---

用户在 Qoder 那边改了子代理
",
    )
    .unwrap();
    let subagent_drift = drift::rescan(&sandbox.store, &mut sandbox.index)
        .unwrap()
        .into_iter()
        .find(|record| record.asset_type == AssetType::Subagent)
        .expect("改过的子代理该被检出");
    drift::resolve(
        &sandbox.store,
        &mut sandbox.index,
        &sandbox.home,
        base,
        subagent_drift.id,
        ResolveAction::RepushSource,
    )
    .unwrap();
    let repushed = std::fs::read_to_string(&subagent_target).unwrap();
    assert!(repushed.contains("先看 diff，再看测试。"), "{repushed}");
    assert!(!repushed.contains("用户在 Qoder 那边补了一条"));

    // ── 取消落盘：只删自己那份 ──────────────────────────
    std::fs::write(
        sandbox.rule_parent().join("user-own.md"),
        "用户自己的规则\n",
    )
    .unwrap();
    assert_eq!(
        rules::remove(
            &sandbox.store,
            &mut sandbox.index,
            &qoder,
            "style",
            &sandbox.home,
            base
        )
        .unwrap(),
        cc_switch_x_lib::adapters::Removed::Ours
    );
    assert!(!rule_target.exists());
    assert!(sandbox.rule_parent().join("user-own.md").is_file());
    assert!(
        sandbox
            .store
            .root()
            .join("backups")
            .read_dir()
            .unwrap()
            .count()
            > 0,
        "每次写入都该留下备份"
    );
}

impl Sandbox {
    fn rule_parent(&self) -> PathBuf {
        deployed(self, &agent("qoder-cn"), AssetType::Rule)
            .parent()
            .unwrap()
            .to_path_buf()
    }
}

/// 真实配置根必须一个字节都没变：本测试只往临时沙箱写。
#[test]
fn real_agent_config_roots_are_untouched() {
    let watched: Vec<PathBuf> = [
        "~/.claude.json",
        "~/.qoder-cn/settings.json",
        "~/.codex/config.toml",
    ]
    .iter()
    .map(|declared| {
        let home = dirs::home_dir().unwrap();
        home.join(declared.strip_prefix("~/").unwrap())
    })
    .filter(|path| path.is_file())
    .collect();
    assert!(!watched.is_empty(), "至少要盯住一个真实配置文件");
    let before: Vec<(PathBuf, Vec<u8>)> = watched
        .iter()
        .map(|path| (path.clone(), std::fs::read(path).unwrap()))
        .collect();

    let mut sandbox = setup();
    let qoder = agent("qoder-cn");
    seed_rule(&sandbox.store);
    rules::deploy(
        &sandbox.store,
        &mut sandbox.index,
        &qoder,
        "style",
        &sandbox.home,
        Some(sandbox.sandbox_root.as_path()),
    )
    .unwrap();

    for (path, bytes) in before {
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{path:?} 被改动了");
    }
}

#[test]
fn hooks_land_in_the_shared_file_without_touching_foreign_keys() {
    use cc_switch_x_lib::adapters::hooks;
    use cc_switch_x_lib::domain::hook::Hook;

    let sandbox = setup();
    let base = sandbox.sandbox_root.as_path();
    let qoder = agent("qoder-cn");
    let settings = base.join(".qoder-cn/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        "{\"mcpServers\":{\"theirs\":{\"command\":\"x\"}},\"model\":\"haiku\",\"hooks\":{\"SessionStart\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"echo user\"}]}]}}",
    )
    .unwrap();
    let before = std::fs::read_to_string(&settings).unwrap();

    let hook = Hook {
        name: "fmt".into(),
        event: "PreToolUse".into(),
        matcher: "Bash".into(),
        command: "cargo fmt".into(),
        command_windows: None,
        timeout: Some(30),
    };
    let plan = hooks::plan_enable(&qoder, "fmt", &hook, &sandbox.home, Some(base)).unwrap();
    assert_eq!(plan.target_path, settings);
    std::fs::write(&settings, &plan.contents).unwrap();
    let after: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
    println!(
        "── 写完的 hooks 子树 ──\n{}",
        serde_json::to_string_pretty(&after["hooks"]).unwrap()
    );
    assert_eq!(
        after["hooks"]["PreToolUse"][0]["description"],
        "[ccswitchx:hook:fmt]"
    );
    assert_eq!(after["hooks"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(after["model"], "haiku", "非管辖键必须原样留着");
    assert_eq!(after["mcpServers"]["theirs"]["command"], "x");
    assert_eq!(
        after["hooks"]["SessionStart"].as_array().unwrap().len(),
        1,
        "用户自己的那条不能被动"
    );
    // 顶层键顺序也保持（serde_json 开了 preserve_order）
    assert_eq!(
        after.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["mcpServers", "model", "hooks"]
    );

    // 取消：只删带标记的那条
    let plan = hooks::plan_disable(&qoder, "fmt", "PreToolUse", &sandbox.home, Some(base)).unwrap();
    let reverted: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
    assert!(reverted["hooks"].get("PreToolUse").is_none());
    assert_eq!(
        reverted["hooks"]["SessionStart"].as_array().unwrap().len(),
        1
    );
    assert_eq!(reverted["model"], "haiku");
    assert!(before.contains("echo user"));
}
