import { useEffect, useState } from "react";
import type { Theme } from "../types";

const DARK_QUERY = "(prefers-color-scheme: dark)";

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

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", resolveDark(theme, prefersDark));
    root.dataset.theme = theme;
  }, [theme, prefersDark]);
}
