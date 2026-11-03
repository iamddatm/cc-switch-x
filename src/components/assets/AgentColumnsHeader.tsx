import { AgentGlyph } from "../agents/AgentGlyph";
import { cn } from "../../lib/cn";
import { enabledFor, switchable, type AgentStatus, type AssetSummary } from "../../types";
import { AGENT_COLUMN_WIDTH, supportedAgents } from "./AgentColumnCells";
import { Menu, type MenuItem } from "../ui/Menu";
import type { MatrixAsset } from "./matrix";

export interface ColumnBulkAction {
  /** 一次批量只动一列；ids 由列头按当前可见行算好，面板不必再关心是哪几根筛选轴。 */
  onBulk: (agentId: string, ids: string[], enabled: boolean) => void;
  /** 有批量在飞（或正在检查/更新）时两个动作都不可点。 */
  busy: boolean;
  /** 作用范围脚注：点之前就说清这次会动「当前筛选结果」还是「全部」。 */
  scopeNote: string;
}

/**
 * 列头：agent 图标 + 当前列表已启用数量。
 * 只有给了 `bulk` 才升级成批量入口，且必须同时过能力闸（`switchable` + 配置目录已就绪）——
 * 未实测的落点连单个勾选都不给，更不能整列批量。没接线的页面保持静态格子。
 */
export function AgentColumnsHeader({
  assetType,
  agents,
  items,
  bulk,
}: {
  assetType: MatrixAsset;
  agents: AgentStatus[];
  items: AssetSummary[];
  bulk?: ColumnBulkAction;
}) {
  const visible = supportedAgents(agents, assetType);
  return (
    <>
      {visible.map((agent) => {
        const enabled = new Set(enabledFor(agent, assetType));
        const onIds = items.filter((item) => enabled.has(item.id)).map((item) => item.id);
        const count = onIds.length;
        const cell = (
          <span className="flex flex-col items-center gap-0.5">
            <AgentGlyph agentId={agent.id} label={agent.displayName} size={16} />
            <span className="text-badge tabular-nums text-fg-3">{count}</span>
          </span>
        );
        if (!bulk || !switchable(agent, assetType) || !agent.detected) {
          return (
            <span
              key={agent.id}
              className={cn(AGENT_COLUMN_WIDTH, "flex flex-col items-center justify-center gap-0.5")}
              title={`${agent.displayName}：当前列表已启用 ${count} 个`}
            >
              {cell}
            </span>
          );
        }
        // 「启用剩余」要的那份 id 只有真给了批量入口才算得出来，静态列不必陪着铺两遍。
        const restIds = items.filter((item) => !enabled.has(item.id)).map((item) => item.id);
        const menuItems: MenuItem[] = [
          {
            label: `已启用 ${count} / ${items.length} 个`,
            disabled: true,
            onSelect: () => {},
          },
          {
            label: restIds.length === 0 ? "全部启用" : `启用剩余 ${restIds.length} 个`,
            disabled: restIds.length === 0 || bulk.busy,
            onSelect: () => bulk.onBulk(agent.id, restIds, true),
          },
          {
            label: "全部关闭",
            disabled: count === 0 || bulk.busy,
            onSelect: () => bulk.onBulk(agent.id, onIds, false),
          },
          { label: bulk.scopeNote, disabled: true, onSelect: () => {} },
        ];
        return (
          <Menu
            key={agent.id}
            label={`${agent.displayName}：当前列表已启用 ${count} / ${items.length} 个，点击批量开关`}
            items={menuItems}
            trigger={cell}
            buttonClassName={cn(
              AGENT_COLUMN_WIDTH,
              "h-11 self-stretch rounded-none gap-0.5 hover:bg-selected",
            )}
          />
        );
      })}
    </>
  );
}
