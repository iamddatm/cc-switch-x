import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { BackupsPanel } from "../panels/BackupsPanel";
import { api } from "../lib/api";
import type { BackupSettings, GitStatus } from "../types";

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
