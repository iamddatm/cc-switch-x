import { useState } from "react";
import { Download, RefreshCw } from "lucide-react";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Notice } from "../ui/Notice";
import type { UpdateController } from "../../hooks/useUpdate";

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const mb = value / (1024 * 1024);
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

export function AboutSection({ update }: { update: UpdateController }) {
  const [confirmOpen, setConfirmOpen] = useState(false);
  const { available, progress, installing, installStarted, checking } = update;
  const percent =
    progress?.total != null && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;

  const handlePrimary = () => {
    if (available) {
      setConfirmOpen(true);
      return;
    }
    void update.check();
  };

  const buttonLabel = installing
    ? "正在更新"
    : available
      ? `下载并安装 v${available.version}`
      : "检查更新";
  const ButtonIcon = available ? Download : RefreshCw;

  return (
    <section className="space-y-3">
      <h3 className="text-strong text-fg-1">关于</h3>

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-control border border-border bg-surface px-4 py-3">
        <div className="space-y-0.5">
          <div className="text-strong text-fg-1">CC Switch X</div>
          <div className="text-caption text-fg-2">
            版本 {update.currentVersion ?? "未知"}
          </div>
        </div>
        <Button
          variant={available ? "primary" : "secondary"}
          loading={checking || installing}
          disabled={checking || installing}
          onClick={handlePrimary}
        >
          {!checking && !installing && <ButtonIcon size={14} aria-hidden />}
          {checking ? "正在检查" : buttonLabel}
        </Button>
      </div>

      {available && (
        <div className="space-y-3 rounded-control border border-border bg-surface px-4 py-3">
          <div className="space-y-1">
            <p className="text-strong text-fg-1">发现新版本 v{available.version}</p>
            {available.notes && (
              <div className="max-h-40 overflow-auto whitespace-pre-wrap text-caption text-fg-2">
                {available.notes}
              </div>
            )}
          </div>

          {installing && (
            <div className="space-y-1.5">
              {installStarted ? (
                <p className="text-caption text-fg-2">
                  正在安装更新，应用将自动重启，请勿关闭窗口。
                </p>
              ) : (
                <>
                  <p className="text-caption text-fg-2">
                    {progress?.total
                      ? `正在下载更新… ${percent}%（${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)}）`
                      : "正在准备下载更新…"}
                  </p>
                  <div className="h-1.5 overflow-hidden rounded-full bg-subtle">
                    <div
                      className="h-full bg-action transition-[width] duration-150"
                      style={{ width: `${percent ?? 3}%` }}
                    />
                  </div>
                </>
              )}
            </div>
          )}

          {!installing &&
            (update.dismissed ? (
              <Button size="sm" onClick={update.resetDismiss}>
                恢复更新提示
              </Button>
            ) : (
              <Button size="sm" onClick={update.dismiss}>
                忽略此版本
              </Button>
            ))}
        </div>
      )}

      {update.error && <Notice tone="danger">{update.error}</Notice>}

      <p className="text-caption text-fg-3">
        更新由 GitHub Releases 提供，仅支持 Windows x64。
      </p>

      <ConfirmDialog
        open={confirmOpen}
        title="安装更新"
        body={`应用会下载并安装 v${available?.version ?? ""}，随后自动重启；未保存的修改会丢失。`}
        confirmLabel="下载并安装"
        pending={installing}
        onConfirm={() => {
          setConfirmOpen(false);
          void update.install();
        }}
        onCancel={() => setConfirmOpen(false)}
      />
    </section>
  );
}
