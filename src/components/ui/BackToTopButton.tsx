import { ArrowUp } from "lucide-react";
import { cn } from "../../lib/cn";

/** 滚动容器超过这个距离才显示按钮，避免刚打开就遮挡内容。 */
export const BACK_TO_TOP_THRESHOLD = 320;

/** 滚动容器超过一定距离后由调用方渲染；悬浮在容器右下角，点击回到顶部。 */
export function BackToTopButton({
  onClick,
  className,
}: {
  onClick: () => void;
  className?: string;
}) {
  return (
    <button
      type="button"
      aria-label="回到顶部"
      title="回到顶部"
      onClick={onClick}
      className={cn(
        "absolute bottom-5 right-5 z-10 flex h-9 w-9 items-center justify-center rounded-full border border-border bg-surface text-fg-2 shadow-md transition-colors duration-150 hover:bg-subtle hover:text-fg-1",
        className,
      )}
    >
      <ArrowUp size={16} aria-hidden />
    </button>
  );
}
