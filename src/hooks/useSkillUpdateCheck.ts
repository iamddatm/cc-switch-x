import { useCallback, useSyncExternalStore } from "react";
import { api } from "../lib/api";
import type { SkillCheckReport } from "../types";

interface SkillUpdateCheckState {
  report: SkillCheckReport | null;
  checking: boolean;
  error: string | null;
  /** 刚结束的一轮检查发现了更新：面板挂载时弹一次「全部更新」确认。 */
  prompt: boolean;
}

const EMPTY: SkillUpdateCheckState = {
  report: null,
  checking: false,
  error: null,
  prompt: false,
};

let state = EMPTY;
let inFlight: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(patch: Partial<SkillUpdateCheckState>) {
  state = { ...state, ...patch };
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * 检查更新放在模块级缓存里：切到别的页面时面板会卸载，
 * 但请求、进度和结果都保留，回来还能看到（对齐 cc-switch 的查询缓存语义）。
 */
export function useSkillUpdateCheck() {
  const snapshot = useSyncExternalStore(
    subscribe,
    () => state,
    () => state,
  );

  const check = useCallback(async (ids?: string[]) => {
    if (inFlight) return inFlight;
    emit({ checking: true, error: null, prompt: false });
    inFlight = (async () => {
      try {
        const report = await api.checkSkillUpdates(ids);
        emit({ report, checking: false, prompt: report.updates.length > 0 });
      } catch (e) {
        emit({ checking: false, error: String(e) });
      } finally {
        inFlight = null;
      }
    })();
    return inFlight;
  }, []);

  const consumePrompt = useCallback(() => emit({ prompt: false }), []);

  /** 更新或删除后从「可更新」列表里剔除，避免还挂着已经处理过的条目。 */
  const dropUpdates = useCallback((ids: string[]) => {
    if (!state.report || ids.length === 0) return;
    const removed = new Set(ids);
    const updates = state.report.updates.filter((update) => !removed.has(update.id));
    if (updates.length === state.report.updates.length) return;
    emit({ report: { ...state.report, updates } });
  }, []);

  return { ...snapshot, check, consumePrompt, dropUpdates };
}

/** 测试用：清空模块级缓存，避免用例之间互相影响。 */
export function resetSkillUpdateCheck() {
  inFlight = null;
  state = EMPTY;
  listeners.forEach((listener) => listener());
}
