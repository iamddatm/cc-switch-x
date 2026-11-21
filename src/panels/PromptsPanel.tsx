import { useCallback, useEffect, useMemo, useState } from "react";
import { BookOpen, Pencil, Plus } from "lucide-react";
import { AgentGlyph } from "../components/agents/AgentGlyph";
import { AssetRow } from "../components/assets/AssetRow";
import { DriftBanner } from "../components/DriftBanner";
import { ImportDialog } from "../components/ImportDialog";
import { PromptFormSheet } from "../components/prompts/PromptFormSheet";
import { PromptImportDialog } from "../components/prompts/PromptImportDialog";
import {
  PiPromptFileSheet,
  PiPromptFilesList,
  PiPromptTemplateSheet,
  PiPromptTemplatesList,
} from "../components/prompts/PiPromptResources";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import { EmptyState } from "../components/ui/EmptyState";
import { FeedbackBar } from "../components/ui/FeedbackBar";
import { IconButton } from "../components/ui/IconButton";
import { Menu } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useDrift } from "../hooks/useDrift";
import { api } from "../lib/api";
import { parsePrompt, type PromptForm } from "../lib/canonical";
import { formatRelativeTime } from "../lib/format";
import {
  supported,
  switchable,
  type AssetSummary,
  type PiPromptFileKind,
  type PiPromptFileSnapshot,
  type PiPromptTemplate,
} from "../types";

type Editing =
  | { mode: "create"; form: PromptForm }
  | { mode: "edit"; id: string; form: PromptForm };

const EMPTY_FORM: PromptForm = { title: "", description: "", body: "" };

export function PromptsPanel() {
  const { items, error, loading, refresh } = useAssets("prompt");
  const { enabledAgents, refresh: refreshAgents } = useAgents();
  const drift = useDrift();
  const promptAgents = useMemo(
    () => enabledAgents.filter((agent) => supported(agent, "prompt")),
    [enabledAgents],
  );
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [piSection, setPiSection] = useState<"library" | "system" | "templates">("library");
  const [piFiles, setPiFiles] = useState<Partial<Record<PiPromptFileKind, PiPromptFileSnapshot>>>({});
  const [piFilesLoading, setPiFilesLoading] = useState(false);
  const [piTemplates, setPiTemplates] = useState<PiPromptTemplate[]>([]);
  const [piTemplatesLoading, setPiTemplatesLoading] = useState(false);
  const [editingPiFile, setEditingPiFile] = useState<PiPromptFileKind | null>(null);
  const [editingPiTemplate, setEditingPiTemplate] = useState<PiPromptTemplate | "new" | null>(
    null,
  );
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [loadingEdit, setLoadingEdit] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [importingTargets, setImportingTargets] = useState(false);
  const [takeover, setTakeover] = useState<{
    promptId: string;
    agentId: string;
    path: string;
  } | null>(null);

  useEffect(() => {
    if (selectedAgentId && promptAgents.some((agent) => agent.id === selectedAgentId)) return;
    setSelectedAgentId(promptAgents[0]?.id ?? null);
  }, [promptAgents, selectedAgentId]);

  const selectedAgent = promptAgents.find((agent) => agent.id === selectedAgentId);
  const activePrompt = selectedAgent?.activePrompt ?? null;
  const canActivate = selectedAgent ? switchable(selectedAgent, "prompt") : false;
  // 提示词是整文件接管：这个 agent 名下任何一条 prompt 的漂移都会挡住启用，所以按 agent 汇总展示
  const promptDrifts = drift.drifts.filter(
    (record) => record.assetType === "prompt" && record.agentId === selectedAgentId,
  );
  const refreshAll = () => {
    void refresh();
    void refreshAgents();
    void drift.refresh();
  };

  // pi 的提示词分三块：提示库仍走 canonical + AGENTS.md，另外两块是 pi 自己的文件。
  const isPi = selectedAgent?.id === "pi" && canActivate;
  const piAgentDir = selectedAgent?.promptPath.replace(/[\\/][^\\/]+$/, "") ?? "";
  const piTemplatesDir = piAgentDir
    ? `${piAgentDir}${piAgentDir.includes("\\") ? "\\" : "/"}prompts`
    : "";

  const refreshPiFiles = useCallback(async () => {
    setPiFilesLoading(true);
    setActionError(null);
    try {
      const [append, override] = await Promise.all([
        api.readPiPromptFile("system_append"),
        api.readPiPromptFile("system_override"),
      ]);
      setPiFiles({ system_append: append, system_override: override });
    } catch (e) {
      setActionError(String(e));
    } finally {
      setPiFilesLoading(false);
    }
  }, []);

  const refreshPiTemplates = useCallback(async () => {
    setPiTemplatesLoading(true);
    setActionError(null);
    try {
      setPiTemplates(await api.listPiPromptTemplates());
    } catch (e) {
      setActionError(String(e));
    } finally {
      setPiTemplatesLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isPi) {
      setPiSection("library");
      setEditingPiFile(null);
      setEditingPiTemplate(null);
      return;
    }
    if (piSection === "system") void refreshPiFiles();
    else if (piSection === "templates") void refreshPiTemplates();
  }, [isPi, piSection, refreshPiFiles, refreshPiTemplates]);

  const visible = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return items;
    return items.filter((item) =>
      [item.name, item.id, item.description ?? ""].join(" ").toLowerCase().includes(normalized),
    );
  }, [items, query]);

  const openEdit = async (item: AssetSummary) => {
    setLoadingEdit(true);
    setActionError(null);
    try {
      // 目标文件与真源不一致属于落盘漂移，由列表上方的漂移横幅统一处置。
      const text = await api.readAsset("prompt", item.id);
      setEditing({ mode: "edit", id: item.id, form: parsePrompt(text) });
    } catch (e) {
      setActionError(String(e));
    } finally {
      setLoadingEdit(false);
    }
  };

  const create = () => {
    setEditing({ mode: "create", form: EMPTY_FORM });
  };

  /** 目标文件由后端按注册表解析；文件还不存在时后端会报错，这里原样展示。 */
  const openTarget = async () => {
    if (!selectedAgentId) return;
    setActionError(null);
    try {
      await api.openPromptTarget(selectedAgentId);
    } catch (e) {
      setActionError(String(e));
    }
  };

  /**
   * 整文件接管被未解决漂移挡住时，光留一串错误文本没法处置：
   * 立刻重扫一次，让处置横幅当场出现。activate 与 takeover 两条路径同一口径。
   */
  const reportWriteError = (e: unknown) => {
    setActionError(String(e));
    void drift.rescan();
  };

  const activate = async (promptId: string) => {
    if (!selectedAgentId) return;
    setBusy(promptId);
    setMessage(null);
    setActionError(null);
    try {
      const outcome = await api.activatePrompt(promptId, selectedAgentId);
      if (outcome.kind === "needsConfirmation") {
        setTakeover({ promptId, agentId: selectedAgentId, path: outcome.path });
      } else {
        setMessage(
          outcome.kind === "activated" ? `已写入 ${outcome.path}` : "该 agent 已经在使用这条提示词",
        );
        refreshAll();
      }
    } catch (e) {
      reportWriteError(e);
    } finally {
      setBusy(null);
    }
  };

  const takeoverAction = async (saveExistingAs?: string) => {
    if (!takeover) return;
    setBusy(takeover.promptId);
    setActionError(null);
    try {
      const options = saveExistingAs
        ? { saveExistingAs }
        : { confirm: true };
      const outcome = await api.activatePrompt(takeover.promptId, takeover.agentId, options);
      if (outcome.kind === "needsConfirmation") {
        setTakeover({ ...takeover, path: outcome.path });
        return;
      }
      setTakeover(null);
      setMessage(`已写入 ${outcome.path}`);
      refreshAll();
    } catch (e) {
      reportWriteError(e);
    } finally {
      setBusy(null);
    }
  };

  const deactivate = async () => {
    if (!selectedAgentId) return;
    setBusy("__deactivate__");
    setMessage(null);
    setActionError(null);
    try {
      const removed = await api.deactivatePrompt(selectedAgentId);
      setMessage(
        removed
          ? "已停用，目标文件已移除"
          : "目标文件不是本工具写的，未做改动",
      );
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = async (item: AssetSummary) => {
    setActionError(null);
    try {
      await api.deleteAsset("prompt", item.id);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    }
  };

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <PageHeader
        icon={BookOpen}
        title="提示词"
        actions={
          isPi && piSection === "system" ? (
            <Button onClick={() => void refreshPiFiles()} disabled={piFilesLoading}>
              刷新
            </Button>
          ) : isPi && piSection === "templates" ? (
            <>
              <Button onClick={() => void refreshPiTemplates()} disabled={piTemplatesLoading}>
                刷新
              </Button>
              <Button variant="primary" onClick={() => setEditingPiTemplate("new")}>
                <Plus size={14} aria-hidden />
                新建模板
              </Button>
            </>
          ) : (
            <>
              <Button variant="primary" onClick={create}>
                <Plus size={14} aria-hidden />
                添加提示词
              </Button>
              <Menu
                label="提示词的更多操作"
                items={[
                  {
                    label: "从 Agent 目标文件导入",
                    onSelect: () => setImportingTargets(true),
                    disabled: promptAgents.length === 0,
                    hint: promptAgents.length === 0 ? "还没有支持提示词的 Agent" : undefined,
                  },
                  { label: "从 cc-switch 导入", onSelect: () => setImporting(true) },
                  { label: "刷新列表", onSelect: refreshAll },
                ]}
              />
            </>
          )
        }
      />

      <div className="flex min-h-14 shrink-0 flex-wrap items-center gap-3 px-6 py-2">
        <div className="flex items-center gap-2">
          {selectedAgent && (
            <AgentGlyph agentId={selectedAgent.id} label={selectedAgent.displayName} size={18} />
          )}
          <Select
            aria-label="选择 Agent"
            value={selectedAgentId ?? ""}
            onChange={(event) => setSelectedAgentId(event.target.value || null)}
            disabled={promptAgents.length === 0}
          >
            {promptAgents.length === 0 && <option value="">没有可用的 Agent</option>}
            {promptAgents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.displayName}
              </option>
            ))}
          </Select>
          {isPi && (
            <SegmentedControl<"library" | "system" | "templates">
              size="sm"
              ariaLabel="pi 提示词区块"
              value={piSection}
              onValueChange={(next) => {
                setPiSection(next);
                setActionError(null);
                setMessage(null);
              }}
              items={[
                { value: "library", label: "提示库" },
                { value: "system", label: "系统提示" },
                { value: "templates", label: "模板" },
              ]}
            />
          )}
        </div>
        {selectedAgent && piSection === "library" && (
          <button
            type="button"
            className="max-w-[360px] cursor-pointer truncate font-mono text-caption text-fg-3 hover:text-fg-1 hover:underline"
            title={`用默认程序打开 ${selectedAgent.promptPath}`}
            onClick={() => void openTarget()}
          >
            目标文件：{selectedAgent.promptPath}
          </button>
        )}
        {isPi && piSection === "system" && (
          <span className="max-w-[360px] truncate font-mono text-caption text-fg-3">
            目标目录：{piAgentDir}
          </span>
        )}
        {isPi && piSection === "templates" && (
          <span className="max-w-[360px] truncate font-mono text-caption text-fg-3">
            目标目录：{piTemplatesDir}
          </span>
        )}
        {(!isPi || piSection === "library") && (
          <>
            <SearchField
              className="ml-auto w-[240px]"
              value={query}
              onValueChange={setQuery}
              placeholder="搜索名称、说明或正文…"
              ariaLabel="搜索提示词"
            />
            <span className="text-caption text-fg-3">{visible.length} 条</span>
          </>
        )}
        {isPi && piSection === "system" && (
          <span className="ml-auto text-caption text-fg-3">2 个文件</span>
        )}
        {isPi && piSection === "templates" && (
          <span className="ml-auto text-caption text-fg-3">{piTemplates.length} 个模板</span>
        )}
      </div>

      {(error || actionError || message) && (
        <FeedbackBar>
          {(error || actionError) && (
            <Notice tone="danger">{error ?? actionError}</Notice>
          )}
          {message && <Notice tone="success">{message}</Notice>}
        </FeedbackBar>
      )}

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
        {isPi && piSection === "system" ? (
          <PiPromptFilesList
            directory={piAgentDir}
            snapshots={piFiles}
            loading={piFilesLoading}
            onEdit={setEditingPiFile}
          />
        ) : isPi && piSection === "templates" ? (
          <PiPromptTemplatesList
            templates={piTemplates}
            loading={piTemplatesLoading}
            onEdit={setEditingPiTemplate}
            onCreate={() => setEditingPiTemplate("new")}
          />
        ) : (
          <>
            <DriftBanner
              drifts={promptDrifts}
              busy={drift.busy}
              onResolve={drift.resolve}
              onRescan={() => void drift.rescan()}
            />
            {drift.error && (
              <Notice tone="danger" className="mb-3">
                {drift.error}
              </Notice>
            )}
            {selectedAgent && !canActivate && (
              <Notice tone="warning" className="mb-3">
                {selectedAgent.displayName} 的提示词落点未实测，暂不提供启用/停用。
              </Notice>
            )}
            {promptAgents.length === 0 && (
              <Notice tone="warning" className="mb-3">
                还没有启用任何支持提示词的 Agent。去左侧 Agents 页选择要管理的 Agent。
              </Notice>
            )}
            {loadingEdit && <p className="mb-2 text-caption text-fg-3">正在读取提示词…</p>}

            {loading && items.length === 0 ? (
              <p className="py-10 text-center text-caption text-fg-3">正在读取提示词…</p>
            ) : items.length === 0 ? (
              <EmptyState
                icon={BookOpen}
                title="还没有提示词"
                description="添加一条长期说明，启用后会整份写入所选 Agent 的目标文件。"
                actions={
                  <>
                    <Button variant="primary" onClick={create}>
                      添加提示词
                    </Button>
                    <Button onClick={() => setImporting(true)}>从 cc-switch 导入</Button>
                  </>
                }
              />
            ) : visible.length === 0 ? (
              <EmptyState icon={BookOpen} title="没有匹配的提示词" description="换个关键词试试。" />
            ) : (
              <div className="overflow-hidden rounded-panel border border-border bg-surface">
                <div className="flex h-11 items-center bg-subtle px-4">
                  <span className="flex-1 text-caption font-semibold text-fg-2">名称</span>
                  <span className="w-12" />
                </div>
                <ul>
                  {visible.map((item) => {
                    const active = activePrompt === item.id;
                    // 时间取索引里的配置更新时间（保存/导入时记录），不用真源文件 mtime。
                    const details = [
                      item.description,
                      `更新于 ${formatRelativeTime(item.updatedAt)}`,
                    ]
                      .filter(Boolean)
                      .join(" · ");
                    return (
                      <AssetRow
                        key={item.id}
                        title={item.name}
                        description={details}
                        active={active}
                        titleBadges={active ? <Badge tone="success">已启用</Badge> : undefined}
                        agentCells={
                          <div className="flex items-center gap-1 px-2">
                            <Button
                              size="sm"
                              variant="secondary"
                              disabled={!canActivate || busy !== null}
                              onClick={() => (active ? void deactivate() : void activate(item.id))}
                            >
                              {active ? "停用" : "启用"}
                            </Button>
                            <IconButton
                              label={`编辑 ${item.name}`}
                              onClick={() => void openEdit(item)}
                            >
                              <Pencil size={14} aria-hidden />
                            </IconButton>
                          </div>
                        }
                        actions={
                          <Menu
                            label={`${item.name} 的更多操作`}
                            items={[
                              {
                                label: "删除",
                                danger: true,
                                disabled: active,
                                hint: active ? "启用中的不能删除，先停用" : undefined,
                                onSelect: () => void remove(item),
                              },
                            ]}
                          />
                        }
                        onOpen={() => void openEdit(item)}
                      />
                    );
                  })}
                </ul>
              </div>
          )}
          </>
        )}
      </div>

      {editingPiFile && piFiles[editingPiFile] && (
        <PiPromptFileSheet
          kind={editingPiFile}
          snapshot={piFiles[editingPiFile]!}
          onClose={() => setEditingPiFile(null)}
          onSaved={() => {
            setEditingPiFile(null);
            void refreshPiFiles();
          }}
        />
      )}

      {editingPiTemplate && (
        <PiPromptTemplateSheet
          template={editingPiTemplate === "new" ? null : editingPiTemplate}
          existingSlugs={piTemplates.map((template) => template.slug)}
          onClose={() => setEditingPiTemplate(null)}
          onSaved={() => {
            setEditingPiTemplate(null);
            void refreshPiTemplates();
          }}
        />
      )}

      {editing && (
        <PromptFormSheet
          mode={editing.mode}
          id={editing.mode === "edit" ? editing.id : ""}
          initial={editing.form}
          existingIds={items.map((item) => item.id)}
          onClose={() => setEditing(null)}
          onSaved={() => {
            const savedId = editing.mode === "edit" ? editing.id : null;
            setEditing(null);
            refreshAll();
            if (savedId && selectedAgentId && activePrompt === savedId) {
              // 编辑的是当前启用项：真源已变，顺手重新写一次目标文件。
              void api
                .activatePrompt(savedId, selectedAgentId, { confirm: true })
                .then(() => refreshAll())
                .catch((e) => setActionError(String(e)));
            }
          }}
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

      {importingTargets && (
        <PromptImportDialog
          agents={promptAgents}
          existing={items}
          onClose={() => {
            setImportingTargets(false);
            refreshAll();
          }}
          onDone={refreshAll}
        />
      )}

      <Dialog
        open={takeover !== null}
        onClose={() => setTakeover(null)}
        role="alertdialog"
        title="目标文件已有内容"
        description={`${takeover?.path ?? ""} 已有内容，且不是本工具写入的。整文件接管会覆盖它，请先选择处置方式。`}
        footer={
          <>
            <Button onClick={() => setTakeover(null)} disabled={busy !== null}>
              取消
            </Button>
            <Button
              onClick={() => void takeoverAction(`salvaged-${Date.now()}`)}
              disabled={busy !== null}
            >
              把现有内容另存为一条 prompt
            </Button>
            <Button
              variant="dangerSolid"
              loading={busy === takeover?.promptId}
              onClick={() => void takeoverAction()}
            >
              确认覆盖
            </Button>
          </>
        }
      />
    </div>
  );
}