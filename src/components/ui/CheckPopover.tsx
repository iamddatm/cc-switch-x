import type { ReactNode } from "react";
import { Checkbox } from "./Checkbox";
import { Popover } from "./Popover";

/**
 * 多选筛选的下拉层。点选选项不收起（方便连续勾选），点外部或 Esc 才关闭。
 * 外壳与交互由 `Popover` 提供；`quiet` 变体用于紧凑的筛选行。
 */
export function CheckPopover({
  label,
  children,
  variant,
  active,
}: {
  label: ReactNode;
  children: ReactNode;
  variant?: "boxed" | "quiet";
  active?: boolean;
}) {
  return (
    <Popover label={label} variant={variant} active={active}>
      {children}
    </Popover>
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
