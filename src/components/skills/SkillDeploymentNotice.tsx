import { Button } from "../ui/Button";
import { Notice } from "../ui/Notice";
import type { SkillDeploymentIssue } from "../../types";

/**
 * Skill 勾选与落点不符的横幅：启用矩阵说「已启用」，盘上那份却不归我们管了
 * （被别的工具换成它自己的链接、被删掉，或真源已经不在了）。
 *
 * 只报告，不自动重刷：自动把落点抢回来会覆盖用户在对侧工具里的选择，方向必须由人来定。
 * 恢复办法写在文案里 —— 重新勾选（外来内容仍在时会被能力闸拒绝，这是对的），或者
 * 直接取消勾选退账（只清我们的记录，不删别人的文件）。
 * 数据与重扫入口由面板传进来，与 `DriftBanner` 同一形状，不在这里再订阅一次盘。
 */
export function SkillDeploymentNotice({
  issues,
  busy,
  onRescan,
}: {
  issues: SkillDeploymentIssue[];
  busy: boolean;
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
              要让勾选重新生效，先在对侧工具里取消同名条目，再在这里重新勾选；
              不想再管这一条，直接取消勾选即可，盘上别人的文件不会被删。
            </span>
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
