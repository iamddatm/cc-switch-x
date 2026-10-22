import { useState } from "react";
import { api } from "../lib/api";
import { switchable, supported, type AgentStatus, type AssetKind } from "../types";

/**
 * 「资产 × agent」开关。能力按资产类型逐格判定：
 * 该 agent 没有这一类落点 → 整行不出现；有落点但没实测 → 只说明、不给开关。
 * 没有实测依据时按「需要重启」提示，不把未知报成安全。
 */
export function AgentMatrix({
  assetType,
  id,
  agents,
  onChanged,
}: {
  assetType: Extract<AssetKind, "mcp" | "skill">;
  id: string;
  agents: AgentStatus[];
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  /** skill 的实测落地方式（symlink / junction / copy），由后端返回，不是配置意图 */
  const [kinds, setKinds] = useState<Record<string, string>>({});

  const toggle = async (agent: AgentStatus, enabled: boolean) => {
    setBusy(agent.id);
    try {
      if (assetType === "mcp") {
        await api.setMcpEnabled(id, agent.id, enabled);
      } else {
        const label = await api.setSkillEnabled(id, agent.id, enabled);
        setKinds((current) => ({ ...current, [agent.id]: label }));
      }
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
          const on = (assetType === "mcp" ? agent.enabledMcp : agent.enabledSkills).includes(id);
          // 控件不能嵌在 <label> 里：那样点一次会触发两次 change（启用后立刻又被取消）
          const inputId = `matrix-${assetType}-${id}-${agent.id}`;
          return (
            <li key={agent.id} className="flex items-center gap-2">
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
              {assetType === "skill" && kinds[agent.id] && (
                <span className="text-slate-500 dark:text-slate-400">
                  落地方式：{kinds[agent.id]}
                </span>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
