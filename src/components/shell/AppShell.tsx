import { useState } from "react";
import type { Pane } from "../../types";
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
      <main className="flex-1 overflow-auto p-6">{content}</main>
    </div>
  );
}
