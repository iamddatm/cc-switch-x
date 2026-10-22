import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

// vitest 未开启 globals，Testing Library 的自动 cleanup 不会注册，
// 必须显式清理，否则同一文件内多个用例的 DOM 会互相污染。
afterEach(() => {
  cleanup();
});

// jsdom 不实现 matchMedia；useTheme 在 theme=system 时会读它。
// 系统偏好本身的判定逻辑抽成 resolveDark 单独做纯函数测试，这里只给最小桩。
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: vi.fn((query: string) => ({
    media: query,
    matches: false,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => true,
  })),
});
