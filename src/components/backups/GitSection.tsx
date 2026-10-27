import { GitBranch } from "lucide-react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Field } from "../ui/Field";
import { Input } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { formatRelativeTime } from "../../lib/format";
import type { BackupSettings, GitCommit, GitStatus } from "../../types";

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

/**
 * Git 备份区：状态概览、冲突处置、远端配置与提交历史。
 * 凭据一律交给系统 Git，应用不保存 token，所以这里只有地址与分支。
 */
export function GitSection({
  git,
  settings,
  commits,
  busy,
  remoteInput,
  branchInput,
  onRemoteInputChange,
  onBranchInputChange,
  onInit,
  onSaveRemote,
  onVerifyRemote,
  onToggle,
  onCommit,
  onPull,
  onPush,
  onRestoreCommit,
  onResolveConflict,
  onRevealStore,
}: {
  git: GitStatus | null;
  settings: BackupSettings | null;
  commits: GitCommit[];
  busy: string | null;
  remoteInput: string;
  branchInput: string;
  onRemoteInputChange: (value: string) => void;
  onBranchInputChange: (value: string) => void;
  onInit: () => void;
  onSaveRemote: () => void;
  onVerifyRemote: () => void;
  onToggle: (patch: Partial<BackupSettings["git"]>) => void;
  onCommit: () => void;
  onPull: () => void;
  onPush: () => void;
  onRestoreCommit: (commit: GitCommit) => void;
  onResolveConflict: (action: "keepLocal" | "keepRemote") => void;
  onRevealStore: () => void;
}) {
  return (
    <section className="rounded-panel border border-border bg-surface">
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <GitBranch size={15} className="text-fg-3" aria-hidden />
        <h3 className="text-strong text-fg-1">Git 备份</h3>
        {git && <Badge tone={gitStateTone(git)}>{gitStateLabel(git)}</Badge>}
        {git?.initialized && (
          <Badge tone={git.remoteVerified ? "success" : "warning"}>
            {git.remoteVerified ? "远端已验证" : "远端未验证"}
          </Badge>
        )}
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
            在存储目录初始化 Git 仓库；之后填写并验证远端，才能启用自动同步。
          </p>
          <Button variant="primary" loading={busy === "git-init"} onClick={onInit}>
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

          {settings?.git.enabled && !git.remoteVerified && (
            <div className="px-4 pt-3">
              <Notice tone="warning">
                远端地址已变更或尚未验证，自动同步已暂停；请重新验证远端后再启用。
              </Notice>
            </div>
          )}

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
                <Button size="sm" variant="danger" onClick={() => onResolveConflict("keepLocal")}>
                  保留本地（强推）
                </Button>
                <Button size="sm" variant="danger" onClick={() => onResolveConflict("keepRemote")}>
                  保留远端（覆盖本地）
                </Button>
                <Button size="sm" onClick={onRevealStore}>
                  打开仓库目录手动处理
                </Button>
              </div>
            </div>
          )}

          <div className="space-y-3 border-b border-border px-4 py-3">
            <div className="flex flex-wrap items-center gap-4">
              <Checkbox
                id="git-auto-commit"
                label="写操作后自动提交"
                checked={settings?.git.autoCommit ?? true}
                onChange={(event) => onToggle({ autoCommit: event.target.checked })}
              />
              <Checkbox
                id="git-auto-pull"
                label="启动/定时拉取"
                checked={settings?.git.autoPull ?? true}
                onChange={(event) => onToggle({ autoPull: event.target.checked })}
              />
            </div>
            <div className="flex flex-wrap items-end gap-2">
              <Field label="远端地址" htmlFor="git-remote" className="min-w-[280px] flex-1">
                <Input
                  id="git-remote"
                  className="font-mono"
                  placeholder="git@github.com:me/assets.git 或 https://…"
                  value={remoteInput}
                  onChange={(event) => onRemoteInputChange(event.target.value)}
                />
              </Field>
              <Field label="分支" htmlFor="git-branch" className="w-36">
                <Input
                  id="git-branch"
                  className="font-mono"
                  value={branchInput}
                  onChange={(event) => onBranchInputChange(event.target.value)}
                />
              </Field>
              <Button className="mb-0.5" loading={busy === "git-remote"} onClick={onSaveRemote}>
                保存远端
              </Button>
              <Button
                className="mb-0.5"
                loading={busy === "git-verify"}
                disabled={!remoteInput.trim() || !git.initialized}
                onClick={onVerifyRemote}
              >
                验证远端
              </Button>
            </div>
            <p className="text-caption text-fg-3">
              凭据交给系统 Git（credential helper / SSH agent），应用不会保存 token；验证远端后才能在上方启用 Git 备份。
            </p>
            <div className="flex flex-wrap gap-2">
              <Button loading={busy === "commit"} onClick={onCommit}>
                立即提交
              </Button>
              <Button loading={busy === "pull"} onClick={onPull}>
                立即拉取
              </Button>
              <Button loading={busy === "push"} onClick={onPush}>
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
                    <span
                      className="min-w-0 flex-1 truncate text-body text-fg-1"
                      title={commit.message}
                    >
                      {commit.message}
                    </span>
                    <span className="shrink-0 text-caption text-fg-3">
                      {formatRelativeTime(commit.timestamp)}
                    </span>
                    <Button size="sm" disabled={!!busy} onClick={() => onRestoreCommit(commit)}>
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
  );
}
