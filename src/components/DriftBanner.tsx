import { Button } from "./ui/Button";
import type { DriftRecord, ResolveAction } from "../types";

const ASSET_LABELS: Record<string, string> = {
  prompt: "提示词",
  rule: "规则",
  subagent: "子代理",
  command: "命令",
};

/**
 * 某资产有未解决漂移时的顶部提示条：给两个按钮（采纳磁盘 / 重刷真源）与目标文件绝对路径，
 * 外加「重新检查」——漂移只靠启动/手动 rescan 报告，没有自动监听，处置完要能立刻再看一次盘。
 * 没有「知道了 / 忽略」这种第三个选项 —— 漂移不处置掉就一直挡着写命令，这是刻意的（spec 5.4）。
 */
export function DriftBanner({
  drifts,
  busy,
  onResolve,
  onRescan,
}: {
  drifts: DriftRecord[];
  busy: boolean;
  onResolve: (drift: DriftRecord, action: ResolveAction) => void;
  onRescan: () => void;
}) {
  if (drifts.length === 0) {
    return null;
  }
  return (
    <div
      role="region"
      aria-label="落盘漂移"
      className="mb-4 space-y-3 rounded-panel border border-warning bg-warning-soft p-3 text-body"
    >
      <div className="flex items-center gap-2">
        <p className="flex-1 font-medium text-warning-text">
          有 {drifts.length} 个文件与真源不一致，处理完才能启用或停用。
        </p>
        <Button variant="secondary" size="sm" disabled={busy} onClick={onRescan}>
          重新检查
        </Button>
      </div>
      {drifts.map((drift) => (
        <div key={drift.id} className="flex flex-wrap items-center gap-2">
          <span>
            {drift.agentId} 的
            {ASSET_LABELS[drift.assetType] ?? drift.assetType} {drift.assetId}：
            {drift.kind === "missing" ? "文件不见了" : "文件被改过"}
          </span>
          <Button
            variant="secondary"
            size="sm"
            disabled={busy || drift.kind === "missing"}
            onClick={() => onResolve(drift, "adoptDisk")}
          >
            采纳磁盘
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() => onResolve(drift, "repushSource")}
          >
            重刷真源
          </Button>
          <code className="w-full break-all text-caption text-fg-2">
            {drift.targetPath}
          </code>
        </div>
      ))}
    </div>
  );
}
