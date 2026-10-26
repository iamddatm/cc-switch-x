import { invoke } from "@tauri-apps/api/core";
import type {
  ActivateOutcome,
  AgentStatus,
  AppSettings,
  AssetSummary,
  AssetType,
  BackupSettings,
  BackupSnapshot,
  BackupUsage,
  CloudRestoreReport,
  CloudSettingsInput,
  CloudSettingsView,
  CloudStatus,
  CloudTestInput,
  ConflictPolicy,
  DetectedProxy,
  DiscoveredSkillInstallInput,
  DriftRecord,
  GitCommit,
  GitRestoreReport,
  GitStatus,
  HooksNeedUserAction,
  ImportPreview,
  ImportSummary,
  ResolveAction,
  ResolveOutcome,
  ResumeInfo,
  InstallSelection,
  ProxySettingsView,
  ProxyTestResult,
  PullOutcome,
  ProjectConflictResolution,
  ProjectSkillView,
  ProjectSkillsView,
  ProjectSyncReport,
  ProjectView,
  MigrationDecision,
  ProjectMigrationReport,
  RestoreBackupReport,
  ScanReport,
  SkillBatchReport,
  SkillCheckReport,
  SkillDiscoveryReport,
  SkillInstallOutcome,
  SkillInstallReport,
  SkillRepo,
  SkillSourceInput,
  SkillSourcePreview,
  SkillUninstallReport,
  SkillUpdateReport,
  SkillsShSearchResult,
  SkillsShTrendingResult,
  SessionDetail,
  SessionMatch,
  SessionProviderStatus,
  SessionScanReport,
  SessionSearchHit,
  SessionSummary,
  Theme,
  UpdateInfo,
} from "../types";

export const api = {
  listAssets: (assetType?: AssetType) => invoke<AssetSummary[]>("list_assets", { assetType }),
  readAsset: (assetType: AssetType, id: string) => invoke<string>("read_asset", { assetType, id }),
  writeAsset: (assetType: AssetType, id: string, content: string) =>
    invoke<AssetSummary>("write_asset", { assetType, id, content }),
  deleteAsset: (assetType: AssetType, id: string) =>
    invoke<boolean>("delete_asset", { assetType, id }),
  importSkillDir: (source: string) => invoke<AssetSummary>("import_skill_dir", { source }),
  previewSkillSource: (input: SkillSourceInput) =>
    invoke<SkillSourcePreview>("preview_skill_source", { input }),
  installSkill: (
    input: SkillSourceInput,
    stagingId: string | null,
    selections: InstallSelection[],
    policy: ConflictPolicy,
  ) =>
    invoke<SkillInstallReport>("install_skill", {
      input,
      stagingId,
      selections,
      policy,
    }),
  checkSkillUpdates: (ids?: string[]) =>
    invoke<SkillCheckReport>("check_skill_updates", { ids }),
  updateSkills: (ids: string[]) =>
    invoke<SkillUpdateReport>("update_skills", { ids }),
  uninstallSkills: (ids: string[]) =>
    invoke<SkillUninstallReport>("uninstall_skills", { ids }),
  setSkillsEnabled: (ids: string[], agentId: string, enabled: boolean) =>
    invoke<SkillBatchReport>("set_skills_enabled", { ids, agentId, enabled }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setTheme: (theme: Theme) => invoke<AppSettings>("set_theme", { theme }),
  setStoreRoot: (storeRoot: string) => invoke<boolean>("set_store_root", { storeRoot }),
  setCloseToTray: (enabled: boolean) =>
    invoke<AppSettings>("set_close_to_tray", { enabled }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  setEnabledAgents: (agentIds: string[]) =>
    invoke<AppSettings>("set_enabled_agents", { agentIds }),
  getProxySettings: () => invoke<ProxySettingsView>("get_proxy_settings"),
  /** password: null = 保留已保存密码，空串 = 清除。 */
  setProxySettings: (url: string, username: string, password: string | null) =>
    invoke<ProxySettingsView>("set_proxy_settings", { url, username, password }),
  testProxySettings: (url: string, username: string, password: string | null) =>
    invoke<ProxyTestResult>("test_proxy_settings", { url, username, password }),
  scanLocalProxies: () => invoke<DetectedProxy[]>("scan_local_proxies"),

  listBackupSnapshots: () => invoke<BackupSnapshot[]>("list_backup_snapshots"),
  createBackupSnapshot: (name?: string) =>
    invoke<BackupSnapshot>("create_backup_snapshot", { name: name ?? null }),
  renameBackupSnapshot: (oldName: string, newName: string) =>
    invoke<BackupSnapshot>("rename_backup_snapshot", { oldName, newName }),
  deleteBackupSnapshot: (name: string) =>
    invoke<void>("delete_backup_snapshot", { name }),
  restoreBackupSnapshot: (name: string) =>
    invoke<RestoreBackupReport>("restore_backup_snapshot", { name }),
  getBackupUsage: () => invoke<BackupUsage>("get_backup_usage"),
  getBackupSettings: () => invoke<BackupSettings>("get_backup_settings"),
  setBackupSettings: (settings: BackupSettings) =>
    invoke<AppSettings>("set_backup_settings", { settings }),
  deleteBackupCategory: (id: string) =>
    invoke<number>("delete_backup_category", { id }),
  revealBackupLocation: (id: string) =>
    invoke<void>("reveal_backup_location", { id }),

  getCloudSettings: () => invoke<CloudSettingsView>("get_cloud_settings"),
  setCloudSettings: (input: CloudSettingsInput) =>
    invoke<CloudSettingsView>("set_cloud_settings", { ...input }),
  testCloudConnection: (input: CloudTestInput) =>
    invoke<number>("test_cloud_connection", { ...input }),
  getCloudStatus: () => invoke<CloudStatus>("get_cloud_status"),
  cloudUploadNow: () => invoke<CloudStatus>("cloud_upload_now"),
  cloudRestoreLatest: () =>
    invoke<CloudRestoreReport>("cloud_restore_latest"),

  getGitStatus: () => invoke<GitStatus>("get_git_status"),
  gitInit: (branch?: string) =>
    invoke<GitStatus>("git_init", { branch: branch ?? null }),
  gitSetRemote: (remote: string | null, branch: string) =>
    invoke<GitStatus>("git_set_remote", { remote, branch }),
  gitVerifyRemote: (remote: string | null, branch: string) =>
    invoke<GitStatus>("git_verify_remote", { remote, branch }),
  gitCommitNow: (message?: string) =>
    invoke<GitStatus>("git_commit_now", { message: message ?? null }),
  gitPushNow: () => invoke<GitStatus>("git_push_now"),
  gitPullNow: () => invoke<PullOutcome>("git_pull_now"),
  gitListCommits: (limit?: number) =>
    invoke<GitCommit[]>("git_list_commits", { limit: limit ?? null }),
  gitRestoreCommit: (hash: string) =>
    invoke<GitRestoreReport>("git_restore_commit", { hash }),
  gitResolveConflict: (action: "keepLocal" | "keepRemote") =>
    invoke<GitStatus>("git_resolve_conflict", { action }),
  getSkillRepos: () => invoke<SkillRepo[]>("get_skill_repos"),
  setSkillRepos: (repos: SkillRepo[]) =>
    invoke<SkillRepo[]>("set_skill_repos", { repos }),
  discoverSkills: () => invoke<SkillDiscoveryReport>("discover_available_skills"),
  installDiscoveredSkill: (input: DiscoveredSkillInstallInput, policy: ConflictPolicy) =>
    invoke<SkillInstallOutcome>("install_discovered_skill", { input, policy }),
  searchSkillsSh: (query: string, limit: number, offset: number) =>
    invoke<SkillsShSearchResult>("search_skills_sh", { query, limit, offset }),
  listSkillsShTrending: (page: number) =>
    invoke<SkillsShTrendingResult>("list_skills_sh_trending", { page }),
  checkAppUpdate: () => invoke<UpdateInfo | null>("check_app_update"),
  installAppUpdate: () => invoke<boolean>("install_app_update"),

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
  deactivatePrompt: (agentId: string) =>
    invoke<boolean>("deactivate_prompt", { agentId }),

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

  listProjects: () => invoke<ProjectView[]>("list_projects"),
  addProject: (path: string) => invoke<ProjectView>("add_project", { path }),
  removeProject: (id: string) => invoke<void>("remove_project", { id }),
  touchProject: (id: string) => invoke<ProjectView>("touch_project", { id }),
  setProjectAutoSync: (id: string, enabled: boolean) =>
    invoke<ProjectView>("set_project_auto_sync", { id, enabled }),
  listProjectSkills: (projectId: string) =>
    invoke<ProjectSkillsView>("list_project_skills", { projectId }),
  revealProjectFolder: (projectId: string) =>
    invoke<void>("reveal_project_folder", { projectId }),
  setProjectSkillTargets: (projectId: string, skillId: string, targets: string[]) =>
    invoke<ProjectSkillView>("set_project_skill_targets", { projectId, skillId, targets }),
  syncProjectSkills: (projectId: string, resolutions: ProjectConflictResolution[] = []) =>
    invoke<ProjectSyncReport>("sync_project_skills", { projectId, resolutions }),
  removeProjectSkillCopy: (projectId: string, skillId: string, dir: string) =>
    invoke<ProjectSyncReport>("remove_project_skill_copy", { projectId, skillId, dir }),
  uninstallProjectSkill: (projectId: string, skillId: string) =>
    invoke<ProjectSyncReport>("uninstall_project_skill", { projectId, skillId }),
  /** v1 台账 → 项目真源的显式迁移；decisions 覆盖所有冲突与候选，否则整体拒绝。 */
  migrateProjectSkills: (projectId: string, decisions: MigrationDecision[]) =>
    invoke<ProjectMigrationReport>("migrate_project_skills", { projectId, decisions }),
  /** 把某个目录里现有的 skill 收进项目真源（先备份旧真源）。 */
  adoptProjectSkill: (projectId: string, skillId: string, dir: string) =>
    invoke<ProjectSyncReport>("adopt_project_skill", { projectId, skillId, dir }),
  readProjectSkillBody: (projectId: string, skillId: string) =>
    invoke<string>("read_project_skill_body", { projectId, skillId }),
  /** 只替换真源的 SKILL.md；渲染留给下一次同步。 */
  writeProjectSkillBody: (projectId: string, skillId: string, content: string) =>
    invoke<ProjectSyncReport>("write_project_skill_body", { projectId, skillId, content }),
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
