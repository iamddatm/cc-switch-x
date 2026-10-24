import { useCallback, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type { UpdateDownloadProgress, UpdateInfo } from "../types";

const DISMISSED_VERSION_KEY = "cc-switch-x:update:dismissedVersion";
const AUTO_CHECK_DELAY_MS = 1000;

export interface UpdateController {
  currentVersion: string | null;
  available: UpdateInfo | null;
  checking: boolean;
  installing: boolean;
  installStarted: boolean;
  progress: UpdateDownloadProgress | null;
  error: string | null;
  dismissed: boolean;
  hasUpdate: boolean;
  check: (options?: { silent?: boolean }) => Promise<void>;
  install: () => Promise<void>;
  dismiss: () => void;
  resetDismiss: () => void;
}

function readDismissedVersion(): string | null {
  try {
    return localStorage.getItem(DISMISSED_VERSION_KEY);
  } catch {
    return null;
  }
}

function hasDismissed(version: string): boolean {
  return readDismissedVersion() === version;
}

export function useUpdate(): UpdateController {
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [available, setAvailable] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [installStarted, setInstallStarted] = useState(false);
  const [progress, setProgress] = useState<UpdateDownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const checkInFlightRef = useRef(false);
  const installInFlightRef = useRef(false);

  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    void getVersion()
      .then((version) => {
        if (!cancelled) setCurrentVersion(version);
      })
      .catch(() => {
        // 版本号只用于展示，读不到就显示未知，不打扰用户。
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const check = useCallback(async (options?: { silent?: boolean }) => {
    if (!isTauri() || checkInFlightRef.current) return;
    checkInFlightRef.current = true;
    setChecking(true);
    if (!options?.silent) setError(null);

    try {
      const result = await api.checkAppUpdate();
      setAvailable(result);
      if (!result) {
        setProgress(null);
        setInstallStarted(false);
      }
      if (!options?.silent) setError(null);
    } catch (e) {
      if (options?.silent) {
        console.warn("自动检查更新失败：", e);
      } else {
        setError(String(e));
      }
    } finally {
      checkInFlightRef.current = false;
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    const timer = window.setTimeout(() => {
      void check({ silent: true });
    }, AUTO_CHECK_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [check]);

  useEffect(() => {
    setDismissed(available !== null && hasDismissed(available.version));
  }, [available]);

  const dismiss = useCallback(() => {
    if (!available) return;
    try {
      localStorage.setItem(DISMISSED_VERSION_KEY, available.version);
    } catch {
      // 存不下就只在本次会话里隐藏。
    }
    setDismissed(true);
  }, [available]);

  const resetDismiss = useCallback(() => {
    try {
      localStorage.removeItem(DISMISSED_VERSION_KEY);
    } catch {
      // 清理失败不影响本次会话恢复提示。
    }
    setDismissed(false);
  }, []);

  const install = useCallback(async () => {
    if (!isTauri() || !available || installInFlightRef.current) return;
    installInFlightRef.current = true;
    setInstalling(true);
    setInstallStarted(false);
    setProgress(null);
    setError(null);

    let unlistenProgress: UnlistenFn | undefined;
    let unlistenStarted: UnlistenFn | undefined;
    try {
      [unlistenProgress, unlistenStarted] = await Promise.all([
        listen<UpdateDownloadProgress>("update-download-progress", (event) => {
          setProgress(event.payload);
        }),
        listen<string>("update-install-started", () => {
          setInstallStarted(true);
          setProgress(null);
        }),
      ]);

      const installed = await api.installAppUpdate();
      if (!installed) {
        setAvailable(null);
        setInstalling(false);
        installInFlightRef.current = false;
      }
      // installed=true 时应用即将重启（Windows 上进程会直接退出），保持安装态。
    } catch (e) {
      setError(String(e));
      setInstalling(false);
      installInFlightRef.current = false;
    } finally {
      unlistenProgress?.();
      unlistenStarted?.();
    }
  }, [available]);

  return {
    currentVersion,
    available,
    checking,
    installing,
    installStarted,
    progress,
    error,
    dismissed,
    hasUpdate: available !== null && !dismissed,
    check,
    install,
    dismiss,
    resetDismiss,
  };
}
