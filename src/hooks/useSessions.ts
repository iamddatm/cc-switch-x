import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type {
  SessionProviderStatus,
  SessionScanReport,
  SessionSearchHit,
  SessionSummary,
} from "../types";

/** 一次加载足够覆盖本机会话的摘要；按项目分组需要完整数据，不能只取一页。 */
const ALL_LIMIT = 2000;
const SEARCH_LIMIT = 500;

export interface SessionsState {
  sessions: SessionSummary[];
  providers: SessionProviderStatus[];
  hits: SessionSearchHit[] | null;
  loading: boolean;
  scanning: boolean;
  scanReport: SessionScanReport | null;
  error: string | null;
  reload: () => Promise<void>;
  scan: () => Promise<void>;
  search: (query: string) => Promise<void>;
  clearSearch: () => void;
}

/**
 * 会话面板的数据源。挂载时只读现有索引，绝不自动扫描：
 * 本机 Claude 主会话可能几百 MB，首次扫描是分钟级操作。
 */
export function useSessions(): SessionsState {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [providers, setProviders] = useState<SessionProviderStatus[]>([]);
  const [hits, setHits] = useState<SessionSearchHit[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [scanReport, setScanReport] = useState<SessionScanReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextSessions, nextProviders] = await Promise.all([
        api.listSessions(undefined, undefined, 0, ALL_LIMIT),
        api.sessionProviders(),
      ]);
      setSessions(nextSessions);
      setProviders(nextProviders);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  const scan = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const report = await api.scanSessions();
      setScanReport(report);
      const [nextSessions, nextProviders] = await Promise.all([
        api.listSessions(undefined, undefined, 0, ALL_LIMIT),
        api.sessionProviders(),
      ]);
      setSessions(nextSessions);
      setProviders(nextProviders);
      setHits(null);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setScanning(false);
    }
  }, []);

  const search = useCallback(async (raw: string) => {
    const query = raw.trim();
    if (!query) {
      setHits(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setHits(await api.searchSessions(query, SEARCH_LIMIT));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  const clearSearch = useCallback(() => setHits(null), []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return {
    sessions,
    providers,
    hits,
    loading,
    scanning,
    scanReport,
    error,
    reload,
    scan,
    search,
    clearSearch,
  };
}