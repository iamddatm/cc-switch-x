import { useState } from "react";
import { supportedAgents } from "./AgentColumnCells";
import type { AgentStatus, ScanReport } from "../../types";
import type { MatrixAsset } from "./matrix";

/** 扫描按钮的文案要按类别说清落点是什么，兜底用类型本身。 */
const KIND_LABEL: Partial<Record<MatrixAsset, string>> = {
  rule: "规则",
  subagent: "子代理",
};

export interface SourceScanState {
  report: ScanReport | null;
  error: string | null;
  running: boolean;
  run: () => Promise<void>;
}

/**
 * 「从 agent 目录扫描导入」的状态。三条纪律：
 * 1. 跨 agent 求和 —— 不能让最后一个把前面的计数挤掉；
 * 2. 只扫真的有这一类落点且已检测到的 agent —— 拿不存在的路径去扫只会得到假失败；
 * 3. 失败单独报，不塞进表单错误 —— 表单错误只在选中资产时才渲染，会看不见。
 */
export function useSourceScan(
  assetType: MatrixAsset,
  agents: AgentStatus[],
  scanOne: (agentId: string) => Promise<ScanReport>,
  onScanned: () => void,
): SourceScanState {
  const [report, setReport] = useState<ScanReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const run = async () => {
    const targets = supportedAgents(agents, assetType).filter((agent) => agent.detected);
    setReport(null);
    setError(null);
    if (targets.length === 0) {
      setError(
        `没有可扫描的${KIND_LABEL[assetType] ?? assetType}落点，先在 Agents 页启用支持它的工具。`,
      );
      return;
    }
    setRunning(true);
    const total: ScanReport = { imported: 0, updated: 0, unchanged: 0, skipped: [] };
    const failures: string[] = [];
    for (const agent of targets) {
      try {
        const one = await scanOne(agent.id);
        total.imported += one.imported;
        total.updated += one.updated;
        total.unchanged += one.unchanged;
        total.skipped.push(...one.skipped);
      } catch (e) {
        failures.push(`${agent.displayName}：${e}`);
      }
    }
    setReport(total);
    if (failures.length > 0) setError(failures.join("；"));
    setRunning(false);
    await onScanned();
  };

  return { report, error, running, run };
}
