import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "../../lib/cn";
import type { MenuItem } from "./Menu";

export interface ContextMenuProps {
  label: string;
  /** 指针位置（viewport 坐标）。 */
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}

/**
 * 右键菜单：在与「⋯」菜单相同的位置弹出同一组操作。
 * 点击菜单外、按 Esc 或选中一项都会关闭。
 */
export function ContextMenu({ label, x, y, items, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  const [position, setPosition] = useState({ top: y, left: x });
  // 父组件每次渲染都会传新的闭包；只留最新一份，避免重挂监听时把焦点抢回第一项。
  onCloseRef.current = onClose;

  // 先按指针位置渲染，量到实际尺寸后再往视口内收，避免贴右/下边缘时被裁掉。
  useLayoutEffect(() => {
    const rect = menuRef.current?.getBoundingClientRect();
    if (!rect) return;
    setPosition({
      top: Math.max(8, Math.min(y, window.innerHeight - rect.height - 8)),
      left: Math.max(8, Math.min(x, window.innerWidth - rect.width - 8)),
    });
  }, [x, y]);

  useEffect(() => {
    menuRef.current
      ?.querySelector<HTMLButtonElement>('[role="menuitem"]:not([disabled])')
      ?.focus();

    const onPointerDown = (event: PointerEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      onCloseRef.current();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onCloseRef.current();
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);

  return createPortal(
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
            onClose();
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
  );
}
