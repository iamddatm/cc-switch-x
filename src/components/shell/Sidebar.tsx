import {
  ArchiveRestore,
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

interface SidebarItem {
  pane: Pane;
  label: string;
  icon: LucideIcon;
}

/** 常用资产放最上面，会话是日常入口，Agents / 网络 / 备份 / 设置沉到底部。 */
const ASSET_ITEMS: SidebarItem[] = [
  { pane: "skill", label: "Skills", icon: Sparkles },
  { pane: "mcp", label: "MCP", icon: Server },
  { pane: "prompt", label: "提示词", icon: BookOpen },
  { pane: "rule", label: "Rules", icon: ScrollText },
  { pane: "subagent", label: "Subagents", icon: Bot },
  { pane: "hook", label: "Hooks", icon: Webhook },
];

const TOOL_ITEMS: SidebarItem[] = [{ pane: "sessions", label: "会话", icon: History }];

const MANAGE_ITEMS: SidebarItem[] = [
  { pane: "agents", label: "Agents", icon: Layers },
  { pane: "network", label: "网络", icon: Globe },
  { pane: "backup", label: "备份", icon: ArchiveRestore },
  { pane: "settings", label: "Settings", icon: Settings },
];

function SidebarGroup({
  label,
  items,
  pane,
  onSelect,
  hasUpdate,
  className,
}: {
  label: string;
  items: SidebarItem[];
  pane: Pane;
  onSelect: (pane: Pane) => void;
  hasUpdate: boolean;
  className?: string;
}) {
  return (
    <div className={className}>
      <p className="px-2.5 pb-1 text-badge text-fg-3">{label}</p>
      <ul className="space-y-0.5">
        {items.map((item) => {
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
    </div>
  );
}

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
    <nav
      aria-label="主导航"
      className="flex w-[200px] shrink-0 flex-col border-r border-border bg-sidebar"
    >
      <div className="min-h-0 flex-1 overflow-auto px-2 py-2">
        <SidebarGroup
          label="资产"
          items={ASSET_ITEMS}
          pane={pane}
          onSelect={onSelect}
          hasUpdate={hasUpdate}
        />
        <SidebarGroup
          label="工具"
          items={TOOL_ITEMS}
          pane={pane}
          onSelect={onSelect}
          hasUpdate={hasUpdate}
          className="mt-3"
        />
      </div>
      <div className="shrink-0 border-t border-border px-2 py-2">
        <SidebarGroup
          label="管理"
          items={MANAGE_ITEMS}
          pane={pane}
          onSelect={onSelect}
          hasUpdate={hasUpdate}
        />
      </div>
    </nav>
  );
}
