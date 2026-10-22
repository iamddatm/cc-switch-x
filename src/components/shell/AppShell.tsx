import { useEffect, useState } from "react";
import type { Pane, Theme } from "../../types";
import { api } from "../../lib/api";
import { useTheme } from "../../hooks/useTheme";
import { ImportDialog } from "../ImportDialog";
import { Sidebar } from "./Sidebar";
import { HooksPanel } from "../../panels/HooksPanel";
import { McpPanel } from "../../panels/McpPanel";
import { PromptsPanel } from "../../panels/PromptsPanel";
import { RulesPanel } from "../../panels/RulesPanel";
import { SessionsPanel } from "../../panels/SessionsPanel";
import { SettingsPanel } from "../../panels/SettingsPanel";
import { SkillsPanel } from "../../panels/SkillsPanel";
import { SubagentsPanel } from "../../panels/SubagentsPanel";

export function AppShell() {
  const [pane, setPane] = useState<Pane>("mcp");
  const [theme, setTheme] = useState<Theme>("system");
  const [importing, setImporting] = useState(false);

  // 主题在壳层生效：否则只有打开过 Settings 才会应用已保存的主题。
  useEffect(() => {
    void api
      .getSettings()
      .then((value) => setTheme(value.theme))
      .catch((e) => console.warn("读取设置失败，主题保持跟随系统：", e));
  }, []);
  useTheme(theme);

  const content = {
    mcp: <McpPanel />,
    skill: <SkillsPanel />,
    prompt: <PromptsPanel />,
    rule: <RulesPanel />,
    subagent: <SubagentsPanel />,
    hook: <HooksPanel />,
    sessions: <SessionsPanel />,
    settings: <SettingsPanel />,
  }[pane];
  return (
    <div className="flex h-full">
      <Sidebar pane={pane} onSelect={setPane} />
      <main className="flex-1 overflow-auto p-6">
        <div className="mb-4 flex justify-end">
          <button
            type="button"
            className="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
            onClick={() => setImporting(true)}
          >
            从 cc-switch 导入
          </button>
        </div>
        {content}
        {importing && <ImportDialog onClose={() => setImporting(false)} />}
      </main>
    </div>
  );
}
