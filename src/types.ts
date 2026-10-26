export type AssetType = "mcp" | "skill" | "prompt" | "rule" | "subagent" | "hook";
export type Pane = AssetType | "sessions" | "agents" | "network" | "backup" | "settings";
export type Theme = "light" | "dark" | "system";

export interface AssetSummary {
  assetType: AssetType;
  id: string;
  name: string;
  updatedAt: number;
  hash: string;
  /** 列表行副标题；由后端从真源内容补齐。 */
  description?: string | null;
  /** Skill 来源仓库 owner/repo。 */
  source?: string | null;
  /** 真源文件 mtime（毫秒）。 */
  mtime?: number | null;
  /** 真源文件字节数。 */
  size?: number | null;
  /** MCP 传输方式。 */
  transport?: string | null;
  /** Skill 的来源与更新状态。 */
  origin?: SkillOrigin | null;
}

export type SkillSourceKind = "github" | "zip" | "local" | "cc-switch";

export interface SkillOrigin {
  kind: SkillSourceKind;
  repo?: string | null;
  branch?: string | null;
  sourceUrl?: string | null;
  readmeUrl?: string | null;
  installedAt?: number | null;
  updatedAt?: number | null;
  lastCheckedAt?: number | null;
  updateAvailable: boolean;
  lastCheckError?: string | null;
  /** 有 owner/repo 才能从上游检查更新。 */
  updatable: boolean;
}

export interface AppSettings {
  storeRoot: string;
  theme: Theme;
  /** null = 全部启用；空数组 = 全部禁用。 */
  enabledAgents: string[] | null;
  /** 关闭窗口时隐藏到托盘。 */
  closeToTray: boolean;
  proxy: { url: string | null; username: string | null };
  /** null = 使用内置默认仓库。 */
  skillRepos: SkillRepo[] | null;
  backup: BackupSettings;
  cloud: CloudBackupSettings;
}

export interface BackupSettings {
  /** 0 = 关闭自动快照。 */
  intervalHours: number;
  retainCount: number;
  git: GitBackupSettings;
}

export interface GitBackupSettings {
  enabled: boolean;
  autoCommit: boolean;
  autoPull: boolean;
  remote: string | null;
  branch: string;
}

export type SnapshotKind = "manual" | "auto" | "preRestore" | "prePull";

export interface BackupSnapshot {
  name: string;
  kind: SnapshotKind;
  /** 毫秒时间戳。 */
  createdAt: number;
  sizeBytes: number;
  files: number;
}

export interface BackupUsageSection {
  id: "snapshots" | "writes" | "logs" | "index";
  path: string;
  bytes: number;
  items: number;
}

export interface BackupUsage {
  sections: BackupUsageSection[];
  totalBytes: number;
}

export interface RestoreBackupReport {
  safetyBackup: string;
  restoredFiles: number;
  removedFiles: number;
}

export interface CloudBackupSettings {
  autoUpload: boolean;
  baseUrl: string;
  username: string;
  remoteDir: string;
}

export interface CloudSettingsView extends CloudBackupSettings {
  hasPassword: boolean;
}

export type CloudSettingsInput = CloudBackupSettings & {
  /** null = 保留已保存的密码；空串 = 清除。 */
  password: string | null;
};

export type CloudTestInput = Omit<CloudSettingsInput, "autoUpload">;

export interface CloudRemoteStatus {
  exists: boolean;
  sizeBytes: number | null;
  modifiedAt: number | null;
  etag: string | null;
}

export interface CloudStatus {
  configured: boolean;
  hasPassword: boolean;
  autoUpload: boolean;
  remote: CloudRemoteStatus | null;
  remoteError: string | null;
  lastUploadAt: number | null;
  lastError: string | null;
}

export interface CloudRestoreReport {
  safetyBackup: string;
  restoredFiles: number;
  removedFiles: number;
  downloadedSnapshot: string;
  remoteSizeBytes: number;
  remoteModifiedAt: number | null;
}

export type GitSyncState =
  | "unavailable"
  | "disabled"
  | "idle"
  | "syncing"
  | "paused"
  | "error";

export interface GitStatus {
  available: boolean;
  initialized: boolean;
  enabled: boolean;
  /** 当前远端地址是否通过了 ls-remote 验证。 */
  remoteVerified: boolean;
  branch: string | null;
  remote: string | null;
  dirty: number;
  ahead: number;
  behind: number;
  lastCommitAt: number | null;
  lastPullAt: number | null;
  state: GitSyncState;
  lastError: string | null;
  conflictFiles: string[];
}

export interface GitCommit {
  hash: string;
  shortHash: string;
  message: string;
  timestamp: number;
}

export interface GitRestoreReport {
  safetyBackup: string;
  commit: string;
  files: number;
}

export type PullOutcome =
  | { kind: "upToDate" }
  | { kind: "noRemote" }
  | { kind: "pulled"; commits: number; safetyBackup: string }
  | { kind: "conflict"; files: string[]; safetyBackup: string };

export interface ProxySettingsView {
  url: string | null;
  username: string | null;
  hasPassword: boolean;
}

export interface ProxyTestResult {
  success: boolean;
  latencyMs: number;
  error: string | null;
}

export interface DetectedProxy {
  url: string;
  proxyType: string;
  port: number;
}

export interface SkillRepo {
  owner: string;
  name: string;
  branch: string;
  enabled: boolean;
}

export interface DiscoverableSkill {
  key: string;
  name: string;
  description: string | null;
  relativePath: string;
  suggestedId: string;
  repo: string;
  branch: string;
  installed: boolean;
}

export interface SkillDiscoveryReport {
  skills: DiscoverableSkill[];
  failures: SkillRepoFailure[];
  /** 本次发现使用的仓库：配置仓库 + 已安装 Skill 的来源仓库（含已停用项）。 */
  repos: SkillRepo[];
}

export interface DiscoveredSkillInstallInput {
  repo: string;
  branch?: string | null;
  relativePath?: string | null;
  /** skills.sh 的 skillId。 */
  directory?: string | null;
  name?: string | null;
}

export interface SkillsShSkill {
  key: string;
  name: string;
  directory: string;
  repoOwner: string;
  repoName: string;
  repoBranch: string;
  installs: number;
  readmeUrl: string;
}

export interface SkillsShSearchResult {
  skills: SkillsShSkill[];
  totalCount: number;
  query: string;
}

/** skills.sh 趋势榜：站点固定每页 200 条，page 从 0 开始。 */
export interface SkillsShTrendingResult {
  skills: SkillsShSkill[];
  hasMore: boolean;
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
  /** PATH 与常见安装目录中是否找得到 CLI；只用于展示，不参与写入门禁。 */
  cliAvailable: boolean;
  /** 探测用的命令名。 */
  cliProgram: string;
  capabilities: CapabilityStatus[];
  /** null = 未实测，UI 保守提示「可能需要重启」 */
  mcpNeedsRestart: boolean | null;
  mcpPath: string;
  /** 提示词整文件落点。 */
  promptPath: string;
  /** skills 目录落点。 */
  skillsDir: string;
  /** 是否在全局启用范围内；false 时面板不展示这一列。 */
  enabled: boolean;
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

export type SkillSourceInput =
  | { kind: "github"; repo: string; branch?: string | null }
  | { kind: "zip"; path: string }
  | { kind: "directory"; path: string };

export interface SkillCandidate {
  relativePath: string;
  name: string;
  description?: string | null;
  suggestedId: string;
  exists: boolean;
}

export interface SkillSourcePreview {
  kind: SkillSourceKind;
  repo?: string | null;
  branch?: string | null;
  stagingId?: string | null;
  candidates: SkillCandidate[];
  warnings: string[];
}

export interface InstallSelection {
  relativePath: string;
  id: string;
}

export interface SkillInstallOutcome {
  id: string;
  name: string;
  status: "installed" | "overwritten" | "skipped" | "failed";
  backupPath?: string | null;
  error?: string | null;
  warnings: string[];
}

export interface SkillInstallReport {
  outcomes: SkillInstallOutcome[];
}

export interface SkillItemFailure {
  id: string;
  error: string;
}

export interface SkillSkipped {
  id: string;
  reason: string;
}

export interface SkillUpdateInfo {
  id: string;
  name: string;
  currentHash?: string | null;
  remoteHash: string;
  missingLocal: boolean;
}

export interface SkillRepoFailure {
  repo: string;
  branch?: string | null;
  error: string;
}

export interface SkillCheckReport {
  checkedAt: number;
  updates: SkillUpdateInfo[];
  upToDate: string[];
  skipped: SkillSkipped[];
  repoFailures: SkillRepoFailure[];
  skillFailures: SkillItemFailure[];
}

export interface SkillUpdateOutcome {
  id: string;
  name: string;
  status: "updated" | "upToDate" | "failed";
  backupPath?: string | null;
  warnings: string[];
  error?: string | null;
}

export interface SkillUpdateReport {
  outcomes: SkillUpdateOutcome[];
}

export interface SkillUninstallOutcome {
  id: string;
  backupPath?: string | null;
  disabledAgents: string[];
  warnings: string[];
  error?: string | null;
}

export interface SkillUninstallReport {
  outcomes: SkillUninstallOutcome[];
}

export interface SkillBatchOk {
  id: string;
  label: string;
}

export interface SkillBatchReport {
  ok: SkillBatchOk[];
  failed: SkillItemFailure[];
}

// ── 项目级 Skills ─────────────────────────────────────────────────

export type ProjectScope = "global" | "project";

export interface ProjectView {
  id: string;
  path: string;
  name: string;
  addedAt: string;
  lastOpenedAt: string;
  autoSync: boolean;
  exists: boolean;
}

export type ProjectSkillDirKey = "claude" | "qoder" | "agents";

export interface ProjectDirMeta {
  key: ProjectSkillDirKey;
  label: string;
  relativePath: string;
  agents: string[];
  verified: boolean;
}

/**
 * 一格相对项目真源的状态。两轴：与上次渲染基线的差 = 是否被外部改过；
 * 与真源的差 = 是否落后。
 * - `stale` 真源已更新，可以直接覆盖（安全）。
 * - `edited` 被外部改过，必须人工裁决，永不自动覆盖。
 * - `untracked` 不在渲染目标里的副本，永不触碰。
 */
export type ProjectDirState =
  | "present"
  | "absent"
  | "stale"
  | "edited"
  | "untracked"
  | "unavailable";

export interface ProjectDirView extends ProjectDirMeta {
  state: ProjectDirState;
}

export interface ProjectSkillSource {
  kind: string;
  repo?: string | null;
  branch?: string | null;
  subpath?: string | null;
  treeHash?: string | null;
}

export interface ProjectSkillOp {
  dir: string;
  action: string;
  at: string;
}

export interface ProjectSkillView {
  id: string;
  name: string;
  description: string | null;
  targets: string[];
  dirs: Record<string, ProjectDirView>;
  source: ProjectSkillSource | null;
  lastOp: ProjectSkillOp | null;
  /** 项目真源 `.ccswitchx/skills/<id>/` 是否已落地。 */
  canonical: boolean;
  /** 账本记着有真源、磁盘上却没有：只能由显式「删除全部副本」处置。 */
  canonicalMissing: boolean;
  /** 上次检查更新的结果；只存本机 `local/project-skill-checks/`，从没检查过时为 null。 */
  check?: ProjectSkillCheck | null;
}

/** 后端 `store::project_checks::ProjectSkillCheck`：项目 skill 的本机检查状态。 */
export interface ProjectSkillCheck {
  updateAvailable: boolean;
  latestHash?: string | null;
  lastCheckedAt?: number | null;
  lastCheckError?: string | null;
  updatable: boolean;
}

export interface ProjectSkillsView {
  project: ProjectView;
  dirs: ProjectDirMeta[];
  skills: ProjectSkillView[];
  conflictCount: number;
  /** v1 台账的迁移计划；已迁移或无事可做时为 null。只读计算，不落盘。 */
  migration: ProjectMigrationPlan | null;
}

/** 迁移的一条采纳：把某个目录的现有字节新建进项目真源，agent 目录本身不动。 */
export interface MigrationItem {
  skillId: string;
  fromDir: string;
  /** 将在真源里新建的文件（相对项目根）。 */
  files: string[];
}

/** 从没被勾选为目标的副本：是否收进真源由用户决定。 */
export interface MigrationProposal extends MigrationItem {
  candidates: string[];
  defaultAdopt: boolean;
  reason: string;
}

export interface MigrationConflict {
  skillId: string;
  dirs: string[];
  recommended: string | null;
  /** targetsDiffer：各目录内容不一致；unresolvable：状态取不到（链接等）。 */
  kind: "targetsDiffer" | "unresolvable" | string;
}

export interface ProjectMigrationPlan {
  adopts: MigrationItem[];
  proposals: MigrationProposal[];
  conflicts: MigrationConflict[];
  /** 台账里有、磁盘上什么都没有：只去条目，不写任何文件。 */
  drops: string[];
  warnings: string[];
}

/** 一条裁决；`dir` 为空串表示这条跳过不动。 */
export interface MigrationDecision {
  skillId: string;
  dir: string;
}

export interface ProjectMigrationReport {
  adopted: string[];
  skipped: string[];
  dropped: string[];
  warnings: string[];
  ledgerBackup?: string | null;
  version: number;
}

export interface ProjectConflict {
  skillId: string;
  dirs: string[];
  recommended: string | null;
  /** targetVsCanonical：副本被外部改过；canonicalMissing：账本有真源、磁盘没有。 */
  kind: "targetVsCanonical" | "canonicalMissing" | string;
}

/**
 * 冲突处置：
 * - `use` dir 是胜出目录，先收进真源再渲染其余。
 * - `keepBoth` dir 是被保住的手改副本，另存为未跟踪 `<id>-copy`。
 * - `purge` 仅用于 canonicalMissing：删除全部副本（先备份）。
 */
export interface ProjectConflictResolution {
  skillId: string;
  dir: string;
  action: "use" | "keepBoth" | "purge";
}

export interface ProjectSyncOutcome {
  skillId: string;
  dir: string;
  /** render：真源→目标；adopt：目录→真源；delete：删副本；keepBoth：保住被外部改过的副本。 */
  action: "render" | "delete" | "adopt" | "keepBoth" | string;
  status: "ok" | "failed" | string;
  backupPath?: string | null;
  error?: string | null;
}

export interface ProjectSyncReport {
  outcomes: ProjectSyncOutcome[];
  conflicts: ProjectConflict[];
  warnings: string[];
}

/**
 * 装进项目真源的结果：`installed` 是真源落地情况，`render` 是同一次调用里
 * 同步引擎顺带铺开的产物（没勾选目标时就是空的）。
 */
export interface ProjectInstallReport {
  installed: SkillInstallOutcome[];
  render: ProjectSyncReport;
  warnings: string[];
}

export type ActivateOutcome =
  | { kind: "activated"; path: string }
  | { kind: "alreadyActive"; path: string }
  | { kind: "needsConfirmation"; path: string };

// ── 会话（只读本地浏览） ───────────────────────────────────────────

export type SessionRole = "user" | "assistant" | "tool" | "system";
export type SessionMessageKind =
  | "text"
  | "thinking"
  | "tool_call"
  | "tool_result"
  | "image"
  | "event";

export interface SessionSummary {
  agentId: string;
  sessionId: string;
  title: string;
  projectPath: string | null;
  sourcePath: string;
  createdAt: number | null;
  updatedAt: number;
  messageCount: number;
  parseError: string | null;
  /** 最后一条文本消息摘要。 */
  preview?: string | null;
}

export interface SessionMessage {
  seq: number;
  role: SessionRole;
  kind: SessionMessageKind;
  content: string;
  toolName: string | null;
}

export interface SessionDetail {
  session: SessionSummary;
  messages: SessionMessage[];
  total: number;
}

export interface SessionSearchHit {
  session: SessionSummary;
  index: number;
  seq: number;
  snippet: string;
}

export interface SessionMatch {
  index: number;
  seq: number;
  snippet: string;
}

export interface SessionProviderStatus {
  agentId: string;
  displayName: string;
  verified: boolean;
  parseReady: boolean;
  roots: string[];
  rootExists: boolean;
  foundFiles: number;
  sessionCount: number;
}

export interface SessionFileFailure {
  agentId: string;
  file: string;
  reason: string;
}

export interface SessionFileWarning {
  agentId: string;
  file: string;
  line: number;
  message: string;
}

export interface SessionScanReport {
  added: number;
  updated: number;
  removed: number;
  unchanged: number;
  failures: SessionFileFailure[];
  warnings: SessionFileWarning[];
}

export interface ResumeInfo {
  agentId: string;
  sessionId: string;
  cwd: string | null;
  command: string;
}

export interface UpdateInfo {
  currentVersion: string;
  version: string;
  notes: string | null;
  pubDate: string | null;
}

export interface UpdateDownloadProgress {
  downloaded: number;
  total: number | null;
}
