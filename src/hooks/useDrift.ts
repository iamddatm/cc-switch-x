import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { DriftRecord, ResolveAction } from "../types";

/**
 * 待解决的漂移清单。启动时读一次（后端在 setup 里已经扫过一遍），
 * 「重新检查」按钮才真的去看磁盘 —— 自动监听在 Plan 5，这里不做。
 */
export function useDrift() {
  const [drifts, setDrifts] = useState<DriftRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    setBusy(true);
    try {
      setDrifts(await api.listDrifts());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const rescan = useCallback(async () => {
    setBusy(true);
    try {
      setDrifts(await api.rescanDrift());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const resolve = useCallback(
    async (drift: DriftRecord, action: ResolveAction) => {
      setBusy(true);
      try {
        await api.resolveDrift(drift.id, action, drift.agentId);
        // 处置完以磁盘为准重新算一遍：解决了一条也可能暴露另一条
        setDrifts(await api.rescanDrift());
        setError(null);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const forAsset = useCallback(
    (assetType: string, assetId: string) =>
      drifts.filter((d) => d.assetType === assetType && d.assetId === assetId),
    [drifts],
  );

  return { drifts, error, busy, refresh, rescan, resolve, forAsset };
}
