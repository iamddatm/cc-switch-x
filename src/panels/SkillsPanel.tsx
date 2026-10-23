import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderInput, Plus, Sparkles } from "lucide-react";
import { AgentColumnCells } from "../components/assets/AgentColumnCells";
import { AgentColumnsHeader } from "../components/assets/AgentColumnsHeader";
import { AssetRow } from "../components/assets/AssetRow";
import { ImportDialog } from "../components/ImportDialog";
import { SkillFormSheet } from "../components/skills/SkillFormSheet";
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
import { parseSkill, type SkillForm } from "../lib/canonical";
import { formatRelativeTime } from "../lib/format";
import { supported, type AssetSummary } from "../types";

type Editing =
  | { mode: "create"; id: string; form: SkillForm }
  | { mode: "edit"; id: string; form: SkillForm };

const EMPTY_FORM: SkillForm = { name: "", description: "", body: "", extraFrontmatter: "" };

export function SkillsPanel() {
  const { items, error, loading, refresh } = useAssets("skill");
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

  const agentsForType = enabledAgents.filter((agent) => supported(agent, "skill"));
  const refreshAll = () => {
    void refresh();
    void refreshAgents();
  };

  const normalized = query.trim().toLowerCase();
  const visible = items.filter((item) => {
    if (
      normalized &&
      ![item.name, item.id, item.description ?? "", item.source ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    ) {
      return false;
    }
    if (filter === "all") return true;
    if (filter === "off") {
      return !agentsForType.some((agent) => agent.enabledSkills.includes(item.id));
    }
    const agentId = filter.slice("on:".length);
    return agents
      .find((agent) => agent.id === agentId)
      ?.enabledSkills.includes(item.id) === true;
  });

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

  const deleteSkill = async () => {
    if (!deleting) return;
    setPendingDelete(true);
    setActionError(null);
    try {
      // 先撤下所有已部署的 agent，避免删除真源后留下断链或残留配置。
      for (const agent of agents) {
        if (agent.enabledSkills.includes(deleting.id)) {
          await api.setSkillEnabled(deleting.id, agent.id, false);
        }
      }
      await api.deleteAsset("skill", deleting.id);
      setDeleting(null);
      refreshAll();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setPendingDelete(false);
    }
  };

  const deployedTo = deleting
    ? agents.filter((agent) => agent.enabledSkills.includes(deleting.id))
    : [];

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <PageHeader
        icon={Sparkles}
        title="Skills"
        actions={
          <>
            <Menu
              label="添加 Skill"
              trigger={
                <>
                  <Plus size={14} aria-hidden />
                  添加 Skill
                </>
              }
              buttonClassName="h-8 w-auto whitespace-nowrap bg-action px-3 text-body font-medium text-white hover:bg-action-hover hover:text-white"
              items={[
                { label: "新建 Skill", onSelect: create },
                { label: "从文件夹导入", onSelect: () => void importDirectory() },
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

      <div className="flex h-12 shrink-0 items-center gap-3 px-6">
        <SearchField
          className="w-[320px]"
          value={query}
          onValueChange={setQuery}
          placeholder="搜索名称、描述或来源…"
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
            description="可以从本地目录导入，或新建一个 SKILL.md。"
            actions={
              <>
                <Button variant="primary" onClick={create}>
                  新建 Skill
                </Button>
                <Button onClick={() => void importDirectory()}>
                  <FolderInput size={14} aria-hidden />
                  从文件夹导入
                </Button>
                <Button onClick={() => setImporting(true)}>从 cc-switch 导入</Button>
              </>
            }
          />
        ) : visible.length === 0 ? (
          <EmptyState icon={Sparkles} title="没有匹配的 Skill" description="换个关键词或筛选条件。" />
        ) : (
          <div className="overflow-hidden rounded-panel border border-border bg-surface">
            <div className="flex h-11 items-center bg-subtle pe-3 ps-4">
              <span className="flex-1 text-caption font-semibold text-fg-2">名称</span>
              <AgentColumnsHeader assetType="skill" agents={agentsForType} items={visible} />
              <span className="w-12" />
            </div>
            <ul>
              {visible.map((item) => {
                const detail = [
                  item.description,
                  item.mtime ? `更新于 ${formatRelativeTime(item.mtime)}` : null,
                ]
                  .filter(Boolean)
                  .join(" · ");
                return (
                  <AssetRow
                    key={item.id}
                    title={item.name}
                    subtitle={item.id}
                    description={detail || undefined}
                    badges={
                      <Badge mono className="max-w-[180px]">
                        {item.source ?? "本地"}
                      </Badge>
                    }
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
                        items={[
                          { label: "编辑", onSelect: () => void openEdit(item) },
                          {
                            label: "删除",
                            danger: true,
                            onSelect: () => setDeleting(item),
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
        confirmLabel="删除 Skill"
        body={
          <div className="space-y-2">
            <p>
              会删除真源目录及其辅助文件，不可撤销。
              {deployedTo.length > 0 &&
                ` 删除前会先从 ${deployedTo.map((agent) => agent.displayName).join("、")} 撤下。`}
            </p>
          </div>
        }
        onConfirm={() => void deleteSkill()}
        onCancel={() => setDeleting(null)}
      />
    </div>
  );
}