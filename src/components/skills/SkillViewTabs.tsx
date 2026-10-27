import { cn } from "../../lib/cn";

/**
 * Skills 面板的「已安装 / 发现」视图切换。用户级与项目级两处形状一致，
 * 只有已安装条数来自各自的行数据，所以标签收在这里。
 */
export function SkillViewTabs({
  view,
  installedCount,
  onSelect,
}: {
  view: "installed" | "discover";
  installedCount: number;
  onSelect: (next: "installed" | "discover") => void;
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
    </div>
  );
}
