import { useEffect, useState } from "react";
import { useTheme } from "../hooks/useTheme";
import { api } from "../lib/api";
import type { AppSettings, Theme } from "../types";

export function SettingsPanel() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [storeRoot, setStoreRoot] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useTheme(settings?.theme ?? "system");

  useEffect(() => {
    void api
      .getSettings()
      .then((value) => {
        setSettings(value);
        setStoreRoot(value.storeRoot);
      })
      .catch((e) => setError(String(e)));
  }, []);

  const changeTheme = async (theme: Theme) => {
    try {
      const updated = await api.setTheme(theme);
      setSettings(updated);
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
    <div className="max-w-2xl space-y-8">
      <h2 className="text-xl font-semibold">Settings</h2>
      {error && <p className="text-sm text-red-600">{error}</p>}
      <section>
        <label className="mb-2 block text-sm font-medium" htmlFor="theme">
          主题
        </label>
        <select
          id="theme"
          className="rounded border border-slate-300 px-3 py-1.5"
          value={settings?.theme ?? "system"}
          onChange={(e) => void changeTheme(e.target.value as Theme)}
        >
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </section>
      <section>
        <label className="mb-2 block text-sm font-medium" htmlFor="store-root">
          存储位置
        </label>
        <div className="flex gap-2">
          <input
            id="store-root"
            className="flex-1 rounded border border-slate-300 px-3 py-1.5 font-mono text-sm"
            value={storeRoot}
            onChange={(e) => setStoreRoot(e.target.value)}
          />
          <button
            type="button"
            className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white"
            onClick={saveStoreRoot}
          >
            保存存储位置
          </button>
        </div>
        {message && <p className="mt-2 text-sm text-green-700">{message}</p>}
      </section>
    </div>
  );
}
