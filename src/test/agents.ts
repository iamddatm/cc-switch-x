import type { AgentStatus, AssetKind, CapabilityStatus } from "../types";

const KINDS: AssetKind[] = ["mcp", "skill", "prompt", "rule", "subagent", "hook", "session"];

/**
 * 造能力表。没在 `on` 里点亮的格子一律 supported=false，与后端 `supports()` 的补 OFF 行为一致；
 * "unverified" 表示「落点在但没实测」——测试面板到底给不给开关，靠的就是这一格。
 */
export function capabilities(
  on: Partial<Record<AssetKind, "verified" | "unverified">>,
): CapabilityStatus[] {
  return KINDS.map((kind) => ({
    kind,
    supported: on[kind] !== undefined,
    verified: on[kind] === "verified",
  }));
}

export function agentStatus(overrides: Partial<AgentStatus> = {}): AgentStatus {
  return {
    id: "claude",
    displayName: "Claude Code",
    detected: true,
    capabilities: capabilities({ mcp: "verified", skill: "verified", prompt: "verified" }),
    mcpNeedsRestart: null,
    mcpPath: "C:/x/.claude.json",
    enabledMcp: [],
    enabledSkills: [],
    enabledRules: [],
    enabledSubagents: [],
    enabledHooks: [],
    activePrompt: null,
    ...overrides,
  };
}
