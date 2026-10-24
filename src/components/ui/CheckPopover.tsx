import type { ReactNode } from "react";
import { ChevronDown } from "lucide-react";
import { Checkbox } from "./Checkbox";

/** 多选筛选的下拉层；用原生 details，点选不会像菜单那样立即收起。 */
export function CheckPopover({
  label,
  children,
}: {
  label: ReactNode;
  children: ReactNode;
}) {
  return (
    <details className="relative shrink-0">
      <summary className="flex h-8 cursor-pointer list-none items-center gap-1.5 rounded-control border border-border bg-surface px-2.5 text-caption text-fg-2 transition-colors hover:bg-subtle hover:text-fg-1 [&::-webkit-details-marker]:hidden">
        {label}
        <ChevronDown size={12} aria-hidden />
      </summary>
      <div className="absolute end-0 z-30 mt-1 max-h-72 w-64 overflow-auto rounded-panel border border-border bg-surface p-2 shadow-v7-md">
        {children}
      </div>
    </details>
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
    <label className="flex items-center gap-2 rounded-control px-1.5 py-1 text-caption text-fg-2">
      <Checkbox
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className={mono ? "truncate font-mono" : undefined}>{label}</span>
    </label>
  );
}
