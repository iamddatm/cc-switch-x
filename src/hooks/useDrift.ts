import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { DriftRecord, ResolveAction } from "../types";

/** 全量重扫完成后广播，状态条与各面板的横幅都靠它同步，不必各自再扫一遍盘。 */
export const DRIFT_RESCAN_EVENT = "ccswitchx:drift-rescanned";

/**
 * 真的去看磁盘。后端 `rescan_drift` 本来就是跨 agent 全量的，所以入口也只需要一个：
 * 顶部状态条与横幅里的「重新检查」都走这里，扫完把结果推给所有订阅方。
 */
export async function rescanDriftEverywhere(): Promise<DriftRecord[]> {
  const drifts = await api.rescanDrift();
  window.dispatchEvent(new CustomEvent<DriftRecord[]>(DRIFT_RESCAN_EVENT, { detail: drifts }));
  return drifts;
}

/**
 * 待解决的漂移清单。启动时读一次（后端在 setup 里已经扫过一遍），
 * 「重新检查」才真的去看磁盘 —— 自动监听在 Plan 5，这里不做。
 */
export function useDrift() {
  const [drifts, setDrifts] = useState<DriftRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 首轮清单还没到手时不能报「一致」，否则启动瞬间会先闪一个错误的绿灯
  const [loading, setLoading] = useState(true);

  const run = useCallback(async (fetch: () => Promise<DriftRecord[]>) => {
    setBusy(true);
    try {
      setDrifts(await fetch());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setLoading(false);
    }
  }, []);

  const refresh = useCallback(() => run(api.listDrifts), [run]);
  const rescan = useCallback(() => run(rescanDriftEverywhere), [run]);

  const resolve = useCallback(
    async (drift: DriftRecord, action: ResolveAction) => {
      await run(async () => {
        await api.resolveDrift(drift.id, action, drift.agentId);
        // 处置完以磁盘为准重新算一遍：解决了一条也可能暴露另一条
        return rescanDriftEverywhere();
      });
    },
    [run],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // 别处（状态条或另一个面板）重扫过，本地清单要跟上，否则两处数字会各说各话
  useEffect(() => {
    const onRescanned = (event: Event) => {
      setDrifts((event as CustomEvent<DriftRecord[]>).detail);
      setLoading(false);
    };
    window.addEventListener(DRIFT_RESCAN_EVENT, onRescanned);
    return () => window.removeEventListener(DRIFT_RESCAN_EVENT, onRescanned);
  }, []);

  const forAsset = useCallback(
    (assetType: string, assetId: string) =>
      drifts.filter((d) => d.assetType === assetType && d.assetId === assetId),
    [drifts],
  );

  return { drifts, error, busy, loading, refresh, rescan, resolve, forAsset };
}
