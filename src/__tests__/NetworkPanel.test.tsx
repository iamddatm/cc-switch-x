import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { NetworkPanel } from "../panels/NetworkPanel";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    getProxySettings: vi.fn(),
    setProxySettings: vi.fn(),
    testProxySettings: vi.fn(),
    scanLocalProxies: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getProxySettings).mockResolvedValue({
    url: "http://127.0.0.1:7890",
    username: "me",
    hasPassword: true,
    enabled: true,
  });
  vi.mocked(api.setProxySettings).mockImplementation(
    async (url, username, password, enabled) => ({
      url: url || null,
      username: username || null,
      hasPassword: password !== null ? password !== "" : true,
      enabled: Boolean(url) && enabled,
    }),
  );
  vi.mocked(api.testProxySettings).mockResolvedValue({
    success: true,
    latencyMs: 42,
    error: null,
  });
  vi.mocked(api.scanLocalProxies).mockResolvedValue([
    { url: "socks5://127.0.0.1:1080", proxyType: "socks5", port: 1080 },
  ]);
});

test("loads the saved proxy and keeps the stored password when saving", async () => {
  render(<NetworkPanel />);
  const input = await screen.findByLabelText("代理地址");
  await waitFor(() => expect(input).toHaveValue("http://127.0.0.1:7890"));
  expect(screen.getByLabelText("用户名（可选）")).toHaveValue("me");

  fireEvent.change(input, { target: { value: "http://127.0.0.1:7891" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(api.setProxySettings).toHaveBeenCalledWith(
      "http://127.0.0.1:7891",
      "me",
      null,
      true,
    ),
  );
  expect(await screen.findByText("代理设置已保存。")).toBeInTheDocument();
});

test("disabling the proxy keeps the address and saves enabled=false", async () => {
  render(<NetworkPanel />);
  const input = await screen.findByLabelText("代理地址");
  await waitFor(() => expect(input).toHaveValue("http://127.0.0.1:7890"));

  fireEvent.click(screen.getByLabelText("启用代理"));
  fireEvent.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(api.setProxySettings).toHaveBeenCalledWith(
      "http://127.0.0.1:7890",
      "me",
      null,
      false,
    ),
  );
  expect(
    await screen.findByText("代理已停用，出站请求将直连。"),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("代理地址")).toHaveValue(
    "http://127.0.0.1:7890",
  );
});

test("scans local proxies and fills the selected address", async () => {
  render(<NetworkPanel />);
  await screen.findByLabelText("代理地址");
  fireEvent.click(screen.getByRole("button", { name: "扫描本地代理" }));

  const detected = await screen.findByRole("button", {
    name: "socks5://127.0.0.1:1080",
  });
  fireEvent.click(detected);
  expect(screen.getByLabelText("代理地址")).toHaveValue(
    "socks5://127.0.0.1:1080",
  );
});

test("tests the proxy and reports latency", async () => {
  render(<NetworkPanel />);
  await screen.findByLabelText("代理地址");
  fireEvent.click(screen.getByRole("button", { name: "测试代理" }));
  expect(await screen.findByText("代理可用，延迟 42 ms。")).toBeInTheDocument();
});
