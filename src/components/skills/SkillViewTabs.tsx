import { cn } from "../../lib/cn";

/**
 * Skills 面板的视图。`unmanaged` 只有用户级面板有：项目级的「未跟踪」走的是
 * 另一套台账，不在这里出现。
 */
export type SkillView = "installed" | "discover" | "unmanaged";

/**
 * Skills 面板的视图切换。用户级与项目级两处形状一致，
 * 只有已安装条数与是否有「未纳管」页签来自各自的数据，所以标签收在这里。
 */
export function SkillViewTabs({
  view,
  installedCount,
  unmanagedCount,
  onSelect,
}: {
  view: SkillView;
  installedCount: number;
  /** 传了才渲染「未纳管」页签；数字是本次扫描的待处理条数。 */
  unmanagedCount?: number;
  onSelect: (next: SkillView) => void;
}) {
  const tabClass = (active: boolean) =>
    cn(
      "relative flex h-full items-center px-2 text-body transition-colors",
      active
        ? "font-medium text-fg-1 after:absolute after:inset-x-1 after:bottom-0 after:h-0.5 after:bg-action"
        : "text-fg-2 hover:text-fg-1",
    );

  return (
    <div
      role="tablist"
      aria-label="Skills 视图"
      className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-6"
    >
      <button
        type="button"
        role="tab"
        aria-selected={view === "installed"}
        onClick={() => onSelect("installed")}
        className={tabClass(view === "installed")}
      >
        已安装
        <span className="ms-1.5 text-caption text-fg-3">{installedCount}</span>
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={view === "discover"}
        onClick={() => onSelect("discover")}
        className={tabClass(view === "discover")}
      >
        发现
      </button>
      {unmanagedCount !== undefined && (
        <button
          type="button"
          role="tab"
          aria-selected={view === "unmanaged"}
          onClick={() => onSelect("unmanaged")}
          className={tabClass(view === "unmanaged")}
        >
          未纳管
          <span className="ms-1.5 text-caption text-fg-3">{unmanagedCount}</span>
        </button>
      )}
    </div>
  );
}
