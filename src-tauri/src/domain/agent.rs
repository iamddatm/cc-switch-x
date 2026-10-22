use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub mcp: bool,
    pub skills: bool,
    pub prompts: bool,
    pub rules: bool,
    pub subagents: bool,
    pub hooks: bool,
    pub sessions: bool,
}

/// MCP 配置落在哪种文件里。Claude 的用户级 MCP 在 `~/.claude.json`，
/// 它是 `~/.claude/` 的**同级文件**而不是配置根里的文件，所以路径一律写完整的 `~/...`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum McpTarget {
    JsonRoot { file: String, key: String },
    TomlTable { file: String, table: String },
}

/// Hooks 一律是「某个 JSON 文件里的某个子树」；Codex 用的是独立的 hooks.json 而非 settings.json。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum HooksTarget {
    Json { file: String, key: String },
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPaths {
    pub mcp: McpTarget,
    pub skills_dir: String,
    pub prompt_file: String,
    /// None 表示该 agent 没有可映射的 markdown 规则目录（例如 Codex 的 rules 是执行策略 DSL）。
    pub rules_dir: Option<String>,
    pub agents_dir: Option<String>,
    pub hooks: HooksTarget,
}

impl AgentPaths {
    pub fn mcp_file(&self) -> &str {
        match &self.mcp {
            McpTarget::JsonRoot { file, .. } | McpTarget::TomlTable { file, .. } => file,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub display_name: String,
    pub config_root: String,
    pub supports: AgentCapabilities,
    pub paths: AgentPaths,
    /// None = 未实测。UI 按保守处理（提示需要重启），不得当成「不用重启」。
    pub mcp_needs_restart: Option<bool>,
    /// false = 落点或读取方式未经实测，UI 隐藏开关（Plan 1 全局约束）。
    pub verified: bool,
}

fn declared(root: &str, rest: &str) -> String {
    format!("{root}/{rest}")
}

/// cc-switch 的 `enabled_<app>` 列里，哪些 app 是我们注册表里认识的 agent。
pub fn is_known_agent(app: &str) -> bool {
    builtin_agents().iter().any(|agent| agent.id == app)
}

pub fn builtin_agents() -> Vec<AgentDescriptor> {
    let all = |sessions: bool| AgentCapabilities {
        mcp: true,
        skills: true,
        prompts: true,
        rules: true,
        subagents: true,
        hooks: true,
        sessions,
    };
    vec![
        AgentDescriptor {
            id: "claude".into(),
            display_name: "Claude Code".into(),
            config_root: "~/.claude".into(),
            supports: all(true),
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: "~/.claude.json".into(),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.claude", "skills"),
                prompt_file: declared("~/.claude", "CLAUDE.md"),
                // 本机不存在 ~/.claude/rules，官方落点未确认，Plan 3 实测后再开
                rules_dir: None,
                agents_dir: Some(declared("~/.claude", "agents")),
                hooks: HooksTarget::Json {
                    file: declared("~/.claude", "settings.json"),
                    key: "hooks".into(),
                },
            },
            mcp_needs_restart: None,
            verified: true,
        },
        AgentDescriptor {
            id: "codex".into(),
            display_name: "Codex".into(),
            config_root: "~/.codex".into(),
            supports: AgentCapabilities {
                rules: false, // rules/default.rules 是命令执行策略 DSL，不是 markdown 规则
                ..all(true)
            },
            paths: AgentPaths {
                mcp: McpTarget::TomlTable {
                    file: declared("~/.codex", "config.toml"),
                    table: "mcp_servers".into(),
                },
                skills_dir: declared("~/.codex", "skills"),
                prompt_file: declared("~/.codex", "AGENTS.md"),
                rules_dir: None,
                agents_dir: Some(declared("~/.codex", "agents")),
                hooks: HooksTarget::Json {
                    file: declared("~/.codex", "hooks.json"),
                    key: "hooks".into(),
                },
            },
            mcp_needs_restart: None,
            verified: true,
        },
        AgentDescriptor {
            id: "gemini".into(),
            display_name: "Gemini CLI".into(),
            config_root: "~/.gemini".into(),
            supports: AgentCapabilities {
                rules: false,
                subagents: false,
                hooks: false,
                sessions: false,
                ..all(false)
            },
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: declared("~/.gemini", "settings.json"),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.gemini", "skills"),
                prompt_file: declared("~/.gemini", "GEMINI.md"),
                rules_dir: None,
                agents_dir: None,
                hooks: HooksTarget::Unsupported,
            },
            mcp_needs_restart: None,
            verified: true,
        },
        AgentDescriptor {
            id: "qoder".into(),
            display_name: "Qoder".into(),
            config_root: "~/.qoder".into(),
            supports: all(false),
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: declared("~/.qoder", "mcp.json"),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.qoder", "skills"),
                prompt_file: declared("~/.qoder", "AGENTS.md"),
                rules_dir: None,
                agents_dir: None,
                hooks: HooksTarget::Json {
                    file: declared("~/.qoder", "settings.json"),
                    key: "hooks".into(),
                },
            },
            mcp_needs_restart: None,
            // spec A2b：IDE 究竟读 mcp.json 还是 settings.json 的 mcpServers 未实测
            verified: false,
        },
        AgentDescriptor {
            id: "pi".into(),
            display_name: "Pi Coding Agent".into(),
            // 证据：pi 自带 docs/configuration.md（agent dir = ~/.pi/agent）
            // 与 dist/extensions/mcp/config.js（读 <agentDir>/mcp.json 的 mcpServers）。
            // 提示词落点已用 sha1 与 cc-switch 的 pi 行比对一致。
            config_root: "~/.pi/agent".into(),
            supports: AgentCapabilities {
                mcp: true,
                skills: true,
                prompts: true,
                // pi 没有 markdown 规则目录
                rules: false,
                // pi 的子代理是 TS extension，不是声明式资产
                subagents: false,
                hooks: false,
                sessions: true,
            },
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: "~/.pi/agent/mcp.json".into(),
                    key: "mcpServers".into(),
                },
                skills_dir: "~/.pi/agent/skills".into(),
                prompt_file: "~/.pi/agent/AGENTS.md".into(),
                rules_dir: None,
                agents_dir: None,
                hooks: HooksTarget::Unsupported,
            },
            mcp_needs_restart: None,
            verified: true,
        },
        AgentDescriptor {
            id: "qoder-cn".into(),
            display_name: "Qoder CN".into(),
            config_root: "~/.qoder-cn".into(),
            supports: all(true),
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: declared("~/.qoder-cn", "settings.json"),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.qoder-cn", "skills"),
                prompt_file: declared("~/.qoder-cn", "AGENTS.md"),
                rules_dir: Some(declared("~/.qoder-cn", "rules")),
                agents_dir: Some(declared("~/.qoder-cn", "agents")),
                hooks: HooksTarget::Json {
                    file: declared("~/.qoder-cn", "settings.json"),
                    key: "hooks".into(),
                },
            },
            mcp_needs_restart: Some(true),
            verified: true,
        },
    ]
}
