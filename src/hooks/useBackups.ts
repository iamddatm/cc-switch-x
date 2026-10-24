import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import type {
  BackupSettings,
  BackupSnapshot,
  BackupUsage,
  GitCommit,
  GitStatus,
} from "../types";

/** 备份面板的数据源：一次拉全量，Git 状态单独轮询，避免频繁重读快照列表。 */
export function useBackups() {
  const [snapshots, setSnapshots] = useState<BackupSnapshot[]>([]);
  const [usage, setUsage] = useState<BackupUsage | null>(null);
  const [settings, setSettings] = useState<BackupSettings | null>(null);
  const [git, setGit] = useState<GitStatus | null>(null);
  const [commits, setCommits] = useState<GitCommit[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const [snapshotList, usageView, backupSettings, status] = await Promise.all([
        api.listBackupSnapshots(),
        api.getBackupUsage(),
        api.getBackupSettings(),
        api.getGitStatus(),
      ]);
      if (!alive.current) return;
      setSnapshots(snapshotList);
      setUsage(usageView);
      setSettings(backupSettings);
      setGit(status);
      if (status.initialized) {
        const history = await api.gitListCommits(50);
        if (alive.current) setCommits(history);
      } else {
        setCommits([]);
      }
      setError(null);
    } catch (reason) {
      if (alive.current) setError(String(reason));
    } finally {
      if (alive.current) setLoading(false);
    }
  }, []);

  const refreshGit = useCallback(async () => {
    try {
      const status = await api.getGitStatus();
      if (!alive.current) return;
      setGit(status);
      if (status.initialized) {
        const history = await api.gitListCommits(50);
        if (alive.current) setCommits(history);
      } else {
        setCommits([]);
      }
    } catch (reason) {
      if (alive.current) setError(String(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void refreshGit();
    }, 8000);
    return () => window.clearInterval(timer);
  }, [refreshGit]);

  const run = useCallback(
    async <T,>(key: string, action: () => Promise<T>): Promise<T> => {
      setBusy(key);
      setError(null);
      try {
        const value = await action();
        await refresh();
        return value;
      } catch (reason) {
        if (alive.current) setError(String(reason));
        throw reason;
      } finally {
        if (alive.current) setBusy(null);
      }
    },
    [refresh],
  );

  return {
    snapshots,
    usage,
    settings,
    git,
    commits,
    loading,
    busy,
    error,
    refresh,
    run,
  };
}
