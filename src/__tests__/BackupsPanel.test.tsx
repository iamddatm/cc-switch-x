import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { BackupsPanel } from "../panels/BackupsPanel";
import { api } from "../lib/api";
import type {
  BackupSettings,
  CloudSettingsView,
  CloudStatus,
  GitStatus,
} from "../types";

// jsdom 里没有 Tauri 事件桥：按事件名记录回调，供进度与自动同步用例手动触发。
const eventBridge = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (name: string, handler: (event: { payload: unknown }) => void) => {
      eventBridge.handlers.set(name, handler);
      return () => {};
    },
  ),
}));

vi.mock("../lib/api", () => ({
  api: {
    listBackupSnapshots: vi.fn(),
    createBackupSnapshot: vi.fn(),
    renameBackupSnapshot: vi.fn(),
    deleteBackupSnapshot: vi.fn(),
    restoreBackupSnapshot: vi.fn(),
    getBackupUsage: vi.fn(),
    getBackupSettings: vi.fn(),
    setBackupSettings: vi.fn(),
    deleteBackupCategory: vi.fn(),
    revealBackupLocation: vi.fn(),
    getCloudSettings: vi.fn(),
    getCloudStatus: vi.fn(),
    getCloudStatusCached: vi.fn(),
    setCloudSettings: vi.fn(),
    testCloudConnection: vi.fn(),
    cloudUploadNow: vi.fn(),
    cloudRestoreLatest: vi.fn(),
    getGitStatus: vi.fn(),
    gitInit: vi.fn(),
    gitSetRemote: vi.fn(),
    gitVerifyRemote: vi.fn(),
    gitCommitNow: vi.fn(),
    gitPushNow: vi.fn(),
    gitPullNow: vi.fn(),
    gitListCommits: vi.fn(),
    gitRestoreCommit: vi.fn(),
    gitResolveConflict: vi.fn(),
  },
}));

const snapshot = {
  name: "backup_20261025_120000",
  kind: "manual" as const,
  createdAt: Date.now() - 60_000,
  sizeBytes: 2048,
  files: 6,
};

const settings: BackupSettings = {
  intervalHours: 24,
  retainCount: 10,
  git: {
    enabled: true,
    autoCommit: true,
    autoPull: true,
    remote: "git@github.com:me/assets.git",
    branch: "main",
  },
};

const healthyGit: GitStatus = {
  available: true,
  initialized: true,
  enabled: true,
  remoteVerified: true,
  branch: "main",
  remote: "git@github.com:me/assets.git",
  dirty: 1,
  ahead: 0,
  behind: 0,
  lastCommitAt: Date.now() - 120_000,
  lastPullAt: Date.now() - 600_000,
  state: "idle",
  lastError: null,
  conflictFiles: [],
};

const commit = {
  hash: "0123456789abcdef0123456789abcdef01234567",
  shortHash: "01234567",
  message: "backup: test",
  timestamp: Date.now() - 120_000,
};

const cloudView: CloudSettingsView = {
  enabled: false,
  autoUpload: false,
  autoDownload: false,
  uploadDelayHours: 1,
  baseUrl: "https://dav.example.com/dav",
  username: "me",
  remoteDir: "ccswitchx",
  hasPassword: true,
};

const cloudStatus: CloudStatus = {
  configured: true,
  hasPassword: true,
  autoUpload: false,
  autoDownload: false,
  uploadDelayHours: 1,
  remote: {
    exists: true,
    sizeBytes: 2048,
    modifiedAt: Date.now() - 300_000,
    etag: '"abc"',
  },
  remoteError: null,
  manifest: {
    appVersion: "0.10.0",
    deviceName: "DESKTOP-TEST",
    createdAt: Date.now() - 300_000,
    files: 12,
    schema: 1,
    compatible: true,
  },
  manifestError: null,
  lastUploadAt: Date.now() - 300_000,
  lastDownloadAt: null,
  lastError: null,
  lastDownloadError: null,
  uploadedBytesMonth: 0,
  fromCache: false,
};

function mockBase(git: GitStatus = healthyGit) {
  vi.mocked(api.listBackupSnapshots).mockResolvedValue([snapshot]);
  vi.mocked(api.getBackupUsage).mockResolvedValue({
    sections: [
      { id: "snapshots", path: "C:/data/snapshots", bytes: 2048, items: 1 },
      { id: "writes", path: "C:/data/backups", bytes: 1024, items: 2 },
      { id: "logs", path: "C:/data/logs", bytes: 0, items: 0 },
      { id: "index", path: "C:/data/index.db", bytes: 512, items: 1 },
    ],
    totalBytes: 3584,
  });
  vi.mocked(api.getBackupSettings).mockResolvedValue(settings);
  vi.mocked(api.getCloudSettings).mockResolvedValue(cloudView);
  vi.mocked(api.getCloudStatus).mockResolvedValue(cloudStatus);
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({
    ...cloudStatus,
    fromCache: true,
  });
  vi.mocked(api.setCloudSettings).mockResolvedValue(cloudView);
  vi.mocked(api.testCloudConnection).mockResolvedValue(42);
  vi.mocked(api.cloudUploadNow).mockResolvedValue(cloudStatus);
  vi.mocked(api.cloudRestoreLatest).mockResolvedValue({
    safetyBackup: "backup_safety",
    restoredFiles: 6,
    removedFiles: 1,
    downloadedSnapshot: "cloud_restore_20261026_120000",
    remoteSizeBytes: 2048,
    remoteModifiedAt: Date.now() - 300_000,
  });
  vi.mocked(api.getGitStatus).mockResolvedValue(git);
  vi.mocked(api.gitListCommits).mockResolvedValue([commit]);
  vi.mocked(api.createBackupSnapshot).mockResolvedValue(snapshot);
  vi.mocked(api.renameBackupSnapshot).mockResolvedValue(snapshot);
  vi.mocked(api.deleteBackupSnapshot).mockResolvedValue(undefined);
  vi.mocked(api.restoreBackupSnapshot).mockResolvedValue({
    safetyBackup: "backup_safety",
    restoredFiles: 6,
    removedFiles: 1,
  });
  vi.mocked(api.setBackupSettings).mockResolvedValue({
    storeRoot: "C:/data",
    theme: "system",
    enabledAgents: null,
    agentOrder: null,
    closeToTray: true,
    proxy: { url: null, username: null },
    skillRepos: null,
    backup: settings,
    cloud: {
      enabled: false,
      autoUpload: false,
      autoDownload: false,
      uploadDelayHours: 1,
      baseUrl: "",
      username: "",
      remoteDir: "ccswitchx",
    },
  });
  vi.mocked(api.deleteBackupCategory).mockResolvedValue(1024);
  vi.mocked(api.revealBackupLocation).mockResolvedValue(undefined);
  vi.mocked(api.gitInit).mockResolvedValue({ ...healthyGit, initialized: true });
  vi.mocked(api.gitSetRemote).mockResolvedValue(healthyGit);
  vi.mocked(api.gitVerifyRemote).mockResolvedValue(healthyGit);
  vi.mocked(api.gitCommitNow).mockResolvedValue(healthyGit);
  vi.mocked(api.gitPushNow).mockResolvedValue(healthyGit);
  vi.mocked(api.gitPullNow).mockResolvedValue({ kind: "upToDate" });
  vi.mocked(api.gitRestoreCommit).mockResolvedValue({
    safetyBackup: "backup_safety",
    commit: "abcdefabcdefabcdefabcdefabcdefabcdefabcd",
    files: 2,
  });
  vi.mocked(api.gitResolveConflict).mockResolvedValue({
    ...healthyGit,
    state: "idle",
    conflictFiles: [],
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  eventBridge.handlers.clear();
  mockBase();
});

test("shows snapshots, usage and git history", async () => {
  render(<BackupsPanel />);

  expect(await screen.findByText(snapshot.name)).toBeInTheDocument();
  expect(screen.getByText("手动")).toBeInTheDocument();
  expect(screen.getByText("合计 3.5 KB")).toBeInTheDocument();
  expect(screen.getByText("git@github.com:me/assets.git")).toBeInTheDocument();
  expect(await screen.findByText("backup: test")).toBeInTheDocument();
  expect(screen.getByText("已就绪")).toBeInTheDocument();
});

test("creates a snapshot from the header action", async () => {
  render(<BackupsPanel />);
  await screen.findByText(snapshot.name);
  fireEvent.click(screen.getByRole("button", { name: "立即备份" }));
  await waitFor(() => expect(api.createBackupSnapshot).toHaveBeenCalled());
});

test("restore requires confirmation", async () => {
  render(<BackupsPanel />);
  await screen.findByText(snapshot.name);
  fireEvent.click(screen.getAllByRole("button", { name: "恢复" })[0]);
  const dialog = await screen.findByRole("alertdialog", { name: "恢复到此快照？" });
  expect(api.restoreBackupSnapshot).not.toHaveBeenCalled();
  fireEvent.click(within(dialog).getByRole("button", { name: "恢复" }));
  await waitFor(() =>
    expect(api.restoreBackupSnapshot).toHaveBeenCalledWith(snapshot.name),
  );
});

test("renames and deletes with confirmation", async () => {
  render(<BackupsPanel />);
  await screen.findByText(snapshot.name);

  fireEvent.click(screen.getByRole("button", { name: "重命名" }));
  const input = await screen.findByLabelText("快照名称");
  fireEvent.change(input, { target: { value: "before-release" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.renameBackupSnapshot).toHaveBeenCalledWith(
      snapshot.name,
      "before-release",
    ),
  );

  fireEvent.click(screen.getByRole("button", { name: "删除" }));
  const dialog = await screen.findByRole("alertdialog", { name: "删除快照？" });
  fireEvent.click(within(dialog).getByRole("button", { name: "删除" }));
  await waitFor(() =>
    expect(api.deleteBackupSnapshot).toHaveBeenCalledWith(snapshot.name),
  );
});

test("saves retention policy", async () => {
  render(<BackupsPanel />);
  await screen.findByText(snapshot.name);
  fireEvent.change(screen.getByLabelText("保留份数"), { target: { value: "5" } });
  await waitFor(() =>
    expect(api.setBackupSettings).toHaveBeenCalledWith({
      ...settings,
      retainCount: 5,
    }),
  );
});

test("remote backup method is mutually exclusive", async () => {
  render(<BackupsPanel />);
  await screen.findByText(snapshot.name);

  fireEvent.click(screen.getByRole("button", { name: "不启用" }));
  await waitFor(() =>
    expect(api.setBackupSettings).toHaveBeenCalledWith({
      ...settings,
      git: { ...settings.git, enabled: false },
    }),
  );
});

test("enables webdav through the method selector", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "WebDAV 云备份" }));
  await waitFor(() =>
    expect(api.setCloudSettings).toHaveBeenCalledWith({
      baseUrl: cloudView.baseUrl,
      username: cloudView.username,
      password: null,
      remoteDir: cloudView.remoteDir,
      enabled: true,
      autoUpload: false,
      autoDownload: false,
      uploadDelayHours: 1,
    }),
  );
});

test("toggles auto upload and auto download independently", async () => {
  vi.mocked(api.getCloudSettings).mockResolvedValue({ ...cloudView, enabled: true });
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");

  fireEvent.click(screen.getByLabelText("自动上传"));
  await waitFor(() =>
    expect(api.setCloudSettings).toHaveBeenCalledWith({
      baseUrl: cloudView.baseUrl,
      username: cloudView.username,
      password: null,
      remoteDir: cloudView.remoteDir,
      enabled: true,
      autoUpload: true,
      autoDownload: false,
      uploadDelayHours: 1,
    }),
  );

  fireEvent.click(screen.getByLabelText("自动下载"));
  await waitFor(() =>
    expect(api.setCloudSettings).toHaveBeenCalledWith({
      baseUrl: cloudView.baseUrl,
      username: cloudView.username,
      password: null,
      remoteDir: cloudView.remoteDir,
      enabled: true,
      autoUpload: false,
      autoDownload: true,
      uploadDelayHours: 1,
    }),
  );
});

test("changes the upload delay", async () => {
  vi.mocked(api.getCloudSettings).mockResolvedValue({ ...cloudView, enabled: true });
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");

  fireEvent.change(screen.getByLabelText("上传延迟"), { target: { value: "6" } });
  await waitFor(() =>
    expect(api.setCloudSettings).toHaveBeenCalledWith({
      baseUrl: cloudView.baseUrl,
      username: cloudView.username,
      password: null,
      remoteDir: cloudView.remoteDir,
      enabled: true,
      autoUpload: false,
      autoDownload: false,
      uploadDelayHours: 6,
    }),
  );
});

test("auto upload needs an enabled channel first", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  expect(screen.getByLabelText("自动上传")).toBeDisabled();
  expect(screen.getByLabelText("自动下载")).toBeDisabled();
});

test("disables git backup until the remote is verified", async () => {
  vi.mocked(api.getGitStatus).mockResolvedValue({
    ...healthyGit,
    remoteVerified: false,
  });
  render(<BackupsPanel />);
  expect(await screen.findByText("远端未验证")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Git 备份" })).toBeDisabled();
});

test("verifies the filled remote before enabling git backup", async () => {
  vi.mocked(api.getGitStatus).mockResolvedValue({
    ...healthyGit,
    remoteVerified: false,
  });
  render(<BackupsPanel />);
  await screen.findByText("远端未验证");
  fireEvent.click(screen.getByRole("button", { name: "验证远端" }));
  await waitFor(() =>
    expect(api.gitVerifyRemote).toHaveBeenCalledWith(
      "git@github.com:me/assets.git",
      "main",
    ),
  );
  expect(
    await screen.findByText("远端验证通过，可以启用 Git 备份"),
  ).toBeInTheDocument();
});

test("disables git actions when git is unavailable", async () => {
  vi.mocked(api.getGitStatus).mockResolvedValue({
    ...healthyGit,
    available: false,
    initialized: false,
    state: "unavailable",
  });
  render(<BackupsPanel />);
  expect(await screen.findByText(/未检测到 Git/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "初始化 Git 备份" })).toBeNull();
});

test("conflict resolution must be confirmed before force push", async () => {
  vi.mocked(api.getGitStatus).mockResolvedValue({
    ...healthyGit,
    state: "paused",
    conflictFiles: ["assets/mcp/pw.json"],
    lastError: "拉取冲突",
  });
  render(<BackupsPanel />);
  await screen.findByText(/自动同步已暂停/);
  fireEvent.click(screen.getByRole("button", { name: "保留本地（强推）" }));
  const dialog = await screen.findByRole("alertdialog", {
    name: "保留本地并覆盖远端？",
  });
  expect(api.gitResolveConflict).not.toHaveBeenCalled();
  fireEvent.click(within(dialog).getByRole("button", { name: "保留本地" }));
  await waitFor(() =>
    expect(api.gitResolveConflict).toHaveBeenCalledWith("keepLocal"),
  );
});

test("renders cloud backup section and remote status", async () => {
  render(<BackupsPanel />);
  expect(await screen.findByText("云备份（WebDAV）")).toBeInTheDocument();
  expect(screen.getByText(/ccswitchx\/ccswitchx-backup\.zip/)).toBeInTheDocument();
  expect(screen.getByText("云端有备份")).toBeInTheDocument();
  // manifest 描述上传来源：换机时先看清是哪台机器、哪个版本传的包。
  expect(screen.getByText("DESKTOP-TEST · v0.10.0")).toBeInTheDocument();
  // 自动下载与本月上传统计也要在状态区可见。
  expect(screen.getByText("上次下载")).toBeInTheDocument();
  expect(screen.getByText("本机本月上传")).toBeInTheDocument();
  // WebDAV 是主用的远端通道，区块排在快照之前。
  const cloudHeading = screen.getByText("云备份（WebDAV）");
  const snapshotHeading = screen.getByText("快照", { selector: "h3" });
  expect(
    cloudHeading.compareDocumentPosition(snapshotHeading) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});

test("renders cached cloud status before the live probe resolves", async () => {
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({ ...cloudStatus, fromCache: true });
  vi.mocked(api.getCloudStatus).mockReturnValue(new Promise(() => {}));
  render(<BackupsPanel />);
  expect(await screen.findByText("云端有备份")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeEnabled();
});

test("keeps restore enabled while the live status is still loading", async () => {
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({ ...cloudStatus, fromCache: true });
  vi.mocked(api.getCloudStatus).mockReturnValue(new Promise(() => {}));
  render(<BackupsPanel />);
  await screen.findByText("云端有备份");
  // 实时探测没回来不算「已知不可恢复」，恢复按钮不该被检查中卡住。
  expect(screen.getByText("检查中…")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeEnabled();
});

test("refreshes cloud status after auto sync event", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  await waitFor(() =>
    expect(eventBridge.handlers.get("cloud-auto-synced")).toBeTruthy(),
  );

  const calls = vi.mocked(api.getCloudStatus).mock.calls.length;
  act(() =>
    eventBridge.handlers.get("cloud-auto-synced")?.({
      payload: {
        kind: "download",
        restoredFiles: 3,
        removedFiles: 1,
        safetyBackup: "backup_auto",
      },
    }),
  );

  await waitFor(() =>
    expect(vi.mocked(api.getCloudStatus).mock.calls.length).toBeGreaterThan(calls),
  );
  expect(
    await screen.findByText(
      "已自动从云端恢复 3 个文件，删除 1 个；安全快照：backup_auto",
    ),
  ).toBeInTheDocument();
});

test("starts the cached cloud probe in parallel with local data", async () => {
  // 本地批次永不返回：缓存请求仍应在挂载后立刻发出，而不是等本地数据齐了才发。
  vi.mocked(api.getCloudSettings).mockReturnValue(new Promise(() => {}));
  render(<BackupsPanel />);
  await waitFor(() => expect(api.getCloudStatusCached).toHaveBeenCalled());
});

test("warns when the remote manifest is incompatible", async () => {
  vi.mocked(api.getCloudStatus).mockResolvedValue({
    ...cloudStatus,
    manifest: { ...cloudStatus.manifest!, schema: 99, compatible: false },
  });
  render(<BackupsPanel />);
  expect(await screen.findByText(/请从源设备重新上传/)).toBeInTheDocument();
});

test("blocks restore when the remote manifest is missing", async () => {
  vi.mocked(api.getCloudStatus).mockResolvedValue({ ...cloudStatus, manifest: null });
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({
    ...cloudStatus,
    manifest: null,
    fromCache: true,
  });
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
});

test("shows the last auto download error", async () => {
  vi.mocked(api.getCloudStatus).mockResolvedValue({
    ...cloudStatus,
    lastDownloadError: "下载校验失败",
  });
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({
    ...cloudStatus,
    lastDownloadError: "下载校验失败",
    fromCache: true,
  });
  render(<BackupsPanel />);
  expect(
    await screen.findByText("上次自动下载失败：下载校验失败"),
  ).toBeInTheDocument();
});

test("saves cloud settings without overwriting the saved password", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.change(screen.getByLabelText("WebDAV 地址"), {
    target: { value: "https://dav.example.com/dav" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存云备份" }));
  await waitFor(() =>
    expect(api.setCloudSettings).toHaveBeenCalledWith({
      baseUrl: "https://dav.example.com/dav",
      username: "me",
      password: null,
      remoteDir: "ccswitchx",
      enabled: false,
      autoUpload: false,
      autoDownload: false,
      uploadDelayHours: 1,
    }),
  );
});

test("tests the cloud connection", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
  await waitFor(() => expect(api.testCloudConnection).toHaveBeenCalled());
  expect(await screen.findByText(/连接成功（42 ms）/)).toBeInTheDocument();
});

test("surfaces the cloud test failure in the pinned notice band", async () => {
  vi.mocked(api.testCloudConnection).mockRejectedValueOnce(
    "WebDAV MKCOL 请求失败：error sending request",
  );
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "测试连接" }));

  // 失败提示在滚动区之外：面板滚到页底的 WebDAV 区时，它也不会被滚出可视区。
  const banner = await screen.findByText(/WebDAV MKCOL 请求失败/);
  expect(banner.closest(".overflow-auto")).toBeNull();
});

test("uploads and restores only after confirmation", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "立即上传" }));
  await waitFor(() => expect(api.cloudUploadNow).toHaveBeenCalled());

  fireEvent.click(screen.getByRole("button", { name: "从云端恢复" }));
  const dialog = await screen.findByRole("alertdialog", { name: "从云端恢复？" });
  expect(api.cloudRestoreLatest).not.toHaveBeenCalled();
  // 弹框同时会去云端确认最新状态，确认按钮要等这次刷新结束才可用。
  const confirm = within(dialog).getByRole("button", { name: "恢复" });
  await waitFor(() => expect(confirm).toBeEnabled());
  fireEvent.click(confirm);
  await waitFor(() => expect(api.cloudRestoreLatest).toHaveBeenCalled());
});

test("shows the cloud backup identity before restoring", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "从云端恢复" }));
  const dialog = await screen.findByRole("alertdialog", { name: "从云端恢复？" });

  // 恢复前先看清远端包是哪台机器、什么时候传的、多大，避免拿旧包覆盖真源。
  expect(within(dialog).getByText("DESKTOP-TEST · v0.10.0")).toBeInTheDocument();
  expect(within(dialog).getByText("5 分钟前")).toBeInTheDocument();
  expect(within(dialog).getByText("12 个")).toBeInTheDocument();
  expect(within(dialog).getByText("2.0 KB")).toBeInTheDocument();
  expect(api.cloudRestoreLatest).not.toHaveBeenCalled();
});

test("blocks upload and restore while the cloud form has unsaved changes", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.change(screen.getByLabelText("WebDAV 地址"), {
    target: { value: "https://other.example.com/dav" },
  });

  // 上传与恢复走的是已保存的连接，草稿没保存时不能让人以为操作的是新地址。
  expect(screen.getByText(/配置有未保存的更改/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "立即上传" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
});

test("disables restore when the remote backup is incompatible", async () => {
  vi.mocked(api.getCloudStatus).mockResolvedValue({
    ...cloudStatus,
    manifest: { ...cloudStatus.manifest!, schema: 99, compatible: false },
  });
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({
    ...cloudStatus,
    manifest: { ...cloudStatus.manifest!, schema: 99, compatible: false },
    fromCache: true,
  });
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
});

test("disables restore when the remote has no backup", async () => {
  vi.mocked(api.getCloudStatus).mockResolvedValue({
    ...cloudStatus,
    remote: { ...cloudStatus.remote!, exists: false },
  });
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({
    ...cloudStatus,
    remote: { ...cloudStatus.remote!, exists: false },
    fromCache: true,
  });
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
});

test("shows download progress while restoring", async () => {
  vi.mocked(api.cloudRestoreLatest).mockImplementation(
    () => new Promise(() => {}),
  );
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "从云端恢复" }));
  const dialog = await screen.findByRole("alertdialog", { name: "从云端恢复？" });
  const confirm = within(dialog).getByRole("button", { name: "恢复" });
  await waitFor(() => expect(confirm).toBeEnabled());
  fireEvent.click(confirm);

  await waitFor(() =>
    expect(eventBridge.handlers.get("cloud-restore-progress")).toBeTruthy(),
  );
  act(() =>
    eventBridge.handlers.get("cloud-restore-progress")?.({
      payload: { received: 512, total: 2048 },
    }),
  );

  expect(await within(dialog).findByText(/512 B/)).toBeInTheDocument();
  expect(within(dialog).getByText(/25%/)).toBeInTheDocument();
});

test("disables cloud actions without config", async () => {
  vi.mocked(api.getCloudSettings).mockResolvedValue({
    enabled: false,
    autoUpload: false,
    autoDownload: false,
    uploadDelayHours: 1,
    baseUrl: "",
    username: "",
    remoteDir: "ccswitchx",
    hasPassword: false,
  });
  const unconfigured: CloudStatus = {
    configured: false,
    hasPassword: false,
    autoUpload: false,
    autoDownload: false,
    uploadDelayHours: 1,
    remote: null,
    remoteError: null,
    manifest: null,
    manifestError: null,
    lastUploadAt: null,
    lastDownloadAt: null,
    lastError: null,
    lastDownloadError: null,
    uploadedBytesMonth: 0,
    fromCache: false,
  };
  vi.mocked(api.getCloudStatus).mockResolvedValue(unconfigured);
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({
    ...unconfigured,
    fromCache: true,
  });
  render(<BackupsPanel />);
  expect(await screen.findByText("未配置")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "立即上传" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "WebDAV 云备份" })).toBeDisabled();
});
