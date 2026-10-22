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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub display_name: String,
    pub config_root: String,
    pub supports: AgentCapabilities,
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
            id: "qoder-cn".into(),
            display_name: "Qoder CN".into(),
            config_root: "~/.qoder-cn".into(),
            supports: all(true),
        },
        AgentDescriptor {
            id: "qoder".into(),
            display_name: "Qoder".into(),
            config_root: "~/.qoder".into(),
            supports: all(false),
        },
    ]
}
