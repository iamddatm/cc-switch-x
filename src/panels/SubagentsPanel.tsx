import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import { DriftBanner } from "../components/DriftBanner";
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
        <h2 className="mb-3 text-xl font-semibold">Subagents</h2>
        {error && <p className="text-sm text-red-600 dark:text-red-400">{error}</p>}
        <ul className="space-y-1">
          {items.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className="w-full rounded px-2 py-1 text-left hover:bg-slate-100 dark:hover:bg-slate-700"
                onClick={() => setSelected(item.id)}
              >
                {item.name}
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-4 flex gap-2">
          <input
            aria-label="新子代理 ID"
            className="min-w-0 flex-1 rounded border border-slate-300 bg-white px-2 py-1 text-sm text-slate-900 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
            placeholder="新子代理 ID"
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
          />
          <button
            type="button"
            className="rounded border border-slate-300 px-2 py-1 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
            onClick={create}
          >
            新建
          </button>
        </div>
        <button
          type="button"
          className="mt-2 w-full rounded border border-slate-300 px-2 py-1 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
          onClick={() => void scanSubagents()}
        >
          从 agent 目录扫描导入
        </button>
        {scan && (
          <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
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
              <button
                type="button"
                className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white dark:bg-slate-600"
                onClick={() => void save()}
              >
                保存
              </button>
              <button
                type="button"
                className="rounded border border-red-300 px-3 py-1.5 text-sm text-red-700 dark:border-red-800 dark:text-red-300"
                onClick={() => void remove()}
              >
                删除
              </button>
            </div>
            {formError && <p className="mb-2 text-sm text-red-600 dark:text-red-400">{formError}</p>}
            <div className="space-y-2 text-sm">
              <label className="block">
                名称
                <input
                  aria-label="子代理名称"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                />
              </label>
              <label className="block">
                一句话描述
                <input
                  aria-label="子代理描述"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.description}
                  onChange={(e) => setForm({ ...form, description: e.target.value })}
                />
              </label>
              <label className="block">
                模型（留空用各工具默认）
                <input
                  aria-label="子代理模型"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.model}
                  onChange={(e) => setForm({ ...form, model: e.target.value })}
                />
              </label>
              <label className="block">
                工具白名单（逗号分隔）
                <input
                  aria-label="子代理工具白名单"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={fromList(form.tools)}
                  onChange={(e) => setForm({ ...form, tools: toList(e.target.value) })}
                />
              </label>
              <label className="block">
                指令正文
                <textarea
                  aria-label="子代理指令"
                  rows={10}
                  className="mt-1 w-full rounded border border-slate-300 bg-white p-2 font-mono text-xs dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
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
            <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
              Codex 落盘是 .toml（不写 tools，多行指令走 &apos;&apos;&apos; 字面量）；
              claude / Qoder 是带 frontmatter 的 .md。工具的私有字段留在 tool_extras，
              换个 agent 渲染时不会带出去。
            </p>
          </>
        ) : (
          <p className="text-sm text-slate-500 dark:text-slate-400">选择左侧子代理进行编辑。</p>
        )}
      </section>
    </div>
  );
}
