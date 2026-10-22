import { useEffect } from "react";
import type { Theme } from "../types";

export function useTheme(theme: Theme) {
  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.dataset.theme = theme;
  }, [theme]);
}
