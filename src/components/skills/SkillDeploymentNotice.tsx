import { Button } from "../ui/Button";
import { Notice } from "../ui/Notice";
import type { SkillDeploymentIssue } from "../../types";

/**
 * Skill 勾选与落点不符的横幅：启用矩阵说「已启用」，盘上那份却不归我们管了
 * （被别的工具换成它自己的链接、被删掉，或真源已经不在了）。
 *
 * 这个组件本身仍然只报告；「一键处理」只是把处置交给面板去确认与执行 ——
 * 核对（`verify_skill_deployments`）永远只读，动盘的只有 `repair_skill_deployments` 那一条路。
 * 抢不回来的（落点是一份与真源内容不同的外来目录）后端会原样保留并说明原因，
 * 出路仍是「先在对侧工具里取消同名条目，再在这里重新勾选」。
 * 数据与重扫入口由面板传进来，与 `DriftBanner` 同一形状，不在这里再订阅一次盘。
 */
export function SkillDeploymentNotice({
  issues,
  busy,
  onRepair,
  onRescan,
}: {
  issues: SkillDeploymentIssue[];
  busy: boolean;
  onRepair: () => void;
  onRescan: () => void;
}) {
  if (issues.length === 0) {
    return null;
  }
  return (
    <div className="px-6 pb-3">
      <Notice tone="warning">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="flex-1">
              {issues.length} 个 Skill 已勾选，但落点上的内容不是本工具部署的。
              一键处理会重刷空掉的落点、把对侧的链接换成我们的、清掉真源已删的悬空勾选；
              抢不回来的原样保留并逐条说明原因。
            </span>
            <Button size="sm" variant="secondary" disabled={busy} onClick={onRepair}>
              一键处理
            </Button>
            <Button size="sm" variant="ghost" disabled={busy} onClick={onRescan}>
              重新检查
            </Button>
          </div>
          <ul className="max-h-32 space-y-1 overflow-auto text-fg-2">
            {issues.map((issue) => (
              <li key={`${issue.agentId}/${issue.skillId}`}>
                {issue.agentId} / {issue.skillId}：{issue.detail}
              </li>
            ))}
          </ul>
        </div>
      </Notice>
    </div>
  );
}
