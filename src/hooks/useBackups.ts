import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import type {
  BackupSettings,
  BackupSnapshot,
  BackupUsage,
  CloudSettingsView,
  CloudStatus,
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
  const [cloud, setCloud] = useState<CloudSettingsView | null>(null);
  const [cloudStatus, setCloudStatus] = useState<CloudStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const commitKey = useRef("");

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const refreshCloudStatus = useCallback(async () => {
    try {
      const status = await api.getCloudStatus();
      if (alive.current) setCloudStatus(status);
    } catch (reason) {
      if (alive.current) {
        setCloudStatus(null);
        setError(String(reason));
      }
    }
  }, []);

  /** 提交历史只在 HEAD 提交时间变化时重拉，避免轮询反复跑 git log。 */
  const syncCommits = useCallback(async (status: GitStatus) => {
    const key = status.initialized ? String(status.lastCommitAt ?? "") : "";
    if (key === commitKey.current) return;
    if (!status.initialized) {
      commitKey.current = "";
      setCommits([]);
      return;
    }
    const history = await api.gitListCommits(50);
    if (!alive.current) return;
    commitKey.current = key;
    setCommits(history);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const [snapshotList, usageView, backupSettings, status, cloudSettings] =
        await Promise.all([
          api.listBackupSnapshots(),
          api.getBackupUsage(),
          api.getBackupSettings(),
          api.getGitStatus(),
          api.getCloudSettings(),
        ]);
      if (!alive.current) return;
      setSnapshots(snapshotList);
      setUsage(usageView);
      setSettings(backupSettings);
      setGit(status);
      setCloud(cloudSettings);
      await syncCommits(status);
      setError(null);
      void refreshCloudStatus();
    } catch (reason) {
      if (alive.current) setError(String(reason));
    } finally {
      if (alive.current) setLoading(false);
    }
  }, [refreshCloudStatus, syncCommits]);

  const refreshGit = useCallback(async () => {
    try {
      const status = await api.getGitStatus();
      if (!alive.current) return;
      setGit(status);
      await syncCommits(status);
    } catch (reason) {
      if (alive.current) setError(String(reason));
    }
  }, [syncCommits]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void refreshGit();
    }, 15000);
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
    cloud,
    cloudStatus,
    loading,
    busy,
    error,
    refresh,
    refreshCloudStatus,
    run,
  };
}
