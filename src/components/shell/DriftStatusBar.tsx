import { useDrift } from "../../hooks/useDrift";

/**
 * 全局落盘核对提示条：只在真有问题或检查失败时出现，一致时完全隐藏。
 * 两类问题都只报告不自动改；重扫入口在设置面板，这里只负责让问题可见。
 * 清单直接吃 `useDrift`，别再抄一份订阅，否则两处实现要手动保持同步。
 */
export function DriftStatusBar() {
  const { drifts, skillIssues, error, loading } = useDrift();
  if (loading) return null;
  if (!error && drifts.length === 0 && skillIssues.length === 0) return null;

  return (
    <div className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-b border-border bg-sidebar px-6 py-1 text-caption text-fg-3">
      {error ? (
        <span className="min-w-0 truncate text-danger-text">落盘检查失败：{error}</span>
      ) : (
        <>
          {drifts.length > 0 && (
            <span aria-live="polite" className="min-w-0 truncate">
              落盘有 {drifts.length} 处与真源不一致，请到对应面板逐条处置
            </span>
          )}
          {skillIssues.length > 0 && (
            <span aria-live="polite" className="min-w-0 truncate text-warning-text">
              {skillIssues.length} 个 Skill 已勾选但落点不归本工具管，请到 Skills 面板核对
            </span>
          )}
        </>
      )}
    </div>
  );
}
