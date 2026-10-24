import type { AgentStatus } from "../../types";
import { Button } from "../ui/Button";
import { Menu } from "../ui/Menu";

export interface SkillBatchBarProps {
  count: number;
  agents: AgentStatus[];
  /** 选中项里至少给一个 agent 启用了，才提供停用入口。 */
  enabledAgents: AgentStatus[];
  updatableCount: number;
  busy?: boolean;
  onCheck: () => void;
  onUpdate: () => void;
  onEnable: (agentId: string) => void;
  onDisable: (agentId: string) => void;
  onDelete: () => void;
  onClear: () => void;
}

/** 多选后出现的批量操作条；只操作当前可见列表里被选中的 Skill。 */
export function SkillBatchBar({
  count,
  agents,
  enabledAgents,
  updatableCount,
  busy = false,
  onCheck,
  onUpdate,
  onEnable,
  onDisable,
  onDelete,
  onClear,
}: SkillBatchBarProps) {
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-panel border border-border bg-subtle px-3 py-2">
      <span className="text-caption font-medium text-fg-1">已选 {count} 个</span>
      <Button size="sm" onClick={onCheck} disabled={busy}>
        检查更新
      </Button>
      <Button
        size="sm"
        onClick={onUpdate}
        disabled={busy || updatableCount === 0}
        title={updatableCount === 0 ? "选中的 Skill 没有可检查的上游仓库" : undefined}
      >
        更新
      </Button>
      <Menu
        label="批量启用给"
        trigger="启用给"
        buttonClassName="h-7 w-auto whitespace-nowrap px-2.5 text-caption font-medium"
        items={agents.map((agent) => ({
          label: agent.displayName,
          disabled: busy,
          onSelect: () => onEnable(agent.id),
        }))}
      />
      <Menu
        label="批量停用"
        trigger="停用"
        buttonClassName="h-7 w-auto whitespace-nowrap px-2.5 text-caption font-medium"
        items={
          enabledAgents.length === 0
            ? [{ label: "没有已启用的 Agent", disabled: true, onSelect: () => {} }]
            : enabledAgents.map((agent) => ({
                label: agent.displayName,
                disabled: busy,
                onSelect: () => onDisable(agent.id),
              }))
        }
      />
      <Button size="sm" variant="danger" onClick={onDelete} disabled={busy}>
        删除
      </Button>
      <Button size="sm" variant="ghost" className="ml-auto" onClick={onClear} disabled={busy}>
        取消选择
      </Button>
    </div>
  );
}
