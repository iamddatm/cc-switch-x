import type { ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  size?: "sm" | "md";
}

/** 图标按钮必须以 label 提供可访问名；视觉上只有图标。 */
export function IconButton({ label, size = "sm", className, children, ...rest }: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      {...rest}
      className={cn(
        "inline-flex items-center justify-center rounded-control text-fg-2 transition-colors duration-150",
        "hover:bg-subtle hover:text-fg-1 disabled:pointer-events-none disabled:opacity-45",
        size === "sm" ? "h-7 w-7" : "h-8 w-8",
        className,
      )}
    >
      {children}
    </button>
  );
}