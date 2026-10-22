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
