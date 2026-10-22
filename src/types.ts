export type AssetType = "mcp" | "skill" | "prompt" | "rule" | "subagent" | "hook";
export type Pane = AssetType | "sessions" | "settings";
export type Theme = "light" | "dark" | "system";

export interface AssetSummary {
  assetType: AssetType;
  id: string;
  name: string;
  updatedAt: number;
  hash: string;
}

export interface AppSettings {
  storeRoot: string;
  theme: Theme;
}

/** 后端 AssetState 的 snake_case 序列化结果 */
export type AssetState = "off" | "written" | "effective" | "drifted";

/** 后端 AssetKind 的 snake_case 序列化结果：比 AssetType 多一个 session。 */
export type AssetKind = AssetType | "session";

export interface CapabilityStatus {
  kind: AssetKind;
  /** false = 该 agent 没有这一类落点（Codex 的 rules 是执行策略 DSL） */
  supported: boolean;
  /** true 才给开关；supported 而 !verified 只是「落点在，没实测」 */
  verified: boolean;
}

export interface AgentStatus {
  id: string;
  displayName: string;
  detected: boolean;
  capabilities: CapabilityStatus[];
  /** null = 未实测，UI 保守提示「可能需要重启」 */
  mcpNeedsRestart: boolean | null;
  mcpPath: string;
  enabledMcp: string[];
  enabledSkills: string[];
  activePrompt: string | null;
}

export function supported(agent: AgentStatus, kind: AssetKind): boolean {
  return agent.capabilities.some((c) => c.kind === kind && c.supported);
}

/** 唯一允许出现开关的条件；面板不许自己拼 supported && verified。 */
export function switchable(agent: AgentStatus, kind: AssetKind): boolean {
  return agent.capabilities.some((c) => c.kind === kind && c.supported && c.verified);
}

export interface ImportPreview {
  mcpCount: number;
  promptsCount: number;
  skillsCount: number;
  ignoredColumns: string[];
  missingSkillDirs: string[];
  conflicts: string[];
}

export interface ImportSummary {
  mcpImported: number;
  promptsImported: number;
  skillsImported: number;
  unchanged: number;
  skipped: number;
  failed: string[];
  missingSkillDirs: string[];
  ambiguousActive: string[];
  preview: ImportPreview;
}

export type ConflictPolicy = "skip" | "overwrite" | "keepBoth";

export type ActivateOutcome =
  | { kind: "activated"; path: string }
  | { kind: "alreadyActive"; path: string }
  | { kind: "needsConfirmation"; path: string };
