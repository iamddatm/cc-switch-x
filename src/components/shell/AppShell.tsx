import { useCallback, useEffect, useState } from "react";
import { FolderPlus, Sparkles } from "lucide-react";
import type { Pane, ProjectScope, Theme } from "../../types";
import { api } from "../../lib/api";
import { useProjects } from "../../hooks/useProjects";
import { useTheme } from "../../hooks/useTheme";
import { useUpdate } from "../../hooks/useUpdate";
import { Button } from "../ui/Button";
import { EmptyState } from "../ui/EmptyState";
import { Notice } from "../ui/Notice";
import { PageHeader } from "../ui/PageHeader";
import { Sidebar } from "./Sidebar";
import { TitleBar } from "./TitleBar";
import { AgentsPanel } from "../../panels/AgentsPanel";
import { BackupsPanel } from "../../panels/BackupsPanel";
import { HooksPanel } from "../../panels/HooksPanel";
import { McpPanel } from "../../panels/McpPanel";
import { NetworkPanel } from "../../panels/NetworkPanel";
import { ProjectSkillsPanel } from "../../panels/ProjectSkillsPanel";
import { PromptsPanel } from "../../panels/PromptsPanel";
import { RulesPanel } from "../../panels/RulesPanel";
import { SessionsPanel } from "../../panels/SessionsPanel";
import { SettingsPanel } from "../../panels/SettingsPanel";
import { SkillsPanel } from "../../panels/SkillsPanel";
import { SubagentsPanel } from "../../panels/SubagentsPanel";

const SCOPE_STORAGE_KEY = "ccswitchx.scope";

/** 项目范围只有 Skills 是项目级资产；停在这些 pane 上时自动跳到 Skills。会话仍可见，但按当前项目过滤。 */
const GLOBAL_ONLY_PANES = new Set<Pane>(["mcp", "prompt", "rule", "subagent", "hook"]);

function ProjectEmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex h-full flex-col">
      <PageHeader icon={Sparkles} title="Skills" description="项目级 Skills" />
      <div className="flex min-h-0 flex-1 items-center justify-center">
        <EmptyState
          icon={FolderPlus}
          title="还没有项目"
          description="添加一个项目后，就能查看并同步它里面各 agent 的 .claude / .qoder / .agents skills。"
          actions={<Button onClick={onAdd}>添加项目…</Button>}
        />
      </div>
    </div>
  );
}

export function AppShell() {
  const [pane, setPane] = useState<Pane>("mcp");
  const [theme, setTheme] = useState<Theme>("system");
  const [scope, setScopeState] = useState<ProjectScope>(() =>
    localStorage.getItem(SCOPE_STORAGE_KEY) === "project" ? "project" : "global",
  );
  const projects = useProjects();

  const setScope = useCallback((next: ProjectScope) => {
    setScopeState(next);
    localStorage.setItem(SCOPE_STORAGE_KEY, next);
  }, []);

  useEffect(() => {
    if (scope === "project" && GLOBAL_ONLY_PANES.has(pane)) setPane("skill");
  }, [pane, scope]);

  // 主题只在壳层应用一次：Settings 改完回传这里，避免两个 useTheme 互相覆盖。
  useEffect(() => {
    void api
      .getSettings()
      .then((value) => setTheme(value.theme))
      .catch((e) => console.warn("读取设置失败，主题保持跟随系统：", e));
  }, []);
  useTheme(theme);
  const update = useUpdate();

  const relocate = async (id: string) => {
    const added = await projects.add();
    if (added && added.id !== id) await projects.remove(id);
  };

  const content = {
    mcp: <McpPanel />,
    skill:
      scope === "project" ? (
        projects.active ? (
          <ProjectSkillsPanel key={projects.active.id} project={projects.active} />
        ) : (
          <ProjectEmptyState onAdd={() => void projects.add()} />
        )
      ) : (
        <SkillsPanel />
      ),
    prompt: <PromptsPanel />,
    rule: <RulesPanel />,
    subagent: <SubagentsPanel />,
    hook: <HooksPanel />,
    sessions: <SessionsPanel project={scope === "project" ? projects.active : null} />,
    agents: <AgentsPanel />,
    network: <NetworkPanel />,
    backup: <BackupsPanel />,
    settings: <SettingsPanel theme={theme} onThemeChange={setTheme} update={update} />,
  }[pane];

  return (
    <div className="flex h-full flex-col bg-app">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar
          pane={pane}
          onSelect={setPane}
          hasUpdate={update.hasUpdate}
          scope={scope}
          projects={projects.projects}
          activeProjectId={projects.activeId}
          onScopeChange={setScope}
          onSelectProject={(id) => void projects.select(id)}
          onAddProject={() => void projects.add()}
          onRemoveProject={(id) => void projects.remove(id)}
          onRelocateProject={(id) => void relocate(id)}
        />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {projects.error && (
            <div className="px-6 pt-3">
              <Notice tone="danger">项目注册表操作失败：{projects.error}</Notice>
            </div>
          )}
          {content}
        </main>
      </div>
    </div>
  );
}
