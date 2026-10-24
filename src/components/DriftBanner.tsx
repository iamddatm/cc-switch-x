import { Button } from "./ui/Button";
import type { DriftRecord, ResolveAction } from "../types";

/**
 * 某资产有未解决漂移时的顶部提示条：给两个按钮（采纳磁盘 / 重刷真源）与目标文件绝对路径。
 * 没有「知道了 / 忽略」这种第三个选项 —— 漂移不处置掉就一直挡着写命令，这是刻意的（spec 5.4）。
 */
export function DriftBanner({
  drifts,
  busy,
  onResolve,
}: {
  drifts: DriftRecord[];
  busy: boolean;
  onResolve: (drift: DriftRecord, action: ResolveAction) => void;
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
      <p className="font-medium text-warning-text">
        磁盘与真源不一致的有 {drifts.length} 处。先决定怎么处理，再改开关。
      </p>
      {drifts.map((drift) => (
        <div key={drift.id} className="flex flex-wrap items-center gap-2">
          <span>
            {drift.agentId} / {drift.assetType} / {drift.assetId}：
            {drift.kind === "missing" ? "落盘的文件不见了" : "落盘的内容被改过"}
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
