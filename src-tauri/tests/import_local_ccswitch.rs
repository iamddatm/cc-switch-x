//! 真机验证：拿本机真实 cc-switch 数据跑一遍导入。
//! 默认被 `#[ignore]` 挡住，因为它依赖本机装了 cc-switch；手动执行：
//! `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored`

use cc_switch_x_lib::domain::AssetType;
use cc_switch_x_lib::import::source::connect;
use cc_switch_x_lib::import::{self, mcp, prompts, skills, ImportSource};
use cc_switch_x_lib::index::Index;
use cc_switch_x_lib::store::secrets::Secrets;
use cc_switch_x_lib::store::Store;
use std::path::PathBuf;

fn ccswitch_home() -> Option<PathBuf> {
    let home = dirs::home_dir()?.join(".cc-switch");
    home.join("cc-switch.db").is_file().then_some(home)
}

/// 收集 cc-switch 里的明文凭据（env 值、headers 值、URL 查询参数值）。
fn collect_plaintext_secrets(conn: &rusqlite::Connection) -> Vec<String> {
    let mut values = Vec::new();
    let mut stmt = conn
        .prepare("SELECT server_config FROM mcp_servers")
        .unwrap();
    for config in stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .flatten()
    {
        let parsed: serde_json::Value = match serde_json::from_str(&config) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for key in ["env", "headers"] {
            if let Some(map) = parsed.get(key).and_then(|value| value.as_object()) {
                for value in map.values() {
                    if let Some(text) = value.as_str() {
                        if text.len() >= 8 {
                            values.push(text.to_string());
                        }
                    }
                }
            }
        }
        if let Some(url) = parsed.get("url").and_then(|value| value.as_str()) {
            if let Some((_, query)) = url.split_once('?') {
                for pair in query.split('&') {
                    if let Some((name, value)) = pair.split_once('=') {
                        let lowered = name.to_lowercase();
                        let credential = ["key", "token", "secret", "password", "auth"]
                            .iter()
                            .any(|marker| lowered.contains(marker));
                        if credential && value.len() >= 8 {
                            values.push(value.to_string());
                        }
                    }
                }
            }
        }
    }
    values
}

fn walk_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_files(&path, out);
            } else {
                out.push(path);
            }
        }
    }
}

#[test]
#[ignore = "依赖本机真实 cc-switch 安装"]
fn imports_local_ccswitch_without_leaking_secrets() {
    let Some(home) = ccswitch_home() else {
        eprintln!("跳过：本机没有 ~/.cc-switch/cc-switch.db");
        return;
    };
    let db = home.join("cc-switch.db");
    let before = std::fs::read(&db).unwrap();

    let temp = tempfile::tempdir().unwrap();
    let store = Store::init(temp.path().join("store")).unwrap();
    let mut index = Index::open(&temp.path().join("index.db")).unwrap();
    let mut secrets = Secrets::default();
    let source = ImportSource::InstalledDb(db.clone());
    let skills_root = home.join("skills");

    let preview = import::preview(&store, &source, &skills_root).unwrap();
    let opened = connect(&source).unwrap();
    let mcp_report = mcp::run(
        &store,
        &mut secrets,
        &mut index,
        opened.conn(),
        mcp::ConflictPolicy::Skip,
    )
    .unwrap();
    let prompt_report = prompts::run(&store, &mut index, opened.conn()).unwrap();
    let skill_report = skills::run(&store, &mut index, opened.conn(), &skills_root).unwrap();
    drop(opened);

    println!("预览: {preview:?}");
    println!("MCP: {mcp_report:?}");
    println!("Prompts: {prompt_report:?}");
    println!("Skills: {skill_report:?}");

    // 1) 计数必须与预览一致（预览读的是同一张表）
    assert_eq!(
        store.list_ids(AssetType::Mcp).unwrap().len(),
        preview.mcp_count as usize - mcp_report.failed.len(),
        "导入条数与预览对不上"
    );
    assert_eq!(
        store.list_ids(AssetType::Skill).unwrap().len() + skill_report.missing_dirs.len(),
        preview.skills_count as usize
    );

    // 2) 真源目录与索引里都不得出现 cc-switch 的明文凭据
    let plaintext = {
        let opened = connect(&source).unwrap();
        collect_plaintext_secrets(opened.conn())
    };
    assert!(
        !plaintext.is_empty(),
        "本机 cc-switch 里应有凭据可供检查；若为空请核实数据来源"
    );
    let mut files = Vec::new();
    walk_files(&temp.path().join("store/assets"), &mut files);
    files.push(temp.path().join("store/index.db"));
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes).to_string();
        for secret in &plaintext {
            assert!(
                !text.contains(secret),
                "{} 里泄漏了明文凭据",
                path.display()
            );
        }
    }

    // 3) 密钥确实落在本机保管文件里，且该文件在 .gitignore 覆盖范围内
    let stored = std::fs::read_to_string(temp.path().join("store/secrets.local.json")).unwrap();
    assert!(
        stored.contains("mcp."),
        "secrets.local.json 内容异常：{stored}"
    );
    let ignore = std::fs::read_to_string(temp.path().join("store/.gitignore")).unwrap();
    assert!(ignore.contains("secrets.local.json"));

    // 4) 只读导入：cc-switch 的库必须逐字节不变
    assert_eq!(std::fs::read(&db).unwrap(), before, "不得写 cc-switch 的库");
}
