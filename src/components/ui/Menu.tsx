import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { MoreHorizontal } from "lucide-react";
import { cn } from "../../lib/cn";

export interface MenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
  hint?: string;
}

export interface MenuProps {
  label: string;
  items: MenuItem[];
  trigger?: ReactNode;
  align?: "start" | "end";
  className?: string;
  /** 给了 trigger 内容时通常需要覆盖默认的 28px 图标按钮尺寸。 */
  buttonClassName?: string;
}

/**
 * 行尾「⋯」菜单。用 fixed 定位 + portal，避免被列表的 overflow 裁掉；
 * 点击外部或 Esc 关闭，打开后焦点落到第一项。
 */
export function Menu({
  label,
  items,
  trigger,
  align = "end",
  className,
  buttonClassName,
}: MenuProps) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ top: 0, left: 0 });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const show = () => {
    const rect = triggerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const width = 176;
    const left = align === "end" ? Math.max(8, rect.right - width) : rect.left;
    const top = Math.min(rect.bottom + 4, window.innerHeight - 16);
    setPosition({ top, left });
    setOpen(true);
  };

  useEffect(() => {
    if (!open) return;
    const menu = menuRef.current;
    menu?.querySelector<HTMLButtonElement>('[role="menuitem"]:not([disabled])')?.focus();

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (menuRef.current?.contains(target) || triggerRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        title={label}
        onClick={() => (open ? setOpen(false) : show())}
        className={cn(
          "inline-flex h-7 w-7 items-center justify-center rounded-control text-fg-2 transition-colors duration-150",
          "hover:bg-subtle hover:text-fg-1",
          className,
          buttonClassName,
        )}
      >
        {trigger ?? <MoreHorizontal size={16} aria-hidden />}
      </button>
      {open &&
        createPortal(
          <div
            ref={menuRef}
            role="menu"
            aria-label={label}
            style={{ top: position.top, left: position.left }}
            className="fixed z-[70] w-44 rounded-panel border border-border bg-surface p-1 shadow-v7-md"
          >
            {items.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                disabled={item.disabled}
                onClick={() => {
                  setOpen(false);
                  item.onSelect();
                }}
                className={cn(
                  "flex w-full flex-col items-start rounded-control px-2.5 py-1.5 text-left text-body transition-colors duration-150",
                  "disabled:pointer-events-none disabled:opacity-45",
                  item.danger
                    ? "text-danger-text hover:bg-danger-soft"
                    : "text-fg-1 hover:bg-subtle",
                )}
              >
                <span>{item.label}</span>
                {item.hint && <span className="text-caption text-fg-3">{item.hint}</span>}
              </button>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}