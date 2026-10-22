import type { Pane } from "../../types";

const ITEMS: { pane: Pane; label: string }[] = [
  { pane: "mcp", label: "MCP" },
  { pane: "skill", label: "Skills" },
  { pane: "prompt", label: "Prompts" },
  { pane: "rule", label: "Rules" },
  { pane: "subagent", label: "Subagents" },
  { pane: "hook", label: "Hooks" },
  { pane: "sessions", label: "Sessions" },
  { pane: "settings", label: "Settings" },
];

export function Sidebar({ pane, onSelect }: { pane: Pane; onSelect: (pane: Pane) => void }) {
  return (
    <nav className="w-48 border-r border-slate-200 bg-slate-50 p-3 dark:border-slate-700 dark:bg-slate-800">
      <div className="mb-4 px-2 text-lg font-semibold">CC Switch X</div>
      <ul className="space-y-1">
        {ITEMS.map((item) => (
          <li key={item.pane}>
            <button
              type="button"
              onClick={() => onSelect(item.pane)}
              aria-current={pane === item.pane ? "page" : undefined}
              className={`w-full rounded px-2 py-1.5 text-left text-sm ${
                pane === item.pane
                  ? "bg-slate-800 text-white dark:bg-slate-600"
                  : "hover:bg-slate-200 dark:hover:bg-slate-700"
              }`}
            >
              {item.label}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}
