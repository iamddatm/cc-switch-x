import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { api } from "../../lib/api";
import { DRIFT_RESCAN_EVENT, rescanDriftEverywhere } from "../../hooks/useDrift";
import type { DriftRecord } from "../../types";
import { Button } from "../ui/Button";

/**
 * 全局唯一的「落盘是否还和真源一致」入口。
 * 漂移只报告不自动改，而 `rescan_drift` 本来就是跨 agent 全量的 —— 所以只给一处常驻控件：
 * 这里负责「发现」，逐条处置仍回到各面板顶部的漂移横幅。
 */
export function DriftStatusBar() {
  const [count, setCount] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // 启动时后端已经扫过一遍，这里只取现成结果，不重复扫盘
    void api
      .listDrifts()
      .then((rows) => setCount(rows.length))
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    // 横幅里的「重新检查」也走同一次全量重扫，数字要跟着动，否则两处各说各话
    const onRescanned = (event: Event) =>
      setCount((event as CustomEvent<DriftRecord[]>).detail.length);
    window.addEventListener(DRIFT_RESCAN_EVENT, onRescanned);
    return () => window.removeEventListener(DRIFT_RESCAN_EVENT, onRescanned);
  }, []);

  const rescan = async () => {
    setBusy(true);
    setError(null);
    try {
      await rescanDriftEverywhere();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

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
