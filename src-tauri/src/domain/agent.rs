use super::AssetType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 能力表的键。与 `AssetType` 一一对应，另外多一个 Session —— 会话不是真源资产，
/// 但也要在能力表里占一格，否则前端得为它单独开一条通路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Mcp,
    Skill,
    Prompt,
    Rule,
    Subagent,
    Hook,
    Session,
}

impl AssetKind {
    pub const ALL: [AssetKind; 7] = [
        Self::Mcp,
        Self::Skill,
        Self::Prompt,
        Self::Rule,
        Self::Subagent,
        Self::Hook,
        Self::Session,
    ];

    pub fn from_asset_type(asset_type: AssetType) -> Self {
        match asset_type {
            AssetType::Mcp => Self::Mcp,
            AssetType::Skill => Self::Skill,
            AssetType::Prompt => Self::Prompt,
            AssetType::Rule => Self::Rule,
            AssetType::Subagent => Self::Subagent,
            AssetType::Hook => Self::Hook,
        }
    }

    pub fn try_as_asset_type(self) -> Result<AssetType, Self> {
        match self {
            Self::Mcp => Ok(AssetType::Mcp),
            Self::Skill => Ok(AssetType::Skill),
            Self::Prompt => Ok(AssetType::Prompt),
            Self::Rule => Ok(AssetType::Rule),
            Self::Subagent => Ok(AssetType::Subagent),
            Self::Hook => Ok(AssetType::Hook),
            Self::Session => Err(self),
        }
    }

    /// 真源目录名；Session 不落真源，所以是 None。
    pub fn dir_name(self) -> Option<&'static str> {
        self.try_as_asset_type().ok().map(AssetType::dir_name)
    }

    /// 台账与前端用的稳定 snake 名。Session 也有一格，不能用 `dir_name()` 代替。
    pub fn snake_name(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skill => "skill",
            Self::Prompt => "prompt",
            Self::Rule => "rule",
            Self::Subagent => "subagent",
            Self::Hook => "hook",
            Self::Session => "session",
        }
    }

    /// 台账里存的 `assetType` 读回能力。认不出来就是数据坏了，必须显式失败。
    pub fn from_snake_name(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.snake_name() == value)
    }
}

/// 某类资产在某个 agent 上的状态。两个维度必须分开：
/// `supported=false` 是该 agent 根本没有这个概念（如 Codex 的 rules 是执行策略 DSL）；
/// `supported=true, verified=false` 是落点存在但我们没真机确证，UI 只说明、不开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSupport {
    pub supported: bool,
    pub verified: bool,
}

impl AssetSupport {
    pub const OFF: Self = Self {
        supported: false,
        verified: false,
    };
    pub const UNVERIFIED: Self = Self {
        supported: true,
        verified: false,
    };
    pub const VERIFIED: Self = Self {
        supported: true,
        verified: true,
    };
}

pub type SupportMap = BTreeMap<AssetKind, AssetSupport>;

/// 只列有能力的项，其余按 OFF 补齐，避免漏写一格被误读成「已验证为不支持」。
pub fn supports(entries: &[(AssetKind, AssetSupport)]) -> SupportMap {
    let mut map: SupportMap = AssetKind::ALL
        .iter()
        .map(|kind| (*kind, AssetSupport::OFF))
        .collect();
    for (kind, support) in entries {
        map.insert(*kind, *support);
    }
    map
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityStatus {
    pub kind: AssetKind,
    pub supported: bool,
    pub verified: bool,
}

/// MCP 配置落在哪种文件里。Claude 的用户级 MCP 在 `~/.claude.json`，
/// 它是 `~/.claude/` 的**同级文件**而不是配置根里的文件，所以路径一律写完整的 `~/...`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum McpTarget {
    JsonRoot { file: String, key: String },
    TomlTable { file: String, table: String },
}

/// 该工具的 hooks 要用户自己先打开的功能开关。**我们只检测、不代写**：
/// `[features].hooks` 是用户级功能开关，改它等于改用户可感知的行为。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksGate {
    /// 开关所在的 TOML 文件（声明式路径，`~/` 开头）。
    pub file: String,
    pub table: String,
    pub key: String,
}

/// Hooks 一律是「某个 JSON 文件里的某个子树」；Codex 用的是独立的 hooks.json 而非 settings.json。
/// `matcher=false` 表示该工具的条目里没有 `matcher` 字段（Codex 实测如此），渲染时就不许写它。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum HooksTarget {
    Json {
        file: String,
        key: String,
        matcher: bool,
        gate: Option<HooksGate>,
    },
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
    /// 该 agent 的独立 CLI 命令名；只用于「CLI 是否安装」徽标，不参与写入门禁。
    pub cli_program: &'static str,
    /// 按资产类型逐一标注；`AssetKind::ALL` 每格都有值（缺失补 OFF）。
    pub support: SupportMap,
    pub paths: AgentPaths,
    /// None = 未实测。UI 按保守处理（提示需要重启），不得当成「不用重启」。
    pub mcp_needs_restart: Option<bool>,
}

impl AgentDescriptor {
    fn support_of(&self, kind: AssetKind) -> AssetSupport {
        self.support
            .get(&kind)
            .copied()
            .unwrap_or(AssetSupport::OFF)
    }

    /// 只有既支持又真机确证过才允许出现开关（Plan 1 全局约束）。
    pub fn switchable(&self, kind: AssetKind) -> bool {
        let support = self.support_of(kind);
        support.supported && support.verified
    }

    pub fn supported(&self, kind: AssetKind) -> bool {
        self.support_of(kind).supported
    }

    pub fn capabilities(&self) -> Vec<CapabilityStatus> {
        self.support
            .iter()
            .map(|(kind, support)| CapabilityStatus {
                kind: *kind,
                supported: support.supported,
                verified: support.verified,
            })
            .collect()
    }
}

fn declared(root: &str, rest: &str) -> String {
    format!("{root}/{rest}")
}

/// cc-switch 的 `enabled_<app>` 列里，哪些 app 是我们注册表里认识的 agent。
pub fn is_known_agent(app: &str) -> bool {
    builtin_agents().iter().any(|agent| agent.id == app)
}

/// 能力标注的证据分档，`verified: true` 必须落在前两档：
/// - **A 真机写入**：Plan 2 的沙箱 e2e 实际写过并 diff 过（Claude 的 MCP / Skills）。
/// - **B 本机实物**：这台机器上能看到该落点的真实文件与形状（Claude 的 hooks、Codex 的 MCP…）。
/// - **C 二手推断**：只有官方源码/schema/文档推断，或目录存在但从未写入 → `verified: false`。
///
/// 升档只能靠 Plan 3 Task 11 的真机复核，不能因为单测全绿就改。
pub fn builtin_agents() -> Vec<AgentDescriptor> {
    vec![
        AgentDescriptor {
            id: "claude".into(),
            display_name: "Claude Code".into(),
            config_root: "~/.claude".into(),
            cli_program: "claude",
            // MCP/Skills 属 A（沙箱真机写入）；prompt/hooks 属 B（CLAUDE.md 与 settings.json 实测）；
            // subagent 属 C（本机 ~/.claude/agents 不存在，只有 teamai 代码级证据）
            support: supports(&[
                (AssetKind::Mcp, AssetSupport::VERIFIED),
                (AssetKind::Skill, AssetSupport::VERIFIED),
                (AssetKind::Prompt, AssetSupport::VERIFIED),
                (AssetKind::Hook, AssetSupport::VERIFIED),
                (AssetKind::Subagent, AssetSupport::UNVERIFIED),
                (AssetKind::Session, AssetSupport::UNVERIFIED),
            ]),
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: "~/.claude.json".into(),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.claude", "skills"),
                prompt_file: declared("~/.claude", "CLAUDE.md"),
                // 本机不存在 ~/.claude/rules，官方落点未确认，按不支持处理
                rules_dir: None,
                agents_dir: Some(declared("~/.claude", "agents")),
                hooks: HooksTarget::Json {
                    file: declared("~/.claude", "settings.json"),
                    key: "hooks".into(),
                    matcher: true,
                    gate: None,
                },
            },
            mcp_needs_restart: None,
        },
        AgentDescriptor {
            id: "codex".into(),
            display_name: "Codex".into(),
            config_root: "~/.codex".into(),
            cli_program: "codex",
            support: supports(&[
                (AssetKind::Mcp, AssetSupport::VERIFIED),
                (AssetKind::Skill, AssetSupport::VERIFIED),
                (AssetKind::Prompt, AssetSupport::VERIFIED),
                (AssetKind::Hook, AssetSupport::VERIFIED),
                (AssetKind::Subagent, AssetSupport::UNVERIFIED),
                (AssetKind::Session, AssetSupport::UNVERIFIED),
            ]),
            paths: AgentPaths {
                mcp: McpTarget::TomlTable {
                    file: declared("~/.codex", "config.toml"),
                    table: "mcp_servers".into(),
                },
                skills_dir: declared("~/.codex", "skills"),
                prompt_file: declared("~/.codex", "AGENTS.md"),
                // rules/default.rules 是 `prefix_rule(...)` 执行策略 DSL，不是 markdown 规则
                rules_dir: None,
                agents_dir: Some(declared("~/.codex", "agents")),
                hooks: HooksTarget::Json {
                    file: declared("~/.codex", "hooks.json"),
                    key: "hooks".into(),
                    // 本机 hooks.json 的条目里没有 matcher（实测形状）
                    matcher: false,
                    gate: Some(HooksGate {
                        file: declared("~/.codex", "config.toml"),
                        table: "features".into(),
                        key: "hooks".into(),
                    }),
                },
            },
            mcp_needs_restart: None,
        },
        AgentDescriptor {
            id: "gemini".into(),
            display_name: "Gemini CLI".into(),
            config_root: "~/.gemini".into(),
            cli_program: "gemini",
            // 本机只见到 GEMINI.md、settings.json.mcpServers 与一个空的 skills/
            support: supports(&[
                (AssetKind::Mcp, AssetSupport::VERIFIED),
                (AssetKind::Skill, AssetSupport::UNVERIFIED),
                (AssetKind::Prompt, AssetSupport::VERIFIED),
            ]),
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
        },
        AgentDescriptor {
            id: "qoder".into(),
            display_name: "Qoder".into(),
            config_root: "~/.qoder".into(),
            cli_program: "qoder",
            // A2b：IDE 究竟读 mcp.json 还是 settings.json.mcpServers 未确证，所以全部只标不验证
            support: supports(&[
                (AssetKind::Mcp, AssetSupport::UNVERIFIED),
                (AssetKind::Skill, AssetSupport::UNVERIFIED),
                (AssetKind::Prompt, AssetSupport::UNVERIFIED),
                (AssetKind::Rule, AssetSupport::UNVERIFIED),
                (AssetKind::Subagent, AssetSupport::UNVERIFIED),
                (AssetKind::Hook, AssetSupport::UNVERIFIED),
            ]),
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: declared("~/.qoder", "mcp.json"),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.qoder", "skills"),
                prompt_file: declared("~/.qoder", "AGENTS.md"),
                rules_dir: Some(declared("~/.qoder", "rules")),
                agents_dir: Some(declared("~/.qoder", "agents")),
                hooks: HooksTarget::Json {
                    file: declared("~/.qoder", "settings.json"),
                    key: "hooks".into(),
                    matcher: true,
                    gate: None,
                },
            },
            mcp_needs_restart: None,
        },
        AgentDescriptor {
            id: "qoder-cn".into(),
            display_name: "Qoder CN".into(),
            config_root: "~/.qoder-cn".into(),
            cli_program: "qoderclicn",
            // MCP 有二进制自带 config schema（requiresRestart），skills 目录本机可见；
            // prompt 属 B：本机 ~/.qoder-cn/AGENTS.md 确实被 Qoder CN CLI 当作全局约定读进来了；
            // rules/agents 两目录本机根本不存在、也没有任何样本文件 → C 档，不开开关。
            support: supports(&[
                (AssetKind::Mcp, AssetSupport::VERIFIED),
                (AssetKind::Skill, AssetSupport::VERIFIED),
                (AssetKind::Prompt, AssetSupport::VERIFIED),
                (AssetKind::Rule, AssetSupport::UNVERIFIED),
                (AssetKind::Subagent, AssetSupport::UNVERIFIED),
                (AssetKind::Hook, AssetSupport::UNVERIFIED),
                (AssetKind::Session, AssetSupport::VERIFIED),
            ]),
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
                    matcher: true,
                    gate: None,
                },
            },
            mcp_needs_restart: Some(true),
        },
        AgentDescriptor {
            id: "pi".into(),
            display_name: "Pi Coding Agent".into(),
            // 证据：pi 自带 docs/configuration.md（agent dir = ~/.pi/agent）
            // 与 dist/extensions/mcp/config.js（读 <agentDir>/mcp.json 的 mcpServers）。
            // 提示词落点已用 sha1 与 cc-switch 的 pi 行比对一致。
            config_root: "~/.pi/agent".into(),
            cli_program: "pi",
            support: supports(&[
                (AssetKind::Mcp, AssetSupport::UNVERIFIED),
                (AssetKind::Skill, AssetSupport::VERIFIED),
                (AssetKind::Prompt, AssetSupport::VERIFIED),
                (AssetKind::Session, AssetSupport::UNVERIFIED),
            ]),
            paths: AgentPaths {
                mcp: McpTarget::JsonRoot {
                    file: "~/.pi/agent/mcp.json".into(),
                    key: "mcpServers".into(),
                },
                skills_dir: declared("~/.pi/agent", "skills"),
                prompt_file: declared("~/.pi/agent", "AGENTS.md"),
                // pi 不读 ~/.pi/agent/rules（teamai 的 LEGACY_RULE_DIRS 明写），规则走 AGENTS.md
                rules_dir: None,
                agents_dir: None,
                hooks: HooksTarget::Unsupported,
            },
            mcp_needs_restart: None,
        },
    ]
}

#[cfg(test)]
mod support_tests {
    use super::*;

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn verification_is_per_capability_not_one_global_flag() {
        let qoder_cn = agent("qoder-cn");
        assert!(qoder_cn.support[&AssetKind::Mcp].verified, "MCP 有实测依据");
        // 规则/子代理/hook 三个落点本机连样本都没有,不能跟着 MCP 一起亮灯
        assert!(!qoder_cn.support[&AssetKind::Rule].verified);
        assert!(!qoder_cn.support[&AssetKind::Subagent].verified);
        assert!(!qoder_cn.support[&AssetKind::Hook].verified);
        assert!(
            qoder_cn.support[&AssetKind::Rule].supported,
            "落点存在只是没验证,不该被当成不支持"
        );
    }

    #[test]
    fn agents_without_a_capability_say_so_not_unverified() {
        let codex = agent("codex");
        assert!(
            !codex.support[&AssetKind::Rule].supported,
            "Codex 的 rules 目录是执行策略 DSL,不是 markdown 规则"
        );
        assert!(
            !agent("pi").support[&AssetKind::Rule].supported,
            "Pi 不读 ~/.pi/agent/rules"
        );
    }

    #[test]
    fn a_capability_absent_from_the_table_is_off_not_unverified() {
        // 漏写一格不能被读成「有落点但没验证」，UI 会因此给出错误的「未实测」文案
        assert_eq!(
            agent("gemini").support[&AssetKind::Hook],
            AssetSupport::OFF,
            "{:?}",
            agent("gemini").support
        );
    }

    #[test]
    fn asset_kind_and_dir_name_are_inverse() {
        for asset_type in AssetType::ALL {
            let kind = AssetKind::from_asset_type(asset_type);
            assert_eq!(kind.dir_name(), Some(asset_type.dir_name()));
            assert_eq!(AssetType::from_kind(kind), Ok(asset_type));
        }
        // Session 不是真源资产,没有目录
        assert_eq!(
            AssetType::from_kind(AssetKind::Session),
            Err(AssetKind::Session)
        );
        assert_eq!(AssetKind::Session.dir_name(), None);
    }

    #[test]
    fn capability_list_is_complete_and_stably_ordered() {
        for descriptor in builtin_agents() {
            let kinds: Vec<AssetKind> = descriptor
                .capabilities()
                .iter()
                .map(|capability| capability.kind)
                .collect();
            assert_eq!(kinds, AssetKind::ALL.to_vec());
            assert!(
                descriptor
                    .capabilities()
                    .iter()
                    .all(|c| !c.verified || c.supported),
                "verified 不能脱离 supported 单独出现：{}",
                descriptor.id
            );
        }
    }

    #[test]
    fn switchable_is_the_only_gate_the_ui_may_use() {
        let qoder = agent("qoder");
        assert!(
            !qoder.switchable(AssetKind::Mcp),
            "A2b 未确证，国际版一类开关都不给"
        );
        assert!(agent("claude").switchable(AssetKind::Mcp));
        assert!(!agent("claude").switchable(AssetKind::Rule));
    }
}
