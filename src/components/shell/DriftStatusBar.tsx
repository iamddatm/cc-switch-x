import { RefreshCw } from "lucide-react";
import { useDrift } from "../../hooks/useDrift";
import { Button } from "../ui/Button";

/**
 * 全局唯一的「落盘是否还和真源一致」入口。
 * 漂移只报告不自动改，而 `rescan_drift` 本来就是跨 agent 全量的 —— 所以只给一处常驻控件：
 * 这里负责「发现」，逐条处置仍回到各面板顶部的漂移横幅。
 * 清单与重扫都直接吃 `useDrift`，别再抄一份订阅，否则两处实现要手动保持同步。
 */
export function DriftStatusBar() {
  const { drifts, error, busy, loading, rescan } = useDrift();
  const count = loading ? null : drifts.length;

  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-border bg-sidebar px-6 py-1 text-caption text-fg-3">
      {error ? (
        <span className="min-w-0 truncate text-danger-text">漂移检查失败：{error}</span>
      ) : (
        <span aria-live="polite" className="min-w-0 truncate">
          {count === null
            ? "正在读取落盘状态…"
            : count === 0
              ? "落盘与真源一致"
              : `落盘有 ${count} 处与真源不一致，请到对应面板逐条处置`}
        </span>
      )}
      <Button
        size="sm"
        variant="ghost"
        className="ml-auto shrink-0"
        loading={busy}
        onClick={() => void rescan()}
      >
        <RefreshCw size={12} aria-hidden />
        重扫落盘
      </Button>
    </div>
  );
}
