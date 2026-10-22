import { renderHook } from "@testing-library/react";
import { expect, test } from "vitest";
import { resolveDark, useTheme } from "../hooks/useTheme";

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
