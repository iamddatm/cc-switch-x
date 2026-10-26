import { useEffect, useRef, useState } from "react";
import {
  ArchiveRestore,
  Cloud,
  Eye,
  EyeOff,
  GitBranch,
  HardDrive,
  RefreshCw,
} from "lucide-react";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Checkbox } from "../components/ui/Checkbox";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Dialog } from "../components/ui/Dialog";
import { EmptyState } from "../components/ui/EmptyState";
import { Field } from "../components/ui/Field";
import { IconButton } from "../components/ui/IconButton";
import { Input } from "../components/ui/Input";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { Select } from "../components/ui/Select";
import { useBackups } from "../hooks/useBackups";
import { api } from "../lib/api";
import { formatBytes, formatRelativeTime } from "../lib/format";
import type {
  BackupSettings,
  CloudStatus,
  GitCommit,
  GitStatus,
  SnapshotKind,
} from "../types";

const INTERVALS = [0, 6, 12, 24, 48, 168];
const RETAINS = [3, 5, 10, 15, 20, 30, 50];

const KIND_LABEL: Record<SnapshotKind, string> = {
  manual: "手动",
  auto: "自动",
  preRestore: "恢复前",
  prePull: "拉取前",
};

const USAGE_LABEL: Record<string, string> = {
  snapshots: "快照",
  writes: "写入备份",
  logs: "日志",
  index: "索引",
};

function withCurrent(options: number[], current: number): number[] {
  return options.includes(current)
    ? options
    : [...options, current].sort((left, right) => left - right);
}

function gitStateTone(git: GitStatus): "neutral" | "success" | "warning" | "danger" {
  switch (git.state) {
    case "idle":
      return "success";
    case "paused":
      return "warning";
    case "error":
      return "danger";
    default:
      return "neutral";
  }
}

function gitStateLabel(git: GitStatus): string {
  switch (git.state) {
    case "unavailable":
      return "未安装 Git";
    case "disabled":
      return git.initialized ? "已停用" : "未初始化";
    case "idle":
      return "已就绪";
    case "syncing":
      return "同步中";
    case "paused":
      return "冲突暂停";
    case "error":
      return "上次同步失败";
  }
}

function cloudState(status: CloudStatus | null): {
  label: string;
  tone: "neutral" | "success" | "warning" | "danger";
} {
  if (!status?.configured) return { label: "未配置", tone: "neutral" };
  if (status.remoteError) return { label: "连接失败", tone: "danger" };
  if (status.remote?.exists) return { label: "云端有备份", tone: "success" };
  return { label: "已配置", tone: "neutral" };
}

export function BackupsPanel() {
  const {
    snapshots,
    usage,
    settings,
    git,
    commits,
    cloud,
    cloudStatus,
    loading,
    busy,
    error,
    refresh,
    refreshCloudStatus,
    run,
  } = useBackups();
  const [notice, setNotice] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<string | null>(null);
  const [cleanupTarget, setCleanupTarget] = useState<"writes" | "logs" | null>(null);
  const [gitRestoreTarget, setGitRestoreTarget] = useState<GitCommit | null>(null);
  const [conflictAction, setConflictAction] = useState<"keepLocal" | "keepRemote" | null>(
    null,
  );
  const [remoteInput, setRemoteInput] = useState("");
  const [branchInput, setBranchInput] = useState("main");
  const [cloudBaseUrl, setCloudBaseUrl] = useState("");
  const [cloudUsername, setCloudUsername] = useState("");
  const [cloudPassword, setCloudPassword] = useState("");
  const [cloudPasswordDirty, setCloudPasswordDirty] = useState(false);
  const [showCloudPassword, setShowCloudPassword] = useState(false);
  const [cloudRemoteDir, setCloudRemoteDir] = useState("ccswitchx");
  const [cloudAutoUpload, setCloudAutoUpload] = useState(false);
  const [cloudRestoreOpen, setCloudRestoreOpen] = useState(false);
  const inputsReady = useRef(false);
  const cloudInputsReady = useRef(false);

  useEffect(() => {
    if (inputsReady.current || !settings) return;
    setRemoteInput(settings.git.remote ?? "");
    setBranchInput(settings.git.branch);
    inputsReady.current = true;
  }, [settings]);

  useEffect(() => {
    if (cloudInputsReady.current || !cloud) return;
    setCloudBaseUrl(cloud.baseUrl);
    setCloudUsername(cloud.username);
    setCloudRemoteDir(cloud.remoteDir);
    setCloudAutoUpload(cloud.autoUpload);
    cloudInputsReady.current = true;
  }, [cloud]);

  const act = async (key: string, action: () => Promise<unknown>, success?: string) => {
    setNotice(null);
    try {
      await run(key, action);
      if (success) setNotice(success);
    } catch {
      // hook 已把错误写进 error
    }
  };

  const savePolicy = (patch: Partial<Pick<BackupSettings, "intervalHours" | "retainCount">>) => {
    if (!settings) return;
    void act(
      "policy",
      () => api.setBackupSettings({ ...settings, ...patch }),
      "备份策略已保存",
    );
  };

  const saveGitToggle = (patch: Partial<BackupSettings["git"]>) => {
    if (!settings) return;
    void act("git-settings", () =>
      api.setBackupSettings({
        ...settings,
        git: { ...settings.git, ...patch },
      }),
    );
  };

  const saveRemote = () => {
    void act(
      "git-remote",
      () =>
        api.gitSetRemote(
          remoteInput.trim() ? remoteInput.trim() : null,
          branchInput.trim() || "main",
        ),
      "远端设置已保存",
    );
  };

  const createSnapshot = () => {
    void act(
      "create",
      async () => {
        const info = await api.createBackupSnapshot();
        setNotice(`已创建快照：${info.name}`);
      },
    );
  };

  const confirmRename = async () => {
    if (!renameTarget || !renameValue.trim()) return;
    setNotice(null);
    try {
      await run("rename", () =>
        api.renameBackupSnapshot(renameTarget, renameValue.trim()),
      );
      setNotice(`快照已重命名为「${renameValue.trim()}」`);
      setRenameTarget(null);
    } catch {
      // 错误已展示
    }
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    setNotice(null);
    try {
      await run("delete", () => api.deleteBackupSnapshot(deleteTarget));
      setNotice(`已删除快照「${deleteTarget}」`);
      setDeleteTarget(null);
    } catch {
      // 错误已展示
    }
  };

  const confirmRestore = async () => {
    if (!restoreTarget) return;
    setNotice(null);
    try {
      const report = await run("restore", () =>
        api.restoreBackupSnapshot(restoreTarget),
      );
      setNotice(
        `已恢复 ${report.restoredFiles} 个文件，删除 ${report.removedFiles} 项；安全快照：${report.safetyBackup}`,
      );
      setRestoreTarget(null);
    } catch {
      // 错误已展示
    }
  };

  const confirmCleanup = async () => {
    if (!cleanupTarget) return;
    setNotice(null);
    try {
      const freed = await run("cleanup", () =>
        api.deleteBackupCategory(cleanupTarget),
      );
      setNotice(`已清理，释放 ${formatBytes(freed)}`);
      setCleanupTarget(null);
    } catch {
      // 错误已展示
    }
  };

  const pullNow = async () => {
    setNotice(null);
    try {
      const outcome = await run("pull", () => api.gitPullNow());
      if (outcome.kind === "pulled") {
        setNotice(
          `已拉取 ${outcome.commits} 个提交；安全快照：${outcome.safetyBackup}`,
        );
      } else if (outcome.kind === "conflict") {
        setNotice(`拉取冲突：${outcome.files.join("、")}`);
      } else if (outcome.kind === "noRemote") {
        setNotice("远端还没有这个分支，先推送一次即可");
      } else {
        setNotice("远端没有新提交");
      }
    } catch {
      // 错误已展示
    }
  };

  const confirmGitRestore = async () => {
    if (!gitRestoreTarget) return;
    setNotice(null);
    try {
      const report = await run("git-restore", () =>
        api.gitRestoreCommit(gitRestoreTarget.hash),
      );
      setNotice(
        `已恢复到 ${gitRestoreTarget.shortHash}，新提交 ${report.commit.slice(0, 8)}；安全快照：${report.safetyBackup}`,
      );
      setGitRestoreTarget(null);
    } catch {
      // 错误已展示
    }
  };

  const confirmConflict = async () => {
    if (!conflictAction) return;
    setNotice(null);
    try {
      await run("conflict", () => api.gitResolveConflict(conflictAction));
      setNotice(
        conflictAction === "keepLocal"
          ? "已保留本地并覆盖远端"
          : "已用远端覆盖本地；恢复前快照保留在快照列表里",
      );
      setConflictAction(null);
    } catch {
      // 错误已展示
    }
  };

  const cloudPasswordValue = () => (cloudPasswordDirty ? cloudPassword : null);

  const saveCloud = async () => {
    setNotice(null);
    try {
      const saved = await run("cloud-save", () =>
        api.setCloudSettings({
          baseUrl: cloudBaseUrl.trim(),
          username: cloudUsername.trim(),
          password: cloudPasswordValue(),
          remoteDir: cloudRemoteDir.trim(),
          autoUpload: cloudAutoUpload,
        }),
      );
      setCloudBaseUrl(saved.baseUrl);
      setCloudUsername(saved.username);
      setCloudRemoteDir(saved.remoteDir);
      setCloudAutoUpload(saved.autoUpload);
      setCloudPassword("");
      setCloudPasswordDirty(false);
      setShowCloudPassword(false);
      setNotice("云备份设置已保存");
    } catch {
      // 错误已展示
    }
  };

  const testCloud = async () => {
    setNotice(null);
    try {
      const latency = await run("cloud-test", () =>
        api.testCloudConnection({
          baseUrl: cloudBaseUrl.trim(),
          username: cloudUsername.trim(),
          password: cloudPasswordValue(),
          remoteDir: cloudRemoteDir.trim(),
        }),
      );
      setNotice(`连接成功（${latency} ms）`);
    } catch {
      // 错误已展示
    }
  };

  const uploadCloud = async () => {
    setNotice(null);
    try {
      const status = await run("cloud-upload", () => api.cloudUploadNow());
      const size = status.remote?.sizeBytes;
      setNotice(
        size != null
          ? `已上传云端备份（${formatBytes(size)}）`
          : "已上传云端备份",
      );
    } catch {
      // 错误已展示
    }
  };

  const confirmCloudRestore = async () => {
    setNotice(null);
    try {
      const report = await run("cloud-restore", () => api.cloudRestoreLatest());
      setCloudRestoreOpen(false);
      setNotice(
        `已从云端恢复 ${report.restoredFiles} 个文件，删除 ${report.removedFiles} 个；安全快照：${report.safetyBackup}`,
      );
    } catch {
      // 错误已展示
    }
  };

  const interval = settings?.intervalHours ?? 24;
  const retain = settings?.retainCount ?? 10;
  const cloudBadge = cloudState(cloudStatus);
  const cloudRemotePath = cloudBaseUrl.trim()
    ? `${cloudBaseUrl.trim().replace(/\/+$/, "")}/${cloudRemoteDir
        .trim()
        .replace(/^\/+/, "")}/ccswitchx-backup.zip`
    : "—";

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        icon={ArchiveRestore}
        title="备份"
        description="快照可跨机器恢复；Git 备份保留提交历史并可同步远端。"
        actions={
          <>
            <Button
              variant="primary"
              onClick={createSnapshot}
              loading={busy === "create"}
              disabled={!!busy && busy !== "create"}
            >
              立即备份
            </Button>
            <Button onClick={() => void refresh()} loading={loading}>
              刷新
            </Button>
          </>
        }
      />
      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">
        <div className="max-w-4xl space-y-6">
          {error && <Notice tone="danger">{error}</Notice>}
          {notice && <Notice tone="success">{notice}</Notice>}

          <section className="rounded-panel border border-border bg-surface">
            <div className="flex items-center gap-2 border-b border-border px-4 py-3">
              <ArchiveRestore size={15} className="text-fg-3" aria-hidden />
              <h3 className="text-strong text-fg-1">快照</h3>
              <span className="text-caption text-fg-3">
                只包含 assets/、agents/ 与 .gitignore
              </span>
            </div>

            <div className="flex flex-wrap items-end gap-3 border-b border-border px-4 py-3">
              <Field label="自动备份间隔" htmlFor="backup-interval" className="w-40">
                <Select
                  id="backup-interval"
                  value={String(interval)}
                  disabled={!settings || busy === "policy"}
                  onChange={(event) =>
                    savePolicy({ intervalHours: Number(event.target.value) })
                  }
                >
                  {withCurrent(INTERVALS, interval).map((hours) => (
                    <option key={hours} value={hours}>
                      {hours === 0 ? "关闭" : `${hours} 小时`}
                    </option>
                  ))}
                </Select>
              </Field>
              <Field label="保留份数" htmlFor="backup-retain" className="w-32">
                <Select
                  id="backup-retain"
                  value={String(retain)}
                  disabled={!settings || busy === "policy"}
                  onChange={(event) =>
                    savePolicy({ retainCount: Number(event.target.value) })
                  }
                >
                  {withCurrent(RETAINS, retain).map((count) => (
                    <option key={count} value={count}>
                      {count}
                    </option>
                  ))}
                </Select>
              </Field>
              <p className="pb-1.5 text-caption text-fg-3">
                间隔为 0 时关闭自动快照；清理只作用于 snapshots 目录。
              </p>
            </div>

            {snapshots.length === 0 ? (
              <EmptyState
                icon={ArchiveRestore}
                title="还没有快照"
                description="备份会打包可移植真源，恢复前还会自动创建安全快照。"
                actions={
                  <Button variant="primary" onClick={createSnapshot}>
                    创建第一份快照
                  </Button>
                }
              />
            ) : (
              <div className="max-h-[360px] overflow-auto">
                {snapshots.map((snapshot) => (
                  <div
                    key={snapshot.name}
                    className="flex items-center gap-3 border-t border-border px-4 py-2.5 first:border-t-0"
                  >
                    <Badge tone="neutral">{KIND_LABEL[snapshot.kind]}</Badge>
                    <div className="min-w-0">
                      <p className="truncate text-body text-fg-1">{snapshot.name}</p>
                      <p className="text-caption text-fg-3">
                        {formatRelativeTime(snapshot.createdAt)} ·{" "}
                        {formatBytes(snapshot.sizeBytes)} · {snapshot.files} 个文件
                      </p>
                    </div>
                    <div className="ml-auto flex shrink-0 gap-1.5">
                      <Button
                        size="sm"
                        variant="primary"
                        disabled={!!busy}
                        onClick={() => setRestoreTarget(snapshot.name)}
                      >
                        恢复
                      </Button>
                      <Button
                        size="sm"
                        disabled={!!busy}
                        onClick={() => {
                          setRenameValue(snapshot.name);
                          setRenameTarget(snapshot.name);
                        }}
                      >
                        重命名
                      </Button>
                      <Button
                        size="sm"
                        variant="danger"
                        disabled={!!busy}
                        onClick={() => setDeleteTarget(snapshot.name)}
                      >
                        删除
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>

          <section className="rounded-panel border border-border bg-surface">
            <div className="flex items-center gap-2 border-b border-border px-4 py-3">
              <HardDrive size={15} className="text-fg-3" aria-hidden />
              <h3 className="text-strong text-fg-1">占用空间</h3>
              {usage && (
                <span className="text-caption text-fg-3">
                  合计 {formatBytes(usage.totalBytes)}
                </span>
              )}
            </div>
            {usage ? (
              <>
                <div className="grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-4">
                  {usage.sections.map((section) => (
                    <div key={section.id} title={section.path}>
                      <p className="text-caption text-fg-3">
                        {USAGE_LABEL[section.id] ?? section.id}
                      </p>
                      <p className="text-body text-fg-1">
                        {formatBytes(section.bytes)}
                      </p>
                      <p className="text-caption text-fg-3">{section.items} 项</p>
                    </div>
                  ))}
                </div>
                <div className="flex flex-wrap items-center gap-2 border-t border-border px-4 py-3">
                  <Button
                    size="sm"
                    onClick={() =>
                      void act("reveal", () => api.revealBackupLocation("writes"))
                    }
                  >
                    打开备份目录
                  </Button>
                  <Button
                    size="sm"
                    onClick={() =>
                      void act("reveal-store", () => api.revealBackupLocation("store"))
                    }
                  >
                    打开存储目录
                  </Button>
                  <div className="ml-auto flex gap-2">
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={!!busy}
                      onClick={() => setCleanupTarget("writes")}
                    >
                      清理写入备份
                    </Button>
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={!!busy}
                      onClick={() => setCleanupTarget("logs")}
                    >
                      清理日志
                    </Button>
                  </div>
                </div>
              </>
            ) : (
              <p className="px-4 py-3 text-caption text-fg-3">正在统计占用…</p>
            )}
          </section>

          <section className="rounded-panel border border-border bg-surface">
            <div className="flex items-center gap-2 border-b border-border px-4 py-3">
              <GitBranch size={15} className="text-fg-3" aria-hidden />
              <h3 className="text-strong text-fg-1">Git 备份</h3>
              {git && <Badge tone={gitStateTone(git)}>{gitStateLabel(git)}</Badge>}
              <span className="ml-auto flex gap-1.5">
                <Button
                  size="sm"
                  onClick={() => void refresh()}
                  loading={loading}
                  title="刷新状态"
                >
                  <RefreshCw size={13} aria-hidden />
                </Button>
              </span>
            </div>

            {!git ? (
              <p className="px-4 py-3 text-caption text-fg-3">正在读取 Git 状态…</p>
            ) : !git.available ? (
              <div className="px-4 py-3">
                <Notice tone="warning">
                  未检测到 Git。请安装 Git 并加入 PATH；其余备份功能不受影响。
                </Notice>
              </div>
            ) : !git.initialized ? (
              <div className="space-y-3 px-4 py-3">
                <p className="text-body text-fg-2">
                  在存储目录初始化 Git 仓库，之后每次写操作自动提交；配置远端后还会推送。
                </p>
                <Button
                  variant="primary"
                  loading={busy === "git-init"}
                  onClick={() =>
                    void act("git-init", () => api.gitInit(), "Git 备份已初始化")
                  }
                >
                  初始化 Git 备份
                </Button>
              </div>
            ) : (
              <>
                <div className="grid grid-cols-2 gap-3 border-b border-border px-4 py-3 sm:grid-cols-4">
                  <div>
                    <p className="text-caption text-fg-3">分支</p>
                    <p className="text-body text-fg-1">{git.branch ?? "—"}</p>
                  </div>
                  <div className="min-w-0">
                    <p className="text-caption text-fg-3">远端</p>
                    <p className="truncate text-body text-fg-1" title={git.remote ?? ""}>
                      {git.remote ?? "未配置"}
                    </p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">未提交</p>
                    <p className="text-body text-fg-1">{git.dirty} 个文件</p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">领先 / 落后</p>
                    <p className="text-body text-fg-1">
                      {git.ahead} / {git.behind}
                    </p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">上次提交</p>
                    <p className="text-body text-fg-1">
                      {git.lastCommitAt ? formatRelativeTime(git.lastCommitAt) : "—"}
                    </p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">上次拉取</p>
                    <p className="text-body text-fg-1">
                      {git.lastPullAt ? formatRelativeTime(git.lastPullAt) : "—"}
                    </p>
                  </div>
                </div>

                {git.lastError && git.conflictFiles.length === 0 && (
                  <div className="px-4 pt-3">
                    <Notice tone="danger">{git.lastError}</Notice>
                  </div>
                )}

                {git.conflictFiles.length > 0 && (
                  <div className="space-y-2 border-b border-border px-4 py-3">
                    <Notice tone="warning">
                      自动同步已暂停。冲突文件：{git.conflictFiles.join("、")}
                    </Notice>
                    <div className="flex flex-wrap gap-2">
                      <Button
                        size="sm"
                        variant="danger"
                        onClick={() => setConflictAction("keepLocal")}
                      >
                        保留本地（强推）
                      </Button>
                      <Button
                        size="sm"
                        variant="danger"
                        onClick={() => setConflictAction("keepRemote")}
                      >
                        保留远端（覆盖本地）
                      </Button>
                      <Button
                        size="sm"
                        onClick={() =>
                          void act("reveal", () => api.revealBackupLocation("store"))
                        }
                      >
                        打开仓库目录手动处理
                      </Button>
                    </div>
                  </div>
                )}

                <div className="space-y-3 border-b border-border px-4 py-3">
                  <div className="flex flex-wrap items-center gap-4">
                    <Checkbox
                      id="git-enabled"
                      label="启用自动同步"
                      checked={settings?.git.enabled ?? false}
                      onChange={(event) =>
                        saveGitToggle({ enabled: event.target.checked })
                      }
                    />
                    <Checkbox
                      id="git-auto-commit"
                      label="写操作后自动提交"
                      checked={settings?.git.autoCommit ?? true}
                      onChange={(event) =>
                        saveGitToggle({ autoCommit: event.target.checked })
                      }
                    />
                    <Checkbox
                      id="git-auto-pull"
                      label="启动/定时拉取"
                      checked={settings?.git.autoPull ?? true}
                      onChange={(event) =>
                        saveGitToggle({ autoPull: event.target.checked })
                      }
                    />
                  </div>
                  <div className="flex flex-wrap items-end gap-2">
                    <Field label="远端地址" htmlFor="git-remote" className="min-w-[280px] flex-1">
                      <Input
                        id="git-remote"
                        className="font-mono"
                        placeholder="git@github.com:me/assets.git 或 https://…"
                        value={remoteInput}
                        onChange={(event) => setRemoteInput(event.target.value)}
                      />
                    </Field>
                    <Field label="分支" htmlFor="git-branch" className="w-36">
                      <Input
                        id="git-branch"
                        className="font-mono"
                        value={branchInput}
                        onChange={(event) => setBranchInput(event.target.value)}
                      />
                    </Field>
                    <Button
                      className="mb-0.5"
                      loading={busy === "git-remote"}
                      onClick={saveRemote}
                    >
                      保存远端
                    </Button>
                  </div>
                  <p className="text-caption text-fg-3">
                    凭据交给系统 Git（credential helper / SSH agent），应用不会保存 token。
                  </p>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      loading={busy === "commit"}
                      onClick={() =>
                        void act("commit", () => api.gitCommitNow(), "已提交当前真源")
                      }
                    >
                      立即提交
                    </Button>
                    <Button loading={busy === "pull"} onClick={() => void pullNow()}>
                      立即拉取
                    </Button>
                    <Button
                      loading={busy === "push"}
                      onClick={() =>
                        void act("push", () => api.gitPushNow(), "已推送到远端")
                      }
                    >
                      立即推送
                    </Button>
                  </div>
                </div>

                <div className="border-b border-border px-4 py-3">
                  <h4 className="mb-2 text-caption font-medium text-fg-2">提交历史</h4>
                  {commits.length === 0 ? (
                    <p className="text-caption text-fg-3">还没有提交。</p>
                  ) : (
                    <div className="max-h-72 overflow-auto">
                      {commits.map((commit) => (
                        <div
                          key={commit.hash}
                          className="flex items-center gap-2 border-t border-border py-2 first:border-t-0"
                        >
                          <Badge mono>{commit.shortHash}</Badge>
                          <span className="min-w-0 flex-1 truncate text-body text-fg-1" title={commit.message}>
                            {commit.message}
                          </span>
                          <span className="shrink-0 text-caption text-fg-3">
                            {formatRelativeTime(commit.timestamp)}
                          </span>
                          <Button
                            size="sm"
                            disabled={!!busy}
                            onClick={() => setGitRestoreTarget(commit)}
                          >
                            恢复
                          </Button>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </>
            )}
          </section>

          <section className="rounded-panel border border-border bg-surface">
            <div className="flex items-center gap-2 border-b border-border px-4 py-3">
              <Cloud size={15} className="text-fg-3" aria-hidden />
              <h3 className="text-strong text-fg-1">云备份（WebDAV）</h3>
              <Badge tone={cloudBadge.tone}>{cloudBadge.label}</Badge>
              <span className="ml-auto flex gap-1.5">
                <IconButton
                  label="刷新云状态"
                  disabled={loading}
                  onClick={() => void refreshCloudStatus()}
                >
                  <RefreshCw size={13} aria-hidden />
                </IconButton>
              </span>
            </div>

            {!cloud ? (
              <p className="px-4 py-3 text-caption text-fg-3">正在读取云备份设置…</p>
            ) : (
              <>
                <div className="space-y-3 border-b border-border px-4 py-3">
                  {cloudBaseUrl.startsWith("http://") && (
                    <Notice tone="warning">
                      HTTP 会明文传输凭据，仅建议在内网使用。
                    </Notice>
                  )}
                  <div className="flex flex-wrap items-end gap-2">
                    <Field
                      label="WebDAV 地址"
                      htmlFor="cloud-base-url"
                      className="min-w-[280px] flex-1"
                    >
                      <Input
                        id="cloud-base-url"
                        className="font-mono"
                        placeholder="https://dav.example.com/dav"
                        value={cloudBaseUrl}
                        onChange={(event) => setCloudBaseUrl(event.target.value)}
                      />
                    </Field>
                    <Field label="用户名" htmlFor="cloud-username" className="w-40">
                      <Input
                        id="cloud-username"
                        value={cloudUsername}
                        onChange={(event) => setCloudUsername(event.target.value)}
                      />
                    </Field>
                  </div>
                  <div className="flex flex-wrap items-end gap-2">
                    <Field label="密码" htmlFor="cloud-password" className="w-56">
                      <div className="flex items-center gap-1">
                        <Input
                          id="cloud-password"
                          type={showCloudPassword ? "text" : "password"}
                          placeholder={
                            cloud.hasPassword && !cloudPasswordDirty
                              ? "已保存，留空保持不变"
                              : ""
                          }
                          value={cloudPassword}
                          onChange={(event) => {
                            setCloudPassword(event.target.value);
                            setCloudPasswordDirty(true);
                          }}
                        />
                        <IconButton
                          label={showCloudPassword ? "隐藏密码" : "显示密码"}
                          onClick={() => setShowCloudPassword((value) => !value)}
                        >
                          {showCloudPassword ? (
                            <EyeOff size={14} aria-hidden />
                          ) : (
                            <Eye size={14} aria-hidden />
                          )}
                        </IconButton>
                      </div>
                    </Field>
                    <Field label="远端目录" htmlFor="cloud-remote-dir" className="w-40">
                      <Input
                        id="cloud-remote-dir"
                        className="font-mono"
                        value={cloudRemoteDir}
                        onChange={(event) => setCloudRemoteDir(event.target.value)}
                      />
                    </Field>
                  </div>
                  <Checkbox
                    id="cloud-auto-upload"
                    label="自动快照后自动上传"
                    checked={cloudAutoUpload}
                    onChange={(event) => setCloudAutoUpload(event.target.checked)}
                  />
                  <div className="flex flex-wrap gap-2">
                    <Button
                      onClick={() => void testCloud()}
                      loading={busy === "cloud-test"}
                      disabled={!cloudBaseUrl.trim() || (!!busy && busy !== "cloud-test")}
                    >
                      测试连接
                    </Button>
                    <Button
                      onClick={() => void saveCloud()}
                      loading={busy === "cloud-save"}
                      disabled={!!busy && busy !== "cloud-save"}
                    >
                      保存云备份
                    </Button>
                    <Button
                      onClick={() => void uploadCloud()}
                      loading={busy === "cloud-upload"}
                      disabled={!cloudStatus?.configured || (!!busy && busy !== "cloud-upload")}
                    >
                      立即上传
                    </Button>
                    <Button
                      variant="danger"
                      disabled={!cloudStatus?.configured || !!busy}
                      onClick={() => setCloudRestoreOpen(true)}
                    >
                      从云端恢复
                    </Button>
                  </div>
                  <p className="text-caption text-fg-3">
                    上传会把当前真源打包并覆盖云端唯一一份备份；恢复前会自动创建本地安全快照。
                  </p>
                </div>

                <div className="grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-3">
                  <div className="min-w-0">
                    <p className="text-caption text-fg-3">远端对象</p>
                    <p
                      className="truncate font-mono text-caption text-fg-1"
                      title={cloudRemotePath}
                    >
                      {cloudRemotePath}
                    </p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">远端大小</p>
                    <p className="text-body text-fg-1">
                      {cloudStatus?.remote?.sizeBytes != null
                        ? formatBytes(cloudStatus.remote.sizeBytes)
                        : "—"}
                    </p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">远端修改时间</p>
                    <p className="text-body text-fg-1">
                      {cloudStatus?.remote?.modifiedAt
                        ? formatRelativeTime(cloudStatus.remote.modifiedAt)
                        : "—"}
                    </p>
                  </div>
                  <div>
                    <p className="text-caption text-fg-3">上次上传</p>
                    <p className="text-body text-fg-1">
                      {cloudStatus?.lastUploadAt
                        ? formatRelativeTime(cloudStatus.lastUploadAt)
                        : "—"}
                    </p>
                  </div>
                </div>

                {cloudStatus?.remoteError && (
                  <div className="px-4 pb-3">
                    <Notice tone="danger">{cloudStatus.remoteError}</Notice>
                  </div>
                )}
                {cloudStatus?.lastError && (
                  <div className="px-4 pb-3">
                    <Notice tone="warning">
                      上次上传失败：{cloudStatus.lastError}
                    </Notice>
                  </div>
                )}
              </>
            )}
          </section>
        </div>
      </div>

      <Dialog
        open={!!renameTarget}
        onClose={() => setRenameTarget(null)}
        title="重命名快照"
        footer={
          <>
            <Button onClick={() => setRenameTarget(null)} disabled={busy === "rename"}>
              取消
            </Button>
            <Button
              variant="primary"
              onClick={() => void confirmRename()}
              loading={busy === "rename"}
            >
              保存
            </Button>
          </>
        }
      >
        <Field label="快照名称" htmlFor="backup-rename">
          <Input
            id="backup-rename"
            value={renameValue}
            onChange={(event) => setRenameValue(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void confirmRename();
            }}
          />
        </Field>
      </Dialog>

      <ConfirmDialog
        open={cloudRestoreOpen}
        title="从云端恢复？"
        body={
          <>
            将下载云端最新包并覆盖 <code>assets/</code> 与 <code>agents/</code>，
            删除包中不存在的资产。恢复前会自动创建安全快照；本机{" "}
            <code>local/</code>、密钥与索引不会被动。
          </>
        }
        confirmLabel="恢复"
        pending={busy === "cloud-restore"}
        onConfirm={() => void confirmCloudRestore()}
        onCancel={() => setCloudRestoreOpen(false)}
      />

      <ConfirmDialog
        open={!!deleteTarget}
        title="删除快照？"
        body={`「${deleteTarget ?? ""}」将被永久删除，无法恢复。`}
        danger
        confirmLabel="删除"
        pending={busy === "delete"}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteTarget(null)}
      />

      <ConfirmDialog
        open={!!restoreTarget}
        title="恢复到此快照？"
        body={
          <>
            将用「{restoreTarget}」覆盖 <code>assets/</code> 与 <code>agents/</code>，
            删除快照里没有的资产。恢复前会自动创建安全快照；本机{" "}
            <code>local/</code>、密钥与索引不会被动。
          </>
        }
        confirmLabel="恢复"
        pending={busy === "restore"}
        onConfirm={() => void confirmRestore()}
        onCancel={() => setRestoreTarget(null)}
      />

      <ConfirmDialog
        open={!!cleanupTarget}
        title={cleanupTarget === "logs" ? "清理全部日志？" : "清理写入备份？"}
        body={
          cleanupTarget === "logs"
            ? "logs/ 下的日志会被永久删除，不影响资产与快照。"
            : "backups/ 下除 snapshots 外的写入留底会被永久删除；快照列表不受影响。"
        }
        danger
        confirmLabel="清理"
        pending={busy === "cleanup"}
        onConfirm={() => void confirmCleanup()}
        onCancel={() => setCleanupTarget(null)}
      />

      <ConfirmDialog
        open={!!gitRestoreTarget}
        title="恢复到这次提交？"
        body={
          <>
            将把工作区恢复到 {gitRestoreTarget?.shortHash} 的内容，并生成一个新提交。
            恢复前会自动创建安全快照；<code>local/</code>、<code>state/</code>{" "}
            与密钥不在 Git 中，不会被改动。
          </>
        }
        confirmLabel="恢复"
        pending={busy === "git-restore"}
        onConfirm={() => void confirmGitRestore()}
        onCancel={() => setGitRestoreTarget(null)}
      />

      <ConfirmDialog
        open={conflictAction === "keepLocal"}
        title="保留本地并覆盖远端？"
        body="会用本地提交强推覆盖远端分支。远端上尚未合并的提交会被覆盖，操作前请确认。"
        danger
        confirmLabel="保留本地"
        pending={busy === "conflict"}
        onConfirm={() => void confirmConflict()}
        onCancel={() => setConflictAction(null)}
      />

      <ConfirmDialog
        open={conflictAction === "keepRemote"}
        title="保留远端并覆盖本地？"
        body="会用远端分支覆盖本地内容和本地提交。覆盖前会自动创建安全快照，可在快照列表里找回。"
        danger
        confirmLabel="保留远端"
        pending={busy === "conflict"}
        onConfirm={() => void confirmConflict()}
        onCancel={() => setConflictAction(null)}
      />
    </div>
  );
}
