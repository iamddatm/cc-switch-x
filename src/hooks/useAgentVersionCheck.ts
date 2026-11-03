import { useCallback, useSyncExternalStore } from "react";
import { api } from "../lib/api";
import type { AgentVersionReport } from "../types";

interface AgentVersionCheckState {
  report: AgentVersionReport | null;
  checking: boolean;
  error: string | null;
}

const EMPTY: AgentVersionCheckState = { report: null, checking: false, error: null };

let state = EMPTY;
let inFlight: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(patch: Partial<AgentVersionCheckState>) {
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
 * Agent CLI 版本检查。结果与请求都放模块级：离开 Agents 页面板会卸载，
 * 但一轮探测的账要留着，回来还能看到，不必为了看一眼版本重跑一遍外部命令。
 */
export function useAgentVersionCheck() {
  const snapshot = useSyncExternalStore(
    subscribe,
    () => state,
    () => state,
  );

  const check = useCallback(async () => {
    if (inFlight) return inFlight;
    emit({ checking: true, error: null });
    inFlight = (async () => {
      try {
        const report = await api.checkAgentVersions();
        emit({ report, checking: false });
      } catch (e) {
        emit({ checking: false, error: String(e) });
      } finally {
        inFlight = null;
      }
    })();
    return inFlight;
  }, []);

  return { ...snapshot, check };
}

/** 测试用：清空模块级缓存，避免用例之间互相影响。 */
export function resetAgentVersionCheck() {
  inFlight = null;
  state = EMPTY;
  listeners.forEach((listener) => listener());
}
