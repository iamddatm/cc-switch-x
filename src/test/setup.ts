import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// vitest 未开启 globals，Testing Library 的自动 cleanup 不会注册，
// 必须显式清理，否则同一文件内多个用例的 DOM 会互相污染。
afterEach(() => {
  cleanup();
});
