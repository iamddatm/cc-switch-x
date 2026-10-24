import {
  BookOpen,
  Bot,
  Globe,
  History,
  Layers,
  ScrollText,
  Server,
  Settings,
  Sparkles,
  Webhook,
  type LucideIcon,
} from "lucide-react";
import type { Pane } from "../../types";

const ITEMS: { pane: Pane; label: string; icon: LucideIcon }[] = [
  { pane: "mcp", label: "MCP", icon: Server },
  { pane: "skill", label: "Skills", icon: Sparkles },
  { pane: "prompt", label: "提示词", icon: BookOpen },
  { pane: "rule", label: "Rules", icon: ScrollText },
  { pane: "subagent", label: "Subagents", icon: Bot },
  { pane: "hook", label: "Hooks", icon: Webhook },
  { pane: "sessions", label: "会话", icon: History },
  { pane: "agents", label: "Agents", icon: Layers },
  { pane: "network", label: "网络", icon: Globe },
  { pane: "settings", label: "Settings", icon: Settings },
];

export function Sidebar({
  pane,
  onSelect,
  hasUpdate = false,
}: {
  pane: Pane;
  onSelect: (pane: Pane) => void;
  hasUpdate?: boolean;
}) {
  return (
    <nav className="flex w-[200px] shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="flex h-[52px] shrink-0 items-center gap-2 border-b border-border px-4">
        <span className="flex h-6 w-6 items-center justify-center rounded-control bg-action text-white">
          <Layers size={14} aria-hidden />
        </span>
        <span className="text-strong text-fg-1">CC Switch X</span>
      </div>
      <ul className="flex-1 space-y-0.5 overflow-auto p-2">
        {ITEMS.map((item) => {
          const active = pane === item.pane;
          const Icon = item.icon;
          return (
            <li key={item.pane}>
              <button
                type="button"
                onClick={() => onSelect(item.pane)}
                aria-current={active ? "page" : undefined}
                title={
                  item.pane === "settings" && hasUpdate ? "Settings（有可用更新）" : undefined
                }
                className={`flex h-8 w-full items-center gap-2.5 rounded-control px-2.5 text-left text-body transition-colors duration-150 ${
                  active
                    ? "bg-selected font-medium text-fg-1"
                    : "text-fg-2 hover:bg-subtle hover:text-fg-1"
                }`}
              >
                <Icon
                  size={15}
                  aria-hidden
                  className={active ? "text-action-text" : "text-fg-3"}
                />
                <span className="truncate">{item.label}</span>
                {item.pane === "settings" && hasUpdate && (
                  <span
                    className="ml-auto h-1.5 w-1.5 shrink-0 rounded-full bg-action"
                    aria-hidden
                  />
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}