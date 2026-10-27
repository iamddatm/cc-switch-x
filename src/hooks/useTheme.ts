import { useEffect, useLayoutEffect, useState } from "react";
import type { Theme } from "../types";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * 主题的预帧缓存。真源仍是 settings.json，这里只解决一件事：
 * `get_settings` 是异步 IPC，首帧拿不到用户显式选的主题，只能先按系统偏好画一次再跳。
 * index.html 的内联脚本会在打包代码之前读这个键，所以两处键名必须一致。
 */
export const THEME_STORAGE_KEY = "ccswitchx.theme";

const THEMES: Theme[] = ["light", "dark", "system"];

/** 读缓存；缺失或非法值一律按「跟随系统」，与后端默认值一致。 */
export function readStoredTheme(): Theme {
  const stored = localStorage.getItem(THEME_STORAGE_KEY);
  return THEMES.includes(stored as Theme) ? (stored as Theme) : "system";
}

// 只有「跟随系统」才采纳系统偏好；显式 light/dark 不受系统影响。
export function resolveDark(theme: Theme, prefersDark: boolean): boolean {
  if (theme === "system") return prefersDark;
  return theme === "dark";
}

export function useTheme(theme: Theme) {
  const [prefersDark, setPrefersDark] = useState(() => window.matchMedia(DARK_QUERY).matches);

  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);
    const onChange = (event: MediaQueryListEvent) => setPrefersDark(event.matches);
    media.addEventListener("change", onChange);
    setPrefersDark(media.matches);
    return () => media.removeEventListener("change", onChange);
  }, []);

  // 必须在浏览器绘制这一帧之前翻好 class，用 useEffect 会多画一帧过渡。
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", resolveDark(theme, prefersDark));
    root.dataset.theme = theme;
    localStorage.setItem(THEME_STORAGE_KEY, theme);
  }, [theme, prefersDark]);
}
