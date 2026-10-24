import { Check } from "lucide-react";
import { cn } from "../../lib/cn";
import { enabledFor, switchable, supported, type AgentStatus } from "../../types";
import type { MatrixAsset } from "./matrix";
import { useAssetToggles } from "./useAssetToggles";

export const AGENT_COLUMN_WIDTH = "w-9";

export function supportedAgents(agents: AgentStatus[], assetType: MatrixAsset): AgentStatus[] {
  return agents.filter((agent) => supported(agent, assetType));
}

/**
 * 行内 agent 勾选列。未实测的 agent 保留占位格（不缩列），但不给开关；
 * 单元格用 role="checkbox" 而不是原生 checkbox，方便和列头/计数保持 36px 密度。
 */
export function AgentColumnCells({
  assetType,
  id,
  agents,
  onChanged,
  onError,
  onOutcome,
}: {
  assetType: MatrixAsset;
  id: string;
  agents: AgentStatus[];
  onChanged: () => void;
  onError?: (error: string) => void;
  onOutcome?: (label: string) => void;
}) {
  const { busy, outcomes, toggle } = useAssetToggles(
    assetType,
    id,
    onChanged,
    onError,
    onOutcome,
  );

  return (
    <>
      {agents.map((agent) => {
        if (!supported(agent, assetType)) {
          return <span key={agent.id} className={AGENT_COLUMN_WIDTH} aria-hidden />;
        }
        const on = enabledFor(agent, assetType).includes(id);
        if (!switchable(agent, assetType)) {
          return (
            <span
              key={agent.id}
              role="img"
              aria-label={`${agent.displayName}：未实测，暂不提供开关`}
              className={cn(AGENT_COLUMN_WIDTH, "flex items-center justify-center")}
              title={`${agent.displayName}：未实测，暂不提供开关`}
            >
              <span className="h-4 w-4 rounded-[4px] border border-dashed border-border-strong opacity-45" />
            </span>
          );
        }
        const restart =
          on && assetType === "mcp" && agent.mcpNeedsRestart !== false
            ? "（需重启或 reload）"
            : "";
        const outcome = outcomes[agent.id];
        return (
          <span key={agent.id} className={cn(AGENT_COLUMN_WIDTH, "flex items-center justify-center")}>
            <button
              type="button"
              role="checkbox"
              aria-checked={on}
              aria-label={`${agent.displayName} 启用 ${id}`}
              title={`${agent.displayName}${on ? ` 已启用${restart}` : " 未启用"}${
                outcome ? ` · 实测：${outcome}` : ""
              }${!agent.detected ? " · 配置目录不存在" : ""}${
                !agent.cliAvailable ? ` · PATH 中未找到 ${agent.cliProgram}` : ""
              }`}
              disabled={!agent.detected || busy === agent.id}
              onClick={() => void toggle(agent, !on)}
              className={cn(
                "flex h-4 w-4 items-center justify-center rounded-[4px] border-[1.5px] transition-colors duration-150",
                "disabled:pointer-events-none disabled:opacity-45",
                on
                  ? "border-action bg-action text-action-fg"
                  : "border-border-strong bg-surface hover:border-fg-3",
              )}
            >
              {on && <Check size={11} strokeWidth={3} aria-hidden />}
            </button>
          </span>
        );
      })}
    </>
  );
}