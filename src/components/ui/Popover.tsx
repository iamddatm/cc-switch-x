import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "../../lib/cn";

/**
 * 下拉触发器 + 面板的通用外壳。与 Select 同为单选入口，但支持任意内容
 * （多选清单、分组单选），点外部或 Esc 关闭。
 *
 * - `boxed`：带边框的独立控件，用于工具栏里需要一眼认出的筛选；
 * - `quiet`：无边框文字按钮，用于紧凑的筛选行，避免一排框把列表挤窄。
 */
export function Popover({
  label,
  children,
  variant = "boxed",
  active = false,
  panelClassName,
}: {
  label: ReactNode;
  /** 函数形式可拿到 `close`，选中后立即收起面板。 */
  children: ReactNode | ((close: () => void) => ReactNode);
  variant?: "boxed" | "quiet";
  /** 有非默认筛选时加深触发器，让用户知道列表被收窄过。 */
  active?: boolean;
  panelClassName?: string;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current?.contains(event.target as Node)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      setOpen(false);
      triggerRef.current?.focus();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const triggerClass =
    variant === "quiet"
      ? cn(
          "flex h-8 cursor-pointer items-center gap-1 rounded-control px-2 text-body transition-colors duration-150 hover:bg-subtle focus:outline-none",
          active ? "bg-subtle font-medium text-fg-1" : "text-fg-2 hover:text-fg-1",
        )
      : "flex h-8 cursor-pointer items-center gap-1.5 rounded-control border border-border bg-surface px-2.5 text-sm text-fg-1 transition-colors duration-150 hover:bg-subtle focus:border-action focus:outline-none";

  return (
    <div ref={rootRef} className="relative shrink-0">
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className={triggerClass}
      >
        {label}
        <ChevronDown size={12} aria-hidden />
      </button>
      {open && (
        <div
          className={cn(
            "absolute end-0 z-30 mt-1 max-h-72 w-64 overflow-auto rounded-panel border border-border bg-surface p-2 shadow-v7-md",
            panelClassName,
          )}
        >
          {typeof children === "function" ? children(() => setOpen(false)) : children}
        </div>
      )}
    </div>
  );
}
