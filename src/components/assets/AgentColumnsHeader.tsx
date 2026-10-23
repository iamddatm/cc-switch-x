import { AgentGlyph } from "../agents/AgentGlyph";
import { cn } from "../../lib/cn";
import { enabledFor, type AgentStatus, type AssetSummary } from "../../types";
import { AGENT_COLUMN_WIDTH, supportedAgents } from "./AgentColumnCells";
import type { MatrixAsset } from "./matrix";

/** 列头只展示 agent 图标与已启用数量；批量开关留到下一轮。 */
export function AgentColumnsHeader({
  assetType,
  agents,
  items,
}: {
  assetType: MatrixAsset;
  agents: AgentStatus[];
  items: AssetSummary[];
}) {
  const visible = supportedAgents(agents, assetType);
  return (
    <>
      {visible.map((agent) => {
        const enabled = new Set(enabledFor(agent, assetType));
        const count = items.filter((item) => enabled.has(item.id)).length;
        return (
          <span
            key={agent.id}
            className={cn(AGENT_COLUMN_WIDTH, "flex flex-col items-center justify-center gap-0.5")}
            title={`${agent.displayName}：当前列表已启用 ${count} 个`}
          >
            <AgentGlyph agentId={agent.id} label={agent.displayName} size={16} />
            <span className="text-badge tabular-nums text-fg-3">{count}</span>
          </span>
        );
      })}
    </>
  );
}