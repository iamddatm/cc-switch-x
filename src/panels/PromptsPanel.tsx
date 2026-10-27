import { useEffect, useState } from "react";
import { BookOpen, Pencil, Plus } from "lucide-react";
import { AgentGlyph } from "../components/agents/AgentGlyph";
import { AssetRow } from "../components/assets/AssetRow";
import { DriftBanner } from "../components/DriftBanner";
import { ImportDialog } from "../components/ImportDialog";
import { PromptFormSheet } from "../components/prompts/PromptFormSheet";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import { EmptyState } from "../components/ui/EmptyState";
import { IconButton } from "../components/ui/IconButton";
import { Menu } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useDrift } from "../hooks/useDrift";
import { api } from "../lib/api";
import { parsePrompt, type PromptForm } from "../lib/canonical";
import { formatBytes, formatRelativeTime } from "../lib/format";
import { supported, switchable, type AssetSummary } from "../types";

type Editing =
  | { mode: "create"; form: PromptForm }
  | { mode: "edit"; id: string; form: PromptForm };

const EMPTY_FORM: PromptForm = { title: "", description: "", body: "" };

export function PromptsPanel() {
  const { items, error, loading, refresh } = useAssets("prompt");
  const { enabledAgents, refresh: refreshAgents } = useAgents();
  const drift = useDrift();
  const promptAgents = enabledAgents.filter((agent) => supported(agent, "prompt"));
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [loadingEdit, setLoadingEdit] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
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

  const normalized = query.trim().toLowerCase();
  const visible = items.filter(
    (item) =>
      !normalized ||
      [item.name, item.id, item.description ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(normalized),
  );

  const openEdit = async (item: AssetSummary) => {
    setLoadingEdit(true);
    setActionError(null);
    try {
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
      setActionError(String(e));
      // 被漂移挡住时立刻重扫，让处置横幅当场出现，而不是只留一串错误文本
      void drift.rescan();
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
      setActionError(String(e));
      void drift.rescan();
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
          <>
            <Button variant="primary" onClick={create}>
              <Plus size={14} aria-hidden />
              添加提示词
            </Button>
            <Menu
              label="提示词的更多操作"
              items={[
                { label: "从 cc-switch 导入", onSelect: () => setImporting(true) },
                { label: "刷新列表", onSelect: refreshAll },
              ]}
            />
          </>
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
        </div>
        {selectedAgent && (
          <code
            className="max-w-[360px] truncate font-mono text-caption text-fg-3"
            title={selectedAgent.promptPath}
          >
            目标文件：{selectedAgent.promptPath}
          </code>
        )}
        <SearchField
          className="ml-auto w-[240px]"
          value={query}
          onValueChange={setQuery}
          placeholder="搜索名称、说明或正文…"
          ariaLabel="搜索提示词"
        />
        <span className="text-caption text-fg-3">{visible.length} 条</span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
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
        {(error || actionError) && (
          <Notice tone="danger" className="mb-3">
            {error ?? actionError}
          </Notice>
        )}
        {message && (
          <Notice tone="success" className="mb-3">
            {message}
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
                const details = [
                  item.description,
                  item.size != null ? formatBytes(item.size) : null,
                  item.mtime ? `${formatRelativeTime(item.mtime)}更新` : null,
                ]
                  .filter(Boolean)
                  .join(" · ");
                return (
                  <AssetRow
                    key={item.id}
                    title={item.name}
                    subtitle={item.id}
                    description={details || undefined}
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
      </div>

      {editing && (
        <PromptFormSheet
          mode={editing.mode}
          id={editing.mode === "edit" ? editing.id : ""}
          initial={editing.form}
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