import { cn } from "../../lib/cn";

export interface SegmentedItem<T extends string> {
  value: T;
  label: string;
  /** 置灰且不可点击；用于尚未满足前提条件的选项。 */
  disabled?: boolean;
}

export interface SegmentedControlProps<T extends string> {
  items: SegmentedItem<T>[];
  value: T;
  onValueChange: (value: T) => void;
  ariaLabel: string;
  size?: "sm" | "md";
}

/** 页内模式切换（按时间/按项目、表单/JSON）。 */
export function SegmentedControl<T extends string>({
  items,
  value,
  onValueChange,
  ariaLabel,
  size = "md",
}: SegmentedControlProps<T>) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className="inline-flex items-center rounded-control bg-subtle p-0.5"
    >
      {items.map((item) => {
        const active = item.value === value;
        return (
          <button
            key={item.value}
            type="button"
            aria-pressed={active}
            disabled={item.disabled}
            onClick={() => onValueChange(item.value)}
            className={cn(
              "rounded-[4px] font-medium transition-colors duration-150",
              size === "sm" ? "h-6 px-2 text-badge" : "h-7 px-3 text-caption",
              active
                ? "bg-surface text-fg-1 shadow-sm"
                : "text-fg-2 hover:text-fg-1",
              item.disabled && "cursor-not-allowed opacity-50 hover:text-fg-2",
            )}
          >
            {item.label}
          </button>
        );
      })}
    </div>
  );
}