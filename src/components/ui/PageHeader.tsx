import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

export interface PageHeaderProps {
  icon: LucideIcon;
  title: string;
  description?: string;
  actions?: ReactNode;
  className?: string;
}

/** 52px 页头：图标 + 标题 + 右侧操作；标题仍用 h2，保持现有标题层级。 */
export function PageHeader({ icon: Icon, title, description, actions, className }: PageHeaderProps) {
  return (
    <header
      className={cn(
        "flex h-[52px] shrink-0 items-center gap-3 border-b border-border px-6",
        className,
      )}
    >
      <span className="flex h-7 w-7 items-center justify-center rounded-control bg-action-soft text-action-text">
        <Icon size={18} aria-hidden />
      </span>
      <div className="min-w-0">
        <h2 className="truncate text-page text-fg-1">{title}</h2>
        {description && <p className="truncate text-caption text-fg-3">{description}</p>}
      </div>
      {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
    </header>
  );
}