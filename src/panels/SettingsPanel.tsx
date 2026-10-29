import { useEffect, useState } from "react";
import { Settings } from "lucide-react";
import { ImportDialog } from "../components/ImportDialog";
import { AboutSection } from "../components/settings/AboutSection";
import { Button } from "../components/ui/Button";
import { Checkbox } from "../components/ui/Checkbox";
import { FeedbackBar } from "../components/ui/FeedbackBar";
import { Input } from "../components/ui/Input";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { Select } from "../components/ui/Select";
import { api } from "../lib/api";
import { useDrift } from "../hooks/useDrift";
import type { UpdateController } from "../hooks/useUpdate";
import type { AppSettings, Theme } from "../types";

export function SettingsPanel({
  theme,
  onThemeChange,
  update,
}: {
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  update: UpdateController;
}) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [storeRoot, setStoreRoot] = useState("");
  const [closeToTray, setCloseToTray] = useState(true);
  const [autoStart, setAutoStart] = useState<boolean | null>(null);
  const [autoStartBusy, setAutoStartBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const drift = useDrift();

  useEffect(() => {
    void api
      .getSettings()
      .then((value) => {
        setSettings(value);
        setStoreRoot(value.storeRoot);
        setCloseToTray(value.closeToTray);
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => {
    let cancelled = false;
    void api
      .getAutostart()
      .then((value) => {
        if (!cancelled) setAutoStart(value);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const toggleCloseToTray = async (enabled: boolean) => {
    setCloseToTray(enabled);
    setError(null);
    try {
      const updated = await api.setCloseToTray(enabled);
      setSettings(updated);
      setCloseToTray(updated.closeToTray);
    } catch (e) {
      setCloseToTray(!enabled);
      setError(String(e));
    }
  };

  const toggleAutoStart = async (enabled: boolean) => {
    setAutoStart(enabled);
    setAutoStartBusy(true);
    setError(null);
    try {
      setAutoStart(await api.setAutostart(enabled));
    } catch (e) {
      setAutoStart(!enabled);
      setError(String(e));
    } finally {
      setAutoStartBusy(false);
    }
  };

  const changeTheme = async (next: Theme) => {
    try {
      const updated = await api.setTheme(next);
      setSettings(updated);
      onThemeChange(updated.theme);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const saveStoreRoot = async () => {
    try {
      await api.setStoreRoot(storeRoot);
      setMessage("存储位置已保存，重启应用后生效。");
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader icon={Settings} title="Settings" />
      {error && (
        <FeedbackBar className="max-w-2xl">
          <Notice tone="danger">{error}</Notice>
        </FeedbackBar>
      )}
      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">
        <div className="max-w-2xl space-y-8">
          <section className="space-y-2">
            <label className="block text-strong text-fg-1" htmlFor="theme">
              主题
            </label>
            <Select
              id="theme"
              value={theme}
              onChange={(e) => void changeTheme(e.target.value as Theme)}
            >
              <option value="system">跟随系统</option>
              <option value="light">浅色</option>
              <option value="dark">深色</option>
            </Select>
            {settings && <p className="text-caption text-fg-3">外观设置只影响本机。</p>}
          </section>

          <section className="space-y-2">
            <h3 className="text-strong text-fg-1">窗口</h3>
            <Checkbox
              id="close-to-tray"
              label="关闭窗口时最小化到托盘"
              checked={closeToTray}
              onChange={(event) => void toggleCloseToTray(event.target.checked)}
            />
            <p className="text-caption text-fg-3">
              关闭后应用继续在托盘运行；右键托盘图标选择「退出 CC Switch X」才会结束进程。
            </p>
          </section>

          <section className="space-y-2">
            <h3 className="text-strong text-fg-1">启动</h3>
            <Checkbox
              id="auto-start"
              label="开机时自动启动"
              checked={autoStart ?? false}
              disabled={autoStartBusy || autoStart === null}
              onChange={(event) => void toggleAutoStart(event.target.checked)}
            />
            <p className="text-caption text-fg-3">
              登录系统后自动打开 CC Switch X；由系统登录项管理，不需要管理员权限。
            </p>
          </section>

          <section className="space-y-2">
            <h3 className="text-strong text-fg-1">落盘检查</h3>
            <p className="text-caption text-fg-2">
              核对各 agent 配置目录里的文件是否仍与真源一致；漂移只报告，不会自动覆盖。
            </p>
            <div className="flex flex-wrap items-center gap-3">
              <Button loading={drift.busy} onClick={() => void drift.rescan()}>
                重扫落盘
              </Button>
              <span
                className={`text-caption ${drift.error ? "text-danger-text" : "text-fg-3"}`}
              >
                {drift.loading
                  ? "正在读取落盘状态…"
                  : drift.error
                    ? `检查失败：${drift.error}`
                    : drift.drifts.length === 0
                      ? "落盘与真源一致。"
                      : `发现 ${drift.drifts.length} 处与真源不一致，请到对应面板逐条处置。`}
              </span>
            </div>
          </section>

          <section className="space-y-2">
            <label className="block text-strong text-fg-1" htmlFor="store-root">
              存储位置
            </label>
            <div className="flex gap-2">
              <Input
                id="store-root"
                className="font-mono"
                value={storeRoot}
                onChange={(e) => setStoreRoot(e.target.value)}
              />
              <Button
                variant="primary"
                className="shrink-0"
                onClick={() => void saveStoreRoot()}
              >
                保存存储位置
              </Button>
            </div>
            {message && <Notice tone="success">{message}</Notice>}
          </section>

          <section className="space-y-2">
            <h3 className="text-strong text-fg-1">导入</h3>
            <p className="text-caption text-fg-2">
              从 cc-switch 的本地库导入 MCP、提示词与 Skills；导入前会先预览冲突。
            </p>
            <Button onClick={() => setImporting(true)}>从 cc-switch 导入</Button>
          </section>

          <AboutSection update={update} />
        </div>
      </div>
      {importing && <ImportDialog onClose={() => setImporting(false)} />}
    </div>
  );
}