import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import {
  AssetEditorLayout,
  AssetEditorPane,
  AssetSourceList,
} from "../components/assets/AssetEditor";
import { useSourceScan } from "../components/assets/useSourceScan";
import { DriftBanner } from "../components/DriftBanner";
import { Input, Textarea } from "../components/ui/Input";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useDrift } from "../hooks/useDrift";
import { api } from "../lib/api";
import {
  fromList,
  parseSubagent,
  renderSubagent,
  toList,
  type SubagentForm,
} from "../lib/canonical";

const EMPTY: SubagentForm = {
  name: "",
  description: "",
  instructions: "",
  model: "",
  tools: [],
  targets: [],
  toolExtrasRaw: "",
};

export function SubagentsPanel() {
  const { items, error, refresh } = useAssets("subagent");
  const { agents, refresh: refreshAgents } = useAgents();
  const drift = useDrift();
  const scan = useSourceScan(
    "subagent",
    agents,
    (agentId) => api.scanSubagents(agentId),
    refresh,
  );
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<SubagentForm>(EMPTY);
  const [newId, setNewId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected || !items.some((item) => item.id === selected)) return;
    void api
      .readAsset("subagent", selected)
      .then((text) => {
        setForm(parseSubagent(text));
        setFormError(null);
      })
      .catch((e) => setFormError(String(e)));
  }, [selected, items]);

  const create = () => {
    const id = newId.trim();
    if (!id) return;
    setSelected(id);
    setForm({ ...EMPTY, name: id });
    setNewId("");
  };

  const save = async () => {
    if (!selected) return;
    try {
      await api.writeAsset("subagent", selected, renderSubagent(form));
      setFormError(null);
      await refresh();
    } catch (e) {
      setFormError(String(e));
    }
  };

  const remove = async () => {
    if (!selected || !window.confirm(`删除真源里的子代理 ${selected}？`)) return;
    await api.deleteAsset("subagent", selected);
    setSelected(null);
    await refresh();
  };

  return (
    <AssetEditorLayout>
      <AssetSourceList
        title="Subagents"
        items={items}
        selected={selected}
        onSelect={setSelected}
        listError={error}
        newIdLabel="新子代理 ID"
        newIdValue={newId}
        onNewIdChange={setNewId}
        onCreate={create}
        scan={scan}
      />
      <AssetEditorPane
        selected={selected}
        formError={formError}
        onSave={() => void save()}
        onDelete={() => void remove()}
        notSelectedHint="选择左侧子代理进行编辑。"
        header={
          <>
            <DriftBanner
              drifts={selected ? drift.forAsset("subagent", selected) : []}
              busy={drift.busy}
              onResolve={drift.resolve}
              onRescan={() => void drift.rescan()}
            />
            {drift.error && <p className="mb-2 text-caption text-danger-text">{drift.error}</p>}
          </>
        }
      >
        <div className="space-y-2 text-body">
          <label className="block">
            名称
            <Input
              aria-label="子代理名称"
              className="mt-1"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </label>
          <label className="block">
            一句话描述
            <Input
              aria-label="子代理描述"
              className="mt-1"
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
            />
          </label>
          <label className="block">
            模型（留空用各工具默认）
            <Input
              aria-label="子代理模型"
              className="mt-1"
              value={form.model}
              onChange={(e) => setForm({ ...form, model: e.target.value })}
            />
          </label>
          <label className="block">
            工具白名单（逗号分隔）
            <Input
              aria-label="子代理工具白名单"
              className="mt-1"
              value={fromList(form.tools)}
              onChange={(e) => setForm({ ...form, tools: toList(e.target.value) })}
            />
          </label>
          <label className="block">
            指令正文
            <Textarea
              aria-label="子代理指令"
              rows={10}
              className="mt-1 font-mono text-caption"
              value={form.instructions}
              onChange={(e) => setForm({ ...form, instructions: e.target.value })}
            />
          </label>
        </div>
        {selected && (
          <AgentMatrix
            assetType="subagent"
            id={selected}
            agents={agents}
            onChanged={() => {
              void refresh();
              void refreshAgents();
              void drift.refresh();
            }}
          />
        )}
        <p className="mt-2 text-caption text-fg-3">
          Codex 落盘是 .toml（不写 tools，多行指令走 &apos;&apos;&apos; 字面量）；
          claude / Qoder 是带 frontmatter 的 .md。工具的私有字段留在 tool_extras，
          换个 agent 渲染时不会带出去。
        </p>
      </AssetEditorPane>
    </AssetEditorLayout>
  );
}
