import { useState } from "react";
import { api, type ApplyReport } from "../lib/api";
import {
  enabledFor,
  switchable,
  supported,
  type AgentStatus,
  type AssetType,
} from "../types";

/** 面板上「同步到」能出现的资产类型。prompts 是整文件接管，走自己的激活流程。 */
export type MatrixAsset = Exclude<AssetType, "prompt">;

/** 每次开关返回的实测结果：不是配置意图，是后端做完之后到底是什么。 */
function outcomeLabel(result: string | ApplyReport): string {
  if (typeof result === "string") {
    return result;
  }
  const via = result.methodUsed === "cli" ? "CLI" : "文件";
  const count = result.written > 0 ? `写入 ${result.written}` : `无变化 ${result.skipped}`;
  return `${via} · ${count}${result.note ? ` · ${result.note}` : ""}`;
}

async function call(
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

/**
 * 「资产 × agent」开关。能力按资产类型逐格判定：
 * 该 agent 没有这一类落点 → 整行不出现；有落点但没实测 → 只说明、不给开关。
 * 有未解决漂移时后端会直接拒绝（错误显示在下方），这里不给「强制覆盖」的口子。
 */
export function AgentMatrix({
  assetType,
  id,
  agents,
  onChanged,
}: {
  assetType: MatrixAsset;
  id: string;
  agents: AgentStatus[];
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [outcomes, setOutcomes] = useState<Record<string, string>>({});

  const toggle = async (agent: AgentStatus, enabled: boolean) => {
    setBusy(agent.id);
    try {
      const label = outcomeLabel(await call(assetType, id, agent.id, enabled));
      setOutcomes((current) => ({ ...current, [agent.id]: label }));
      setError(null);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="mt-4">
      <h3 className="mb-2 text-sm font-medium">同步到</h3>
      {error && <p className="mb-2 text-sm text-red-600 dark:text-red-400">{error}</p>}
      <ul className="space-y-1 text-sm">
        {agents.map((agent) => {
          if (!supported(agent, assetType)) {
            return null;
          }
          if (!switchable(agent, assetType)) {
            return (
              <li key={agent.id} className="text-slate-500 dark:text-slate-400">
                {agent.displayName}：未实测，暂不提供开关
              </li>
            );
          }
          const on = enabledFor(agent, assetType).includes(id);
          // 控件不能嵌在 <label> 里：那样点一次会触发两次 change（启用后立刻又被取消）
          const inputId = `matrix-${assetType}-${id}-${agent.id}`;
          return (
            <li key={agent.id} className="flex flex-wrap items-center gap-2">
              <input
                id={inputId}
                type="checkbox"
                aria-label={`${agent.displayName} 启用 ${id}`}
                checked={on}
                disabled={!agent.detected || busy === agent.id}
                onChange={(e) => void toggle(agent, e.target.checked)}
              />
              <label htmlFor={inputId}>{agent.displayName}</label>
              {!agent.detected && (
                <span className="text-slate-500 dark:text-slate-400">未检测到</span>
              )}
              {on && assetType === "mcp" && agent.mcpNeedsRestart !== false && (
                <span className="text-amber-700 dark:text-amber-400">需重启或 reload</span>
              )}
              {outcomes[agent.id] && (
                <span className="text-slate-500 dark:text-slate-400">
                  实测：{outcomes[agent.id]}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
