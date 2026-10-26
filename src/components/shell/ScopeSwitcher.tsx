import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, FolderKanban, FolderOpen, Plus } from "lucide-react";
import { api } from "../../lib/api";
import type { ProjectScope, ProjectView } from "../../types";
import { byLastOpenedDesc } from "../../hooks/useProjects";
import { Badge } from "../ui/Badge";
import { Menu, type MenuItem } from "../ui/Menu";

/** 侧栏顶部的项目选择器：选中项目即切到项目范围，选「全局配置」切回全局；项目范围只对 Skills 生效。 */
export function ScopeSwitcher({
  scope,
  projects,
  activeProjectId,
  onScopeChange,
  onSelectProject,
  onAddProject,
  onRemoveProject,
  onRelocateProject,
}: {
  scope: ProjectScope;
  projects: ProjectView[];
  activeProjectId: string | null;
  onScopeChange: (scope: ProjectScope) => void;
  onSelectProject: (id: string) => void;
  onAddProject: () => void;
  onRemoveProject: (id: string) => void;
  onRelocateProject: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const active = projects.find((project) => project.id === activeProjectId) ?? null;
  const ordered = [...projects].sort(byLastOpenedDesc);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      if (rootRef.current?.contains(target)) return;
      // 行内 ⋯ 菜单的弹层是 portal 到 body 的，不算「点到外面」：
      // 在 pointerdown 阶段卸载下拉，菜单项的 click 就永远收不到，三项操作全点不动。
      if (target?.closest('[role="menu"]')) return;
      setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      setOpen(false);
      triggerRef.current?.focus();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const pickGlobal = () => {
    onScopeChange("global");
    setOpen(false);
  };

  // 选中项目即进入项目范围，不再单独提供「全局配置 / 项目」切换。
  const pickProject = (id: string) => {
    onScopeChange("project");
    onSelectProject(id);
    setOpen(false);
  };

  const menuItems = (project: ProjectView): MenuItem[] => {
    const relocate: MenuItem = {
      label: "重新定位…",
      onSelect: () => {
        setOpen(false);
        onRelocateProject(project.id);
      },
    };
    const reveal: MenuItem = {
      label: "在文件夹中显示",
      onSelect: () => {
        setOpen(false);
        void api
          .revealProjectFolder(project.id)
          .catch((e) => console.warn("打开项目文件夹失败：", e));
      },
    };
    return [
      project.exists ? reveal : relocate,
      { label: "移除项目", danger: true, onSelect: () => onRemoveProject(project.id) },
    ];
  };

  return (
    <div ref={rootRef} className="relative px-2 pb-2">
      <button
        ref={triggerRef}
        type="button"
        aria-expanded={open}
        aria-label={scope === "global" ? "配置范围：全局配置" : "配置范围：项目"}
        onClick={() => setOpen((value) => !value)}
        className="flex h-8 w-full items-center gap-2 rounded-control border border-border bg-surface px-2.5 text-left text-body text-fg-1 transition-colors duration-150 hover:bg-subtle focus:border-action focus:outline-none"
      >
        <FolderKanban size={14} aria-hidden className="shrink-0 text-fg-3" />
        <span className="min-w-0 flex-1 truncate">
          {scope === "global" ? "全局配置" : (active?.name ?? "未选择项目")}
        </span>
        {scope === "project" && <Badge tone="brand">项目</Badge>}
        <ChevronDown size={12} aria-hidden className="shrink-0 text-fg-3" />
      </button>

      {open && (
        <div className="absolute inset-x-2 z-40 mt-1 max-h-[70vh] overflow-auto rounded-panel border border-border bg-surface p-2 shadow-v7-md">
          <button
            type="button"
            aria-pressed={scope === "global"}
            onClick={pickGlobal}
            className="flex h-8 w-full items-center gap-2 rounded-control px-1.5 text-left text-sm transition-colors duration-150 hover:bg-subtle"
          >
            <span className="flex h-4 w-4 shrink-0 items-center justify-center">
              {scope === "global" && <Check size={13} aria-hidden className="text-action-text" />}
            </span>
            <span className="truncate text-fg-1">全局配置</span>
          </button>

          <p className="px-1.5 pb-1 pt-2 text-badge text-fg-3">项目</p>
          {ordered.length === 0 ? (
            <p className="px-1.5 py-1 text-caption text-fg-3">还没有添加项目</p>
          ) : (
            <ul className="space-y-0.5">
              {ordered.map((project) => (
                <li key={project.id} className="flex items-center gap-0.5">
                  <button
                    type="button"
                    aria-pressed={scope === "project" && project.id === activeProjectId}
                    onClick={() => pickProject(project.id)}
                    className="flex min-w-0 flex-1 items-center gap-2 rounded-control px-1.5 py-1 text-left transition-colors duration-150 hover:bg-subtle"
                  >
                    <span className="flex h-4 w-4 shrink-0 items-center justify-center">
                      {scope === "project" && project.id === activeProjectId && (
                        <Check size={13} aria-hidden className="text-action-text" />
                      )}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm text-fg-1">{project.name}</span>
                      <span
                        className="block truncate font-mono text-badge text-fg-3"
                        title={project.path}
                      >
                        {project.path}
                      </span>
                    </span>
                    {!project.exists && <Badge tone="danger">路径不存在</Badge>}
                  </button>
                  <Menu label={`${project.name} 更多操作`} items={menuItems(project)} />
                </li>
              ))}
            </ul>
          )}

          <button
            type="button"
            onClick={() => {
              setOpen(false);
              onAddProject();
            }}
            className="mt-1 flex h-7 w-full items-center gap-1.5 rounded-control px-1.5 text-caption text-fg-2 transition-colors duration-150 hover:bg-subtle hover:text-fg-1"
          >
            <FolderOpen size={13} aria-hidden />
            <Plus size={12} aria-hidden />
            <span>添加项目…</span>
          </button>
        </div>
      )}
    </div>
  );
}
