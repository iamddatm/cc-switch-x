import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { BackupsPanel } from "../panels/BackupsPanel";
import { api } from "../lib/api";
import type {
  BackupSettings,
  CloudSettingsView,
  CloudStatus,
  GitStatus,
} from "../types";

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
  autoUpload: false,
  baseUrl: "https://dav.example.com/dav",
  username: "me",
  remoteDir: "ccswitchx",
  hasPassword: true,
};

const cloudStatus: CloudStatus = {
  configured: true,
  hasPassword: true,
  autoUpload: false,
  remote: {
    exists: true,
    sizeBytes: 2048,
    modifiedAt: Date.now() - 300_000,
    etag: '"abc"',
  },
  remoteError: null,
  lastUploadAt: Date.now() - 300_000,
  lastError: null,
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
    closeToTray: true,
    proxy: { url: null, username: null },
    skillRepos: null,
    backup: settings,
    cloud: { autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx" },
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
      autoUpload: true,
    }),
  );
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
      autoUpload: false,
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

test("uploads and restores only after confirmation", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "立即上传" }));
  await waitFor(() => expect(api.cloudUploadNow).toHaveBeenCalled());

  fireEvent.click(screen.getByRole("button", { name: "从云端恢复" }));
  const dialog = await screen.findByRole("alertdialog", { name: "从云端恢复？" });
  expect(api.cloudRestoreLatest).not.toHaveBeenCalled();
  fireEvent.click(within(dialog).getByRole("button", { name: "恢复" }));
  await waitFor(() => expect(api.cloudRestoreLatest).toHaveBeenCalled());
});

test("disables cloud actions without config", async () => {
  vi.mocked(api.getCloudSettings).mockResolvedValue({
    autoUpload: false,
    baseUrl: "",
    username: "",
    remoteDir: "ccswitchx",
    hasPassword: false,
  });
  vi.mocked(api.getCloudStatus).mockResolvedValue({
    configured: false,
    hasPassword: false,
    autoUpload: false,
    remote: null,
    remoteError: null,
    lastUploadAt: null,
    lastError: null,
  });
  render(<BackupsPanel />);
  expect(await screen.findByText("未配置")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "立即上传" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "WebDAV 云备份" })).toBeDisabled();
});
