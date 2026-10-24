import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { useUpdate } from "../hooks/useUpdate";

const mocks = vi.hoisted(() => ({
  isTauri: vi.fn(() => true),
  getVersion: vi.fn(),
  listen: vi.fn(),
  check: vi.fn(),
  install: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ isTauri: mocks.isTauri }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("../lib/api", () => ({
  api: {
    checkAppUpdate: mocks.check,
    installAppUpdate: mocks.install,
  },
}));

const updateInfo = {
  currentVersion: "0.1.0",
  version: "0.1.1",
  notes: "修复更新问题",
  pubDate: "2026-10-23T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.isTauri.mockReturnValue(true);
  mocks.getVersion.mockResolvedValue("0.1.0");
  mocks.check.mockResolvedValue(null);
  mocks.install.mockResolvedValue(false);
  mocks.listen.mockResolvedValue(() => {});
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

test("loads the current version and checks updates on demand", async () => {
  const { result } = renderHook(() => useUpdate());
  await act(async () => {});

  expect(result.current.currentVersion).toBe("0.1.0");

  mocks.check.mockResolvedValue(updateInfo);
  await act(async () => {
    await result.current.check();
  });

  expect(mocks.check).toHaveBeenCalledTimes(1);
  expect(result.current.available).toEqual(updateInfo);
  expect(result.current.hasUpdate).toBe(true);
});

test("checks silently one second after mount", async () => {
  renderHook(() => useUpdate());
  expect(mocks.check).not.toHaveBeenCalled();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });

  expect(mocks.check).toHaveBeenCalledTimes(1);
});

test("remembers a dismissed version and can restore the prompt", async () => {
  mocks.check.mockResolvedValue(updateInfo);
  const { result } = renderHook(() => useUpdate());
  await act(async () => {
    await result.current.check();
  });

  act(() => result.current.dismiss());
  expect(localStorage.getItem("cc-switch-x:update:dismissedVersion")).toBe("0.1.1");
  expect(result.current.dismissed).toBe(true);
  expect(result.current.hasUpdate).toBe(false);

  act(() => result.current.resetDismiss());
  expect(localStorage.getItem("cc-switch-x:update:dismissedVersion")).toBeNull();
  expect(result.current.dismissed).toBe(false);
  expect(result.current.hasUpdate).toBe(true);
});

test("subscribes to progress and keeps installing state until the process exits", async () => {
  mocks.check.mockResolvedValue(updateInfo);
  let resolveInstall: (value: boolean) => void = () => {};
  mocks.install.mockImplementation(
    () =>
      new Promise<boolean>((resolve) => {
        resolveInstall = resolve;
      }),
  );

  const { result } = renderHook(() => useUpdate());
  await act(async () => {
    await result.current.check();
  });

  let installPromise!: Promise<void>;
  await act(async () => {
    installPromise = result.current.install();
    await Promise.resolve();
    await Promise.resolve();
  });

  expect(mocks.listen).toHaveBeenCalledWith(
    "update-download-progress",
    expect.any(Function),
  );
  expect(mocks.listen).toHaveBeenCalledWith(
    "update-install-started",
    expect.any(Function),
  );
  expect(result.current.installing).toBe(true);

  const progressHandler = mocks.listen.mock.calls.find(
    ([event]) => event === "update-download-progress",
  )?.[1] as (event: { payload: unknown }) => void;
  act(() => progressHandler({ payload: { downloaded: 1024, total: 2048 } }));
  expect(result.current.progress).toEqual({ downloaded: 1024, total: 2048 });

  const startedHandler = mocks.listen.mock.calls.find(
    ([event]) => event === "update-install-started",
  )?.[1] as () => void;
  act(() => startedHandler());
  expect(result.current.installStarted).toBe(true);

  await act(async () => {
    resolveInstall(false);
    await installPromise;
  });

  expect(result.current.installing).toBe(false);
  expect(result.current.available).toBeNull();
});

test("surfaces manual check errors", async () => {
  mocks.check.mockRejectedValue(new Error("网络不可用"));
  const { result } = renderHook(() => useUpdate());

  await act(async () => {
    await result.current.check();
  });

  expect(result.current.error).toContain("网络不可用");
});
