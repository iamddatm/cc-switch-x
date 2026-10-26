import type { SkillDeployTarget } from "./skillScope";
import { Button } from "../ui/Button";
import { Menu } from "../ui/Menu";

export interface SkillBatchBarProps {
  count: number;
  /** 落点：全局是已实测的 agent，项目是已实测的渲染目录。 */
  targets: SkillDeployTarget[];
  /** 选中项里至少铺到一个落点上的，才提供撤回入口。 */
  enabledTargets: SkillDeployTarget[];
  updatableCount: number;
  busy?: boolean;
  /** 落点动作的措辞由作用域给，组件自己不判断是全局还是项目。 */
  wording: { targets: string; deployed: string; noneDeployed: string };
  /** 没有可撤回的落点时批量停用菜单上的说明。 */
  noneEnabledLabel: string;
  onCheck: () => void;
  onUpdate: () => void;
  onEnable: (targetKey: string) => void;
  onDisable: (targetKey: string) => void;
  onDelete: () => void;
  onClear: () => void;
}

/** 多选后出现的批量操作条；只操作当前可见列表里被选中的 Skill。 */
export function SkillBatchBar({
  count,
  targets,
  enabledTargets,
  updatableCount,
  busy = false,
  wording,
  noneEnabledLabel,
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
        label={`批量${wording.targets}`}
        trigger={wording.targets}
        buttonClassName="h-7 w-auto whitespace-nowrap px-2.5 text-caption font-medium"
        items={targets.map((target) => ({
          label: target.label,
          disabled: busy,
          onSelect: () => onEnable(target.key),
        }))}
      />
      <Menu
        label="批量停用"
        trigger="停用"
        buttonClassName="h-7 w-auto whitespace-nowrap px-2.5 text-caption font-medium"
        items={
          enabledTargets.length === 0
            ? [{ label: noneEnabledLabel, disabled: true, onSelect: () => {} }]
            : enabledTargets.map((target) => ({
                label: target.label,
                disabled: busy,
                onSelect: () => onDisable(target.key),
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
