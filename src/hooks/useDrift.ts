import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type {
  DriftRecord,
  ResolveAction,
  SkillDeploymentIssue,
  SkillRepairReport,
} from "../types";

/**
 * 一次落盘核对的两类结果。台账类资产比的是字节（drifts），Skill 是链接部署，
 * 没有期望字节可比，比的是归属与勾选是否仍然对得上（skillIssues）。
 */
export interface DeploymentChecks {
  drifts: DriftRecord[];
  skillIssues: SkillDeploymentIssue[];
}

/** 全量重扫完成后广播，状态条与各面板的横幅都靠它同步，不必各自再扫一遍盘。 */
export const DEPLOYMENT_RESCAN_EVENT = "ccswitchx:drift-rescanned";

/**
 * 真的去看磁盘。后端 `rescan_drift` 本来就是跨 agent 全量的，Skill 核对也是现算的，
 * 所以入口只需要一个：顶部状态条、横幅里的「重新检查」与设置面板都走这里。
 */
export async function rescanDeployments(): Promise<DeploymentChecks> {
  const checks: DeploymentChecks = {
    drifts: await api.rescanDrift(),
    skillIssues: await api.verifySkillDeployments(),
  };
  window.dispatchEvent(new CustomEvent<DeploymentChecks>(DEPLOYMENT_RESCAN_EVENT, { detail: checks }));
  return checks;
}

/** 读取现成清单：漂移取后端已经扫过的记录，Skill 核对没有缓存只能现算。 */
async function loadChecks(): Promise<DeploymentChecks> {
  return { drifts: await api.listDrifts(), skillIssues: await api.verifySkillDeployments() };
}

/**
 * 待处置的落盘问题清单。启动时读一次（后端在 setup 里已经扫过漂移），
 * 「重新检查」才真的重去看磁盘 —— 自动监听在 Plan 5，这里不做。
 */
export function useDrift() {
  const [checks, setChecks] = useState<DeploymentChecks>({ drifts: [], skillIssues: [] });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 首轮清单还没到手时不能报「一致」，否则启动瞬间会先闪一个错误的绿灯
  const [loading, setLoading] = useState(true);

  const run = useCallback(async (fetch: () => Promise<DeploymentChecks>) => {
    setBusy(true);
    try {
      setChecks(await fetch());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setLoading(false);
    }
  }, []);

  const refresh = useCallback(() => run(loadChecks), [run]);
  const rescan = useCallback(() => run(rescanDeployments), [run]);

  const resolve = useCallback(
    async (drift: DriftRecord, action: ResolveAction) => {
      await run(async () => {
        await api.resolveDrift(drift.id, action, drift.agentId);
        // 处置完以磁盘为准重新算一遍：解决了一条也可能暴露另一条
        return rescanDeployments();
      });
    },
    [run],
  );

  /**
   * 一键处理 Skill 的勾选与落点差异。清单由后端现算，这里只负责触发与收尾：
   * 处置完必须重扫并广播，横幅与状态条才不会停在处理前的计数上。
   *
   * 与 `resolve` 不同，失败时会往外抛：面板要据此决定报「处理完成」还是保留原横幅，
   * 拿到 `undefined` 报告当成成功会把失败说成没差异。
   */
  const repair = useCallback(async (): Promise<SkillRepairReport> => {
    setBusy(true);
    try {
      const report = await api.repairSkillDeployments();
      setChecks(await rescanDeployments());
      setError(null);
      return report;
    } catch (e) {
      setError(String(e));
      throw e;
    } finally {
      setBusy(false);
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // 别处（状态条或另一个面板）重扫过，本地清单要跟上，否则两处数字会各说各话
  useEffect(() => {
    const onRescanned = (event: Event) => {
      setChecks((event as CustomEvent<DeploymentChecks>).detail);
      setLoading(false);
    };
    window.addEventListener(DEPLOYMENT_RESCAN_EVENT, onRescanned);
    return () => window.removeEventListener(DEPLOYMENT_RESCAN_EVENT, onRescanned);
  }, []);

  const drifts = checks.drifts;
  const forAsset = useCallback(
    (assetType: string, assetId: string) =>
      drifts.filter((d) => d.assetType === assetType && d.assetId === assetId),
    [drifts],
  );

  return {
    drifts,
    skillIssues: checks.skillIssues,
    error,
    busy,
    loading,
    refresh,
    rescan,
    resolve,
    repair,
    forAsset,
  };
}
