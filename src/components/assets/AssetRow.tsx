import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

export interface AssetRowProps {
  title: string;
  /** 等宽副标题（目录 id / transport / command）。 */
  subtitle?: string;
  description?: string;
  /** 标题行内的紧凑 badge（transport、已启用）。 */
  titleBadges?: ReactNode;
  /** 右对齐的 badge（来源仓库等）。 */
  badges?: ReactNode;
  agentCells: ReactNode;
  actions: ReactNode;
  onOpen: () => void;
  error?: string | null;
  footer?: string | null;
  footerTone?: "muted" | "warning";
  active?: boolean;
}

/**
 * 资产列表行：整行透明按钮打开编辑，右侧 agent 勾选与「⋯」浮在上面。
 * 用绝对定位的透明按钮而不是把整行做成 button，避免嵌套交互元素。
 */
export function AssetRow({
  title,
  subtitle,
  description,
  titleBadges,
  badges,
  agentCells,
  actions,
  onOpen,
  error,
  footer,
  footerTone = "muted",
  active = false,
}: AssetRowProps) {
  return (
    <li className="border-t border-border first:border-t-0">
      <div className={cn("relative flex h-14 items-center hover:bg-subtle", active && "bg-subtle")}>
        <button
          type="button"
          aria-label={`编辑 ${title}`}
          onClick={onOpen}
          className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-offset-[-2px]"
        />
        <div className="pointer-events-none relative z-10 flex min-w-0 flex-1 items-center gap-2 px-4">
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 shrink truncate text-body font-medium text-fg-1">
                {title}
              </span>
              {titleBadges && <span className="flex shrink-0 items-center gap-1">{titleBadges}</span>}
              {subtitle && subtitle !== title && (
                <span className="min-w-0 max-w-[240px] truncate font-mono text-caption text-fg-3">
                  {subtitle}
                </span>
              )}
              {badges && (
                <span className="ml-auto flex min-w-0 shrink-0 items-center gap-1">{badges}</span>
              )}
            </div>
            {description && (
              <p className="truncate text-caption text-fg-2" title={description}>
                {description}
              </p>
            )}
          </div>
        </div>
        <div className="relative z-10 flex shrink-0 items-center">{agentCells}</div>
        <div className="relative z-10 flex w-12 shrink-0 items-center justify-center">{actions}</div>
      </div>
      {(error || footer) && (
        <p
          role={error ? "alert" : undefined}
          className={cn(
            "px-4 pb-2 text-caption",
            error
              ? "text-danger-text"
              : footerTone === "warning"
                ? "text-warning-text"
                : "text-fg-3",
          )}
        >
          {error ?? footer}
        </p>
      )}
    </li>
  );
}