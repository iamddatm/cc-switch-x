import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import { DriftBanner } from "../components/DriftBanner";
import { Button } from "../components/ui/Button";
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
import type { ScanReport } from "../types";

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
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<SubagentForm>(EMPTY);
  const [newId, setNewId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [scan, setScan] = useState<ScanReport | null>(null);

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

  const scanSubagents = async () => {
    for (const agent of agents) {
      try {
        setScan(await api.scanSubagents(agent.id));
      } catch (e) {
        setFormError(`${agent.displayName}：${e}`);
      }
    }
    await refresh();
  };

  return (
    <div className="grid h-full grid-cols-[240px_1fr] gap-6 overflow-auto px-6 py-5">
      <section>
        <h2 className="mb-3 text-page text-fg-1">Subagents</h2>
        {error && <p className="text-caption text-danger-text">{error}</p>}
        <ul className="space-y-0.5">
          {items.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className={`w-full rounded-control px-2.5 py-1.5 text-left text-body transition-colors duration-150 ${
                  selected === item.id
                    ? "bg-selected text-fg-1"
                    : "text-fg-2 hover:bg-subtle hover:text-fg-1"
                }`}
                onClick={() => setSelected(item.id)}
              >
                {item.name}
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-4 flex gap-2">
          <Input
            aria-label="新子代理 ID"
            className="min-w-0 flex-1"
            placeholder="新子代理 ID"
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
          />
          <Button variant="secondary" onClick={create}>
            新建
          </Button>
        </div>
        <Button
          variant="secondary"
          className="mt-2 w-full"
          onClick={() => void scanSubagents()}
        >
          从 agent 目录扫描导入
        </Button>
        {scan && (
          <p className="mt-2 text-caption text-fg-3">
            新增 {scan.imported}、更新 {scan.updated}、未变 {scan.unchanged}
            {scan.skipped.length > 0 && `、跳过 ${scan.skipped.length}`}
          </p>
        )}
      </section>
      <section>
        <DriftBanner
          drifts={selected ? drift.forAsset("subagent", selected) : []}
          busy={drift.busy}
          onResolve={drift.resolve}
        />
        {selected ? (
          <>
            <div className="mb-3 flex gap-2">
              <Button variant="primary" onClick={() => void save()}>
                保存
              </Button>
              <Button variant="danger" onClick={() => void remove()}>
                删除
              </Button>
            </div>
            {formError && <p className="mb-2 text-caption text-danger-text">{formError}</p>}
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
            <p className="mt-2 text-caption text-fg-3">
              Codex 落盘是 .toml（不写 tools，多行指令走 &apos;&apos;&apos; 字面量）；
              claude / Qoder 是带 frontmatter 的 .md。工具的私有字段留在 tool_extras，
              换个 agent 渲染时不会带出去。
            </p>
          </>
        ) : (
          <p className="text-body text-fg-3">选择左侧子代理进行编辑。</p>
        )}
      </section>
    </div>
  );
}
