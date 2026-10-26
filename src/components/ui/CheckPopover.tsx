import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import { Checkbox } from "./Checkbox";

/**
 * 多选筛选的下拉层。与 Select 共用同一套触发器样式；
 * 点选选项不收起（方便连续勾选），点外部或 Esc 才关闭。
 */
export function CheckPopover({
  label,
  children,
}: {
  label: ReactNode;
  children: ReactNode;
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

  return (
    <div ref={rootRef} className="relative shrink-0">
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
        className="flex h-8 cursor-pointer items-center gap-1.5 rounded-control border border-border bg-surface px-2.5 text-sm text-fg-1 transition-colors duration-150 hover:bg-subtle focus:border-action focus:outline-none"
      >
        {label}
        <ChevronDown size={12} aria-hidden />
      </button>
      {open && (
        <div className="absolute end-0 z-30 mt-1 max-h-72 w-64 overflow-auto rounded-panel border border-border bg-surface p-2 shadow-v7-md">
          {children}
        </div>
      )}
    </div>
  );
}

/** 下拉层里的一行复选项；整行可点，行文本即无障碍名称。 */
export function CheckOption({
  label,
  checked,
  onChange,
  mono = false,
}: {
  label: ReactNode;
  checked: boolean;
  onChange: (checked: boolean) => void;
  mono?: boolean;
}) {
  return (
    <label className="flex items-center gap-2 rounded-control px-1.5 py-1 text-sm text-fg-1">
      <Checkbox
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className={mono ? "truncate font-mono" : undefined}>{label}</span>
    </label>
  );
}
