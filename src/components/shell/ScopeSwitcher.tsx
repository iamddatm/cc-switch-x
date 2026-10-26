import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, FolderKanban, FolderOpen, Plus } from "lucide-react";
import { api } from "../../lib/api";
import type { ProjectScope, ProjectView } from "../../types";
import { byLastOpenedDesc } from "../../hooks/useProjects";
import { Badge } from "../ui/Badge";
import { Menu, type MenuItem } from "../ui/Menu";

/** 侧栏顶部的全局 / 项目范围切换；项目范围只对 Skills 生效。 */
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
      if (rootRef.current?.contains(event.target as Node)) return;
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

  const pickScope = (next: ProjectScope) => {
    onScopeChange(next);
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
        aria-label={scope === "global" ? "切换范围：全局配置" : "切换范围：项目"}
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
          <p className="px-1.5 pb-1 text-badge text-fg-3">切换范围</p>
          <div className="grid grid-cols-2 gap-1">
            {(
              [
                { value: "global", label: "全局配置" },
                { value: "project", label: "项目" },
              ] as const
            ).map((option) => {
              const selected = scope === option.value;
              return (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => pickScope(option.value)}
                  className={`h-7 rounded-control text-caption transition-colors duration-150 ${
                    selected
                      ? "bg-selected font-medium text-fg-1"
                      : "text-fg-2 hover:bg-subtle hover:text-fg-1"
                  }`}
                >
                  {option.label}
                </button>
              );
            })}
          </div>

          <p className="px-1.5 pb-1 pt-2 text-badge text-fg-3">项目</p>
          {ordered.length === 0 ? (
            <p className="px-1.5 py-1 text-caption text-fg-3">还没有添加项目</p>
          ) : (
            <ul className="space-y-0.5">
              {ordered.map((project) => (
                <li key={project.id} className="flex items-center gap-0.5">
                  <button
                    type="button"
                    onClick={() => {
                      onSelectProject(project.id);
                      setOpen(false);
                    }}
                    className="flex min-w-0 flex-1 items-center gap-2 rounded-control px-1.5 py-1 text-left transition-colors duration-150 hover:bg-subtle"
                  >
                    <span className="flex h-4 w-4 shrink-0 items-center justify-center">
                      {project.id === activeProjectId && (
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
