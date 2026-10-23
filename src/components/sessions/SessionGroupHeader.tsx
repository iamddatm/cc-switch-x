import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "../../lib/cn";
import { formatRelativeTime } from "../../lib/format";
import { projectName } from "../../lib/sessionGroups";

export interface SessionGroupHeaderProps {
  project: string;
  count: number;
  latest: number;
  expanded: boolean;
  onToggle: () => void;
}

/** 按项目分组头：项目名 + 路径 + 会话数/最近活跃；整行可折叠。 */
export function SessionGroupHeader({
  project,
  count,
  latest,
  expanded,
  onToggle,
}: SessionGroupHeaderProps) {
  const name = projectName(project);
  return (
    <button
      type="button"
      aria-expanded={expanded}
      onClick={onToggle}
      className={cn(
        "flex h-9 w-full items-center gap-2 bg-subtle px-3 text-left transition-colors duration-150",
        "hover:bg-selected",
      )}
    >
      {expanded ? (
        <ChevronDown size={14} className="shrink-0 text-fg-3" aria-hidden />
      ) : (
        <ChevronRight size={14} className="shrink-0 text-fg-3" aria-hidden />
      )}
      <span className="shrink-0 text-body font-medium text-fg-1">{name}</span>
      {project !== name && project !== "未知项目" && (
        <span className="min-w-0 truncate font-mono text-caption text-fg-3" title={project}>
          {project}
        </span>
      )}
      <span className="ml-auto shrink-0 text-caption text-fg-3">
        {count} 个会话 · {formatRelativeTime(latest)}
      </span>
    </button>
  );
}