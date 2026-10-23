import type { SelectHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...rest}
      className={cn(
        "h-8 rounded-control border border-border bg-surface px-2.5 text-body text-fg-1",
        "transition-colors duration-150 focus:border-action focus:outline-none disabled:opacity-45",
        className,
      )}
    >
      {children}
    </select>
  );
}