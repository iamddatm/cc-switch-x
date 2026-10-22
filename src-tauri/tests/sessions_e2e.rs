//! 会话浏览端到端：11 个 provider 的合成样本 → 扫描 → 列表/搜索/阅读/导出/恢复。
//!
//! 所有 provider 的 baseOverride 都指向临时沙箱，真实家目录不参与；
//! 样本从 `tests/fixtures/sessions/` 复制，不在仓库外留下内容。

use cc_switch_x_lib::index::Index;
use cc_switch_x_lib::services::agents::save_override;
use cc_switch_x_lib::services::sessions::{
    get_session, list_sessions, scan_all, search_in_session, search_sessions,
};
use cc_switch_x_lib::sessions::export::{render_markdown, resume_info};
use cc_switch_x_lib::sessions::{provider, providers};
use cc_switch_x_lib::store::Store;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const ALL_AGENTS: [&str; 11] = [
    "claude",
    "codex",
    "gemini",
    "grokbuild",
    "hermes",
    "mcode",
    "openclaw",
    "opencode",
    "pi",
    "agy",
    "qoder-cn",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions")
}

fn copy_file(from: &Path, to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(from, to).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn write_mcode_database(database: &Path) {
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let conn = Connection::open(database).unwrap();
    conn.execute_batch(
        "CREATE TABLE local_runtime_sessions (
             session_id TEXT PRIMARY KEY, title TEXT, workspace_dir TEXT,
             created_at_ms INTEGER, updated_at_ms INTEGER, visibility TEXT,
             parent_session_id TEXT, session_kind TEXT, archived INTEGER
         );
         CREATE TABLE local_runtime_messages (
             session_id TEXT, display_messages_json TEXT
         );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO local_runtime_sessions VALUES
         ('mvs_e2e','MiniMax 标题','D:/mcode',1000,2000,'visible',NULL,'conversation',0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO local_runtime_messages VALUES ('mvs_e2e', ?1)",
        [r#"[{"role":"user","msg_content":"MiniMax 问题"},{"role":"assistant","msg_content":"MiniMax 回答"}]"#],
    )
    .unwrap();
}

fn write_opencode_database(database: &Path) {
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let conn = Connection::open(database).unwrap();
    conn.execute_batch(
        "CREATE TABLE session_v2 (
             id TEXT PRIMARY KEY, title TEXT NOT NULL, directory TEXT NOT NULL,
             time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL
         );
         CREATE TABLE session_message (
             id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
             seq INTEGER NOT NULL, data TEXT NOT NULL,
             time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL
         );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_v2 VALUES ('ses_e2e','OpenCode 标题','D:/opencode',1000,2000)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_message VALUES
         ('msg_1','ses_e2e','user',1,'{\"text\":\"OpenCode 问题\"}',1000,1000)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_message VALUES
         ('msg_2','ses_e2e','assistant',2,'{\"content\":[{\"type\":\"text\",\"text\":\"OpenCode 回答\"}]}',2000,2000)",
        [],
    )
    .unwrap();
}

struct Sandbox {
    _dir: tempfile::TempDir,
    store: Store,
    index: Index,
    root: PathBuf,
    fake_home: PathBuf,
}

impl Sandbox {
    fn scan(&mut self) -> cc_switch_x_lib::services::sessions::SessionScanReport {
        scan_all(&self.store, &mut self.index, &self.fake_home).unwrap()
    }
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let index = Index::open(&dir.path().join("index.db")).unwrap();
    let root = dir.path().join("sandbox");
    let fake_home = dir.path().join("home");
    std::fs::create_dir_all(&fake_home).unwrap();
    for entry in providers() {
        save_override(&store, entry.agent_id, Some(&root)).unwrap();
    }

    let fx = fixtures();
    copy_dir(
        &fx.join("claude-projects/p1"),
        &root.join(".claude/projects/p1"),
    );
    copy_file(
        &fx.join("codex.jsonl"),
        &root.join(".codex/sessions/2026/10/20/rollout-c1.jsonl"),
    );
    copy_dir(&fx.join("gemini/demo"), &root.join(".gemini/tmp/demo"));
    copy_dir(
        &fx.join("grokbuild/sessions/p1"),
        &root.join(".grok/sessions/p1"),
    );
    copy_file(
        &fx.join("hermes/sessions/mixed.jsonl"),
        &root.join(".hermes/sessions/mixed.jsonl"),
    );
    copy_dir(
        &fx.join("openclaw/agents/main"),
        &root.join(".openclaw/agents/main"),
    );
    copy_file(
        &fx.join("pi.jsonl"),
        &root.join(".pi/agent/sessions/p/s1.jsonl"),
    );
    copy_dir(
        &fx.join("qoder-projects/proj"),
        &root.join(".qoder-cn/projects/proj"),
    );
    copy_file(
        &fx.join("agy/brain/abc-123/.system_generated/logs/transcript_full.jsonl"),
        &root.join(
            ".gemini/antigravity-cli/brain/abc-123/.system_generated/logs/transcript_full.jsonl",
        ),
    );
    write_mcode_database(&root.join(".minimax/v2/sqlite/runtime-state.sqlite"));
    write_opencode_database(&root.join(".local/share/opencode/opencode.db"));

    Sandbox {
        _dir: dir,
        store,
        index,
        root,
        fake_home,
    }
}

#[test]
fn every_provider_scans_lists_searches_reads_exports_and_resumes_in_a_sandbox() {
    let mut fx = sandbox();
    let first = fx.scan();
    assert_eq!(first.added, 11, "{first:?}");
    assert!(first.failures.is_empty(), "{first:?}");

    for agent in ALL_AGENTS {
        let sessions = list_sessions(&fx.index, Some(agent), None, None, None).unwrap();
        assert_eq!(sessions.len(), 1, "{agent} 应恰好有 1 个会话：{sessions:?}");
    }

    let hits = search_sessions(&fx.index, "修复构建", Some(10)).unwrap();
    assert!(
        hits.iter().any(|hit| hit.session.agent_id == "codex"),
        "{hits:?}"
    );
    let short = search_sessions(&fx.index, "回答", Some(10)).unwrap();
    assert!(!short.is_empty(), "1–2 字的查询要走 LIKE 并且能命中");
    assert!(
        search_sessions(&fx.index, "\" * %", Some(10))
            .unwrap()
            .is_empty(),
        "纯标点查询必须安全返回空"
    );

    let matches = search_in_session(&fx.index, "claude", "s1", "主会话", Some(10)).unwrap();
    assert_eq!(matches.len(), 1, "{matches:?}");
    assert_eq!(matches[0].index, 0);

    let detail = get_session(&fx.index, "claude", "s1", None, None).unwrap();
    assert_eq!(detail.total, 1);
    assert_eq!(detail.messages[0].content, "主会话");

    let markdown = render_markdown(&detail);
    assert!(markdown.starts_with("# 主会话\n"), "{markdown}");
    assert!(markdown.contains("- Agent: Claude Code"), "{markdown}");
    assert!(!markdown.contains("base64"), "导出不得内联图片 base64");
    let export_path = fx._dir.path().join("export/s1.md");
    cc_switch_x_lib::store::atomic::write_atomic(&export_path, markdown.as_bytes()).unwrap();
    assert_eq!(std::fs::read_to_string(&export_path).unwrap(), markdown);

    for agent in ALL_AGENTS {
        let sessions = list_sessions(&fx.index, Some(agent), None, None, None).unwrap();
        let provider = provider(agent).unwrap();
        let info = resume_info(provider, &sessions[0]);
        match provider.resume {
            Some(_) => {
                let info = info.unwrap_or_else(|| panic!("{agent} 缺少恢复命令"));
                let needle = if agent == "pi" {
                    &sessions[0].source_path
                } else {
                    &sessions[0].session_id
                };
                assert!(info.command.contains(needle.as_str()), "{}", info.command);
            }
            None => assert!(info.is_none(), "{agent} 不该有恢复命令"),
        }
    }

    assert_eq!(
        std::fs::read_dir(&fx.fake_home).unwrap().count(),
        0,
        "baseOverride 下不能读取真实家目录"
    );

    let second = fx.scan();
    assert_eq!(
        (
            second.added,
            second.updated,
            second.removed,
            second.unchanged
        ),
        (0, 0, 0, 11),
        "{second:?}"
    );

    let claude_file = fx.root.join(".claude/projects/p1/s1.jsonl");
    let mut text = std::fs::read_to_string(&claude_file).unwrap();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(
        &serde_json::json!({
            "type": "assistant",
            "sessionId": "s1",
            "cwd": "D:/repo",
            "timestamp": "2026-10-20T10:01:00Z",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "追加回答"}]}
        })
        .to_string(),
    );
    text.push('\n');
    std::fs::write(&claude_file, text).unwrap();
    let changed = fx.scan();
    assert_eq!(
        (
            changed.added,
            changed.updated,
            changed.removed,
            changed.unchanged
        ),
        (0, 1, 0, 10),
        "{changed:?}"
    );
    assert_eq!(
        get_session(&fx.index, "claude", "s1", None, None)
            .unwrap()
            .total,
        2
    );

    std::fs::remove_file(fx.root.join(".codex/sessions/2026/10/20/rollout-c1.jsonl")).unwrap();
    let removed = fx.scan();
    assert_eq!(removed.removed, 1, "{removed:?}");
    assert!(list_sessions(&fx.index, Some("codex"), None, None, None)
        .unwrap()
        .is_empty());
}

#[test]
fn only_locally_verified_providers_are_marked_verified() {
    let verified: Vec<_> = providers()
        .iter()
        .filter(|entry| entry.verified)
        .map(|entry| entry.agent_id)
        .collect();
    assert_eq!(verified, vec!["claude", "codex", "pi", "qoder-cn"]);
}
