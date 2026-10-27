import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import {
  AssetEditorLayout,
  AssetEditorPane,
  AssetSourceList,
} from "../components/assets/AssetEditor";
import { useFileAssetEditor } from "../components/assets/useFileAssetEditor";
import { HooksSafety } from "../components/HooksSafety";
import { Input, Textarea } from "../components/ui/Input";
import { Select } from "../components/ui/Select";
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
  const {
    selected,
    setSelected,
    form,
    setForm,
    newId,
    setNewId,
    formError,
    create,
    save,
    remove,
  } = useFileAssetEditor<HookForm>({
    assetType: "hook",
    items,
    refresh,
    empty: EMPTY,
    parse: parseHook,
    render: renderHook,
    removeConfirm: (id) => `删除真源里的 hook ${id}？`,
  });
  const [foreign, setForeign] = useState<{ agent: string; count: number }[]>([]);
  const [gate, setGate] = useState<HooksNeedUserAction | null>(null);

  // 每个可写的 agent 都要问一次「有多少条不是我们写的」与功能开关状态
  useEffect(() => {
    void (async () => {
      const entries: { agent: string; count: number }[] = [];
      for (const item of agents) {
        if (!switchable(item, "hook")) continue;
        try {
          entries.push({ agent: item.displayName, count: await api.foreignHookCount(item.id) });
        } catch {
          // 问不到就不显示那条说明；不能因为一次读取失败把面板写坏
        }
      }
      setForeign(entries);
    })();
    void (async () => {
      for (const item of agents) {
        const status = await api.hooksGateStatus(item.id).catch(() => null);
        if (status) {
          setGate(status);
          return;
        }
      }
      setGate(null);
    })();
  }, [agents]);

  return (
    <AssetEditorLayout>
      <AssetSourceList
        title="Hooks"
        items={items}
        selected={selected}
        onSelect={setSelected}
        listError={error}
        newIdLabel="新 hook ID"
        newIdValue={newId}
        onNewIdChange={setNewId}
        onCreate={create}
      />
      <AssetEditorPane
        selected={selected}
        formError={formError}
        onSave={() => void save()}
        onDelete={() => void remove()}
        notSelectedHint="选择左侧 hook 进行编辑。"
        header={<HooksSafety counts={foreign} gate={gate} />}
      >
        <div className="space-y-2 text-body">
          <label className="block">
            名称
            <Input
              aria-label="hook 名称"
              className="mt-1"
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
          </label>
          <label className="block">
            事件
            <Select
              aria-label="hook 事件"
              className="mt-1"
              value={form.event}
              onChange={(e) => setForm({ ...form, event: e.target.value })}
            >
              {/* 白名单之外的事件名后端会直接拒，下拉里就不给选 */}
              {HOOK_EVENTS.map((event) => (
                <option key={event} value={event}>
                  {event}
                </option>
              ))}
            </Select>
          </label>
          <label className="block">
            matcher（可留空）
            <Input
              aria-label="hook matcher"
              className="mt-1"
              value={form.matcher}
              onChange={(e) => setForm({ ...form, matcher: e.target.value })}
            />
          </label>
          <label className="block">
            命令
            <Textarea
              aria-label="hook 命令"
              rows={3}
              className="mt-1 whitespace-pre font-mono text-caption"
              value={form.command}
              onChange={(e) => setForm({ ...form, command: e.target.value })}
            />
          </label>
          <label className="block">
            Windows 上的替代命令（可留空）
            <Textarea
              aria-label="hook Windows 命令"
              rows={2}
              className="mt-1 whitespace-pre font-mono text-caption"
              value={form.commandWindows}
              onChange={(e) => setForm({ ...form, commandWindows: e.target.value })}
            />
          </label>
          <label className="block">
            超时（秒，可留空）
            <Input
              aria-label="hook 超时秒数"
              type="number"
              min={1}
              className="mt-1"
              value={form.timeout}
              onChange={(e) => setForm({ ...form, timeout: e.target.value })}
            />
          </label>
        </div>
        {/* spec 4.2：hook 是可执行内容，必须完整展示，不许省略号截断 */}
        <div className="mt-3">
          <p className="text-caption text-fg-3">将要执行的完整命令</p>
          <pre
            data-testid="hook-command"
            className="mt-1 overflow-x-auto whitespace-pre-wrap break-all rounded-control border border-border bg-subtle p-2 font-mono text-caption text-fg-1"
          >
            {form.commandWindows ? `${form.command}\n[Windows] ${form.commandWindows}` : form.command}
          </pre>
        </div>
        {selected && (
          <AgentMatrix
            assetType="hook"
            id={selected}
            agents={agents}
            onChanged={() => {
              void refresh();
              void refreshAgents();
            }}
          />
        )}
        <p className="mt-2 text-caption text-fg-3">
          写入只在 hooks 子树里增删带归属标记的那一条；同文件里的其他键与你自己的
          hook 一律不动。
        </p>
      </AssetEditorPane>
    </AssetEditorLayout>
  );
}
