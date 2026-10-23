import { useEffect, useState } from "react";
import { Settings } from "lucide-react";
import { ImportDialog } from "../components/ImportDialog";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { Select } from "../components/ui/Select";
import { api } from "../lib/api";
import type { AppSettings, Theme } from "../types";

export function SettingsPanel({
  theme,
  onThemeChange,
}: {
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
}) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [storeRoot, setStoreRoot] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    void api
      .getSettings()
      .then((value) => {
        setSettings(value);
        setStoreRoot(value.storeRoot);
      })
      .catch((e) => setError(String(e)));
  }, []);

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
      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">
        <div className="max-w-2xl space-y-8">
          {error && <Notice tone="danger">{error}</Notice>}
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
        </div>
      </div>
      {importing && <ImportDialog onClose={() => setImporting(false)} />}
    </div>
  );
}