import { useEffect, useState } from "react";
import type { Pane, Theme } from "../../types";
import { api } from "../../lib/api";
import { useTheme } from "../../hooks/useTheme";
import { useUpdate } from "../../hooks/useUpdate";
import { Sidebar } from "./Sidebar";
import { AgentsPanel } from "../../panels/AgentsPanel";
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

  // 主题只在壳层应用一次：Settings 改完回传这里，避免两个 useTheme 互相覆盖。
  useEffect(() => {
    void api
      .getSettings()
      .then((value) => setTheme(value.theme))
      .catch((e) => console.warn("读取设置失败，主题保持跟随系统：", e));
  }, []);
  useTheme(theme);
  const update = useUpdate();

  const content = {
    mcp: <McpPanel />,
    skill: <SkillsPanel />,
    prompt: <PromptsPanel />,
    rule: <RulesPanel />,
    subagent: <SubagentsPanel />,
    hook: <HooksPanel />,
    sessions: <SessionsPanel />,
    agents: <AgentsPanel />,
    settings: <SettingsPanel theme={theme} onThemeChange={setTheme} update={update} />,
  }[pane];

  return (
    <div className="flex h-full bg-app">
      <Sidebar pane={pane} onSelect={setPane} hasUpdate={update.hasUpdate} />
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">{content}</main>
    </div>
  );
}