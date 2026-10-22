import { invoke } from "@tauri-apps/api/core";
import type {
  ActivateOutcome,
  AgentStatus,
  AppSettings,
  AssetSummary,
  AssetType,
  ConflictPolicy,
  DriftRecord,
  HooksNeedUserAction,
  ImportPreview,
  ImportSummary,
  ResolveAction,
  ResolveOutcome,
  ResumeInfo,
  ScanReport,
  SessionDetail,
  SessionMatch,
  SessionProviderStatus,
  SessionScanReport,
  SessionSearchHit,
  SessionSummary,
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

  /** 返回实测标签：written / removed / absent；外来内容会直接报错 */
  setRuleEnabled: (id: string, agentId: string, enabled: boolean) =>
    invoke<string>("set_rule_enabled", { id, agentId, enabled }),
  scanRules: (agentId: string) => invoke<ScanReport>("scan_rules", { agentId }),
  setSubagentEnabled: (id: string, agentId: string, enabled: boolean) =>
    invoke<string>("set_subagent_enabled", { id, agentId, enabled }),
  scanSubagents: (agentId: string) =>
    invoke<ScanReport>("scan_subagents", { agentId }),
  setHookEnabled: (id: string, agentId: string, enabled: boolean) =>
    invoke<ApplyReport>("set_hook_enabled", { id, agentId, enabled }),
  /** 该 agent 的 hook 文件里不是本工具写的条目数 */
  foreignHookCount: (agentId: string) => invoke<number>("foreign_hook_count", { agentId }),
  /** null = 不需要用户额外打开开关 */
  hooksGateStatus: (agentId: string) =>
    invoke<HooksNeedUserAction | null>("hooks_gate_status", { agentId }),

  rescanDrift: () => invoke<DriftRecord[]>("rescan_drift"),
  listDrifts: () => invoke<DriftRecord[]>("list_drifts"),
  resolveDrift: (id: number, action: ResolveAction, agentId: string) =>
    invoke<ResolveOutcome>("resolve_drift", { id, action, agentId }),

  scanSessions: () => invoke<SessionScanReport>("scan_sessions"),
  listSessions: (agentId?: string, query?: string, offset?: number, limit?: number) =>
    invoke<SessionSummary[]>("list_sessions", { agentId, query, offset, limit }),
  getSession: (agentId: string, sessionId: string, offset?: number, limit?: number) =>
    invoke<SessionDetail>("get_session", { agentId, sessionId, offset, limit }),
  searchSessions: (query: string, limit?: number) =>
    invoke<SessionSearchHit[]>("search_sessions", { query, limit }),
  searchInSession: (agentId: string, sessionId: string, query: string, limit?: number) =>
    invoke<SessionMatch[]>("search_in_session", { agentId, sessionId, query, limit }),
  sessionProviders: () =>
    invoke<SessionProviderStatus[]>("session_providers"),
  sessionResume: (agentId: string, sessionId: string) =>
    invoke<ResumeInfo>("session_resume", { agentId, sessionId }),
  exportSessionMarkdown: (agentId: string, sessionId: string, dest: string) =>
    invoke<string>("export_session_markdown", { agentId, sessionId, dest }),
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
