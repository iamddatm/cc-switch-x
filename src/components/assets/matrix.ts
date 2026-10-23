import { api, type ApplyReport } from "../../lib/api";
import type { AssetType } from "../../types";

/** 面板上「同步到」能出现的资产类型。prompts 是整文件接管，走自己的激活流程。 */
export type MatrixAsset = Exclude<AssetType, "prompt">;

/** 每次开关返回的实测结果：不是配置意图，是后端做完之后到底是什么。 */
export function outcomeLabel(result: string | ApplyReport): string {
  if (typeof result === "string") {
    return result;
  }
  const via = result.methodUsed === "cli" ? "CLI" : "文件";
  const count = result.written > 0 ? `写入 ${result.written}` : `无变化 ${result.skipped}`;
  return `${via} · ${count}${result.note ? ` · ${result.note}` : ""}`;
}

export async function callToggle(
  kind: MatrixAsset,
  id: string,
  agentId: string,
  enabled: boolean,
): Promise<string | ApplyReport> {
  switch (kind) {
    case "mcp":
      return api.setMcpEnabled(id, agentId, enabled);
    case "skill":
      return api.setSkillEnabled(id, agentId, enabled);
    case "rule":
      return api.setRuleEnabled(id, agentId, enabled);
    case "subagent":
      return api.setSubagentEnabled(id, agentId, enabled);
    case "hook":
      return api.setHookEnabled(id, agentId, enabled);
  }
}