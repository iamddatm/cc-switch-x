import { useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderInput, Plus, RefreshCw, Sparkles } from "lucide-react";
import { AssetRow } from "../components/assets/AssetRow";
import { cn } from "../lib/cn";
import { ProjectConflictDialog } from "../components/projects/ProjectConflictDialog";
import { ProjectMigrationDialog } from "../components/projects/ProjectMigrationDialog";
import { SkillBatchBar } from "../components/skills/SkillBatchBar";
import { SkillDetailSheet } from "../components/skills/SkillDetailSheet";
import { SkillDiscoveryPanel } from "../components/skills/SkillDiscoveryPanel";
import { SkillFormSheet } from "../components/skills/SkillFormSheet";
import { SkillInstallSheet } from "../components/skills/SkillInstallSheet";
import { SkillSourceBadge } from "../components/skills/SkillSourceBadge";
import {
  NO_REPO_KEY,
  filterByCommonFilters,
  repoOptions,
  selectedRepoKeys as selectedRepos,
  toggled,
} from "../components/skills/skillFilters";
import { projectRow, projectScope } from "../components/skills/skillScope";
import type { SkillDeployTarget } from "../components/skills/skillScope";
import { Badge, type BadgeTone } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Checkbox } from "../components/ui/Checkbox";
import { CheckOption, CheckPopover } from "../components/ui/CheckPopover";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { EmptyState } from "../components/ui/EmptyState";
import { Menu, type MenuItem } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { Select } from "../components/ui/Select";
import { useProjectSkills } from "../hooks/useProjectSkills";
import { api } from "../lib/api";
import { parseSkill, type SkillForm } from "../lib/canonical";
import { formatRelativeTime } from "../lib/format";
import type {
  AssetSummary,
  MigrationDecision,
  ProjectConflict,
  ProjectConflictResolution,
  ProjectDirMeta,
  ProjectDirState,
  ProjectSkillView,
  ProjectSyncReport,
  ProjectView,
  SkillCheckReport,
} from "../types";

/**
 * 格子状态六态：`stale` 是「真源更新了」，同步会安全覆盖；`edited` 是「这份副本被外部改过」，
 * 必须人工裁决，同步永不替用户决定谁更新。
 */
const STATE_LABEL: Record<ProjectDirState, { label: string; tone: BadgeTone }> = {
  present: { label: "已同步", tone: "success" },
  absent: { label: "无", tone: "neutral" },
  stale: { label: "待更新", tone: "brand" },
  edited: { label: "已改", tone: "warning" },
  untracked: { label: "未跟踪", tone: "neutral" },
  unavailable: { label: "不可用", tone: "neutral" },
};

/** 悬停提示里把状态说成一句人话，光看两三个字容易误判。 */
const STATE_HINT: Record<ProjectDirState, string> = {
  present: "与项目真源一致",
  absent: "还没有渲染到这里的副本",
  stale: "项目真源已更新，同步会把它刷成最新",
  edited: "这份副本被外部改过，需要你裁决，同步不会覆盖它",
  untracked: "不是本项目渲染的副本，工具不会改动它",
  unavailable: "对应 agent 已停用，或该落点尚未实测",
};

/** 未在真机验证的目录不给开关：状态列只显示「待实测」。 */
const UNVERIFIED = "待实测";

const FALLBACK_DIRS = [".claude/skills", ".qoder/skills", ".agents/skills"];

/** 项目里没有 cc-switch 导入这条来源，选项照用全局但要少一项。 */
const SOURCE_FILTERS = [
  { value: "all", label: "全部来源" },
  { value: "github", label: "GitHub 安装" },
  { value: "zip", label: "ZIP 安装" },
  { value: "local", label: "本地目录" },
] as const;

/** 部署状态筛选：与格子的六态一一对应，「全部状态」不参与判断。 */
const STATE_FILTERS: { value: "all" | ProjectDirState; label: string }[] = [
  { value: "all", label: "全部状态" },
  { value: "present", label: "已同步" },
  { value: "stale", label: "待更新" },
  { value: "edited", label: "已改（待裁决）" },
  { value: "absent", label: "无副本" },
  { value: "untracked", label: "未跟踪" },
  { value: "unavailable", label: "不可用" },
];

const EMPTY_FORM: SkillForm = { name: "", description: "", body: "", extraFrontmatter: "" };

/**
 * 一行 = 一个项目 skill。行数据折算成 `AssetSummary` 形状（`projectRow`），
 * 这样全局的 `AssetRow`、来源徽标与筛选谓词都能原样复用，项目现状挂在 `skill` 上。
 */
type ProjectRow = AssetSummary & { skill: ProjectSkillView };

function stateOf(skill: ProjectSkillView, key: string): ProjectDirState {
  return skill.dirs[key]?.state ?? "absent";
}

function dirLabel(dir: ProjectDirMeta): string {
  return dir.label || dir.relativePath;
}

/** 某个目录在该行的渲染产物。 */
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
        aria-label={`${dirLabel(meta)}：未实测，暂不提供开关`}
        title={`${dirLabel(meta)}：未实测，暂不提供同步目标`}
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
      aria-label={`同步到 ${dirLabel(meta)}`}
      title={`${dirLabel(meta)}：${badge.label} · ${STATE_HINT[state]}`}
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
  const [batchBusy, setBatchBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [result, setResult] = useState<ProjectSyncReport | null>(null);
  const [conflicts, setConflicts] = useState<ProjectConflict[]>([]);
  /** 自动同步发现的冲突只挂在这里，用户点开才是对话框。 */
  const [pending, setPending] = useState<ProjectConflict[]>([]);
  const [migrating, setMigrating] = useState(false);
  const [migrationNote, setMigrationNote] = useState<string | null>(null);
  const [autoSync, setAutoSync] = useState(project.autoSync);
  const autoSyncedFor = useRef<string | null>(null);

  const [view, setView] = useState<"installed" | "discover">("installed");
  const [discoverMounted, setDiscoverMounted] = useState(false);
  const [query, setQuery] = useState("");
  const [dirFilter, setDirFilter] = useState("all");
  const [stateFilter, setStateFilter] = useState<"all" | ProjectDirState>("all");
  const [sourceFilter, setSourceFilter] = useState("all");
  const [deselectedRepos, setDeselectedRepos] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [viewing, setViewing] = useState<{ row: ProjectRow; form: SkillForm } | null>(null);
  const [editing, setEditing] = useState<{ mode: "create" | "edit"; id: string; form: SkillForm } | null>(
    null,
  );
  const [loadingEdit, setLoadingEdit] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [checking, setChecking] = useState(false);
  const [checkReport, setCheckReport] = useState<SkillCheckReport | null>(null);
  const [updateTargets, setUpdateTargets] = useState<string[] | null>(null);
  const [updating, setUpdating] = useState(false);
  const [pendingDelete, setPendingDelete] = useState<
    | { kind: "remove"; skill: ProjectSkillView; dir: string }
    | { kind: "uninstall"; rows: ProjectRow[] }
    | { kind: "purge"; skill: ProjectSkillView }
    | null
  >(null);

  const current = data?.project ?? project;
  const dirs = data?.dirs ?? [];
  const skills = data?.skills ?? [];
  const hasTargets = skills.some((skill) => skill.targets.length > 0);
  /** 一个可同步的目录都没有（落点未实测）时，同步按钮只能说明原因，不能让人点了吃一个错。 */
  const selectable = dirs.some((dir) => dir.verified);

  /**
   * 安装、编辑、检查更新的去向都在这里定义：组件只认这个适配器，
   * 于是中央库面板的那套抽屉与市场在项目里也是同一份代码。
   */
  const scope = useMemo(
    () =>
      projectScope(
        project.id,
        skills.map((skill) => skill.id),
        dirs
          .filter((dir) => dir.verified)
          .map((dir) => ({ key: dir.key, label: dirLabel(dir) })),
      ),
    [project.id, skills, dirs],
  );

  const rows = useMemo<ProjectRow[]>(
    () => skills.map((skill) => ({ ...projectRow(skill), skill })),
    [skills],
  );

  useEffect(() => setAutoSync(project.autoSync), [project.autoSync]);

  const repoChoices = useMemo(() => repoOptions(rows), [rows]);
  const activeRepoKeys = useMemo(
    () => selectedRepos(repoChoices.allKeys, deselectedRepos),
    [repoChoices, deselectedRepos],
  );
  const allReposSelected =
    repoChoices.allKeys.length > 0 && activeRepoKeys.size === repoChoices.allKeys.length;

  // 与全局一致：目录筛选只收窄「状态」这一轴的判断范围，本身不删行；状态筛选才删行。
  const visible = useMemo(
    () =>
      filterByCommonFilters(rows, {
        query,
        sourceFilter,
        repoKeys: activeRepoKeys,
      }).filter((row) => {
        if (stateFilter === "all") return true;
        const considered =
          dirFilter === "all" ? Object.keys(row.skill.dirs) : [dirFilter];
        return considered.some((key) => stateOf(row.skill, key) === stateFilter);
      }),
    [rows, query, sourceFilter, activeRepoKeys, stateFilter, dirFilter],
  );

  const updateIds = checkReport?.updates.map((update) => update.id) ?? [];
  const updatesCount = updateIds.length;
  const selectedRows = rows.filter((row) => selected.has(row.id));
  const selectedIds = selectedRows.map((row) => row.id);
  const allVisibleSelected =
    visible.length > 0 && visible.every((row) => selected.has(row.id));
  const updatableSelected = selectedRows.filter((row) => row.origin?.updatable).length;
  /** 只有已实测的目录能作为批量落点，未实测的列本来就不可点。 */
  const deployTargets: SkillDeployTarget[] = useMemo(
    () =>
      dirs
        .filter((dir) => dir.verified)
        .map((dir) => ({ key: dir.key, label: dirLabel(dir) })),
    [dirs],
  );
  const enabledTargets = useMemo(() => {
    const keys = new Set<string>();
    for (const row of selectedRows) {
      for (const target of row.skill.targets) keys.add(target);
    }
    return deployTargets.filter((target) => keys.has(target.key));
  }, [deployTargets, selectedRows]);

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

  /** 迁移由用户确认后才落盘；成功后刷新，计划消失即代表台账已是 v2。 */
  const migrate = (decisions: MigrationDecision[]) =>
    guard(async () => {
      const report = await api.migrateProjectSkills(project.id, decisions);
      setMigrationNote(
        `迁移完成：收进真源 ${report.adopted.length} 个，跳过 ${report.skipped.length} 个，清理条目 ${report.dropped.length} 个。`,
      );
      setMigrating(false);
      await refresh();
    });

  // 自动同步：每个项目一次挂载只跑一次，且没有勾选目标时绝不写盘（首次添加不复制）。
  useEffect(() => {
    if (!data || !autoSync || !hasTargets || !selectable) return;
    if (autoSyncedFor.current === project.id) return;
    autoSyncedFor.current = project.id;
    void sync([], true);
    // sync 依赖 refresh/data，放进依赖会反复触发；这里只按项目 + 是否可自动同步执行。
  }, [autoSync, data, hasTargets, project.id, selectable]);

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

  /** 卸载可以批量：后端一次删一个（真源 + 全部渲染产物，各自先备份），这里把结果合成一份。 */
  const uninstall = (rowsToUninstall: ProjectRow[]) =>
    guard(async () => {
      const outcomes: ProjectSyncReport["outcomes"] = [];
      const conflictList: ProjectConflict[] = [];
      const warnings: string[] = [];
      for (const row of rowsToUninstall) {
        const report = await api.uninstallProjectSkill(project.id, row.id);
        outcomes.push(...report.outcomes);
        conflictList.push(...report.conflicts);
        warnings.push(...report.warnings);
      }
      setResult({ outcomes, conflicts: conflictList, warnings });
      setConflicts(conflictList);
      setSelected(new Set());
      await refresh();
    });

  /** 收进真源：把某个目录里现有的 skill 目录变成项目真源，来源目录不受影响。 */
  const adopt = (skillId: string, dir: string) =>
    guard(async () => {
      setResult(await api.adoptProjectSkill(project.id, skillId, dir));
      await refresh();
    });

  /** 真源没了：只有用户显式要求，才允许删掉全部副本（后端会先备份）。 */
  const purge = (skillId: string) =>
    guard(async () => {
      setResult(await api.syncProjectSkills(project.id, [{ skillId, dir: "", action: "purge" }]));
      await refresh();
    });

  /**
   * 批量勾选/取消勾选某个渲染目录。目标列表是整条替换，所以要先并上这条已有的目标，
   * 全部改完再同步一次：一次同步 = 一个批次 = 一份备份时间戳。
   */
  const setEnabledFor = async (dirKey: string, enabled: boolean) => {
    if (selectedIds.length === 0) return;
    setBatchBusy(true);
    setActionError(null);
    setActionMessage(null);
    try {
      for (const row of selectedRows) {
        const next = enabled
          ? Array.from(new Set([...row.skill.targets, dirKey]))
          : row.skill.targets.filter((target) => target !== dirKey);
        await api.setProjectSkillTargets(project.id, row.id, next);
      }
      const report = await api.syncProjectSkills(project.id);
      setResult(report);
      setConflicts(report.conflicts);
      setActionMessage(
        `${enabled ? "渲染" : "取消渲染"}完成：${selectedIds.length} 个 Skill 已更新目标。`,
      );
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBatchBusy(false);
    }
  };

  const toggleSelected = (id: string, checked: boolean) => {
    setSelected((previous) => toggled(previous, id, checked));
  };

  const toggleAllVisible = () => {
    setSelected((previous) => {
      const next = new Set(previous);
      if (allVisibleSelected) {
        visible.forEach((row) => next.delete(row.id));
      } else {
        visible.forEach((row) => next.add(row.id));
      }
      return next;
    });
  };

  const toggleRepo = (key: string, checked: boolean) => {
    // 存的是「已排除」集合，勾选对应移出。
    setDeselectedRepos((previous) => toggled(previous, key, !checked));
  };

  const toggleAllRepos = (checked: boolean) => {
    setDeselectedRepos(checked ? new Set() : new Set(repoChoices.allKeys));
  };

  // 查看与编辑读的是项目真源；失败时统一报错并返回 null。
  const readBody = async (row: ProjectRow): Promise<string | null> => {
    setLoadingEdit(true);
    setActionError(null);
    try {
      return await scope.readBody(row.id);
    } catch (e) {
      setActionError(String(e));
      return null;
    } finally {
      setLoadingEdit(false);
    }
  };

  const openView = async (row: ProjectRow) => {
    const text = await readBody(row);
    if (text === null) return;
    setEditing(null);
    setViewing({ row, form: parseSkill(text) });
  };

  const openEdit = async (row: ProjectRow) => {
    const text = await readBody(row);
    if (text === null) return;
    setViewing(null);
    setEditing({ mode: "edit", id: row.id, form: parseSkill(text) });
  };

  const editViewed = () => {
    if (!viewing) return;
    setEditing({ mode: "edit", id: viewing.row.id, form: viewing.form });
    setViewing(null);
  };

  const importDirectory = async () => {
    setActionError(null);
    setActionMessage(null);
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "选择包含 SKILL.md 的目录",
      });
      if (typeof picked !== "string") return;
      const outcome = await scope.importDir(picked);
      setActionMessage(
        outcome.status === "installed"
          ? `已把 ${outcome.id} 装进项目真源；勾选渲染目录后同步即可铺开。`
          : `导入结果：${outcome.status === "failed" ? (outcome.error ?? outcome.status) : outcome.status}`,
      );
      await refresh();
    } catch (e) {
      setActionError(String(e));
    }
  };

  const checkUpdates = async (ids?: string[]) => {
    setChecking(true);
    setActionError(null);
    setActionMessage(null);
    try {
      setCheckReport(await scope.checkUpdates(ids));
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setChecking(false);
    }
  };

  const runUpdate = async () => {
    if (!updateTargets) return;
    setUpdating(true);
    setActionError(null);
    setActionMessage(null);
    try {
      const report = await scope.updateSkills(updateTargets);
      const failed = report.outcomes.filter((outcome) => outcome.status === "failed");
      const succeeded = report.outcomes.length - failed.length;
      setActionMessage(
        `更新完成：成功 ${succeeded} 个${failed.length ? `，失败 ${failed.length} 个` : ""}。旧版本已备份到备份目录；只换了项目真源，要让渲染产物跟上请点「立即同步」。`,
      );
      if (failed.length > 0) {
        setActionError(
          failed
            .map((outcome) => `${outcome.name}：${outcome.error ?? "更新失败"}`)
            .join("；"),
        );
      }
      setCheckReport(null);
      setSelected(new Set());
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setUpdating(false);
      setUpdateTargets(null);
    }
  };

  const rowMenuItems = (row: ProjectRow): MenuItem[] => {
    const skill = row.skill;
    const menu: MenuItem[] = [
      { label: "查看", onSelect: () => void openView(row) },
      { label: "编辑", onSelect: () => void openEdit(row) },
    ];
    if (row.origin?.updatable) {
      menu.push(
        { label: "检查更新", onSelect: () => void checkUpdates([row.id]) },
        { label: "更新", onSelect: () => setUpdateTargets([row.id]) },
      );
    }
    // 还没进真源的副本：先收进真源，之后才谈渲染。
    for (const dir of dirs) {
      if (!skill.canonical && stateOf(skill, dir.key) !== "absent") {
        menu.push({
          label: `把 ${dirLabel(dir)} 的副本收进真源`,
          onSelect: () => void adopt(skill.id, dir.key),
        });
      }
    }
    if (skill.canonicalMissing) {
      menu.push({
        label: "真源已丢失：删除全部副本",
        danger: true,
        onSelect: () => setPendingDelete({ kind: "purge", skill }),
      });
    }
    for (const dir of dirs) {
      menu.push({
        label: `移除 ${dirLabel(dir)} 副本`,
        disabled: stateOf(skill, dir.key) === "absent",
        onSelect: () => setPendingDelete({ kind: "remove", skill, dir: dir.key }),
      });
    }
    menu.push({
      label: "卸载",
      danger: true,
      onSelect: () => setPendingDelete({ kind: "uninstall", rows: [row] }),
    });
    return menu;
  };

  const checkFailures = checkReport
    ? [
        ...checkReport.repoFailures.map((failure) => `${failure.repo}：${failure.error}`),
        ...checkReport.skillFailures.map((failure) => `${failure.id}：${failure.error}`),
      ]
    : [];
  const updateEntries = (updateTargets ?? []).map((id) => {
    const info = checkReport?.updates.find((update) => update.id === id);
    const row = rows.find((entry) => entry.id === id);
    return { id, name: info?.name ?? row?.name ?? id, repo: row?.origin?.repo ?? null };
  });

  const renders = result?.outcomes.filter((item) => item.action === "render").length ?? 0;
  const adopts = result?.outcomes.filter((item) => item.action === "adopt").length ?? 0;
  const deletes = result?.outcomes.filter((item) => item.action === "delete").length ?? 0;
  const keeps = result?.outcomes.filter((item) => item.action === "keepBoth").length ?? 0;
  const failed = result?.outcomes.filter((item) => item.status !== "ok") ?? [];
  const dirOf = (key: string) => dirs.find((dir) => dir.key === key);

  return (
    <div className="relative flex h-full min-h-0 flex-col">
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
            {updatesCount > 0 && !checking && (
              <Button
                variant="primary"
                onClick={() => setUpdateTargets(updateIds)}
                disabled={updating}
              >
                全部更新（{updatesCount} 个）
              </Button>
            )}
            {/* 与用户级面板同纪律：检查更新常驻，不被「全部更新」顶掉 */}
            <Button
              onClick={() => void checkUpdates()}
              loading={checking}
              disabled={rows.length === 0 || updating}
            >
              <RefreshCw size={14} aria-hidden />
              检查更新
            </Button>
            <Menu
              label="添加 Skill"
              trigger={
                <>
                  <Plus size={14} aria-hidden />
                  添加 Skill
                </>
              }
              buttonClassName="h-8 w-auto whitespace-nowrap bg-action px-3 text-body font-medium text-action-fg hover:bg-action-hover hover:text-action-fg"
              items={[
                { label: "新建 Skill", onSelect: () => setEditing({ mode: "create", id: "", form: EMPTY_FORM }) },
                { label: "从文件夹导入", onSelect: () => void importDirectory() },
                { label: "从来源安装…", onSelect: () => setInstalling(true) },
              ]}
            />
            <Button
              variant="primary"
              loading={busy && !batchBusy}
              disabled={!selectable}
              title={selectable ? undefined : "项目级落点尚未在真机验证"}
              onClick={() => void sync()}
            >
              立即同步
            </Button>
            <Menu
              label="项目 Skills 的更多操作"
              items={[
                {
                  label: "打开文件夹",
                  onSelect: () =>
                    void api
                      .revealProjectFolder(project.id)
                      .catch((e) => console.warn("打开项目文件夹失败：", e)),
                },
                { label: "刷新列表", onSelect: () => void refresh() },
              ]}
            />
          </>
        }
      />

      <div
        role="tablist"
        aria-label="Skills 视图"
        className="flex h-10 shrink-0 items-center gap-1 border-b border-border px-6"
      >
        <button
          type="button"
          role="tab"
          aria-selected={view === "installed"}
          onClick={() => setView("installed")}
          className={`relative flex h-full items-center gap-1.5 px-2 text-body transition-colors ${
            view === "installed"
              ? "font-medium text-fg-1 after:absolute after:inset-x-1 after:bottom-0 after:h-0.5 after:bg-action"
              : "text-fg-2 hover:text-fg-1"
          }`}
        >
          已安装
          <span className="text-caption text-fg-3">{rows.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={view === "discover"}
          onClick={() => {
            setView("discover");
            setDiscoverMounted(true);
          }}
          className={`relative flex h-full items-center px-2 text-body transition-colors ${
            view === "discover"
              ? "font-medium text-fg-1 after:absolute after:inset-x-1 after:bottom-0 after:h-0.5 after:bg-action"
              : "text-fg-2 hover:text-fg-1"
          }`}
        >
          发现
        </button>
      </div>

      <div className={view === "installed" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
        <div className="flex h-12 shrink-0 items-center gap-2 px-6">
          <SearchField
            className="w-[240px]"
            value={query}
            onValueChange={setQuery}
            placeholder="搜索名称、描述、来源或仓库…"
            ariaLabel="搜索 Skill"
          />
          {repoChoices.allKeys.length > 0 && (
            <CheckPopover
              label={
                allReposSelected
                  ? `仓库：全部（${repoChoices.allKeys.length}）`
                  : `仓库：${activeRepoKeys.size}/${repoChoices.allKeys.length}`
              }
            >
              <CheckOption label="全部仓库" checked={allReposSelected} onChange={toggleAllRepos} />
              {repoChoices.repos.map((repo) => (
                <CheckOption
                  key={repo.key}
                  mono
                  label={repo.label}
                  checked={activeRepoKeys.has(repo.key)}
                  onChange={(checked) => toggleRepo(repo.key, checked)}
                />
              ))}
              {repoChoices.hasNoRepo && (
                <CheckOption
                  label="无来源仓库"
                  checked={activeRepoKeys.has(NO_REPO_KEY)}
                  onChange={(checked) => toggleRepo(NO_REPO_KEY, checked)}
                />
              )}
            </CheckPopover>
          )}
          <Select
            aria-label="按渲染目录筛选"
            value={dirFilter}
            onChange={(event) => setDirFilter(event.target.value)}
          >
            <option value="all">全部目录</option>
            {dirs.map((dir) => (
              <option key={dir.key} value={dir.key}>
                {dirLabel(dir)}
              </option>
            ))}
          </Select>
          <Select
            aria-label="按部署状态筛选"
            value={stateFilter}
            onChange={(event) => setStateFilter(event.target.value as "all" | ProjectDirState)}
          >
            {STATE_FILTERS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </Select>
          <Select
            aria-label="按来源筛选"
            value={sourceFilter}
            onChange={(event) => setSourceFilter(event.target.value)}
          >
            {SOURCE_FILTERS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </Select>
          <span className="ml-auto text-caption text-fg-3">
            {visible.length} / {rows.length} 个 Skill
          </span>
        </div>

        <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
          {(error || actionError) && (
            <Notice tone="danger" className="mb-3">
              {actionError ?? `读取项目 Skills 失败：${error}`}
            </Notice>
          )}
          {actionMessage && (
            <Notice tone="success" className="mb-3">
              {actionMessage}
            </Notice>
          )}
          {checking && (
            <Notice tone="info" className="mb-3">
              正在读取上游仓库并比对项目真源…
            </Notice>
          )}
          {checkReport && (
            <div className="mb-3 space-y-2">
              {checkReport.updates.length > 0 ? (
                <Notice tone="warning">
                  发现 {checkReport.updates.length} 个可更新
                  {checkReport.upToDate.length > 0
                    ? `，${checkReport.upToDate.length} 个已是最新`
                    : ""}
                  。
                </Notice>
              ) : checkFailures.length === 0 ? (
                <Notice tone="success">
                  已检查 {checkReport.upToDate.length} 个，全部是最新。
                </Notice>
              ) : (
                <Notice tone="warning">本次没有发现可更新项，但有检查失败。</Notice>
              )}
              {checkReport.skipped.length > 0 && (
                <Notice tone="info">
                  {checkReport.skipped.length} 个项目 skill 没有可检查的 GitHub 上游，已跳过。
                </Notice>
              )}
              {checkFailures.length > 0 && (
                <Notice tone="danger">{checkFailures.join("；")}</Notice>
              )}
            </div>
          )}
          {data?.migration && (
            <Notice tone="warning" className="mb-3">
              <div className="flex flex-wrap items-center gap-2">
                <span>
                  这个项目的台账还是旧格式（各目录互为副本）。迁移后
                  <code className="mx-1 font-mono">.ccswitchx/skills/</code>
                  成为项目真源，agent 目录变成它的渲染产物；迁移只新建文件，不改动任何 agent 目录。
                </span>
                <Button size="sm" onClick={() => setMigrating(true)}>
                  查看迁移方案
                </Button>
              </div>
            </Notice>
          )}
          {migrationNote && (
            <Notice tone="success" className="mb-3">
              {migrationNote}
            </Notice>
          )}
          {!selectable && dirs.length > 0 && (
            <Notice tone="warning" className="mb-3">
              这些项目级落点（{dirs.map((dir) => dir.relativePath).join("、")}）尚未在真机验证，
              因此不能勾选为同步目标，也不能同步。请在临时项目里实测后再回来。
            </Notice>
          )}
          {!current.exists && (
            <Notice tone="danger" className="mb-3">
              项目路径不存在：{current.path}
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
          {result &&
            (renders > 0 || adopts > 0 || deletes > 0 || keeps > 0 || result.warnings.length > 0) && (
              <Notice tone={failed.length > 0 ? "warning" : "success"} className="mb-3">
                <div className="flex flex-wrap gap-x-3 gap-y-1">
                  <span>渲染 {renders}</span>
                  {adopts > 0 && <span>收进真源 {adopts}</span>}
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
          {selectedRows.length > 0 && (
            <div className="mb-3">
              <SkillBatchBar
                count={selectedRows.length}
                targets={deployTargets}
                enabledTargets={enabledTargets}
                updatableCount={updatableSelected}
                busy={batchBusy || busy || checking || updating}
                wording={scope.wording}
                noneEnabledLabel="没有渲染到任何目录"
                onCheck={() => void checkUpdates(selectedIds)}
                onUpdate={() =>
                  setUpdateTargets(
                    selectedRows.filter((row) => row.origin?.updatable).map((row) => row.id),
                  )
                }
                onEnable={(dirKey) => void setEnabledFor(dirKey, true)}
                onDisable={(dirKey) => void setEnabledFor(dirKey, false)}
                onDelete={() => setPendingDelete({ kind: "uninstall", rows: selectedRows })}
                onClear={() => setSelected(new Set())}
              />
            </div>
          )}
          {loadingEdit && <p className="mb-2 text-caption text-fg-3">正在读取 SKILL.md…</p>}

          {loading && rows.length === 0 ? (
            <p className="py-10 text-center text-caption text-fg-3">正在读取项目 Skills…</p>
          ) : rows.length === 0 ? (
            <EmptyState
              icon={Sparkles}
              title="项目里还没有 Skills"
              description={`真源在 <项目>/.ccswitchx/skills/，渲染产物会从这些目录读写：${(dirs.length > 0
                ? dirs.map((dir) => dir.relativePath)
                : FALLBACK_DIRS
              ).join("、")}。`}
              actions={
                <>
                  <Button
                    variant="primary"
                    onClick={() => setEditing({ mode: "create", id: "", form: EMPTY_FORM })}
                  >
                    新建 Skill
                  </Button>
                  <Button onClick={() => void importDirectory()}>
                    <FolderInput size={14} aria-hidden />
                    从文件夹导入
                  </Button>
                  <Button onClick={() => setInstalling(true)}>从来源安装</Button>
                </>
              }
            />
          ) : visible.length === 0 ? (
            <EmptyState
              icon={Sparkles}
              title="没有匹配的 Skill"
              description="换个关键词或筛选条件。"
            />
          ) : (
            <div className="overflow-hidden rounded-panel border border-border bg-surface">
              <div className="flex h-11 items-center bg-subtle ps-4">
                <span className="flex w-6 shrink-0 items-center">
                  <Checkbox
                    aria-label="全选当前列表"
                    checked={allVisibleSelected}
                    onChange={toggleAllVisible}
                  />
                </span>
                <span className="flex-1 text-caption font-semibold text-fg-2">名称</span>
                {dirs.map((dir) => {
                  const count = visible.filter((row) => row.skill.targets.includes(dir.key)).length;
                  return (
                    <span
                      key={dir.key}
                      className={cn(
                        "flex w-14 shrink-0 flex-col items-center justify-center gap-0.5",
                        !dir.verified && "opacity-45",
                      )}
                      title={`${dirLabel(dir)} · ${dir.relativePath}：当前列表已渲染 ${count} 个${
                        dir.verified ? "" : "（未实测，不能作为目标）"
                      }`}
                    >
                      <span className="w-full truncate text-center text-badge text-fg-2">
                        {dirLabel(dir)}
                      </span>
                      <span className="text-badge tabular-nums text-fg-3">{count}</span>
                    </span>
                  );
                })}
                <span className="w-12" />
              </div>
              <ul>
                {visible.map((row) => {
                  const detail = [
                    row.description,
                    row.origin?.lastCheckedAt
                      ? `检查于 ${formatRelativeTime(row.origin.lastCheckedAt)}`
                      : null,
                    row.skill.canonical
                      ? null
                      : "还没有项目真源：用行菜单「收进真源」把它纳管",
                  ]
                    .filter(Boolean)
                    .join(" · ");
                  return (
                    <AssetRow
                      key={row.id}
                      title={row.name}
                      subtitle={row.id}
                      description={detail || undefined}
                      leading={
                        <Checkbox
                          aria-label={`选择 ${row.name}`}
                          checked={selected.has(row.id)}
                          onChange={(event) => toggleSelected(row.id, event.target.checked)}
                        />
                      }
                      selected={selected.has(row.id)}
                      titleBadges={
                        <>
                          {row.origin?.updateAvailable ? (
                            <Badge tone="warning">可更新</Badge>
                          ) : null}
                          {row.skill.canonicalMissing ? (
                            <Badge tone="danger">真源已丢失</Badge>
                          ) : null}
                        </>
                      }
                      badges={<SkillSourceBadge item={row} />}
                      agentCells={
                        <>
                          {dirs.map((dir) => (
                            <span key={dir.key} className="flex w-14 shrink-0 justify-center">
                              <DirCell
                                meta={dir}
                                skill={row.skill}
                                busy={busy || batchBusy}
                                onToggle={(targets) => void setTargets(row.skill, targets)}
                              />
                            </span>
                          ))}
                        </>
                      }
                      actions={<Menu label={`${row.name} 更多操作`} items={rowMenuItems(row)} />}
                      footer={
                        row.origin?.lastCheckError
                          ? `上次检查失败：${row.origin.lastCheckError}`
                          : undefined
                      }
                      footerTone="warning"
                      openLabel={`查看 ${row.name}`}
                      onOpen={() => void openView(row)}
                    />
                  );
                })}
              </ul>
            </div>
          )}
        </div>
      </div>

      {discoverMounted && (
        <div className={view === "discover" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
          <SkillDiscoveryPanel
            installedItems={rows}
            scope={scope}
            targets={{
              options: deployTargets,
              label: (chosen) =>
                deployTargets.length === 0
                  ? "未启用落点"
                  : chosen.length === deployTargets.length
                    ? `全部目录（${deployTargets.length}）`
                    : chosen.length === 0
                      ? "未选择目录"
                      : chosen.map((target) => target.label).join("、"),
              emptyHint: "没有已实测的项目落点，装完只落在项目真源里。",
            }}
            onInstalled={() => void refresh()}
            onShowInstalled={() => setView("installed")}
          />
        </div>
      )}

      {viewing && (
        <SkillDetailSheet
          item={viewing.row}
          form={viewing.form}
          onEdit={editViewed}
          onClose={() => setViewing(null)}
          extra={
            <p className="text-caption text-fg-3">
              项目真源：<code className="font-mono">.ccswitchx/skills/{viewing.row.id}</code>
              {" · 已渲染到："}
              {viewing.row.skill.targets.length > 0
                ? viewing.row.skill.targets
                    .map((key) => dirOf(key)?.relativePath ?? key)
                    .join("、")
                : "（还没有渲染目录）"}
            </p>
          }
        />
      )}

      {editing && (
        <SkillFormSheet
          mode={editing.mode}
          id={editing.id}
          initial={editing.form}
          scope={scope}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void refresh();
          }}
        />
      )}

      {installing && (
        <SkillInstallSheet
          scope={scope}
          onClose={() => setInstalling(false)}
          onInstalled={() => void refresh()}
        />
      )}

      <ConfirmDialog
        open={updateTargets !== null}
        title={`更新 ${updateTargets?.length ?? 0} 个项目 Skill？`}
        pending={updating}
        confirmLabel={`更新 ${updateTargets?.length ?? 0} 个`}
        body={
          <div className="space-y-2">
            <p>
              更新会用仓库里的内容整目录替换项目真源
              <code className="mx-1 font-mono">.ccswitchx/skills/</code>
              ，本地改过的地方只保留在备份里。渲染产物不会被动，要点「立即同步」才会铺开。
            </p>
            {updateEntries.length > 0 && (
              <ul
                aria-label="将要更新的 Skill"
                className="max-h-60 divide-y divide-border overflow-y-auto rounded-control border border-border"
              >
                {updateEntries.map((entry) => (
                  <li key={entry.id} className="flex items-baseline gap-2 px-3 py-2 text-body">
                    <span className="min-w-0 truncate font-medium text-fg-1">{entry.name}</span>
                    {entry.repo && (
                      <span className="ms-auto shrink-0 font-mono text-caption text-fg-3">
                        {entry.repo}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        }
        onConfirm={() => void runUpdate()}
        onCancel={() => setUpdateTargets(null)}
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        title={
          pendingDelete?.kind === "uninstall"
            ? pendingDelete.rows.length > 1
              ? `卸载 ${pendingDelete.rows.length} 个 Skill？`
              : `卸载 ${pendingDelete.rows[0]?.name ?? ""}？`
            : pendingDelete?.kind === "purge"
              ? `删除 ${pendingDelete.skill.name} 的全部副本`
              : `移除 ${dirOf(pendingDelete?.dir ?? "")?.label ?? pendingDelete?.dir} 的副本`
        }
        danger
        pending={busy}
        confirmLabel={
          pendingDelete?.kind === "uninstall"
            ? "确认卸载"
            : pendingDelete?.kind === "purge"
              ? "确认删除全部副本"
              : "确认移除"
        }
        body={
          pendingDelete?.kind === "uninstall" ? (
            <div className="space-y-1">
              <p>
                删除项目真源{" "}
                <code className="font-mono">
                  .ccswitchx/skills/[{pendingDelete.rows.map((row) => row.id).join("、")}]
                </code>{" "}
                与所有已跟踪副本，并移除 .ccswitchx/skills.json 里的条目。
              </p>
              <p>
                会删到的目录：
                <code className="font-mono">
                  {[...new Set(pendingDelete.rows.flatMap((row) => row.skill.targets))]
                    .map((key) => dirOf(key)?.relativePath ?? key)
                    .join("、") || "（只有真源，没有已勾选的渲染目录）"}
                </code>
              </p>
              <p className="text-caption text-fg-3">
                磁盘上与基线不一致的改动会先备份到备份目录，但不会保留在原位置；未跟踪的副本一个都不碰。
              </p>
            </div>
          ) : pendingDelete?.kind === "purge" ? (
            <div className="space-y-1">
              <p>
                账本记着这个项目有真源，但 <code className="font-mono">.ccswitchx/skills/</code>{" "}
                下已经找不到它了。删除所有还留着的副本并移除条目；副本之外不会碰任何文件。
              </p>
              <p className="text-caption text-fg-3">
                删除前会先备份到备份目录。想留下这些副本的话，改成在文件管理器里把内容放回真源。
              </p>
            </div>
          ) : (
            <div className="space-y-1">
              <p>
                只删除{" "}
                <code className="font-mono">
                  {dirOf(pendingDelete?.dir ?? "")?.relativePath}/{pendingDelete?.skill.id}
                </code>
                ，其他目录的副本与项目真源保持不变。
              </p>
              <p className="text-caption text-fg-3">删除前会先备份到备份目录。</p>
            </div>
          )
        }
        onConfirm={() => {
          const request = pendingDelete;
          setPendingDelete(null);
          if (!request) return;
          if (request.kind === "uninstall") void uninstall(request.rows);
          else if (request.kind === "purge") void purge(request.skill.id);
          else void removeCopy(request.skill, request.dir);
        }}
        onCancel={() => setPendingDelete(null)}
      />

      {migrating && data?.migration && (
        <ProjectMigrationDialog
          plan={data.migration}
          dirs={dirs}
          busy={busy}
          onCancel={() => setMigrating(false)}
          onConfirm={(decisions) => void migrate(decisions)}
        />
      )}

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
