import {
  enabledFor,
  switchable,
  supported,
  type AgentStatus,
  type AssetType,
} from "../types";
import { useAssetToggles } from "./assets/useAssetToggles";

/** 面板上「同步到」能出现的资产类型。prompts 是整文件接管，走自己的激活流程。 */
export type MatrixAsset = Exclude<AssetType, "prompt">;

/**
 * 「资产 × agent」开关（纵向列表，Rules/Subagents/Hooks 使用）。
 * 能力按资产类型逐格判定：该 agent 没有这一类落点 → 整行不出现；
 * 有落点但没实测 → 只说明、不给开关。全局禁用的 agent 不出现在这里。
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
  const { busy, error, outcomes, toggle } = useAssetToggles(assetType, id, onChanged);

  return (
    <div className="mt-4">
      <h3 className="mb-2 text-sm font-medium">同步到</h3>
      {error && <p className="mb-2 text-sm text-danger-text">{error}</p>}
      <ul className="space-y-1 text-sm">
        {agents.map((agent) => {
          if (!agent.enabled) return null;
          if (!supported(agent, assetType)) {
            return null;
          }
          if (!switchable(agent, assetType)) {
            return (
              <li key={agent.id} className="text-fg-2">
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
                className="h-4 w-4 accent-action"
              />
              <label htmlFor={inputId}>{agent.displayName}</label>
              {!agent.detected && <span className="text-fg-2">配置目录不存在</span>}
              {on && assetType === "mcp" && agent.mcpNeedsRestart !== false && (
                <span className="text-warning-text">需重启或 reload</span>
              )}
              {outcomes[agent.id] && (
                <span className="text-fg-2">实测：{outcomes[agent.id]}</span>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}