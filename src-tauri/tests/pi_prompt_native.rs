//! pi 原生提示词资源的沙箱真机：系统提示文件与斜杠命令模板。
//!
//! 全部经 `base_override` 落到临时目录，不碰真实的 `~/.pi/agent`；
//! 覆盖两条纪律：外部改过的文件不许用旧视图覆盖，写入/删除前必须留底。

use cc_switch_x_lib::domain::agent::{builtin_agents, AgentDescriptor};
use cc_switch_x_lib::services::pi_prompt::{self, PiPromptFileKind};
use cc_switch_x_lib::store::Store;

fn pi() -> AgentDescriptor {
    builtin_agents()
        .into_iter()
        .find(|agent| agent.id == "pi")
        .expect("注册表里没有 pi")
}

fn backups(store: &Store) -> Vec<String> {
    std::fs::read_dir(store.root().join("backups").join("pi-prompts"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn system_file_writes_are_backed_up_and_external_edits_win() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let home = dir.path().join("home");
    let sandbox = dir.path().join("sandbox");
    let agent = pi();
    let agent_dir = pi_prompt::agent_dir(&agent, &home, Some(&sandbox));

    let missing = pi_prompt::read_file(
        &agent,
        PiPromptFileKind::SystemAppend,
        &home,
        Some(&sandbox),
    )
    .unwrap();
    assert!(!missing.exists);

    let saved = pi_prompt::replace_file(
        &store,
        &agent,
        PiPromptFileKind::SystemAppend,
        &missing.revision,
        "第一条\n",
        &home,
        Some(&sandbox),
    )
    .unwrap();
    assert!(agent_dir.join("APPEND_SYSTEM.md").is_file());

    // 第二次覆盖前会留底：备份里有第一版，目标文件是第二版
    let replaced = pi_prompt::replace_file(
        &store,
        &agent,
        PiPromptFileKind::SystemAppend,
        &saved.revision,
        "第二条\n",
        &home,
        Some(&sandbox),
    )
    .unwrap();
    assert_eq!(replaced.content, "第二条\n");
    assert!(!backups(&store).is_empty());

    // 外部改了文件：旧 revision 保存必须被拒绝，且不动磁盘
    std::fs::write(agent_dir.join("APPEND_SYSTEM.md"), "外部改动\n").unwrap();
    let error = pi_prompt::replace_file(
        &store,
        &agent,
        PiPromptFileKind::SystemAppend,
        &replaced.revision,
        "覆盖\n",
        &home,
        Some(&sandbox),
    )
    .unwrap_err();
    assert!(error.to_string().contains("在外部被改过"));
    assert_eq!(
        std::fs::read_to_string(agent_dir.join("APPEND_SYSTEM.md")).unwrap(),
        "外部改动\n"
    );
}

#[test]
fn templates_can_be_renamed_and_deleted_through_the_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let home = dir.path().join("home");
    let sandbox = dir.path().join("sandbox");
    let agent = pi();
    let prompts = pi_prompt::agent_dir(&agent, &home, Some(&sandbox)).join("prompts");

    let created = pi_prompt::save_template(
        &store,
        &agent,
        "draft",
        None,
        "missing",
        "先看风险\n",
        &home,
        Some(&sandbox),
    )
    .unwrap();
    let renamed = pi_prompt::save_template(
        &store,
        &agent,
        "review",
        Some("draft"),
        &created.revision,
        "先看风险\n再看结论\n",
        &home,
        Some(&sandbox),
    )
    .unwrap();

    assert!(!prompts.join("draft.md").exists());
    assert_eq!(
        std::fs::read_to_string(prompts.join("review.md")).unwrap(),
        "先看风险\n再看结论\n"
    );
    assert_eq!(
        pi_prompt::list_templates(&agent, &home, Some(&sandbox)).unwrap(),
        vec![renamed.clone()]
    );

    assert!(pi_prompt::delete_template(
        &store,
        &agent,
        "review",
        &renamed.revision,
        &home,
        Some(&sandbox)
    )
    .unwrap());
    assert!(pi_prompt::list_templates(&agent, &home, Some(&sandbox))
        .unwrap()
        .is_empty());
}
