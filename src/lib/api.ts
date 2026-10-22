import { invoke } from "@tauri-apps/api/core";
import type {
  ActivateOutcome,
  AgentStatus,
  AppSettings,
  AssetSummary,
  AssetType,
  ConflictPolicy,
  ImportPreview,
  ImportSummary,
  Theme,
} from "../types";

export const api = {
  listAssets: (assetType?: AssetType) => invoke<AssetSummary[]>("list_assets", { assetType }),
  readAsset: (assetType: AssetType, id: string) => invoke<string>("read_asset", { assetType, id }),
  writeAsset: (assetType: AssetType, id: string, content: string) =>
    invoke<AssetSummary>("write_asset", { assetType, id, content }),
  deleteAsset: (assetType: AssetType, id: string) =>
    invoke<boolean>("delete_asset", { assetType, id }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setTheme: (theme: Theme) => invoke<AppSettings>("set_theme", { theme }),
  setStoreRoot: (storeRoot: string) => invoke<boolean>("set_store_root", { storeRoot }),

  listAgents: () => invoke<AgentStatus[]>("list_agents"),
  importPreview: () => invoke<ImportPreview>("import_preview"),
  importApply: (policy: ConflictPolicy) => invoke<ImportSummary>("import_apply", { policy }),
  revertLast: () => invoke<ApplyReport>("revert_last"),
  setMcpEnabled: (id: string, agentId: string, enabled: boolean) =>
    invoke<ApplyReport>("set_mcp_enabled", { id, agentId, enabled }),
  /** 返回实测的落地方式：symlink / junction / copy / already / removed */
  setSkillEnabled: (id: string, agentId: string, enabled: boolean) =>
    invoke<string>("set_skill_enabled", { id, agentId, enabled }),
  activatePrompt: (
    promptId: string,
    agentId: string,
    options?: { confirm?: boolean; saveExistingAs?: string },
  ) =>
    invoke<ActivateOutcome>("activate_prompt", {
      promptId,
      agentId,
      confirm: options?.confirm ?? false,
      saveExistingAs: options?.saveExistingAs,
    }),
};

export interface ApplyReport {
  written: number;
  skipped: number;
  ok: boolean;
  backupDir: string;
  error: string | null;
  /** 这批写入实际走了哪条路径：CLI 确认过的才配得上「已生效」 */
  methodUsed: "cli" | "file";
  /** 为什么走这条（CLI 失败原因 / 没装 CLI / 已回滚）。没有额外说明时是 null。 */
  note: string | null;
}
