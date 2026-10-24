import type { ButtonHTMLAttributes } from "react";
import { Loader2 } from "lucide-react";
import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "dangerSolid";
type Size = "sm" | "md";

const VARIANTS: Record<Variant, string> = {
  primary: "bg-action text-action-fg hover:bg-action-hover",
  secondary: "border border-border bg-surface text-fg-1 hover:bg-subtle",
  ghost: "text-fg-2 hover:bg-subtle hover:text-fg-1",
  danger: "border border-border bg-surface text-danger-text hover:bg-danger-soft",
  dangerSolid: "bg-danger text-action-fg hover:opacity-90",
};

const SIZES: Record<Size, string> = {
  sm: "h-7 px-2.5 text-caption",
  md: "h-8 px-3 text-body",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  loading?: boolean;
}

/** 统一按钮：主操作只有一种橙色 solid，其余用描边或 ghost。 */
export function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  className,
  disabled,
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      type="button"
      {...rest}
      disabled={disabled || loading}
      className={cn(
        "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-control font-medium transition-colors duration-150",
        "disabled:pointer-events-none disabled:opacity-45",
        VARIANTS[variant],
        SIZES[size],
        className,
      )}
    >
      {loading && <Loader2 size={14} className="animate-spin" aria-hidden />}
      {children}
    </button>
  );
}