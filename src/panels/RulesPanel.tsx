import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import { DriftBanner } from "../components/DriftBanner";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useDrift } from "../hooks/useDrift";
import { api } from "../lib/api";
import { fromList, parseRule, renderRule, toList, type RuleForm } from "../lib/canonical";
import type { ScanReport } from "../types";

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
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<RuleForm>(EMPTY);
  const [newId, setNewId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [scan, setScan] = useState<ScanReport | null>(null);

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

  const scanRules = async () => {
    for (const agent of agents) {
      try {
        const report = await api.scanRules(agent.id);
        setScan(report);
      } catch (e) {
        setScan(null);
        setFormError(`${agent.displayName}：${e}`);
      }
    }
    await refresh();
  };

  return (
    <div className="grid h-full grid-cols-[240px_1fr] gap-6 overflow-auto px-6 py-5">
      <section>
        <h2 className="mb-3 text-xl font-semibold">Rules</h2>
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
        {items.length === 0 && (
          <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">
            真源里还没有规则。可以在下面新建，或从某个 agent 的规则目录扫描进来。
          </p>
        )}
        <div className="mt-4 flex gap-2">
          <input
            aria-label="新规则 ID"
            className="min-w-0 flex-1 rounded border border-slate-300 bg-white px-2 py-1 text-sm text-slate-900 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
            placeholder="新规则 ID"
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
          onClick={() => void scanRules()}
        >
          从 agent 目录扫描导入
        </button>
        {scan && (
          <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
            新增 {scan.imported}、更新 {scan.updated}、未变 {scan.unchanged}
            {scan.skipped.length > 0 && `、跳过 ${scan.skipped.length}（${scan.skipped.map((s) => s.file).join("、")}）`}
          </p>
        )}
      </section>
      <section>
        <DriftBanner
          drifts={selected ? drift.forAsset("rule", selected) : []}
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
            {drift.error && <p className="mb-2 text-sm text-red-600 dark:text-red-400">{drift.error}</p>}
            <div className="space-y-2 text-sm">
              <label className="block">
                名称
                <input
                  aria-label="规则名称"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                />
              </label>
              <label className="block">
                说明
                <input
                  aria-label="规则说明"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.description}
                  onChange={(e) => setForm({ ...form, description: e.target.value })}
                />
              </label>
              <label className="block">
                生效范围（逗号分隔的 glob）
                <input
                  aria-label="规则生效范围"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={fromList(form.paths)}
                  onChange={(e) => setForm({ ...form, paths: toList(e.target.value) })}
                />
              </label>
              <label className="block">
                trigger
                <select
                  aria-label="规则 trigger"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.trigger}
                  onChange={(e) => setForm({ ...form, trigger: e.target.value })}
                >
                  {TRIGGERS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block">
                正文（markdown）
                <textarea
                  aria-label="规则正文"
                  rows={10}
                  className="mt-1 w-full rounded border border-slate-300 bg-white p-2 font-mono text-xs dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.body}
                  onChange={(e) => setForm({ ...form, body: e.target.value })}
                />
              </label>
            </div>
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
            <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
              落盘时按各 agent 的形状渲染（Qoder 用 trigger + 单行 glob），回推只换正文，
              真源自己的生效范围不会被工具侧的派生字段反向覆盖。
            </p>
          </>
        ) : (
          <p className="text-sm text-slate-500 dark:text-slate-400">选择左侧规则进行编辑。</p>
        )}
      </section>
    </div>
  );
}
