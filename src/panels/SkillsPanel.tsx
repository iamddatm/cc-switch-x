import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderInput, Plus, RefreshCw, Sparkles } from "lucide-react";
import { AgentColumnCells } from "../components/assets/AgentColumnCells";
import { AgentColumnsHeader } from "../components/assets/AgentColumnsHeader";
import { AssetRow } from "../components/assets/AssetRow";
import { ImportDialog } from "../components/ImportDialog";
import { SkillBatchBar } from "../components/skills/SkillBatchBar";
import { SkillDiscoveryPanel } from "../components/skills/SkillDiscoveryPanel";
import { SkillInstallSheet } from "../components/skills/SkillInstallSheet";
import { SkillSourceBadge } from "../components/skills/SkillSourceBadge";
import { SkillFormSheet } from "../components/skills/SkillFormSheet";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Checkbox } from "../components/ui/Checkbox";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { EmptyState } from "../components/ui/EmptyState";
import { Menu, type MenuItem } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";
import { parseSkill, type SkillForm } from "../lib/canonical";
import { formatRelativeTime } from "../lib/format";
import { supported, type AssetSummary, type SkillCheckReport } from "../types";

type Editing =
  | { mode: "create"; id: string; form: SkillForm }
  | { mode: "edit"; id: string; form: SkillForm };

const EMPTY_FORM: SkillForm = { name: "", description: "", body: "", extraFrontmatter: "" };

const SOURCE_FILTERS = [
  { value: "all", label: "全部来源" },
  { value: "github", label: "GitHub 安装" },
  { value: "zip", label: "ZIP 安装" },
  { value: "local", label: "本地目录" },
  { value: "cc-switch", label: "cc-switch 导入" },
] as const;

function sourceKey(item: AssetSummary): string {
  return item.origin?.kind ?? (item.source ? "github" : "local");
}

export function SkillsPanel() {
  const { items, error, loading, refresh } = useAssets("skill");
  const { agents, enabledAgents, refresh: refreshAgents } = useAgents();
  const [view, setView] = useState<"installed" | "discover">("installed");
  const [discoverMounted, setDiscoverMounted] = useState(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [sourceFilter, setSourceFilter] = useState("all");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [loadingEdit, setLoadingEdit] = useState(false);
  const [deleting, setDeleting] = useState<AssetSummary[] | null>(null);
  const [pendingDelete, setPendingDelete] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [checking, setChecking] = useState(false);
  const [checkReport, setCheckReport] = useState<SkillCheckReport | null>(null);
  const [updateTargets, setUpdateTargets] = useState<string[] | null>(null);
  const [updating, setUpdating] = useState(false);
  const [batchBusy, setBatchBusy] = useState(false);

  const agentsForType = enabledAgents.filter((agent) => supported(agent, "skill"));
  const refreshAll = () => {
    void refresh();
    void refreshAgents();
  };

  const normalized = query.trim().toLowerCase();
  const visible = items.filter((item) => {
    if (
      normalized &&
      ![item.name, item.id, item.description ?? "", item.source ?? "", item.origin?.repo ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    ) {
      return false;
    }
    if (sourceFilter !== "all" && sourceKey(item) !== sourceFilter) return false;
    if (filter === "all") return true;
    if (filter === "off") {
      return !agentsForType.some((agent) => agent.enabledSkills.includes(item.id));
    }
    const agentId = filter.slice("on:".length);
    return (
      agents.find((agent) => agent.id === agentId)?.enabledSkills.includes(item.id) === true
    );
  });

  const selectedItems = items.filter((item) => selected.has(item.id));
  const selectedIds = selectedItems.map((item) => item.id);
  const allVisibleSelected =
    visible.length > 0 && visible.every((item) => selected.has(item.id));
  const enabledAgentsForSelected = agentsForType.filter((agent) =>
    selectedItems.some((item) => agent.enabledSkills.includes(item.id)),
  );
  const updatableSelected = selectedItems.filter((item) => item.origin?.updatable).length;

  const toggleSelected = (id: string, checked: boolean) => {
    setSelected((previous) => {
      const next = new Set(previous);
      if (checked) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const toggleAllVisible = () => {
    setSelected((previous) => {
      const next = new Set(previous);
      if (allVisibleSelected) {
        visible.forEach((item) => next.delete(item.id));
      } else {
        visible.forEach((item) => next.add(item.id));
      }
      return next;
    });
  };

  const openEdit = async (item: AssetSummary) => {
    setLoadingEdit(true);
    setActionError(null);
    try {
      const text = await api.readAsset("skill", item.id);
      setEditing({ mode: "edit", id: item.id, form: parseSkill(text) });
    } catch (e) {
      setActionError(String(e));
    } finally {
      setLoadingEdit(false);
    }
  };

  const create = () => {
    setEditing({ mode: "create", id: "", form: EMPTY_FORM });
  };

  const importDirectory = async () => {
    setActionError(null);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择包含 SKILL.md 的目录",
      });
      if (typeof selected !== "string") return;
      await api.importSkillDir(selected);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    }
  };

  const checkUpdates = async (ids?: string[]) => {
    setChecking(true);
    setActionError(null);
    setActionMessage(null);
    setCheckReport(null);
    try {
      const report = await api.checkSkillUpdates(ids);
      setCheckReport(report);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setChecking(false);
    }
  };

  const askUpdate = (ids: string[]) => {
    if (ids.length > 0) setUpdateTargets(ids);
  };

  const runUpdate = async () => {
    if (!updateTargets) return;
    setUpdating(true);
    setActionError(null);
    setActionMessage(null);
    try {
      const report = await api.updateSkills(updateTargets);
      const failed = report.outcomes.filter((outcome) => outcome.status === "failed");
      const succeeded = report.outcomes.length - failed.length;
      setActionMessage(
        `更新完成：成功 ${succeeded} 个${failed.length ? `，失败 ${failed.length} 个` : ""}。旧版本已备份到 backups/skills（保留最近 20 份）。`,
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
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setUpdating(false);
      setUpdateTargets(null);
    }
  };

  const confirmDelete = async () => {
    if (!deleting) return;
    setPendingDelete(true);
    setActionError(null);
    setActionMessage(null);
    const ids = deleting.map((item) => item.id);
    try {
      const report = await api.uninstallSkills(ids);
      const failed = report.outcomes.filter((outcome) => outcome.error);
      const succeeded = report.outcomes.length - failed.length;
      setActionMessage(
        `已删除 ${succeeded} 个 Skill，旧版本备份在 backups/skills（保留最近 20 份）。`,
      );
      if (failed.length > 0) {
        setActionError(
          failed.map((outcome) => `${outcome.id}：${outcome.error}`).join("；"),
        );
      }
      setSelected((previous) => {
        const next = new Set(previous);
        ids.forEach((id) => next.delete(id));
        return next;
      });
      setDeleting(null);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setPendingDelete(false);
    }
  };

  const setEnabledFor = async (agentId: string, enabled: boolean) => {
    if (selectedIds.length === 0) return;
    setBatchBusy(true);
    setActionError(null);
    setActionMessage(null);
    try {
      const report = await api.setSkillsEnabled(selectedIds, agentId, enabled);
      const label = enabled ? "启用" : "停用";
      setActionMessage(
        `${label}完成：成功 ${report.ok.length} 个${report.failed.length ? `，失败 ${report.failed.length} 个` : ""}。`,
      );
      if (report.failed.length > 0) {
        setActionError(
          report.failed.map((item) => `${item.id}：${item.error}`).join("；"),
        );
      }
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBatchBusy(false);
    }
  };

  const rowMenuItems = (item: AssetSummary): MenuItem[] => {
    const menu: MenuItem[] = [{ label: "编辑", onSelect: () => void openEdit(item) }];
    if (item.origin?.updatable) {
      menu.push(
        { label: "检查更新", onSelect: () => void checkUpdates([item.id]) },
        { label: "更新", onSelect: () => askUpdate([item.id]) },
      );
    }
    menu.push({ label: "删除", danger: true, onSelect: () => setDeleting([item]) });
    return menu;
  };

  const deployedTo = deleting
    ? agents.filter((agent) => deleting.some((item) => agent.enabledSkills.includes(item.id)))
    : [];

  const checkFailures = checkReport
    ? [...checkReport.repoFailures.map((failure) => `${failure.repo}：${failure.error}`), ...checkReport.skillFailures.map((failure) => `${failure.id}：${failure.error}`)]
    : [];

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <PageHeader
        icon={Sparkles}
        title="Skills"
        actions={
          <>
            <Button
              onClick={() => void checkUpdates()}
              loading={checking}
              disabled={items.length === 0 || updating || pendingDelete}
            >
              <RefreshCw size={14} aria-hidden />
              检查更新
            </Button>
            {checkReport && checkReport.updates.length > 0 && (
              <Button
                variant="primary"
                onClick={() => askUpdate(checkReport.updates.map((update) => update.id))}
                disabled={updating}
              >
                更新 {checkReport.updates.length} 个
              </Button>
            )}
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
                { label: "新建 Skill", onSelect: create },
                { label: "从文件夹导入", onSelect: () => void importDirectory() },
                { label: "从来源安装…", onSelect: () => setInstalling(true) },
              ]}
            />
            <Menu
              label="Skills 的更多操作"
              items={[
                { label: "从 cc-switch 导入", onSelect: () => setImporting(true) },
                { label: "刷新列表", onSelect: refreshAll },
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
          <span className="text-caption text-fg-3">{items.length}</span>
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
        <div className="flex h-12 shrink-0 items-center gap-3 px-6">
          <SearchField
          className="w-[280px]"
          value={query}
          onValueChange={setQuery}
          placeholder="搜索名称、描述、来源或仓库…"
          ariaLabel="搜索 Skill"
        />
        <Select
          aria-label="按 Agent 筛选"
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        >
          <option value="all">全部 Agent</option>
          <option value="off">未启用任何 Agent</option>
          {agentsForType.map((agent) => (
            <option key={agent.id} value={`on:${agent.id}`}>
              已启用给 {agent.displayName}
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
          {visible.length} / {items.length} 个 Skill
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
        {(error || actionError) && (
          <Notice tone="danger" className="mb-3">
            {error ?? actionError}
          </Notice>
        )}
        {actionMessage && (
          <Notice tone="success" className="mb-3">
            {actionMessage}
          </Notice>
        )}
        {checking && (
          <Notice tone="info" className="mb-3">
            正在读取上游仓库并比对内容…
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
                {checkReport.skipped.length} 个 Skill 没有可检查的 GitHub 上游，已跳过。
              </Notice>
            )}
            {checkFailures.length > 0 && (
              <Notice tone="danger">{checkFailures.join("；")}</Notice>
            )}
          </div>
        )}
        {selectedItems.length > 0 && (
          <div className="mb-3">
            <SkillBatchBar
              count={selectedItems.length}
              agents={agentsForType}
              enabledAgents={enabledAgentsForSelected}
              updatableCount={updatableSelected}
              busy={batchBusy || checking || updating}
              onCheck={() => void checkUpdates(selectedIds)}
              onUpdate={() =>
                askUpdate(
                  selectedItems
                    .filter((item) => item.origin?.updatable)
                    .map((item) => item.id),
                )
              }
              onEnable={(agentId) => void setEnabledFor(agentId, true)}
              onDisable={(agentId) => void setEnabledFor(agentId, false)}
              onDelete={() => setDeleting(selectedItems)}
              onClear={() => setSelected(new Set())}
            />
          </div>
        )}
        {enabledAgents.length === 0 && (
          <Notice tone="warning" className="mb-3">
            还没有启用任何 Agent，右侧不会出现勾选列。去左侧 Agents 页选择要管理的 Agent。
          </Notice>
        )}
        {loadingEdit && <p className="mb-2 text-caption text-fg-3">正在读取 SKILL.md…</p>}

        {loading && items.length === 0 ? (
          <p className="py-10 text-center text-caption text-fg-3">正在读取 Skill…</p>
        ) : items.length === 0 ? (
          <EmptyState
            icon={Sparkles}
            title="还没有 Skill"
            description="可以从本地目录导入、从来源安装，或新建一个 SKILL.md。"
            actions={
              <>
                <Button variant="primary" onClick={create}>
                  新建 Skill
                </Button>
                <Button onClick={() => void importDirectory()}>
                  <FolderInput size={14} aria-hidden />
                  从文件夹导入
                </Button>
                <Button onClick={() => setInstalling(true)}>从来源安装</Button>
                <Button onClick={() => setImporting(true)}>从 cc-switch 导入</Button>
              </>
            }
          />
        ) : visible.length === 0 ? (
          <EmptyState icon={Sparkles} title="没有匹配的 Skill" description="换个关键词或筛选条件。" />
        ) : (
          <div className="overflow-hidden rounded-panel border border-border bg-surface">
            <div className="flex h-11 items-center bg-subtle pe-3 ps-4">
              <span className="flex w-6 shrink-0 items-center">
                <Checkbox
                  aria-label="全选当前列表"
                  checked={allVisibleSelected}
                  onChange={toggleAllVisible}
                />
              </span>
              <span className="flex-1 text-caption font-semibold text-fg-2">名称</span>
              <AgentColumnsHeader assetType="skill" agents={agentsForType} items={visible} />
              <span className="w-12" />
            </div>
            <ul>
              {visible.map((item) => {
                const detail = [
                  item.description,
                  item.mtime ? `更新于 ${formatRelativeTime(item.mtime)}` : null,
                  item.origin?.lastCheckedAt
                    ? `检查于 ${formatRelativeTime(item.origin.lastCheckedAt)}`
                    : null,
                ]
                  .filter(Boolean)
                  .join(" · ");
                return (
                  <AssetRow
                    key={item.id}
                    title={item.name}
                    subtitle={item.id}
                    description={detail || undefined}
                    leading={
                      <Checkbox
                        aria-label={`选择 ${item.name}`}
                        checked={selected.has(item.id)}
                        onChange={(event) => toggleSelected(item.id, event.target.checked)}
                      />
                    }
                    selected={selected.has(item.id)}
                    titleBadges={
                      item.origin?.updateAvailable ? <Badge tone="warning">可更新</Badge> : undefined
                    }
                    badges={<SkillSourceBadge item={item} />}
                    agentCells={
                      <AgentColumnCells
                        assetType="skill"
                        id={item.id}
                        agents={agentsForType}
                        onChanged={refreshAll}
                        onError={setActionError}
                        onOutcome={(label) => {
                          setActionError(null);
                          setActionMessage(`实测：${label}`);
                        }}
                      />
                    }
                    actions={
                      <Menu
                        label={`${item.name} 的更多操作`}
                        items={rowMenuItems(item)}
                      />
                    }
                    footer={
                      item.origin?.lastCheckError
                        ? `上次检查失败：${item.origin.lastCheckError}`
                        : undefined
                    }
                    footerTone="warning"
                    onOpen={() => void openEdit(item)}
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
            installedItems={items}
            agents={agentsForType}
            onInstalled={refreshAll}
            onShowInstalled={() => setView("installed")}
          />
        </div>
      )}

      {editing && (
        <SkillFormSheet
          mode={editing.mode}
          id={editing.id}
          initial={editing.form}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            refreshAll();
          }}
        />
      )}

      {installing && (
        <SkillInstallSheet
          onClose={() => setInstalling(false)}
          onInstalled={refreshAll}
        />
      )}

      {importing && (
        <ImportDialog
          onClose={() => {
            setImporting(false);
            refreshAll();
          }}
        />
      )}

      <ConfirmDialog
        open={updateTargets !== null}
        title={`更新 ${updateTargets?.length ?? 0} 个 Skill？`}
        pending={updating}
        confirmLabel="开始更新"
        body={
          <div className="space-y-2">
            <p>会从 GitHub 重新下载并整目录替换；本地修改将被覆盖。</p>
            <p>替换前会自动备份到 backups/skills，保留最近 20 份。</p>
            {updateTargets && (
              <p className="text-caption text-fg-3">{updateTargets.join("、")}</p>
            )}
          </div>
        }
        onConfirm={() => void runUpdate()}
        onCancel={() => setUpdateTargets(null)}
      />

      <ConfirmDialog
        open={deleting !== null}
        title={
          deleting && deleting.length > 1
            ? `删除 ${deleting.length} 个 Skill？`
            : `删除 ${deleting?.[0]?.name ?? ""}？`
        }
        danger
        pending={pendingDelete}
        confirmLabel="删除 Skill"
        body={
          <div className="space-y-2">
            <p>
              会先整目录备份到 backups/skills（保留最近 20 份），再从已启用的 agent 撤下并删除真源。
            </p>
            {deployedTo.length > 0 && (
              <p>
                将从 {deployedTo.map((agent) => agent.displayName).join("、")} 撤下。
              </p>
            )}
          </div>
        }
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleting(null)}
      />
    </div>
  );
}
