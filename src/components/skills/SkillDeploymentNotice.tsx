import { rescanDeployments, useDrift } from "../../hooks/useDrift";
import { Button } from "../ui/Button";
import { Notice } from "../ui/Notice";

/**
 * Skill 勾选与落点不符的横幅：启用矩阵说「已启用」，盘上那份却不归我们管了
 * （被别的工具换成它自己的链接、被删掉，或真源已经不在了）。
 *
 * 只报告，不自动重刷：自动部署会把用户在对侧工具里的选择覆盖掉，方向必须由人来定。
 * 恢复办法是重新勾选（走正常部署流水线），但前提是先把对侧的同名条目取消，
 * 否则下一次对侧同步又会把落点换回去。
 */
export function SkillDeploymentNotice() {
  const { skillIssues, busy } = useDrift();
  if (skillIssues.length === 0) {
    return null;
  }
  return (
    <div className="px-6 pb-3">
      <Notice tone="warning">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="flex-1">
              {skillIssues.length} 个 Skill 已勾选，但落点上的内容不是本工具部署的。
              要让勾选重新生效，先在对侧工具里取消同名条目，再在这里重新勾选部署。
            </span>
            <Button
              size="sm"
              variant="ghost"
              loading={busy}
              onClick={() => void rescanDeployments()}
            >
              重新检查
            </Button>
          </div>
          <ul className="max-h-32 space-y-1 overflow-auto text-fg-2">
            {skillIssues.map((issue) => (
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
