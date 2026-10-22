use crate::adapters::paths::{expand_root, WriteTarget};
use crate::domain::agent::{AgentDescriptor, AgentPaths, McpTarget};
use crate::domain::mcp::{McpServer, Transport};
use crate::domain::AssetState;
use crate::error::AppError;
use crate::index::Index;
use crate::services::apply::{Action, ApplyReport, ApplyService, PlanItem};
use crate::store::secrets::Secrets;
use crate::store::Store;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{Array, DocumentMut, Item, Table};

/// 一条 MCP 在某个 agent 里的落点内容。
/// JSON 侧带 `type`；Codex 的 TOML 侧只写它认得的键（`command/args/env` 或 `url/http_headers`）。
#[derive(Debug, Clone)]
struct Rendered {
    transport: &'static str,
    command: Option<String>,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    url: Option<String>,
    headers: BTreeMap<String, String>,
}

fn target_path(paths: &AgentPaths, base_override: Option<&Path>, home: &Path) -> PathBuf {
    expand_root(paths.mcp_file(), base_override, home)
}

fn location(paths: &AgentPaths) -> String {
    match &paths.mcp {
        McpTarget::JsonRoot { key, .. } => key.clone(),
        McpTarget::TomlTable { table, .. } => table.clone(),
    }
}

/// 把 canonical 里的 `${secret:…}` 解成明文。解不了就直接失败——
/// 绝不能把空值或残缺引用写进 agent 配置。
fn render(server: &McpServer, secrets: &Secrets) -> Result<Rendered, AppError> {
    let resolve = |value: &str| -> Result<String, AppError> {
        secrets.resolve_text(value).map_err(|error| {
            AppError::InvalidInput(format!("解析密钥失败（{}）：{error}", server.id))
        })
    };
    let mut env = BTreeMap::new();
    for (key, value) in &server.env {
        env.insert(key.clone(), resolve(value)?);
    }
    let mut headers = BTreeMap::new();
    for (key, value) in &server.headers {
        headers.insert(key.clone(), resolve(value)?);
    }
    Ok(Rendered {
        transport: match server.transport {
            Transport::Stdio => "stdio",
            Transport::Http => "http",
            Transport::Sse => "sse",
            Transport::Ws => "ws",
        },
        command: server.command.clone(),
        args: server.args.clone(),
        env,
        url: server.url.as_deref().map(resolve).transpose()?,
        headers,
    })
}

impl Rendered {
    fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("type".into(), Value::String(self.transport.to_string()));
        if let Some(command) = &self.command {
            map.insert("command".into(), Value::String(command.clone()));
        }
        if !self.args.is_empty() {
            map.insert(
                "args".into(),
                Value::Array(self.args.iter().cloned().map(Value::String).collect()),
            );
        }
        if !self.env.is_empty() {
            map.insert("env".into(), string_map(&self.env));
        }
        if let Some(url) = &self.url {
            map.insert("url".into(), Value::String(url.clone()));
        }
        if !self.headers.is_empty() {
            map.insert("headers".into(), string_map(&self.headers));
        }
        Value::Object(map)
    }

    /// Codex 的 TOML 形态：stdio 写 command/args（env 走子表），远程写 url + http_headers。
    fn to_toml_table(&self) -> Table {
        let mut table = Table::new();
        match self.command.as_deref() {
            Some(command) => {
                table.insert("command", toml_edit::value(command));
                if !self.args.is_empty() {
                    let mut array = Array::new();
                    for arg in &self.args {
                        array.push(arg.clone());
                    }
                    table.insert("args", toml_edit::Value::Array(array).into());
                }
            }
            None => {
                if let Some(url) = &self.url {
                    table.insert("url", toml_edit::value(url.as_str()));
                }
                if !self.headers.is_empty() {
                    let mut sub = Table::new();
                    for (key, value) in &self.headers {
                        sub.insert(key, toml_edit::value(value.as_str()));
                    }
                    table.insert("http_headers", Item::Table(sub));
                }
            }
        }
        if !self.env.is_empty() {
            let mut sub = Table::new();
            for (key, value) in &self.env {
                sub.insert(key, toml_edit::value(value.as_str()));
            }
            table.insert("env", Item::Table(sub));
        }
        table
    }
}

fn string_map(values: &BTreeMap<String, String>) -> Value {
    let mut map = Map::new();
    for (key, value) in values {
        map.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(map)
}

pub fn plan_enable(
    agent: &AgentDescriptor,
    server: &McpServer,
    secrets: &Secrets,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PlanItem, AppError> {
    let rendered = render(server, secrets)?;
    let path = target_path(&agent.paths, base_override, home);
    let current = read_target(&path)?;
    let contents = match &agent.paths.mcp {
        McpTarget::JsonRoot { key, .. } => {
            merge_json(&current, key, &server.id, Some(&rendered.to_json()))?
        }
        McpTarget::TomlTable { table, .. } => {
            merge_toml(&current, table, &server.id, Some(&rendered))?
        }
    };
    Ok(plan_item(agent, &server.id, path, contents, Action::Enable))
}

pub fn plan_disable(
    agent: &AgentDescriptor,
    server_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PlanItem, AppError> {
    let path = target_path(&agent.paths, base_override, home);
    let current = read_target(&path)?;
    let contents = match &agent.paths.mcp {
        McpTarget::JsonRoot { key, .. } => merge_json(&current, key, server_id, None)?,
        McpTarget::TomlTable { table, .. } => merge_toml(&current, table, server_id, None)?,
    };
    Ok(plan_item(agent, server_id, path, contents, Action::Disable))
}

/// CLI 优先落盘（spec 5.3 的写入顺序）。`target.channel` 说清这条路径适不适用：
/// 没装 CLI 或 CLI 失败都**回落文件写入并在 `note` 里写明原因**，不把降级说成验证过。
pub fn write_enable(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    server: &McpServer,
    secrets: &Secrets,
    target: &WriteTarget,
) -> Result<ApplyReport, AppError> {
    let (home, base_override) = (target.home, target.base_override);
    let plan = plan_enable(agent, server, secrets, home, base_override)?;
    if let Some(cli) = target.channel.cli() {
        let rendered = render(server, secrets)?;
        let json = serde_json::to_string(&rendered.to_json())?;
        // 留底必须在叫 CLI 动手之前：它改的是同一个文件，写完就没有原始内容了。
        let snapshot = ApplyService::snapshot(store, std::slice::from_ref(&plan))?;
        match cli.add_mcp(&server.id, &json) {
            Ok(_) => {
                let confirmed = cli.mcp_present(&server.id);
                let mut report = ApplyService::record_external_write(
                    index,
                    std::slice::from_ref(&plan),
                    &snapshot,
                    if confirmed {
                        AssetState::Effective
                    } else {
                        AssetState::Written
                    },
                )?;
                report.note = Some(if confirmed {
                    "由 qoderclicn 写入，并用 mcp get 读回确认".into()
                } else {
                    // 写命令成功但读回没看到：只能说已写入，不能替用户猜它生效了
                    "qoderclicn 写入后 mcp get 没读到，只算已写入".into()
                });
                return Ok(report);
            }
            Err(error) => {
                return finish_with_file(
                    store,
                    index,
                    plan,
                    Some(format!("qoderclicn 失败：{error}；已改走文件写入")),
                )
            }
        }
    }
    finish_with_file(store, index, plan, target.channel.fallback_note())
}

/// 取消落盘。CLI 删完还要读回确认它真不在了；仍然读得到就用文件路径复核删除，
/// 因为「CLI 说删了但配置还在」正是不能当作完成的那种状态。
pub fn write_disable(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    server_id: &str,
    target: &WriteTarget,
) -> Result<ApplyReport, AppError> {
    let (home, base_override) = (target.home, target.base_override);
    let plan = plan_disable(agent, server_id, home, base_override)?;
    if let Some(cli) = target.channel.cli() {
        let snapshot = ApplyService::snapshot(store, std::slice::from_ref(&plan))?;
        match cli.remove_mcp(server_id) {
            Ok(_) if !cli.mcp_present(server_id) => {
                return ApplyService::record_external_write(
                    index,
                    std::slice::from_ref(&plan),
                    &snapshot,
                    AssetState::Off,
                );
            }
            Ok(_) => {
                return finish_with_file(
                    store,
                    index,
                    plan,
                    Some("qoderclicn 报删除成功但 mcp get 仍读得到，已用文件路径复核删除".into()),
                )
            }
            Err(error) => {
                return finish_with_file(
                    store,
                    index,
                    plan,
                    Some(format!("qoderclicn 失败：{error}；已改走文件写入")),
                )
            }
        }
    }
    finish_with_file(store, index, plan, target.channel.fallback_note())
}

fn finish_with_file(
    store: &Store,
    index: &mut Index,
    plan: PlanItem,
    note: Option<String>,
) -> Result<ApplyReport, AppError> {
    let mut report = ApplyService::apply(store, index, std::slice::from_ref(&plan))?;
    report.note = note;
    Ok(report)
}

fn plan_item(
    agent: &AgentDescriptor,
    server_id: &str,
    path: PathBuf,
    contents: Vec<u8>,
    action: Action,
) -> PlanItem {
    let marker = match action {
        Action::Disable => '-',
        _ => '+',
    };
    PlanItem {
        agent_id: agent.id.clone(),
        asset_type: crate::domain::AssetType::Mcp,
        asset_id: server_id.to_string(),
        action,
        target_path: path,
        contents,
        diff: vec![format!("{marker} {}.{}", location(&agent.paths), server_id)],
        // 没有实测依据时按「要重启」处理，不把未知报成安全
        needs_restart: agent.mcp_needs_restart.unwrap_or(true),
    }
}

fn read_target(path: &Path) -> Result<Vec<u8>, AppError> {
    if path.exists() {
        Ok(std::fs::read(path)?)
    } else {
        Ok(Vec::new())
    }
}

/// 键级合并 JSON：只动 `root_key` 下的那一个子键，其余键与顺序原样保留。
/// 解析不了的 JSON 直接失败，绝不覆盖一个我们看不懂的文件。
fn merge_json(
    current: &[u8],
    root_key: &str,
    id: &str,
    entry: Option<&Value>,
) -> Result<Vec<u8>, AppError> {
    let text = String::from_utf8_lossy(current).to_string();
    let mut doc: Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|error| AppError::InvalidInput(format!("目标 JSON 解析失败：{error}")))?
    };
    let root = doc
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidInput("目标 JSON 根节点不是对象".into()))?;
    let servers = root
        .entry(root_key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidInput(format!("{root_key} 不是对象")))?;
    match entry {
        Some(value) => {
            servers.insert(id.to_string(), value.clone());
        }
        None => {
            servers.remove(id);
        }
    }
    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    Ok(out.into_bytes())
}

/// 键级合并 TOML：用 toml_edit 只增删 `[<table>.<id>]`，注释、空行、别的表逐字节不动。
fn merge_toml(
    current: &[u8],
    table_name: &str,
    id: &str,
    entry: Option<&Rendered>,
) -> Result<Vec<u8>, AppError> {
    let text = String::from_utf8_lossy(current).to_string();
    let mut doc: DocumentMut = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse()
            .map_err(|error| AppError::InvalidInput(format!("目标 TOML 解析失败：{error}")))?
    };
    if !doc[table_name].is_table() {
        doc[table_name] = Item::Table(Table::new());
    }
    let table = doc
        .get_mut(table_name)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| AppError::InvalidInput(format!("{table_name} 不是表")))?;
    match entry {
        Some(rendered) => {
            table.insert(id, Item::Table(rendered.to_toml_table()));
        }
        None => {
            table.remove(id);
        }
    }
    Ok(doc.to_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::domain::mcp::McpServer;
    use crate::store::secrets::Secrets;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn agent(id: &str) -> crate::domain::agent::AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn server(id: &str) -> McpServer {
        McpServer {
            id: id.into(),
            name: id.into(),
            transport: crate::domain::mcp::Transport::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into(), "pkg".into()],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            description: String::new(),
            homepage: String::new(),
            docs: String::new(),
            tags: Vec::new(),
        }
    }

    fn seeded(dir: &std::path::Path, agent_id: &str, content: &str) -> std::path::PathBuf {
        let descriptor = agent(agent_id);
        let path = expand_root_for(&descriptor, dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    /// 测试里把「home」直接当成临时目录，落点就是 dir/<去掉 ~/ 的部分>
    fn expand_root_for(
        descriptor: &crate::domain::agent::AgentDescriptor,
        home: &std::path::Path,
    ) -> std::path::PathBuf {
        crate::adapters::paths::expand_root(descriptor.paths.mcp_file(), None, home)
    }

    #[test]
    fn json_target_keeps_foreign_keys_and_their_order() {
        let dir = tempdir().unwrap();
        let path = seeded(
            dir.path(),
            "claude",
            r#"{"permissions":{"defaultMode":"acceptEdits"},"mcpServers":{"theirs":{"command":"x"}},"statusLine":{"type":"command","command":"echo hi"},"env":{"FOO":"bar"}}"#,
        );

        let plan = plan_enable(
            &agent("claude"),
            &server("newone"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        assert_eq!(plan.target_path, path);
        let after: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        let keys: Vec<String> = after.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            vec!["permissions", "mcpServers", "statusLine", "env"],
            "未管辖键必须原样保留且顺序不变"
        );
        assert_eq!(after["permissions"]["defaultMode"], "acceptEdits");
        assert_eq!(after["statusLine"]["command"], "echo hi");
        assert_eq!(after["env"]["FOO"], "bar");
        assert_eq!(
            after["mcpServers"]["theirs"]["command"], "x",
            "别人的 MCP 不能被删"
        );
        assert_eq!(after["mcpServers"]["newone"]["command"], "npx");
        assert_eq!(after["mcpServers"]["newone"]["type"], "stdio");
        assert_eq!(
            after["mcpServers"]["newone"]["args"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn json_disable_removes_only_our_key() {
        let dir = tempdir().unwrap();
        let plan = plan_disable(&agent("claude"), "theirs", dir.path(), None);
        // 目标文件不存在时不应炸，而是当作空对象处理
        let plan = plan.unwrap();
        let after: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        assert!(after["mcpServers"].get("theirs").is_none());
    }

    #[test]
    fn toml_target_touches_only_its_own_table() {
        let dir = tempdir().unwrap();
        let original = "# 顶部注释\nmodel = \"gpt\"\n\n[features]\nhooks = true\n\n[mcp_servers.theirs]\ncommand = \"keep\"\nargs = [\"a\"]\n\n# 尾部注释\n";
        let path = seeded(dir.path(), "codex", original);

        let plan = plan_enable(
            &agent("codex"),
            &server("codegraph"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        let after = String::from_utf8(plan.contents.clone()).unwrap();
        let original_lines: Vec<&str> = original.lines().collect();
        let after_lines: Vec<&str> = after.lines().collect();

        // 原有每一行都必须按原顺序保留（注释与空行也算）
        let mut cursor = 0usize;
        for line in &original_lines {
            let found = after_lines[cursor..]
                .iter()
                .position(|candidate| candidate == line)
                .unwrap_or_else(|| panic!("原有行被改掉或删掉：{line:?}\n--- 写入后 ---\n{after}"));
            cursor += found + 1;
        }
        // 多出来的行必须只有我们那一段
        let extra: Vec<&&str> = after_lines
            .iter()
            .filter(|line| !original_lines.contains(line))
            .collect();
        assert_eq!(
            extra,
            vec![
                &"[mcp_servers.codegraph]",
                &"command = \"npx\"",
                &"args = [\"-y\", \"pkg\"]"
            ],
            "只能多出我们自己的表：\n{after}"
        );
        assert!(path.is_file());
    }

    #[test]
    fn dotted_server_id_is_quoted_in_toml() {
        let dir = tempdir().unwrap();
        seeded(dir.path(), "codex", "[mcp_servers]\n");
        let plan = plan_enable(
            &agent("codex"),
            &server("code.graph"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        let after = String::from_utf8(plan.contents).unwrap();
        assert!(after.contains("[mcp_servers.\"code.graph\"]"), "{after}");
    }

    #[test]
    fn remote_transport_writes_url_not_command() {
        let dir = tempdir().unwrap();
        let mut remote = server("tavily");
        remote.transport = crate::domain::mcp::Transport::Http;
        remote.command = None;
        remote.url = Some("https://api.tavily.com/mcp".into());

        let plan = plan_enable(
            &agent("gemini"),
            &remote,
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        assert_eq!(value["mcpServers"]["tavily"]["type"], "http");
        assert_eq!(
            value["mcpServers"]["tavily"]["url"],
            "https://api.tavily.com/mcp"
        );
        assert!(value["mcpServers"]["tavily"].get("command").is_none());
    }

    #[test]
    fn unresolved_secret_fails_the_plan() {
        let dir = tempdir().unwrap();
        let mut with_secret = server("pw");
        with_secret
            .env
            .insert("API_TOKEN".into(), "${secret:mcp.pw.API_TOKEN}".into());
        let error = plan_enable(
            &agent("claude"),
            &with_secret,
            &Secrets::default(),
            dir.path(),
            None,
        )
        .expect_err("引用解析不了就不能出计划，更不能写空值");
        assert!(error.to_string().contains("缺少密钥"), "{error}");
    }

    #[test]
    fn resolved_secret_is_written_as_plaintext_into_the_agent_config() {
        let dir = tempdir().unwrap();
        let mut with_secret = server("pw");
        with_secret
            .env
            .insert("API_TOKEN".into(), "${secret:mcp.pw.API_TOKEN}".into());
        let mut secrets = Secrets::default();
        secrets.insert(
            &crate::domain::secret::SecretRef {
                scope: "mcp.pw".into(),
                key: "API_TOKEN".into(),
            },
            "sk-live",
        );

        let plan = plan_enable(&agent("claude"), &with_secret, &secrets, dir.path(), None).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&plan.contents).unwrap();
        assert_eq!(value["mcpServers"]["pw"]["env"]["API_TOKEN"], "sk-live");
    }

    #[test]
    fn an_override_root_relocates_every_write() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("real-home");
        let sandbox = dir.path().join("sandbox");
        std::fs::create_dir_all(&sandbox).unwrap();
        seeded(&sandbox, "claude", "{\"mcpServers\":{}}");

        let plan = plan_enable(
            &agent("claude"),
            &server("pw"),
            &Secrets::default(),
            &home,
            Some(&sandbox),
        )
        .unwrap();
        assert_eq!(plan.target_path, sandbox.join(".claude.json"));
        assert!(!plan.target_path.starts_with(&home), "绝不能写到真实 home");
    }

    #[test]
    fn agents_that_need_restart_are_flagged_on_the_plan() {
        let dir = tempdir().unwrap();
        seeded(dir.path(), "qoder-cn", "{\"mcpServers\":{}}");
        let plan = plan_enable(
            &agent("qoder-cn"),
            &server("pw"),
            &Secrets::default(),
            dir.path(),
            None,
        )
        .unwrap();
        assert!(plan.needs_restart);
        assert!(!plan.diff.is_empty());
    }
}

#[cfg(test)]
mod cli_write_tests {
    use super::*;
    use crate::adapters::cli::{CliChannel, CliError, QoderCli};
    use crate::domain::agent::builtin_agents;
    use crate::domain::mcp::{McpServer, Transport};
    use crate::index::Index;
    use crate::store::secrets::Secrets;
    use crate::store::Store;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn agent(id: &str) -> crate::domain::agent::AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn server(id: &str) -> McpServer {
        McpServer {
            id: id.into(),
            name: id.into(),
            transport: Transport::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into()],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            description: String::new(),
            homepage: String::new(),
            docs: String::new(),
            tags: Vec::new(),
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        index: Index,
        home: PathBuf,
        target: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        let home = dir.path().join("home");
        let target = home.join(".qoder-cn/settings.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            &target,
            "{\"model\":\"haiku\",\"mcpServers\":{\"theirs\":{\"command\":\"x\"}}}",
        )
        .unwrap();
        Fixture {
            _dir: dir,
            store,
            index,
            home,
            target,
        }
    }

    fn channel(run: fn(&Path, &[String]) -> Result<String, CliError>) -> CliChannel {
        CliChannel::Present(QoderCli::with_runner(PathBuf::from("fake-qoderclicn"), run))
    }

    fn fails(_: &Path, _: &[String]) -> Result<String, CliError> {
        Err(CliError::Failed {
            code: Some(1),
            message: "scope 必须是 project".into(),
        })
    }

    fn succeeds(_: &Path, _: &[String]) -> Result<String, CliError> {
        Ok("ok".into())
    }

    /// 只接受带 `--scope user` 的调用，其余一律算失败：
    /// 于是「结果=CLI」这件事本身就证明命令带对了 scope。
    fn strict_scope_user(_: &Path, args: &[String]) -> Result<String, CliError> {
        if args
            .windows(2)
            .any(|pair| pair[0] == "--scope" && pair[1] == "user")
        {
            Ok("ok".into())
        } else {
            Err(CliError::Failed {
                code: None,
                message: format!("没带 --scope user：{args:?}"),
            })
        }
    }

    /// 写命令成功，但 `mcp get` 读不到（比如它其实落到了别的 scope）。
    fn written_but_not_readable(_: &Path, args: &[String]) -> Result<String, CliError> {
        if args.get(1).map(String::as_str) == Some("get") {
            return Err(CliError::Failed {
                code: Some(1),
                message: "not found".into(),
            });
        }
        Ok("ok".into())
    }

    fn enable(fx: &mut Fixture, channel: CliChannel) -> ApplyReport {
        let home = fx.home.clone();
        let target = WriteTarget {
            home: &home,
            base_override: None,
            channel,
        };
        write_enable(
            &fx.store,
            &mut fx.index,
            &agent("qoder-cn"),
            &server("playwright"),
            &Secrets::default(),
            &target,
        )
        .unwrap()
    }

    fn state_of(fx: &Fixture) -> crate::domain::AssetState {
        fx.index
            .agent_states()
            .unwrap()
            .into_iter()
            .find(|row| row.asset_id == "playwright")
            .expect("落过盘的项必须有状态行")
            .state
    }

    #[test]
    fn a_failing_cli_falls_back_to_file_writing_and_says_so() {
        let mut fx = fixture();
        let before = std::fs::read_to_string(&fx.target).unwrap();
        let report = enable(&mut fx, channel(fails));

        assert_eq!(report.method_used, crate::adapters::cli::MethodUsed::File);
        let after = std::fs::read_to_string(&fx.target).unwrap();
        assert!(
            after.contains("playwright"),
            "文件回落后必须真的写上：{after}"
        );
        assert!(after.contains("theirs"), "非管辖键不能丢：{after}");
        let note = report.note.expect("降级要写明原因");
        assert!(note.contains("已改走文件写入"), "{note}");
        assert!(
            note.contains("scope 必须是 project"),
            "CLI 的原话要带出来：{note}"
        );
        assert_ne!(after, before);
    }

    #[test]
    fn the_cli_is_only_used_when_the_command_is_correctly_scoped() {
        let mut fx = fixture();
        let report = enable(&mut fx, channel(strict_scope_user));
        assert_eq!(report.method_used, crate::adapters::cli::MethodUsed::Cli);
        assert!(
            report.note.as_deref().unwrap().contains("mcp get 读回确认"),
            "{}",
            report.note.unwrap()
        );
        // CLI 路径不自己写文件，那份内容归 CLI 负责，我们只留底不改写
        assert!(!std::fs::read_to_string(&fx.target)
            .unwrap()
            .contains("playwright"));
    }

    #[test]
    fn read_back_reports_effective_only_when_the_cli_confirms_it() {
        let mut confirmed = fixture();
        enable(&mut confirmed, channel(succeeds));
        assert_eq!(
            state_of(&confirmed),
            crate::domain::AssetState::Effective,
            "CLI 写成功且读回确认 → 已生效"
        );

        let mut unconfirmed = fixture();
        let report = enable(&mut unconfirmed, channel(written_but_not_readable));
        assert_eq!(report.method_used, crate::adapters::cli::MethodUsed::Cli);
        assert!(
            report.note.as_deref().unwrap().contains("只算已写入"),
            "{}",
            report.note.unwrap()
        );
        assert_eq!(
            state_of(&unconfirmed),
            crate::domain::AssetState::Written,
            "读回没看到就不能说已生效"
        );
    }

    #[test]
    fn a_cli_write_is_backed_up_before_it_runs_and_can_be_reverted() {
        let mut fx = fixture();
        let before = std::fs::read_to_string(&fx.target).unwrap();
        let report = enable(&mut fx, channel(succeeds));

        let backed_up = std::fs::read_to_string(
            report
                .backup_dir
                .join("qoder-cn")
                .join("mcp")
                .join("playwright")
                .join("settings.json"),
        )
        .expect("CLI 路径也必须留底");
        assert_eq!(backed_up, before, "留底必须是 CLI 动手之前的原文");

        ApplyService::revert_last(&mut fx.index).unwrap();
        assert_eq!(std::fs::read_to_string(&fx.target).unwrap(), before);
    }

    #[test]
    fn an_unavailable_cli_says_why_it_went_the_file_way() {
        let mut fx = fixture();
        let report = enable(&mut fx, CliChannel::Unavailable);
        assert_eq!(report.method_used, crate::adapters::cli::MethodUsed::File);
        assert!(
            report
                .note
                .as_deref()
                .unwrap()
                .contains("没找到 qoderclicn"),
            "{}",
            report.note.unwrap()
        );
    }

    #[test]
    fn agents_without_a_cli_channel_need_no_explanation() {
        assert!(
            matches!(CliChannel::for_agent("claude"), CliChannel::NotApplicable),
            "Claude 没有官方写命令，不该为它去翻 PATH"
        );
        let mut fx = fixture();
        let plan = plan_enable(
            &agent("claude"),
            &server("playwright"),
            &Secrets::default(),
            &fx.home,
            None,
        )
        .unwrap();
        let report = ApplyService::apply(&fx.store, &mut fx.index, &[plan]).unwrap();
        assert_eq!(report.note, None, "不适用这条路径时不需要借口");
    }

    #[test]
    fn a_removal_the_cli_claims_but_still_reads_back_is_verified_by_file() {
        let mut fx = fixture();
        let plan = plan_enable(
            &agent("qoder-cn"),
            &server("playwright"),
            &Secrets::default(),
            &fx.home,
            None,
        )
        .unwrap();
        ApplyService::apply(&fx.store, &mut fx.index, &[plan]).unwrap();
        assert!(std::fs::read_to_string(&fx.target)
            .unwrap()
            .contains("playwright"));

        // fake CLI 报删除成功，但 mcp get 仍读得到（succeeds 对所有调用都返回 Ok）
        let target = WriteTarget {
            home: &fx.home,
            base_override: None,
            channel: channel(succeeds),
        };
        let report = write_disable(
            &fx.store,
            &mut fx.index,
            &agent("qoder-cn"),
            "playwright",
            &target,
        )
        .unwrap();
        assert_eq!(report.method_used, crate::adapters::cli::MethodUsed::File);
        assert!(
            report.note.as_deref().unwrap().contains("仍读得到"),
            "{}",
            report.note.unwrap()
        );
        let after = std::fs::read_to_string(&fx.target).unwrap();
        assert!(
            !after.contains("playwright"),
            "回落之后必须真的删掉：{after}"
        );
        assert!(after.contains("theirs"), "用户那一条不能跟着丢：{after}");
    }
}
