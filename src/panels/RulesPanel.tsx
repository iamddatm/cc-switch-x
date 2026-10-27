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
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useDrift } from "../hooks/useDrift";
import { api } from "../lib/api";
import { fromList, parseRule, renderRule, toList, type RuleForm } from "../lib/canonical";

const TRIGGERS = [
  { value: "", label: "按 paths 派生" },
  { value: "always_on", label: "always_on" },
  { value: "glob", label: "glob" },
  { value: "manual", label: "manual" },
  { value: "model_decision", label: "model_decision" },
];

const EMPTY: RuleForm = { name: "", description: "", paths: [], trigger: "", body: "" };

export function RulesPanel() {
  const { items, error, refresh } = useAssets("rule");
  const { agents, refresh: refreshAgents } = useAgents();
  const drift = useDrift();
  const scan = useSourceScan("rule", agents, (agentId) => api.scanRules(agentId), refresh);
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<RuleForm>(EMPTY);
  const [newId, setNewId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected || !items.some((item) => item.id === selected)) return;
    void api
      .readAsset("rule", selected)
      .then((text) => {
        setForm(parseRule(text));
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
      await api.writeAsset("rule", selected, renderRule(form));
      setFormError(null);
      await refresh();
    } catch (e) {
      setFormError(String(e));
    }
  };

  const remove = async () => {
    if (!selected || !window.confirm(`删除真源里的规则 ${selected}？已落盘的文件要先在下面取消勾选。`))
      return;
    await api.deleteAsset("rule", selected);
    setSelected(null);
    await refresh();
  };

  return (
    <AssetEditorLayout>
      <AssetSourceList
        title="Rules"
        items={items}
        selected={selected}
        onSelect={setSelected}
        listError={error}
        emptyHint="真源里还没有规则。可以在下面新建，或从某个 agent 的规则目录扫描进来。"
        newIdLabel="新规则 ID"
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
        notSelectedHint="选择左侧规则进行编辑。"
        header={
          <>
            <DriftBanner
              drifts={selected ? drift.forAsset("rule", selected) : []}
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
              aria-label="规则名称"
              className="mt-1"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </label>
          <label className="block">
            说明
            <Input
              aria-label="规则说明"
              className="mt-1"
              value={form.description}
              onChange={(e) => setForm({ ...form, description: e.target.value })}
            />
          </label>
          <label className="block">
            生效范围（逗号分隔的 glob）
            <Input
              aria-label="规则生效范围"
              className="mt-1"
              value={fromList(form.paths)}
              onChange={(e) => setForm({ ...form, paths: toList(e.target.value) })}
            />
          </label>
          <label className="block">
            trigger
            <Select
              aria-label="规则 trigger"
              className="mt-1"
              value={form.trigger}
              onChange={(e) => setForm({ ...form, trigger: e.target.value })}
            >
              {TRIGGERS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </Select>
          </label>
          <label className="block">
            正文（markdown）
            <Textarea
              aria-label="规则正文"
              rows={10}
              className="mt-1 font-mono text-caption"
              value={form.body}
              onChange={(e) => setForm({ ...form, body: e.target.value })}
            />
          </label>
        </div>
        {selected && (
          <AgentMatrix
            assetType="rule"
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
          落盘时按各 agent 的形状渲染（Qoder 用 trigger + 单行 glob），回推只换正文，
          真源自己的生效范围不会被工具侧的派生字段反向覆盖。
        </p>
      </AssetEditorPane>
    </AssetEditorLayout>
  );
}
