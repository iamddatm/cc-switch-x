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
  enabledRules: string[];
  enabledSubagents: string[];
  enabledHooks: string[];
  activePrompt: string | null;
}

export function supported(agent: AgentStatus, kind: AssetKind): boolean {
  return agent.capabilities.some((c) => c.kind === kind && c.supported);
}

/** 唯一允许出现开关的条件；面板不许自己拼 supported && verified。 */
export function switchable(agent: AgentStatus, kind: AssetKind): boolean {
  return agent.capabilities.some((c) => c.kind === kind && c.supported && c.verified);
}

/** 该 agent 已勾选的资产 id。矩阵一次取全，省掉每个资产问一次的往返。 */
export function enabledFor(agent: AgentStatus, kind: AssetType): string[] {
  switch (kind) {
    case "mcp":
      return agent.enabledMcp;
    case "skill":
      return agent.enabledSkills;
    case "rule":
      return agent.enabledRules;
    case "subagent":
      return agent.enabledSubagents;
    case "hook":
      return agent.enabledHooks;
    default:
      // prompts 同时只能激活一份，走 activePrompt，不在这张表里
      return [];
  }
}

export type DriftKind = "modified" | "missing";

/** 后端 drift::DriftRecord。targetPath 是绝对路径，处置前用户要能看到到底在动哪个文件。 */
export interface DriftRecord {
  id: number;
  assetType: AssetType;
  assetId: string;
  agentId: string;
  kind: DriftKind;
  detectedAt: number;
  targetPath: string;
}

export type ResolveAction = "adoptDisk" | "repushSource";

export type ResolveOutcome =
  | { kind: "adopted"; path: string }
  | { kind: "repushed"; path: string; written: number; skipped: number };

export interface SkippedFile {
  file: string;
  reason: string;
}

export interface ScanReport {
  imported: number;
  updated: number;
  unchanged: number;
  skipped: SkippedFile[];
}

/** 该工具的 hooks 要用户自己打开功能开关；我们只报告，不代改。 */
export interface HooksNeedUserAction {
  file: string;
  why: string;
}

/** hook 事件白名单，与后端 `domain::hook::KNOWN_EVENTS` 同源（Qoder CN 的事件枚举）。 */
export const HOOK_EVENTS: string[] = [
  "PreToolUse",
  "PostToolUse",
  "PostToolUseFailure",
  "SubagentStart",
  "Notification",
  "SubagentStop",
  "SessionStart",
  "Setup",
  "SessionEnd",
  "PreCompact",
  "PostCompact",
  "Stop",
  "QueryEnd",
  "UserPromptSubmit",
  "ConfigChange",
  "StopFailure",
  "TeammateIdle",
  "InstructionsLoaded",
  "CwdChanged",
  "FileChanged",
  "WorktreeCreate",
  "WorktreeRemove",
  "Elicitation",
  "ElicitationResult",
  "TaskCreated",
  "TaskCompleted",
  "PermissionRequest",
  "PermissionDenied",
];

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
