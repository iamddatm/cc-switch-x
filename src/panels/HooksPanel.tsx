import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import { HooksSafety } from "../components/HooksSafety";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";
import { parseHook, renderHook, type HookForm } from "../lib/canonical";
import { HOOK_EVENTS, switchable, type HooksNeedUserAction } from "../types";

const EMPTY: HookForm = {
  name: "",
  event: "PreToolUse",
  matcher: "",
  command: "",
  commandWindows: "",
  timeout: "",
};

export function HooksPanel() {
  const { items, error, refresh } = useAssets("hook");
  const { agents, refresh: refreshAgents } = useAgents();
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<HookForm>(EMPTY);
  const [newId, setNewId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [foreign, setForeign] = useState<Record<string, number>>({});
  const [gate, setGate] = useState<HooksNeedUserAction | null>(null);

  useEffect(() => {
    if (!selected || !items.some((item) => item.id === selected)) return;
    void api
      .readAsset("hook", selected)
      .then((text) => {
        setForm(parseHook(text));
        setFormError(null);
      })
      .catch((e) => setFormError(String(e)));
  }, [selected, items]);

  // 每个可写的 agent 都要问一次「有多少条不是我们写的」与功能开关状态
  useEffect(() => {
    void (async () => {
      const counts: Record<string, number> = {};
      for (const agent of agents) {
        if (!switchable(agent, "hook")) continue;
        try {
          counts[agent.id] = await api.foreignHookCount(agent.id);
        } catch {
          // 问不到就不显示那条说明；不能因为一次读取失败把面板写坏
        }
      }
      setForeign(counts);
    })();
    void (async () => {
      for (const agent of agents) {
        const status = await api.hooksGateStatus(agent.id).catch(() => null);
        if (status) {
          setGate(status);
          return;
        }
      }
      setGate(null);
    })();
  }, [agents]);

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
      await api.writeAsset("hook", selected, renderHook(form));
      setFormError(null);
      await refresh();
    } catch (e) {
      setFormError(String(e));
    }
  };

  const remove = async () => {
    if (!selected || !window.confirm(`删除真源里的 hook ${selected}？`)) return;
    await api.deleteAsset("hook", selected);
    setSelected(null);
    await refresh();
  };

  return (
    <div className="grid grid-cols-[240px_1fr] gap-6">
      <section>
        <h2 className="mb-3 text-xl font-semibold">Hooks</h2>
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
            aria-label="新 hook ID"
            className="min-w-0 flex-1 rounded border border-slate-300 bg-white px-2 py-1 text-sm text-slate-900 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
            placeholder="新 hook ID"
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
      </section>
      <section>
        <HooksSafety
          foreignCount={Object.values(foreign).reduce((sum, count) => sum + count, 0)}
          gate={gate}
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
                  aria-label="hook 名称"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                />
              </label>
              <label className="block">
                事件
                <select
                  aria-label="hook 事件"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.event}
                  onChange={(e) => setForm({ ...form, event: e.target.value })}
                >
                  {/* 白名单之外的事件名后端会直接拒，下拉里就不给选 */}
                  {HOOK_EVENTS.map((event) => (
                    <option key={event} value={event}>
                      {event}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block">
                matcher（可留空）
                <input
                  aria-label="hook matcher"
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.matcher}
                  onChange={(e) => setForm({ ...form, matcher: e.target.value })}
                />
              </label>
              <label className="block">
                命令
                <textarea
                  aria-label="hook 命令"
                  rows={3}
                  className="mt-1 w-full whitespace-pre rounded border border-slate-300 bg-white p-2 font-mono text-xs dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.command}
                  onChange={(e) => setForm({ ...form, command: e.target.value })}
                />
              </label>
              <label className="block">
                Windows 上的替代命令（可留空）
                <textarea
                  aria-label="hook Windows 命令"
                  rows={2}
                  className="mt-1 w-full whitespace-pre rounded border border-slate-300 bg-white p-2 font-mono text-xs dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.commandWindows}
                  onChange={(e) => setForm({ ...form, commandWindows: e.target.value })}
                />
              </label>
              <label className="block">
                超时（秒，可留空）
                <input
                  aria-label="hook 超时秒数"
                  type="number"
                  min={1}
                  className="mt-1 w-full rounded border border-slate-300 bg-white px-2 py-1 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
                  value={form.timeout}
                  onChange={(e) => setForm({ ...form, timeout: e.target.value })}
                />
              </label>
            </div>
            {/* spec 4.2：hook 是可执行内容，必须完整展示，不许省略号截断 */}
            <div className="mt-3">
              <p className="text-xs text-slate-500 dark:text-slate-400">将要执行的完整命令</p>
              <pre
                data-testid="hook-command"
                className="mt-1 overflow-x-auto whitespace-pre-wrap break-all rounded border border-slate-300 bg-slate-50 p-2 font-mono text-xs dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
              >
                {form.commandWindows ? `${form.command}\n[Windows] ${form.commandWindows}` : form.command}
              </pre>
            </div>
            <AgentMatrix
              assetType="hook"
              id={selected}
              agents={agents}
              onChanged={() => {
                void refresh();
                void refreshAgents();
              }}
            />
            <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
              写入只在 hooks 子树里增删带归属标记的那一条；同文件里的其他键与你自己的
              hook 一律不动。
            </p>
          </>
        ) : (
          <p className="text-sm text-slate-500 dark:text-slate-400">选择左侧 hook 进行编辑。</p>
        )}
      </section>
    </div>
  );
}
