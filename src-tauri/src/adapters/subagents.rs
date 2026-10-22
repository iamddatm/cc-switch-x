//! 子代理按 agent 的落盘形态：canonical 真源是 YAML（`domain/subagent.rs`），
//! 渲染出去是各家自己的 `.md` / `.toml`，反向解析时非公共字段收进 `tool_extras.<agent>`。
//!
//! 映射对齐 teamai-cli 的 `agent-format.ts`（MIT，见 THIRD_PARTY_NOTICES.md）：
//! 只借字段映射与「literal string」这两个已验证的机制，不复制其代码。

use super::{Deployed, Removed};
use crate::adapters::ledger;
use crate::adapters::paths::expand_root;
use crate::domain::agent::AgentDescriptor;
use crate::domain::frontmatter::split_frontmatter;
use crate::domain::subagent::{parse_subagent, Subagent};
use crate::domain::{AssetKind, AssetType, DomainError};
use crate::error::AppError;
use crate::index::{sha256_hex, Index};
use crate::services::apply::{Action, ApplyReport, ApplyService, PlanItem};
use crate::store::Store;
use serde_json::{Map, Value as JsonValue};
use std::path::{Path, PathBuf};

/// 渲染结果。只在适配器内部与测试之间流转：面板看到的是落盘后的实际文件，
/// 不是这份字节，所以不参与序列化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub extension: &'static str,
    pub content: String,
}

/// 该 agent 的子代理文件扩展名。没有 agents_dir 的 agent 走不到这里，
/// 但 codex 之外的 markdown 工具都用 `.md`。
pub fn extension_for(agent: &AgentDescriptor) -> &'static str {
    match agent.id.as_str() {
        "codex" => ".toml",
        _ => ".md",
    }
}

fn extras_for(agent: &AgentDescriptor, sub: &Subagent) -> JsonValue {
    sub.tool_extras
        .get(agent.id.as_str())
        .cloned()
        .unwrap_or(JsonValue::Null)
}

fn yaml_value(value: &JsonValue) -> Result<serde_yaml::Value, DomainError> {
    serde_yaml::to_value(value).map_err(|e| DomainError::Yaml(e.to_string()))
}

/// `.md`：frontmatter `name/description/model?/tools?` + 工具私有字段平铺，正文 = instructions。
fn render_markdown(agent: &AgentDescriptor, sub: &Subagent) -> Result<Rendered, DomainError> {
    let mut frontmatter = serde_yaml::Mapping::new();
    frontmatter.insert("name".into(), sub.name.clone().into());
    frontmatter.insert("description".into(), sub.description.clone().into());
    if let Some(model) = &sub.model {
        frontmatter.insert("model".into(), model.clone().into());
    }
    if !sub.tools.is_empty() {
        let tools = sub
            .tools
            .iter()
            .map(|tool| serde_yaml::Value::from(tool.clone()))
            .collect::<Vec<_>>();
        frontmatter.insert("tools".into(), serde_yaml::Value::from(tools));
    }
    merge_extras(&mut frontmatter, &extras_for(agent, sub))?;
    let yaml = serde_yaml::to_string(&frontmatter).map_err(|e| DomainError::Yaml(e.to_string()))?;
    Ok(Rendered {
        extension: extension_for(agent),
        content: format!("---\n{yaml}---\n\n{}\n", sub.instructions.trim()),
    })
}

fn merge_extras(
    frontmatter: &mut serde_yaml::Mapping,
    extras: &JsonValue,
) -> Result<(), DomainError> {
    let JsonValue::Object(map) = extras else {
        return Ok(());
    };
    for (key, value) in map {
        frontmatter.insert(serde_yaml::Value::from(key.clone()), yaml_value(value)?);
    }
    Ok(())
}

/// TOML 里没有 null：一个 null 落进 agent 配置文件比整项失败更难排查。
fn toml_value(value: &JsonValue) -> Result<toml_edit::Value, DomainError> {
    Ok(match value {
        JsonValue::Null => {
            return Err(DomainError::Yaml(format!(
                "{value} 不是合法的 TOML 值（TOML 没有 null）"
            )))
        }
        JsonValue::Bool(flag) => toml_edit::Value::from(*flag),
        JsonValue::Number(number) => {
            if let Some(int) = number.as_i64() {
                toml_edit::Value::from(int)
            } else if let Some(float) = number.as_f64() {
                toml_edit::Value::from(float)
            } else {
                return Err(DomainError::Yaml(format!("{number} 超出 TOML 数值范围")));
            }
        }
        JsonValue::String(text) => toml_edit::Value::from(text.as_str()),
        JsonValue::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(toml_value(item)?);
            }
            toml_edit::Value::from(array)
        }
        JsonValue::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, item) in map {
                table.insert(key, toml_value(item)?);
            }
            toml_edit::Value::from(table)
        }
    })
}

/// 能不能用多行字面量（`'''`）表达。
///
/// 字面量不做转义处理，所以内容必须自己不可能提前结束它：含 `'''`、以 `'` 结尾
/// （会补上分隔符）都不行；`\r`、NUL、DEL 这类 C0 控制字符（制表、换行除外）TOML
/// 规范也不允许出现在字面量里。C1（U+0080-U+009F）按规范是允许的。
fn can_use_toml_literal(value: &str) -> bool {
    if !value.contains('\n') || value.contains("'''") || value.ends_with('\'') {
        return false;
    }
    let before_tab_or_newline = |ch: char| ch != '\t' && ch != '\n';
    !value.chars().any(|ch| {
        ch.is_control() && before_tab_or_newline(ch) && !('\u{80}'..='\u{9f}').contains(&ch)
    })
}

/// 多行值走 `'''` 字面量的原因（照搬 teamai 踩过坑后的结论）：`toml_edit` 只会写基本
/// 字符串，16 行提示词会塌成一条几百字符的转义行，编辑器里读不了、`git diff` 里审不了。
/// 紧跟开界符的那个换行是定界符自己的、规范会吃掉，所以内容前补一个换行正好抵消。
fn toml_literal(value: &str) -> String {
    format!("'''\n{value}'''")
}

/// 公共字段按固定顺序落盘，extras 平铺在后面（同名键覆盖公共字段：真源写了什么就照什么落）。
fn ordered_entries(
    agent: &AgentDescriptor,
    sub: &Subagent,
) -> Result<Vec<(String, JsonValue)>, DomainError> {
    let mut entries: Vec<(String, JsonValue)> = vec![
        ("name".into(), JsonValue::String(sub.name.clone())),
        (
            "description".into(),
            JsonValue::String(sub.description.clone()),
        ),
        (
            "developer_instructions".into(),
            JsonValue::String(sub.instructions.trim().to_string()),
        ),
    ];
    if let Some(model) = &sub.model {
        entries.push(("model".into(), JsonValue::String(model.clone())));
    }
    if let JsonValue::Object(map) = extras_for(agent, sub) {
        entries.extend(map);
    }
    Ok(entries)
}

fn render_toml_document(entries: &[(String, JsonValue)]) -> Result<String, DomainError> {
    let mut document = toml_edit::DocumentMut::new();
    for (key, value) in entries {
        document.insert(key.as_str(), toml_edit::Item::Value(toml_value(value)?));
    }
    let text = document.to_string();

    // 再把合格的多行基本字符串换成字面量。`str::replace` 是字面替换，
    // 内容里的 `$` 不会被吃掉（JS 那边得改用函数替换才有同样的保证）。
    let mut substituted = text;
    for (key, value) in entries {
        let JsonValue::String(text_value) = value else {
            continue;
        };
        if !can_use_toml_literal(text_value) {
            continue;
        }
        let basic = toml_edit::Value::from(text_value.as_str()).to_string();
        let literal = toml_literal(text_value);
        substituted = substituted.replace(
            &format!("{key} = {basic}\n"),
            &format!("{key} = {literal}\n"),
        );
    }
    Ok(substituted)
}

/// `.toml`（Codex）：`name/description/developer_instructions/model?`，**故意不写 `tools`**
/// （Codex 用 `mcp_servers`，写进去只会误导）。
fn render_codex(agent: &AgentDescriptor, sub: &Subagent) -> Result<Rendered, DomainError> {
    Ok(Rendered {
        extension: ".toml",
        content: render_toml_document(&ordered_entries(agent, sub)?)?,
    })
}

pub fn render_subagent(agent: &AgentDescriptor, sub: &Subagent) -> Result<Rendered, DomainError> {
    if sub.name.trim().is_empty() {
        return Err(DomainError::EmptyName);
    }
    if sub.description.trim().is_empty() {
        return Err(DomainError::EmptyDescription);
    }
    if sub.instructions.trim().is_empty() {
        return Err(DomainError::EmptyInstructions);
    }
    match extension_for(agent) {
        ".toml" => render_codex(agent, sub),
        _ => render_markdown(agent, sub),
    }
}

/// 把一条 markdown frontmatter 里的值读成 JSON（canonical 的 tool_extras 是 JSON）。
fn json_from_yaml(value: &serde_yaml::Value) -> JsonValue {
    serde_json::to_value(value).unwrap_or(JsonValue::Null)
}

/// `.md` 反向解析。Qoder 的 `allowed-tools` / `allowed_tools` 归一成 `tools`，
/// 写出时只用 `tools`（Plan 3 不发明新键）。
fn reverse_markdown(
    agent: &AgentDescriptor,
    filename: &str,
    content: &str,
) -> Result<Subagent, DomainError> {
    let parts = split_frontmatter(content)?;
    let instructions = parts.body.trim().to_string();
    let Some(yaml) = parts.yaml else {
        return Err(DomainError::Yaml("子代理缺少 frontmatter".into()));
    };
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&yaml).map_err(|e| DomainError::Yaml(e.to_string()))?;
    let mapping = parsed
        .as_mapping()
        .cloned()
        .ok_or_else(|| DomainError::Yaml("子代理 frontmatter 不是映射".into()))?;

    let stem = filename.trim_end_matches(".md");
    let mut extras = Map::new();
    let mut tools: Vec<String> = Vec::new();
    let mut name = stem.to_string();
    let mut description = String::new();
    let mut model: Option<String> = None;
    for (key, value) in mapping {
        let Some(key) = key.as_str() else { continue };
        match key {
            "name" => name = value.as_str().unwrap_or(stem).to_string(),
            "description" => description = value.as_str().unwrap_or_default().to_string(),
            "model" => model = value.as_str().map(str::to_string),
            "tools" => tools = string_list(&value),
            // Qoder 侧两种写法都认，落到 canonical 只剩 `tools`
            "allowed-tools" | "allowed_tools" => tools = string_list(&value),
            _ => {
                extras.insert(key.to_string(), json_from_yaml(&value));
            }
        }
    }
    finish(agent, name, description, instructions, model, tools, extras)
}

fn reverse_toml(
    agent: &AgentDescriptor,
    filename: &str,
    content: &str,
) -> Result<Subagent, DomainError> {
    let document = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| DomainError::Yaml(format!("TOML 解析失败：{e}")))?;
    let stem = filename.trim_end_matches(".toml");
    let mut extras = Map::new();
    let mut name = stem.to_string();
    let mut description = String::new();
    let mut instructions = String::new();
    let mut model: Option<String> = None;
    for (key, item) in document.iter() {
        let Some(value) = item.as_value() else {
            continue;
        };
        match key {
            "name" => name = value.as_str().unwrap_or(stem).to_string(),
            "description" => description = value.as_str().unwrap_or_default().to_string(),
            "developer_instructions" => {
                instructions = value.as_str().unwrap_or_default().to_string()
            }
            "model" => model = value.as_str().map(str::to_string),
            // Codex 的 tools 不是公共字段：原样当私有字段带回去
            _ => {
                extras.insert(key.to_string(), json_from_toml_item(item));
            }
        }
    }
    // Codex 不读 tools，反向时也不该凭空造出一个列表
    finish(
        agent,
        name,
        description,
        instructions,
        model,
        Vec::new(),
        extras,
    )
}

fn json_from_toml(value: &toml_edit::Value) -> JsonValue {
    match value {
        toml_edit::Value::String(text) => JsonValue::String(text.value().clone()),
        toml_edit::Value::Integer(int) => JsonValue::from(*int.value()),
        toml_edit::Value::Float(float) => JsonValue::from(*float.value()),
        toml_edit::Value::Boolean(flag) => JsonValue::Bool(*flag.value()),
        toml_edit::Value::Datetime(at) => JsonValue::String(at.value().to_string()),
        toml_edit::Value::Array(items) => items.iter().map(json_from_toml).collect(),
        toml_edit::Value::InlineTable(table) => table
            .iter()
            .map(|(k, v)| (k.to_string(), json_from_toml(v)))
            .collect(),
    }
}

fn json_from_toml_item(item: &toml_edit::Item) -> JsonValue {
    match item {
        toml_edit::Item::Value(value) => json_from_toml(value),
        toml_edit::Item::Table(table) => table
            .iter()
            .map(|(k, v)| (k.to_string(), json_from_toml_item(v)))
            .collect(),
        toml_edit::Item::ArrayOfTables(tables) => tables
            .iter()
            .map(|table| {
                table
                    .iter()
                    .map(|(k, v)| (k.to_string(), json_from_toml_item(v)))
                    .collect::<JsonValue>()
            })
            .collect(),
        // 占位条目（`a = ` 之后被删空）没有值可带回去
        toml_edit::Item::None => JsonValue::Null,
    }
}

fn string_list(value: &serde_yaml::Value) -> Vec<String> {
    match value {
        serde_yaml::Value::Sequence(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        // `tools: Read, Grep` 这种逗号写法各家都当列表读
        serde_yaml::Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn finish(
    agent: &AgentDescriptor,
    name: String,
    description: String,
    instructions: String,
    model: Option<String>,
    tools: Vec<String>,
    extras: Map<String, JsonValue>,
) -> Result<Subagent, DomainError> {
    if name.trim().is_empty() {
        return Err(DomainError::EmptyName);
    }
    if description.trim().is_empty() {
        return Err(DomainError::EmptyDescription);
    }
    if instructions.trim().is_empty() {
        return Err(DomainError::EmptyInstructions);
    }
    let mut tool_extras = Map::new();
    if !extras.is_empty() {
        tool_extras.insert(agent.id.clone(), JsonValue::Object(extras));
    }
    Ok(Subagent {
        name,
        description,
        instructions,
        model,
        tools,
        tool_extras: JsonValue::Object(tool_extras),
        targets: Vec::new(),
    })
}

pub fn reverse_subagent(
    agent: &AgentDescriptor,
    filename: &str,
    content: &str,
) -> Result<Subagent, DomainError> {
    match extension_for(agent) {
        ".toml" => reverse_toml(agent, filename, content),
        _ => reverse_markdown(agent, filename, content),
    }
}

// ─── 落盘与卸载 ─────────────────────────────────────────────

/// 子代理文件里没有归属标记（正文是 agent 要读的提示词），归属只走
/// `ledger::bytes_are_ours` 那两条依据。渲染格式若改动，递增版本并把旧形态
/// 加进 `previous_renders`（rules 那边已有先例）。
const RENDERER_VERSION: u32 = 1;

/// 子代理的热加载语义没有实测依据，按「要重启」处理。
const NEEDS_RESTART: bool = true;

fn declared_agents_dir(agent: &AgentDescriptor) -> Result<&str, AppError> {
    agent
        .paths
        .agents_dir
        .as_deref()
        .ok_or_else(|| AppError::Unsupported(format!("{} 没有子代理落点", agent.id)))
}

pub fn agents_dir(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    Ok(expand_root(
        declared_agents_dir(agent)?,
        base_override,
        home,
    ))
}

/// 落盘文件名用真源 ID，扩展名按 agent 的形状（codex 是 `.toml`）。
pub fn target_path(
    agent: &AgentDescriptor,
    subagent_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    Ok(agents_dir(agent, home, base_override)?
        .join(format!("{subagent_id}{}", extension_for(agent))))
}

fn plan_item(
    agent: &AgentDescriptor,
    subagent_id: &str,
    target_path: PathBuf,
    contents: Vec<u8>,
    action: Action,
) -> PlanItem {
    let marker = match action {
        Action::Remove => '-',
        _ => '+',
    };
    PlanItem {
        agent_id: agent.id.clone(),
        asset_type: AssetType::Subagent,
        asset_id: subagent_id.to_string(),
        action,
        target_path,
        contents,
        diff: vec![format!(
            "{marker} {}/{}",
            declared_agents_dir(agent).unwrap_or("agents"),
            subagent_id
        )],
        needs_restart: NEEDS_RESTART,
    }
}

/// 真源可能已被删除或改坏；那种情况下归属只认台账。
fn read_canonical(store: &Store, subagent_id: &str) -> Option<Subagent> {
    let bytes = store.read(AssetType::Subagent, subagent_id).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    parse_subagent(&text).ok()
}

fn is_ours(
    store: &Store,
    agent: &AgentDescriptor,
    subagent_id: &str,
    path: &Path,
    current: &[u8],
) -> Result<bool, AppError> {
    let candidates: Vec<Result<String, DomainError>> = match read_canonical(store, subagent_id) {
        Some(sub) => vec![render_subagent(agent, &sub).map(|rendered| rendered.content)],
        None => Vec::new(),
    };
    ledger::bytes_are_ours(
        store,
        &agent.id,
        AssetKind::Subagent,
        subagent_id,
        path,
        current,
        candidates,
    )
}

fn read_if_exists(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    if path.exists() {
        Ok(Some(std::fs::read(path)?))
    } else {
        Ok(None)
    }
}

fn deploy_inner(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    subagent_id: &str,
    home: &Path,
    base_override: Option<&Path>,
    force: bool,
) -> Result<Option<ApplyReport>, AppError> {
    let canonical = store.read(AssetType::Subagent, subagent_id)?;
    let text = String::from_utf8(canonical)
        .map_err(|_| AppError::InvalidInput("真源里的子代理不是 UTF-8".into()))?;
    let sub = parse_subagent(&text)?;
    let rendered = render_subagent(agent, &sub)?;
    let bytes = rendered.content.into_bytes();
    let path = target_path(agent, subagent_id, home, base_override)?;

    if !force {
        if let Some(current) = read_if_exists(&path)? {
            if !is_ours(store, agent, subagent_id, &path, &current)? {
                return Ok(None);
            }
        }
    }
    let report = ApplyService::apply(
        store,
        index,
        &[plan_item(
            agent,
            subagent_id,
            path.clone(),
            bytes.clone(),
            Action::Enable,
        )],
    )?;
    ledger::record(
        store,
        &agent.id,
        AssetKind::Subagent,
        subagent_id,
        &path,
        &sha256_hex(&bytes),
        RENDERER_VERSION,
    )?;
    Ok(Some(report))
}

pub fn deploy(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    subagent_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Deployed, AppError> {
    Ok(
        match deploy_inner(store, index, agent, subagent_id, home, base_override, false)? {
            Some(report) => Deployed::Written(report),
            None => Deployed::Foreign,
        },
    )
}

/// 「重刷真源」：用户已经看过漂移并选了这一项，所以不再被 Foreign 拦住。
pub fn repush(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    subagent_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ApplyReport, AppError> {
    deploy_inner(store, index, agent, subagent_id, home, base_override, true)?.ok_or_else(|| {
        AppError::InvalidInput(format!("{subagent_id} 的落盘目标无法处理，未做任何改动"))
    })
}

/// 取消落盘。内置的与用户自己写的子代理都不在台账里，一律不动。
pub fn remove(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    subagent_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Removed, AppError> {
    let path = target_path(agent, subagent_id, home, base_override)?;
    let Some(current) = read_if_exists(&path)? else {
        ledger::forget(store, &agent.id, AssetKind::Subagent, subagent_id)?;
        return Ok(Removed::Absent);
    };
    if !is_ours(store, agent, subagent_id, &path, &current)? {
        return Ok(Removed::Foreign);
    }
    ApplyService::apply(
        store,
        index,
        &[plan_item(
            agent,
            subagent_id,
            path,
            Vec::new(),
            Action::Remove,
        )],
    )?;
    ledger::forget(store, &agent.id, AssetKind::Subagent, subagent_id)?;
    Ok(Removed::Ours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use crate::domain::subagent::{parse_subagent, render_canonical};

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    fn spec() -> Subagent {
        Subagent {
            name: "reviewer".into(),
            description: "代码审查".into(),
            instructions: "正文最后一行".into(),
            model: None,
            tools: vec!["Read".into(), "Grep".into()],
            tool_extras: JsonValue::Object(Map::new()),
            targets: Vec::new(),
        }
    }

    #[test]
    fn claude_and_qoder_render_markdown_frontmatter() {
        for id in ["claude", "qoder", "qoder-cn"] {
            let rendered = render_subagent(&agent(id), &spec()).unwrap();
            assert_eq!(rendered.extension, ".md", "{id}");
            assert!(
                rendered
                    .content
                    .starts_with("---\nname: reviewer\ndescription: 代码审查\n"),
                "{id} 渲染结果：{}",
                rendered.content
            );
            // serde_yaml 用 libyaml 的规范风格（序列不额外缩进），仍是合法 YAML，
            // 两家都用 YAML 解析器读 frontmatter；字节风格只与自家渲染比对
            assert!(
                rendered.content.contains("tools:\n- Read\n- Grep"),
                "{}",
                rendered.content
            );
            assert!(rendered.content.trim_end().ends_with("正文最后一行"));
        }
    }

    #[test]
    fn codex_renames_instructions_and_drops_tools() {
        let rendered = render_subagent(&agent("codex"), &spec()).unwrap();
        assert_eq!(rendered.extension, ".toml");
        assert!(
            rendered.content.contains("name = \"reviewer\""),
            "{}",
            rendered.content
        );
        assert!(
            rendered
                .content
                .contains("developer_instructions = \"正文最后一行\""),
            "单行值没什么可读性诉求，基本字符串即可：{}",
            rendered.content
        );
        assert!(
            !rendered.content.contains("tools"),
            "Codex 不认 tools，写进去只会误导：{}",
            rendered.content
        );
        assert_eq!(
            reverse_subagent(&agent("codex"), "reviewer.toml", &rendered.content)
                .unwrap()
                .instructions,
            "正文最后一行"
        );
    }

    #[test]
    fn a_multiline_prompt_survives_the_toml_round_trip_byte_for_byte() {
        let mut spec = spec();
        spec.instructions = "行1\n行2 含 ' 单引号\n\t缩进行\n最后一行不带换行".to_string();
        let rendered = render_subagent(&agent("codex"), &spec).unwrap();
        assert!(
            rendered.content.contains("developer_instructions = '''\n"),
            "多行提示词要走字面量：{}",
            rendered.content
        );
        assert_eq!(
            reverse_subagent(&agent("codex"), "a.toml", &rendered.content)
                .unwrap()
                .instructions,
            spec.instructions
        );
    }

    #[test]
    fn a_value_that_cannot_use_a_literal_still_round_trips() {
        let base = spec();

        // 含 ''' 的内容会把字面量提前关掉
        let mut quote = base.clone();
        quote.instructions = "行1\n分隔符 ''' 出现在正文里\n行3".to_string();
        let rendered = render_subagent(&agent("codex"), &quote).unwrap();
        assert!(
            !rendered.content.contains("developer_instructions = '''\n"),
            "这种内容不能走字面量：{}",
            rendered.content
        );
        assert_eq!(
            reverse_subagent(&agent("codex"), "a.toml", &rendered.content)
                .unwrap()
                .instructions,
            quote.instructions,
            "换用其他 TOML 字符串形式也必须逐字节还原"
        );

        // 以单引号结尾：紧跟的 ' 会关掉分隔符
        let mut trailing = base.clone();
        trailing.instructions = "多行\n以单引号结束'".to_string();
        let rendered = render_subagent(&agent("codex"), &trailing).unwrap();
        assert_eq!(
            reverse_subagent(&agent("codex"), "a.toml", &rendered.content)
                .unwrap()
                .instructions,
            trailing.instructions
        );

        // \r 是字面量装不下的控制字符
        let mut carriage = base.clone();
        carriage.instructions = "行1\r\n行2\n".to_string();
        let rendered = render_subagent(&agent("codex"), &carriage).unwrap();
        assert_eq!(
            reverse_subagent(&agent("codex"), "a.toml", &rendered.content)
                .unwrap()
                .instructions,
            "行1\r\n行2",
            "{}",
            rendered.content
        );
    }

    #[test]
    fn tool_private_fields_land_in_tool_extras_and_come_back() {
        let file = "---\nname: r\ndescription: d\nwhen-to-use: 需要审查时\n---\n\n正文\n";
        let reversed = reverse_subagent(&agent("qoder-cn"), "r.md", file).unwrap();
        assert_eq!(
            reversed.tool_extras["qoder-cn"]["when-to-use"],
            "需要审查时"
        );
        let rendered = render_subagent(&agent("qoder-cn"), &reversed).unwrap();
        assert!(
            rendered.content.contains("when-to-use: 需要审查时"),
            "{}",
            rendered.content
        );
        // 私有字段属于那一家，别家渲染时不该带出去
        let other = render_subagent(&agent("claude"), &reversed).unwrap();
        assert!(!other.content.contains("when-to-use"), "{}", other.content);
    }

    #[test]
    fn qoder_allowed_tools_variants_normalise_to_tools() {
        for key in ["allowed-tools", "allowed_tools"] {
            let file =
                format!("---\nname: r\ndescription: d\n{key}:\n- Read\n- Grep\n---\n\n正文\n");
            let reversed = reverse_subagent(&agent("qoder-cn"), "r.md", &file).unwrap();
            assert_eq!(
                reversed.tools,
                vec!["Read".to_string(), "Grep".to_string()],
                "{key}"
            );
            let rendered = render_subagent(&agent("qoder-cn"), &reversed).unwrap();
            assert!(
                rendered.content.contains("tools:\n- Read"),
                "{}",
                rendered.content
            );
            assert!(
                !rendered.content.contains(key),
                "写出时只用 tools，不发明新键"
            );
        }
    }

    #[test]
    fn codex_private_fields_round_trip_through_tool_extras() {
        let mut spec = spec();
        let mut extras = Map::new();
        extras.insert(
            "model_reasoning_effort".into(),
            JsonValue::String("high".into()),
        );
        extras.insert("sandbox_mode".into(), JsonValue::String("read-only".into()));
        spec.tool_extras = JsonValue::Object(Map::from_iter([(
            "codex".to_string(),
            JsonValue::Object(extras),
        )]));
        spec.tools = Vec::new();

        let rendered = render_subagent(&agent("codex"), &spec).unwrap();
        assert!(
            rendered
                .content
                .contains("model_reasoning_effort = \"high\""),
            "{}",
            rendered.content
        );
        let reversed =
            reverse_subagent(&agent("codex"), "reviewer.toml", &rendered.content).unwrap();
        assert_eq!(
            reversed.tool_extras["codex"]["model_reasoning_effort"],
            "high"
        );
        assert_eq!(reversed.tool_extras["codex"]["sandbox_mode"], "read-only");
        assert!(reversed.tools.is_empty(), "Codex 那边没有 tools 概念");
    }

    #[test]
    fn a_missing_frontmatter_field_is_named_not_defaulted() {
        let no_description = "---\nname: r\n---\n\n正文\n";
        assert_eq!(
            reverse_subagent(&agent("claude"), "r.md", no_description).unwrap_err(),
            DomainError::EmptyDescription
        );
        let no_body = "---\nname: r\ndescription: d\n---\n\n";
        assert_eq!(
            reverse_subagent(&agent("claude"), "r.md", no_body).unwrap_err(),
            DomainError::EmptyInstructions
        );
        let no_name = "---\ndescription: d\n---\n\n正文\n";
        assert_eq!(
            reverse_subagent(&agent("claude"), "r.md", no_name).unwrap(),
            {
                let mut s = spec();
                s.name = "r".into();
                s.description = "d".into();
                s.instructions = "正文".into();
                s.tools = Vec::new();
                s
            },
            "没有 name 时用文件名兜底（各家都这么认）"
        );
    }

    #[test]
    fn rendering_refuses_an_empty_canonical_field() {
        let mut spec = spec();
        spec.description = "  ".into();
        assert_eq!(
            render_subagent(&agent("claude"), &spec).unwrap_err(),
            DomainError::EmptyDescription
        );
        spec.description = "d".into();
        spec.instructions = "\n  \n".into();
        assert_eq!(
            render_subagent(&agent("claude"), &spec).unwrap_err(),
            DomainError::EmptyInstructions
        );
    }

    #[test]
    fn a_null_in_extras_fails_instead_of_dropping_the_key() {
        let mut spec = spec();
        spec.tools = Vec::new();
        let mut extras = Map::new();
        extras.insert("effort".into(), JsonValue::Null);
        spec.tool_extras = JsonValue::Object(Map::from_iter([(
            "codex".to_string(),
            JsonValue::Object(extras),
        )]));
        let error = render_subagent(&agent("codex"), &spec).unwrap_err();
        assert!(
            matches!(error, DomainError::Yaml(_)),
            "TOML 没有 null，静默丢键等于改掉用户写的东西：{error}"
        );
    }

    #[test]
    fn the_canonical_yaml_round_trips_through_parse_and_render() {
        let canonical = render_canonical(&spec()).unwrap();
        let parsed = parse_subagent(&canonical).unwrap();
        assert_eq!(parsed, spec());
        let rendered = render_subagent(&agent("claude"), &parsed).unwrap();
        assert!(rendered.content.contains("name: reviewer"));
    }
}

#[cfg(test)]
mod deploy_tests {
    use super::{deploy, remove, target_path};
    use crate::adapters::ledger;
    use crate::adapters::{Deployed, Removed};
    use crate::domain::agent::{builtin_agents, AgentDescriptor};
    use crate::domain::subagent::{parse_subagent, render_canonical, Subagent};
    use crate::domain::{AssetKind, AssetType};
    use crate::index::Index;
    use crate::store::Store;
    use serde_json::{Map, Value as JsonValue};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn spec() -> Subagent {
        Subagent {
            name: "reviewer".into(),
            description: "代码审查".into(),
            instructions: "先看 diff 再看测试".into(),
            model: None,
            tools: vec!["Read".into(), "Grep".into()],
            tool_extras: JsonValue::Object(Map::new()),
            targets: vec!["claude".into()],
        }
    }

    struct Fixture {
        /// 只在 Drop 时保住临时目录。
        _dir: TempDir,
        store: Store,
        index: Index,
        home: PathBuf,
        agent: AgentDescriptor,
    }

    fn fixture(id: &str) -> Fixture {
        let dir = TempDir::new().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let agent = builtin_agents().into_iter().find(|a| a.id == id).unwrap();
        Fixture {
            _dir: dir,
            store,
            index,
            home,
            agent,
        }
    }

    impl Fixture {
        fn target(&self) -> PathBuf {
            target_path(&self.agent, "reviewer", &self.home, None).unwrap()
        }

        fn agents_dir(&self) -> PathBuf {
            self.target().parent().unwrap().to_path_buf()
        }

        fn seed(&self) {
            self.store
                .write(
                    AssetType::Subagent,
                    "reviewer",
                    render_canonical(&spec()).unwrap().as_bytes(),
                )
                .unwrap();
        }

        fn write_foreign(&self, name: &str, text: &str) {
            std::fs::create_dir_all(self.agents_dir()).unwrap();
            std::fs::write(self.agents_dir().join(name), text).unwrap();
        }
    }

    #[test]
    fn deploy_uses_the_tool_extension() {
        let mut markdown = fixture("qoder-cn");
        markdown.seed();
        deploy(
            &markdown.store,
            &mut markdown.index,
            &markdown.agent,
            "reviewer",
            &markdown.home,
            None,
        )
        .unwrap();
        assert!(
            markdown.target().ends_with("reviewer.md"),
            "{:?}",
            markdown.target()
        );
        let written = std::fs::read_to_string(markdown.target()).unwrap();
        assert!(written.contains("先看 diff 再看测试"), "{written}");

        let mut toml = fixture("codex");
        toml.seed();
        deploy(
            &toml.store,
            &mut toml.index,
            &toml.agent,
            "reviewer",
            &toml.home,
            None,
        )
        .unwrap();
        assert!(
            toml.target().ends_with("reviewer.toml"),
            "{:?}",
            toml.target()
        );
        assert!(
            std::fs::read_to_string(toml.target())
                .unwrap()
                .contains("developer_instructions"),
            "Codex 那边必须是 TOML"
        );
    }

    #[test]
    fn removing_a_subagent_keeps_the_builtin_list() {
        let mut fx = fixture("claude");
        fx.seed();
        // 内置的与用户自己写的子代理都不在我们台账里
        fx.write_foreign(
            "helper.md",
            "---\nname: helper\ndescription: 内置\n---\n\n别人的提示词\n",
        );

        deploy(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            "reviewer",
            &fx.home,
            None,
        )
        .unwrap();
        assert_eq!(
            remove(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "reviewer",
                &fx.home,
                None
            )
            .unwrap(),
            Removed::Ours
        );
        assert!(!fx.target().exists());
        let untouched = fx.agents_dir().join("helper.md");
        assert!(untouched.is_file(), "取消一个资产不能连带清掉别人的文件");
        assert!(std::fs::read_to_string(untouched)
            .unwrap()
            .contains("别人的提示词"));
    }

    #[test]
    fn a_foreign_file_with_the_same_name_is_never_touched() {
        let mut fx = fixture("claude");
        fx.seed();
        fx.write_foreign(
            "reviewer.md",
            "---\nname: reviewer\ndescription: 用户自己写的\n---\n\n用户的提示词\n",
        );

        assert_eq!(
            deploy(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "reviewer",
                &fx.home,
                None
            )
            .unwrap(),
            Deployed::Foreign
        );
        assert_eq!(
            remove(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "reviewer",
                &fx.home,
                None
            )
            .unwrap(),
            Removed::Foreign
        );
        assert!(std::fs::read_to_string(fx.target())
            .unwrap()
            .contains("用户的提示词"));
    }

    #[test]
    fn a_hand_edited_copy_is_neither_redeployable_nor_removable() {
        let mut fx = fixture("qoder-cn");
        fx.seed();
        deploy(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            "reviewer",
            &fx.home,
            None,
        )
        .unwrap();
        std::fs::write(fx.target(), "用户在 Qoder 那边改了提示词\n").unwrap();

        assert_eq!(
            deploy(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "reviewer",
                &fx.home,
                None
            )
            .unwrap(),
            Deployed::Foreign,
            "改过的内容要留给漂移报告处置，不能在这里静默覆盖"
        );
        assert_eq!(
            remove(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "reviewer",
                &fx.home,
                None
            )
            .unwrap(),
            Removed::Foreign
        );
    }

    #[test]
    fn deploy_records_the_ledger_and_marks_the_state_written() {
        let mut fx = fixture("codex");
        fx.seed();
        deploy(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            "reviewer",
            &fx.home,
            None,
        )
        .unwrap();

        let table = ledger::load(&fx.store, "codex").unwrap();
        let entry = table.get(AssetKind::Subagent, "reviewer").unwrap();
        assert_eq!(entry.target_path, fx.target());
        assert_eq!(entry.rendered_by, 1);
        let row = fx
            .index
            .agent_states()
            .unwrap()
            .into_iter()
            .find(|row| row.asset_type == AssetType::Subagent)
            .unwrap();
        assert_eq!(row.state, crate::domain::AssetState::Written);

        // 改真源再落一次：自家文件可以替换
        let mut edited = spec();
        edited.instructions = "改过的指令".into();
        fx.store
            .write(
                AssetType::Subagent,
                "reviewer",
                render_canonical(&edited).unwrap().as_bytes(),
            )
            .unwrap();
        assert!(matches!(
            deploy(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "reviewer",
                &fx.home,
                None
            )
            .unwrap(),
            Deployed::Written(_)
        ));
        assert!(std::fs::read_to_string(fx.target())
            .unwrap()
            .contains("改过的指令"));
    }

    #[test]
    fn an_agent_without_an_agents_target_is_refused() {
        let mut fx = fixture("gemini");
        fx.seed();
        let error = deploy(
            &fx.store,
            &mut fx.index,
            &fx.agent,
            "reviewer",
            &fx.home,
            None,
        );
        assert!(matches!(
            error.unwrap_err(),
            crate::error::AppError::Unsupported(_)
        ));
    }

    #[test]
    fn the_canonical_targets_field_survives_a_render() {
        // targets 只存在于真源侧，渲染与反推都不能它弄丢（扫描那边有专门的断言）
        let parsed = parse_subagent(&render_canonical(&spec()).unwrap()).unwrap();
        assert_eq!(parsed.targets, vec!["claude".to_string()]);
    }
}
