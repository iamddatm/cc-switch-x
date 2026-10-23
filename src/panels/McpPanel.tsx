import { useState } from "react";
import { Plus, Server } from "lucide-react";
import { AgentColumnCells } from "../components/assets/AgentColumnCells";
import { AgentColumnsHeader } from "../components/assets/AgentColumnsHeader";
import { AssetRow } from "../components/assets/AssetRow";
import { ImportDialog } from "../components/ImportDialog";
import { McpFormSheet } from "../components/mcp/McpFormSheet";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { EmptyState } from "../components/ui/EmptyState";
import { Menu } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";
import { emptyMcpForm, parseMcp, type McpForm } from "../lib/mcp";
import { supported, type AssetSummary } from "../types";

type Editing =
  | { mode: "create"; form: McpForm }
  | { mode: "edit"; id: string; form: McpForm };

export function McpPanel() {
  const { items, error, loading, refresh } = useAssets("mcp");
  const { agents, enabledAgents, refresh: refreshAgents } = useAgents();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [editing, setEditing] = useState<Editing | null>(null);
  const [loadingEdit, setLoadingEdit] = useState(false);
  const [deleting, setDeleting] = useState<AssetSummary | null>(null);
  const [pendingDelete, setPendingDelete] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionMessage, setActionMessage] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  const agentsForType = enabledAgents.filter((agent) => supported(agent, "mcp"));
  const refreshAll = () => {
    void refresh();
    void refreshAgents();
  };

  const normalized = query.trim().toLowerCase();
  const visible = items.filter((item) => {
    if (
      normalized &&
      [item.name, item.id, item.description ?? "", item.transport ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    ) {
      return false;
    }
    if (filter === "all") return true;
    if (filter === "off") {
      return !agentsForType.some((agent) => agent.enabledMcp.includes(item.id));
    }
    const agentId = filter.slice("on:".length);
    return agents.find((agent) => agent.id === agentId)?.enabledMcp.includes(item.id) === true;
  });

  const openEdit = async (item: AssetSummary) => {
    setLoadingEdit(true);
    setActionError(null);
    try {
      const text = await api.readAsset("mcp", item.id);
      setEditing({ mode: "edit", id: item.id, form: parseMcp(text) });
    } catch (e) {
      setActionError(String(e));
    } finally {
      setLoadingEdit(false);
    }
  };

  const create = () => {
    setEditing({ mode: "create", form: emptyMcpForm() });
  };

  const deleteServer = async () => {
    if (!deleting) return;
    setPendingDelete(true);
    setActionError(null);
    try {
      for (const agent of agents) {
        if (agent.enabledMcp.includes(deleting.id)) {
          await api.setMcpEnabled(deleting.id, agent.id, false);
        }
      }
      await api.deleteAsset("mcp", deleting.id);
      setDeleting(null);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setPendingDelete(false);
    }
  };

  const deployedTo = deleting
    ? agents.filter((agent) => agent.enabledMcp.includes(deleting.id))
    : [];

  const restartHint = (id: string) => {
    const names = enabledAgents
      .filter((agent) => agent.enabledMcp.includes(id) && agent.mcpNeedsRestart !== false)
      .map((agent) => agent.displayName);
    return names.length > 0 ? `${names.join("、")} 需重启或 reload` : null;
  };

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <PageHeader
        icon={Server}
        title="MCP"
        actions={
          <>
            <Button variant="primary" onClick={create}>
              <Plus size={14} aria-hidden />
              添加 MCP
            </Button>
            <Menu
              label="MCP 的更多操作"
              items={[
                { label: "从 cc-switch 导入", onSelect: () => setImporting(true) },
                { label: "刷新列表", onSelect: refreshAll },
              ]}
            />
          </>
        }
      />

      <div className="flex h-12 shrink-0 items-center gap-3 px-6">
        <SearchField
          className="w-[320px]"
          value={query}
          onValueChange={setQuery}
          placeholder="搜索名称、描述或类型…"
          ariaLabel="搜索 MCP"
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
        <span className="ml-auto text-caption text-fg-3">
          {visible.length} / {items.length} 个 MCP
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
        {enabledAgents.length === 0 && (
          <Notice tone="warning" className="mb-3">
            还没有启用任何 Agent，右侧不会出现勾选列。去左侧 Agents 页选择要管理的 Agent。
          </Notice>
        )}
        {loadingEdit && <p className="mb-2 text-caption text-fg-3">正在读取 MCP 配置…</p>}

        {loading && items.length === 0 ? (
          <p className="py-10 text-center text-caption text-fg-3">正在读取 MCP…</p>
        ) : items.length === 0 ? (
          <EmptyState
            icon={Server}
            title="还没有 MCP 服务器"
            description="添加一个服务器，或从 cc-switch 导入已有配置。"
            actions={
              <>
                <Button variant="primary" onClick={create}>
                  添加 MCP
                </Button>
                <Button onClick={() => setImporting(true)}>从 cc-switch 导入</Button>
              </>
            }
          />
        ) : visible.length === 0 ? (
          <EmptyState icon={Server} title="没有匹配的 MCP" description="换个关键词或筛选条件。" />
        ) : (
          <div className="overflow-hidden rounded-panel border border-border bg-surface">
            <div className="flex h-11 items-center bg-subtle pe-3 ps-4">
              <span className="flex-1 text-caption font-semibold text-fg-2">名称</span>
              <AgentColumnsHeader assetType="mcp" agents={agentsForType} items={visible} />
              <span className="w-12" />
            </div>
            <ul>
              {visible.map((item) => (
                <AssetRow
                  key={item.id}
                  title={item.name}
                  subtitle={item.id}
                  description={item.description ?? undefined}
                  titleBadges={<Badge mono>{item.transport ?? "?"}</Badge>}
                  agentCells={
                    <AgentColumnCells
                      assetType="mcp"
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
                  footer={restartHint(item.id)}
                  footerTone="warning"
                  actions={
                    <Menu
                      label={`${item.name} 的更多操作`}
                      items={[
                        { label: "编辑", onSelect: () => void openEdit(item) },
                        { label: "删除", danger: true, onSelect: () => setDeleting(item) },
                      ]}
                    />
                  }
                  onOpen={() => void openEdit(item)}
                />
              ))}
            </ul>
          </div>
        )}
      </div>

      {editing && (
        <McpFormSheet
          mode={editing.mode}
          initial={editing.form}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            refreshAll();
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

      <ConfirmDialog
        open={deleting !== null}
        title={`删除 ${deleting?.name ?? ""}？`}
        danger
        pending={pendingDelete}
        confirmLabel="删除 MCP"
        body={
          <p>
            会删除真源里的 MCP 配置，不可撤销。
            {deployedTo.length > 0 &&
              ` 删除前会先从 ${deployedTo.map((agent) => agent.displayName).join("、")} 的配置里移除。`}
          </p>
        }
        onConfirm={() => void deleteServer()}
        onCancel={() => setDeleting(null)}
      />
    </div>
  );
}