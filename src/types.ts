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

export interface AgentStatus {
  id: string;
  displayName: string;
  detected: boolean;
  verified: boolean;
  /** null = 未实测，UI 保守提示「可能需要重启」 */
  mcpNeedsRestart: boolean | null;
  mcpPath: string;
  enabledMcp: string[];
  enabledSkills: string[];
  activePrompt: string | null;
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
