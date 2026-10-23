import type { KeyboardEventHandler } from "react";
import { Search, X } from "lucide-react";
import { cn } from "../../lib/cn";

export interface SearchFieldProps {
  value: string;
  onValueChange: (value: string) => void;
  placeholder: string;
  ariaLabel: string;
  clearLabel?: string;
  className?: string;
  onKeyDown?: KeyboardEventHandler<HTMLInputElement>;
}

export function SearchField({
  value,
  onValueChange,
  placeholder,
  ariaLabel,
  clearLabel = "清除搜索",
  className,
  onKeyDown,
}: SearchFieldProps) {
  return (
    <div className={cn("relative", className)}>
      <Search
        size={14}
        aria-hidden
        className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-fg-3"
      />
      <input
        type="text"
        value={value}
        aria-label={ariaLabel}
        placeholder={placeholder}
        onChange={(event) => onValueChange(event.target.value)}
        onKeyDown={onKeyDown}
        className="h-8 w-full rounded-control border border-border bg-surface pl-8 pr-8 text-body text-fg-1 placeholder:text-fg-3 transition-colors duration-150 focus:border-action focus:outline-none"
      />
      {value && (
        <button
          type="button"
          aria-label={clearLabel}
          onClick={() => onValueChange("")}
          className="absolute right-1.5 top-1/2 flex h-5 w-5 -translate-y-1/2 items-center justify-center rounded text-fg-3 hover:bg-subtle hover:text-fg-1"
        >
          <X size={12} aria-hidden />
        </button>
      )}
    </div>
  );
}