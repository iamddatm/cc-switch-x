import { renderHook } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { THEME_STORAGE_KEY, readStoredTheme, resolveDark, useTheme } from "../hooks/useTheme";

/** 覆盖 setup.ts 里的 matchMedia 桩，用来模拟「系统是深色」。 */
function stubSystemDark(matches: boolean) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    configurable: true,
    value: vi.fn((query: string) => ({
      media: query,
      matches,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => true,
    })),
  });
}

afterEach(() => {
  localStorage.clear();
  stubSystemDark(false);
});

test("resolveDark 只在跟随系统时采纳系统偏好", () => {
  expect(resolveDark("system", true)).toBe(true);
  expect(resolveDark("system", false)).toBe(false);
  expect(resolveDark("dark", false)).toBe(true);
  expect(resolveDark("light", true)).toBe(false);
});

test("useTheme 把解析结果写到 html 上", () => {
  renderHook(() => useTheme("dark"));
  expect(document.documentElement.classList.contains("dark")).toBe(true);
  expect(document.documentElement.dataset.theme).toBe("dark");

  renderHook(() => useTheme("light"));
  expect(document.documentElement.classList.contains("dark")).toBe(false);
});

// 启动闪烁的守卫：显式选浅色时，系统深色也不能给 html 加 dark ——
// index.html 的预帧脚本按同一规则决定首帧，两边不一致就会看到一次跳变。
test("系统深色下显式浅色仍然不加 dark class", () => {
  stubSystemDark(true);
  renderHook(() => useTheme("light"));
  expect(document.documentElement.classList.contains("dark")).toBe(false);

  renderHook(() => useTheme("system"));
  expect(document.documentElement.classList.contains("dark")).toBe(true);
});

test("useTheme 同步写 localStorage 镜像，供预帧脚本读取", () => {
  expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  renderHook(() => useTheme("dark"));
  expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
  expect(readStoredTheme()).toBe("dark");
});

test("readStoredTheme 只认三档，缺失或非法都回落 system", () => {
  expect(readStoredTheme()).toBe("system");
  localStorage.setItem(THEME_STORAGE_KEY, "light");
  expect(readStoredTheme()).toBe("light");
  for (const junk of ["", "nosuch", "SYSTEM", "auto"]) {
    localStorage.setItem(THEME_STORAGE_KEY, junk);
    expect(readStoredTheme()).toBe("system");
  }
});
