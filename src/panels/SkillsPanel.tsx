import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderInput, Plus, RefreshCw, Sparkles } from "lucide-react";
import { AgentColumnCells } from "../components/assets/AgentColumnCells";
import { AgentColumnsHeader } from "../components/assets/AgentColumnsHeader";
import { AssetRow } from "../components/assets/AssetRow";
import { ImportDialog } from "../components/ImportDialog";
import { SkillBatchBar } from "../components/skills/SkillBatchBar";
import { SkillDetailSheet } from "../components/skills/SkillDetailSheet";
import { SkillDiscoveryPanel } from "../components/skills/SkillDiscoveryPanel";
import { SkillInstallSheet } from "../components/skills/SkillInstallSheet";
import { originRepoLabel, SkillSourceBadge } from "../components/skills/SkillSourceBadge";
import { SkillFormSheet } from "../components/skills/SkillFormSheet";
import { SkillRepoFilter } from "../components/skills/SkillRepoFilter";
import { SkillViewTabs } from "../components/skills/SkillViewTabs";
import { useSkillFilters } from "../components/skills/useSkillFilters";
import { storeScope } from "../components/skills/skillScope";
import { checkFailureLines } from "../components/skills/skillFilters";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Checkbox } from "../components/ui/Checkbox";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { EmptyState } from "../components/ui/EmptyState";
import { FeedbackBar } from "../components/ui/FeedbackBar";
import { Menu, type MenuItem } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useSkillUpdateCheck } from "../hooks/useSkillUpdateCheck";
import { api } from "../lib/api";
import { parseSkill, type SkillForm } from "../lib/canonical";
import { formatRelativeTime } from "../lib/format";
import { enabledFor, supported, type AssetSummary } from "../types";

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

export function SkillsPanel() {
  const { items, error, loading, refresh } = useAssets("skill");
  const { agents, enabledAgents, refresh: refreshAgents } = useAgents();
  const skillCheck = useSkillUpdateCheck();
  const [view, setView] = useState<"installed" | "discover">("installed");
  const [discoverMounted, setDiscoverMounted] = useState(false);
  const [agentFilter, setAgentFilter] = useState("all");
  const [statusFilter, setStatusFilter] = useState<"all" | "on" | "off">("all");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [viewing, setViewing] = useState<{ item: AssetSummary; form: SkillForm } | null>(null);
  const [loadingEdit, setLoadingEdit] = useState(false);
  const [deleting, setDeleting] = useState<AssetSummary[] | null>(null);
  const [pendingDelete, setPendingDelete] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [updateTargets, setUpdateTargets] = useState<string[] | null>(null);
  const [updating, setUpdating] = useState(false);
  const [batchBusy, setBatchBusy] = useState(false);

  const {
    report: checkReport,
    checking,
    error: checkError,
    prompt: checkPrompt,
    check: runCheck,
    consumePrompt,
    dropUpdates,
  } = skillCheck;

  const agentsForType = useMemo(
    () => enabledAgents.filter((agent) => supported(agent, "skill")),
    [enabledAgents],
  );
  const scope = useMemo(() => storeScope(agentsForType), [agentsForType]);
  const {
    query,
    setQuery,
    sourceFilter,
    setSourceFilter,
    repoChoices,
    repoKeys,
    allReposSelected,
    matched,
    selected,
    selectedItems,
    selectedIds,
    toggleRepo,
    toggleAllRepos,
    toggleOne,
    toggleAll,
    allVisibleSelected,
    drop,
    clear,
  } = useSkillFilters(items);
  const refreshAll = () => {
    void refresh();
    void refreshAgents();
  };

  useEffect(() => {
    if (agentFilter === "all") return;
    const stillAvailable = agents.some(
      (agent) => agent.id === agentFilter && agent.enabled && supported(agent, "skill"),
    );
    if (!stillAvailable) setAgentFilter("all");
  }, [agentFilter, agents]);

  // 检查可能在别的页面里结束；回来时把「发现更新」的确认弹出来（只在弹一次）。
  useEffect(() => {
    if (!checkPrompt) return;
    consumePrompt();
    setUpdateTargets(checkReport?.updates.map((update) => update.id) ?? []);
  }, [checkPrompt, checkReport, consumePrompt]);

  const statusAgents = useMemo(
    () =>
      agentFilter === "all"
        ? agentsForType
        : agentsForType.filter((agent) => agent.id === agentFilter),
    [agentFilter, agentsForType],
  );
  // 搜索 / 来源 / 仓库三轴在 useSkillFilters 里；「启用状态」这一轴是用户级特有的。
  // 这条要 memo：每个搜索字符都会带着整张启用表重跑一遍谓词。
  const visible = useMemo(
    () =>
      matched.filter((item) => {
        if (statusFilter === "all") return true;
        const enabled = statusAgents.some((agent) => enabledFor(agent, "skill").includes(item.id));
        return statusFilter === "on" ? enabled : !enabled;
      }),
    [matched, statusAgents, statusFilter],
  );
  const updateIds = checkReport?.updates.map((update) => update.id) ?? [];
  const updatesCount = updateIds.length;

  const enabledAgentsForSelected = agentsForType.filter((agent) =>
    selectedItems.some((item) => agent.enabledSkills.includes(item.id)),
  );
  const updatableSelected = selectedItems.filter((item) => item.origin?.updatable).length;

  // 查看与编辑都要先读真源；失败时统一报错并返回 null。
  const readSkill = async (item: AssetSummary): Promise<string | null> => {
    setLoadingEdit(true);
    setActionError(null);
    try {
      return await scope.readBody(item.id);
    } catch (e) {
      setActionError(String(e));
      return null;
    } finally {
      setLoadingEdit(false);
    }
  };

  const openView = async (item: AssetSummary) => {
    const text = await readSkill(item);
    if (text === null) return;
    setEditing(null);
    setViewing({ item, form: parseSkill(text) });
  };

  const openEdit = async (item: AssetSummary) => {
    const text = await readSkill(item);
    if (text === null) return;
    setViewing(null);
    setEditing({ mode: "edit", id: item.id, form: parseSkill(text) });
  };

  const editViewed = () => {
    if (!viewing) return;
    setEditing({ mode: "edit", id: viewing.item.id, form: viewing.form });
    setViewing(null);
  };

  const create = () => {
    setEditing({ mode: "create", id: "", form: EMPTY_FORM });
  };

  const importDirectory = async () => {
    setActionError(null);
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "选择包含 SKILL.md 的目录",
      });
      if (typeof picked !== "string") return;
      await scope.importDir(picked);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    }
  };

  const checkUpdates = async (ids?: string[]) => {
    setActionError(null);
    setActionMessage(null);
    await runCheck(ids);
    refreshAll();
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
      dropUpdates(
        report.outcomes
          .filter((outcome) => outcome.status !== "failed")
          .map((outcome) => outcome.id),
      );
      clear();
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
      drop(ids);
      dropUpdates(ids);
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
    const menu: MenuItem[] = [
      { label: "查看", onSelect: () => void openView(item) },
      { label: "编辑", onSelect: () => void openEdit(item) },
    ];
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

  const checkFailures = checkFailureLines(checkReport);

  const updateEntries = (updateTargets ?? []).map((id) => {
    const info = checkReport?.updates.find((update) => update.id === id);
    const item = items.find((entry) => entry.id === id);
    return {
      id,
      name: info?.name ?? item?.name ?? id,
      repo: originRepoLabel(item?.origin),
    };
  });

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <PageHeader
        icon={Sparkles}
        title="Skills"
        actions={
          <>
            {updatesCount > 0 && !checking && (
              <Button
                variant="primary"
                onClick={() => askUpdate(updateIds)}
                disabled={updating || pendingDelete}
              >
                全部更新（{updatesCount} 个）
              </Button>
            )}
            {/* 检查更新常驻：查到更新后它不能被「全部更新」顶掉，否则重检只剩菜单里一个入口 */}
            <Button
              onClick={() => void checkUpdates()}
              loading={checking}
              disabled={items.length === 0 || updating || pendingDelete}
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

      <SkillViewTabs
        view={view}
        installedCount={items.length}
        onSelect={(next) => {
          setView(next);
          if (next === "discover") setDiscoverMounted(true);
        }}
      />

      <div className={view === "installed" ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
        <div className="flex h-12 shrink-0 items-center gap-2 px-6">
          <SearchField
            className="w-[240px]"
            value={query}
            onValueChange={setQuery}
            placeholder="搜索名称、描述、来源或仓库…"
            ariaLabel="搜索 Skill"
          />
          <SkillRepoFilter
            repoChoices={repoChoices}
            repoKeys={repoKeys}
            allReposSelected={allReposSelected}
            onToggleRepo={toggleRepo}
            onToggleAllRepos={toggleAllRepos}
          />
          <Select
            aria-label="按 Agent 筛选"
            value={agentFilter}
            onChange={(event) => setAgentFilter(event.target.value)}
          >
            <option value="all">全部 Agent</option>
            {agentsForType.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.displayName}
              </option>
            ))}
          </Select>
          <Select
            aria-label="按启用状态筛选"
            value={statusFilter}
            onChange={(event) =>
              setStatusFilter(event.target.value as "all" | "on" | "off")
            }
          >
            <option value="all">全部状态</option>
            <option value="on">已启用</option>
            <option value="off">未启用</option>
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

      {(error || actionError || checkError || actionMessage) && (
        <FeedbackBar>
          {(error || actionError || checkError) && (
            <Notice tone="danger">{error ?? actionError ?? checkError}</Notice>
          )}
          {actionMessage && <Notice tone="success">{actionMessage}</Notice>}
        </FeedbackBar>
      )}

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
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
              targets={scope.deployTargets}
              enabledTargets={enabledAgentsForSelected.map((agent) => ({
                key: agent.id,
                label: agent.displayName,
              }))}
              updatableCount={updatableSelected}
              busy={batchBusy || checking || updating}
              wording={scope.wording}
              noneEnabledLabel="没有已启用的 Agent"
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
              onClear={clear}
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
            <div className="flex h-11 items-center bg-subtle ps-4">
              <span className="flex w-6 shrink-0 items-center">
                <Checkbox
                  aria-label="全选当前列表"
                  checked={allVisibleSelected(visible)}
                  onChange={() => toggleAll(visible)}
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
                    subtitle={
                      item.mtime ? `更新于 ${formatRelativeTime(item.mtime)}` : undefined
                    }
                    description={detail || undefined}
                    leading={
                      <Checkbox
                        aria-label={`选择 ${item.name}`}
                        checked={selected.has(item.id)}
                        onChange={(event) => toggleOne(item.id, event.target.checked)}
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
                    openLabel={`查看 ${item.name}`}
                    onOpen={() => void openView(item)}
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
            scope={scope}
            targets={{
              options: scope.deployTargets,
              label: (selected) =>
                scope.deployTargets.length === 0
                  ? "未启用 Agent"
                  : selected.length === scope.deployTargets.length
                    ? `全部 Agent（${scope.deployTargets.length}）`
                    : selected.length === 0
                      ? "未选择 Agent"
                      : selected.map((target) => target.label).join("、"),
              emptyHint: "没有启用支持 Skills 的 Agent。",
            }}
            onInstalled={refreshAll}
            onShowInstalled={() => setView("installed")}
          />
        </div>
      )}

      {viewing && (
        <SkillDetailSheet
          item={viewing.item}
          form={viewing.form}
          onEdit={editViewed}
          onClose={() => setViewing(null)}
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
            refreshAll();
          }}
        />
      )}

      {installing && (
        <SkillInstallSheet
          scope={scope}
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
        confirmLabel={`更新 ${updateTargets?.length ?? 0} 个`}
        body={
          <div className="space-y-2">
            <p>
              更新会用仓库里的内容整目录替换，本地改过的地方只保留在备份里
              （backups/skills，保留最近 20 份）。
            </p>
            {updateEntries.length > 0 && (
              <ul
                aria-label="将要更新的 Skill"
                className="max-h-60 divide-y divide-border overflow-y-auto rounded-control border border-border"
              >
                {updateEntries.map((entry) => (
                  <li
                    key={entry.id}
                    className="flex items-baseline gap-2 px-3 py-2 text-body"
                  >
                    <span className="min-w-0 truncate font-medium text-fg-1">
                      {entry.name}
                    </span>
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
