import { useEffect, useRef, useState } from "react";
import { FolderOpen, RefreshCw, Sparkles } from "lucide-react";
import { useProjectSkills } from "../hooks/useProjectSkills";
import { api } from "../lib/api";
import { ProjectConflictDialog } from "../components/projects/ProjectConflictDialog";
import type { BadgeTone } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { Menu, type MenuItem } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import type {
  ProjectConflict,
  ProjectConflictResolution,
  ProjectDirMeta,
  ProjectDirState,
  ProjectSkillView,
  ProjectSyncReport,
  ProjectView,
} from "../types";

const STATE_LABEL: Record<ProjectDirState, { label: string; tone: BadgeTone }> = {
  present: { label: "有", tone: "success" },
  absent: { label: "无", tone: "neutral" },
  different: { label: "不同", tone: "warning" },
  untracked: { label: "未跟踪", tone: "neutral" },
  unavailable: { label: "不可用", tone: "neutral" },
};

/** 未在真机验证的目录不给开关：状态列只显示「待实测」。 */
const UNVERIFIED = "待实测";

const FALLBACK_DIRS = [".claude/skills", ".qoder/skills", ".agents/skills"];

function stateOf(skill: ProjectSkillView, key: string): ProjectDirState {
  return skill.dirs[key]?.state ?? "absent";
}

function DirCell({
  meta,
  skill,
  busy,
  onToggle,
}: {
  meta: ProjectDirMeta;
  skill: ProjectSkillView;
  busy: boolean;
  onToggle: (targets: string[]) => void;
}) {
  const state = stateOf(skill, meta.key);
  const badge = STATE_LABEL[state];
  const on = skill.targets.includes(meta.key);

  if (!meta.verified) {
    return (
      <span
        role="img"
        aria-label={`${meta.label}：未实测，暂不提供开关`}
        title={`${meta.label}：未实测，暂不提供同步目标`}
        className="flex h-4 w-14 items-center justify-center rounded-[4px] border-[1.5px] border-dashed border-border-strong opacity-45"
      >
        <span className="text-badge leading-none text-fg-3">{UNVERIFIED}</span>
      </span>
    );
  }

  const disabled = state === "unavailable" || busy;
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={on}
      aria-label={`同步到 ${meta.label}`}
      title={`${meta.label}：${badge.label}${state === "unavailable" ? " · 对应 agent 已停用" : ""}`}
      disabled={disabled}
      onClick={() =>
        onToggle(
          on
            ? skill.targets.filter((target) => target !== meta.key)
            : [...skill.targets, meta.key],
        )
      }
      className={`flex h-[18px] w-9 items-center justify-center rounded-full border text-badge leading-none transition-colors duration-150 ${
        disabled ? "cursor-not-allowed opacity-45" : "cursor-pointer"
      } ${on ? "border-action bg-action-soft text-action-text" : "border-border text-fg-2"}`}
    >
      {badge.label}
    </button>
  );
}

export function ProjectSkillsPanel({ project }: { project: ProjectView }) {
  const { data, loading, error, refresh } = useProjectSkills(project.id);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [result, setResult] = useState<ProjectSyncReport | null>(null);
  const [conflicts, setConflicts] = useState<ProjectConflict[]>([]);
  /** 自动同步发现的冲突只挂在这里，用户点开才是对话框。 */
  const [pending, setPending] = useState<ProjectConflict[]>([]);
  const [autoSync, setAutoSync] = useState(project.autoSync);
  const autoSyncedFor = useRef<string | null>(null);

  const current = data?.project ?? project;
  const dirs = data?.dirs ?? [];
  const skills = data?.skills ?? [];
  const hasTargets = skills.some((skill) => skill.targets.length > 0);

  useEffect(() => setAutoSync(project.autoSync), [project.autoSync]);

  const guard = async (task: () => Promise<void>) => {
    setBusy(true);
    setActionError(null);
    try {
      await task();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const setTargets = (skill: ProjectSkillView, targets: string[]) =>
    guard(async () => {
      await api.setProjectSkillTargets(project.id, skill.id, targets);
      await refresh();
    });

  const sync = (resolutions: ProjectConflictResolution[] = [], automatic = false) =>
    guard(async () => {
      const report = await api.syncProjectSkills(project.id, resolutions);
      setResult(report);
      if (automatic) {
        setPending(report.conflicts);
      } else {
        setConflicts(report.conflicts);
        setPending([]);
      }
      await refresh();
    });

  // 自动同步：每个项目一次挂载只跑一次，且没有勾选目标时绝不写盘（首次添加不复制）。
  useEffect(() => {
    if (!data || !autoSync || !hasTargets) return;
    if (autoSyncedFor.current === project.id) return;
    autoSyncedFor.current = project.id;
    void sync([], true);
    // sync 依赖 refresh/data，放进依赖会反复触发；这里只按项目 + 是否可自动同步执行。
  }, [autoSync, data, hasTargets, project.id]);

  const toggleAutoSync = (enabled: boolean) =>
    guard(async () => {
      const updated = await api.setProjectAutoSync(project.id, enabled);
      setAutoSync(updated.autoSync);
      await refresh();
    });

  const removeCopy = (skill: ProjectSkillView, dir: string) =>
    guard(async () => {
      setResult(await api.removeProjectSkillCopy(project.id, skill.id, dir));
      await refresh();
    });

  const uninstall = (skill: ProjectSkillView) =>
    guard(async () => {
      setResult(await api.uninstallProjectSkill(project.id, skill.id));
      await refresh();
    });

  const menuItems = (skill: ProjectSkillView): MenuItem[] => [
    ...dirs.map((dir) => ({
      label: `移除 ${dir.label || dir.relativePath} 副本`,
      disabled: stateOf(skill, dir.key) === "absent",
      onSelect: () => void removeCopy(skill, dir.key),
    })),
    { label: "卸载", danger: true, onSelect: () => void uninstall(skill) },
  ];

  const copies = result?.outcomes.filter((item) => item.action === "copy").length ?? 0;
  const deletes = result?.outcomes.filter((item) => item.action === "delete").length ?? 0;
  const keeps = result?.outcomes.filter((item) => item.action === "keepBoth").length ?? 0;
  const failed = result?.outcomes.filter((item) => item.status !== "ok") ?? [];

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        icon={Sparkles}
        title="Skills"
        description={`${project.name} · ${project.path}`}
        actions={
          <>
            <label className="mr-1 flex items-center gap-1.5 text-caption text-fg-2">
              <input
                type="checkbox"
                className="h-4 w-4 accent-action"
                checked={autoSync}
                disabled={busy}
                onChange={(event) => void toggleAutoSync(event.target.checked)}
              />
              自动同步
            </label>
            <Button
              size="sm"
              onClick={() =>
                void api
                  .revealProjectFolder(project.id)
                  .catch((e) => console.warn("打开项目文件夹失败：", e))
              }
            >
              <FolderOpen size={14} aria-hidden />
              打开文件夹
            </Button>
            <Button size="sm" variant="primary" loading={busy} onClick={() => void sync()}>
              立即同步
            </Button>
            <Button
              size="sm"
              loading={loading || busy}
              // 自动同步开着时，刷新顺带把无歧义的改动推一遍。
              onClick={() =>
                void (autoSync && hasTargets ? sync([], true) : refresh())
              }
            >
              <RefreshCw size={14} aria-hidden />
              刷新
            </Button>
          </>
        }
      />

      <div className="min-h-0 flex-1 overflow-auto px-6 py-3">
        {!current.exists && (
          <Notice tone="danger" className="mb-3">
            项目路径不存在：{current.path}
          </Notice>
        )}
        {(error || actionError) && (
          <Notice tone="danger" className="mb-3">
            {actionError ?? `读取项目 Skills 失败：${error}`}
          </Notice>
        )}
        {pending.length > 0 && (
          <Notice tone="warning" className="mb-3">
            <div className="flex flex-wrap items-center gap-2">
              <span>自动同步发现 {pending.length} 个冲突待处理，未写盘。</span>
              <Button size="sm" onClick={() => setConflicts(pending)}>
                查看 {pending.length} 个冲突待处理
              </Button>
            </div>
          </Notice>
        )}
        {result && (copies > 0 || deletes > 0 || keeps > 0 || result.warnings.length > 0) && (
          <Notice tone={failed.length > 0 ? "warning" : "success"} className="mb-3">
            <div className="flex flex-wrap gap-x-3 gap-y-1">
              <span>复制 {copies}</span>
              <span>删除 {deletes}</span>
              <span>保留副本 {keeps}</span>
              {failed.length > 0 && <span>失败 {failed.length}</span>}
              {result.warnings.map((warning) => (
                <span key={warning} className="w-full text-fg-3">
                  {warning}
                </span>
              ))}
            </div>
          </Notice>
        )}

        {skills.length === 0 ? (
          loading ? (
            <p className="px-1 py-6 text-caption text-fg-3">正在读取项目 Skills…</p>
          ) : (
            <EmptyState
              icon={Sparkles}
              title="项目里还没有 Skills"
              description={`会从这些目录读取：${(dirs.length > 0 ? dirs.map((dir) => dir.relativePath) : FALLBACK_DIRS).join("、")}`}
            />
          )
        ) : (
          <div className="min-w-[560px] rounded-panel border border-border bg-surface">
            <div className="flex items-center gap-3 border-b border-border px-3 py-2 text-badge text-fg-3">
              <span className="min-w-0 flex-1">名称</span>
              {dirs.map((dir) => (
                <span key={dir.key} className="w-14 min-w-0 shrink-0 truncate text-center" title={`${dir.label} · ${dir.relativePath}`}>
                  {dir.label || dir.relativePath}
                </span>
              ))}
              <span className="w-8 shrink-0 text-center">操作</span>
            </div>
            <ul className="divide-y divide-border">
              {skills.map((skill) => (
                <li key={skill.id} className="flex items-center gap-3 px-3 py-2">
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-body text-fg-1">{skill.name}</span>
                    {skill.description && (
                      <span className="block truncate text-caption text-fg-3">
                        {skill.description}
                      </span>
                    )}
                  </span>
                  {dirs.map((dir) => (
                    <span key={dir.key} className="flex w-14 shrink-0 justify-center">
                      <DirCell
                        meta={dir}
                        skill={skill}
                        busy={busy}
                        onToggle={(targets) => void setTargets(skill, targets)}
                      />
                    </span>
                  ))}
                  <span className="flex w-8 shrink-0 justify-end">
                    <Menu label={`${skill.name} 更多操作`} items={menuItems(skill)} />
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>

      {conflicts.length > 0 && (
        <ProjectConflictDialog
          conflicts={conflicts}
          dirs={dirs}
          busy={busy}
          onCancel={() => setConflicts([])}
          onResolve={(resolutions) => void sync(resolutions)}
        />
      )}
    </div>
  );
}
