import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { AgentsPanel } from "../panels/AgentsPanel";
import { resetAgentVersionCheck } from "../hooks/useAgentVersionCheck";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";
import type { AgentVersionStatus, AppSettings } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAgents: vi.fn(),
    checkAgentVersions: vi.fn(),
    setEnabledAgents: vi.fn(),
    setAgentOrder: vi.fn(),
  },
}));

const savedSettings: AppSettings = {
  storeRoot: "C:/Users/me/.ccswitchx",
  theme: "system",
  enabledAgents: ["claude", "pi"],
  agentOrder: null,
  closeToTray: true,
  proxy: { url: null, username: null },
  skillRepos: null,
  backup: { intervalHours: 24, retainCount: 10, git: { enabled: false, autoCommit: true, autoPull: true, remote: null, branch: "main" } },
  cloud: { enabled: false, autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx" },
};

beforeEach(() => {
  vi.clearAllMocks();
  resetAgentVersionCheck();
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus(),
    agentStatus({
      id: "pi",
      displayName: "Pi Coding Agent",
      detected: false,
      cliAvailable: false,
      cliProgram: "pi",
      enabled: false,
      capabilities: capabilities({ mcp: "unverified" }),
    }),
  ]);
  vi.mocked(api.setEnabledAgents).mockResolvedValue(savedSettings);
  vi.mocked(api.setAgentOrder).mockResolvedValue(savedSettings);
});

test("lists all agents with CLI and config state", async () => {
  render(<AgentsPanel />);
  expect(await screen.findByText("Claude Code")).toBeInTheDocument();
  expect(screen.getByText("Pi Coding Agent")).toBeInTheDocument();
  expect(screen.getByText("CLI 已安装")).toBeInTheDocument();
  expect(screen.getByText("CLI 未安装")).toBeInTheDocument();
  expect(screen.getByText("配置目录已就绪")).toBeInTheDocument();
  expect(screen.getByText("配置目录不存在")).toBeInTheDocument();
  expect(screen.getAllByText(/MCP/).length).toBeGreaterThan(0);
  expect(screen.getAllByText(/未实测/).length).toBeGreaterThan(0);
});

test("toggling an agent persists the new enabled set", async () => {
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "启用 Pi Coding Agent" }));
  await waitFor(() =>
    expect(api.setEnabledAgents).toHaveBeenCalledWith(["claude", "pi"]),
  );
  expect(await screen.findByText(/已启用 Pi Coding Agent/)).toBeInTheDocument();
});

test("disabling an agent keeps the rest enabled", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus(),
    agentStatus({ id: "pi", displayName: "Pi Coding Agent" }),
  ]);
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "启用 Claude Code" }));
  await waitFor(() => expect(api.setEnabledAgents).toHaveBeenCalledWith(["pi"]));
  expect(await screen.findByText(/已停用 Claude Code/)).toBeInTheDocument();
});

test("a failed save surfaces the backend reason", async () => {
  vi.mocked(api.setEnabledAgents).mockRejectedValue("非法输入：未知的 agent：pi");
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "启用 Pi Coding Agent" }));
  expect(await screen.findByText(/未知的 agent/)).toBeInTheDocument();
});

test("dragging a row onto another persists the new order", async () => {
  render(<AgentsPanel />);
  const first = (await screen.findByText("Claude Code")).closest("li")!;
  const second = screen.getByText("Pi Coding Agent").closest("li")!;
  const dataTransfer = {
    effectAllowed: "",
    dropEffect: "",
    setData: vi.fn(),
    getData: vi.fn(() => ""),
  };

  fireEvent.dragStart(first, { dataTransfer });
  fireEvent.dragOver(second, { dataTransfer });
  fireEvent.drop(second, { dataTransfer });

  await waitFor(() =>
    expect(api.setAgentOrder).toHaveBeenCalledWith(["pi", "claude"]),
  );
  expect(await screen.findByText(/已调整 Agent 顺序/)).toBeInTheDocument();
});

// ── 版本检查与命令复制 ───────────────────────────────────────────

function versionStatus(overrides: Partial<AgentVersionStatus> = {}): AgentVersionStatus {
  return {
    id: "claude",
    displayName: "Claude Code",
    cliFound: true,
    installedVersion: "2.0.0",
    latestVersion: "2.0.0",
    updateAvailable: false,
    error: null,
    ...overrides,
  };
}

test("checking versions marks only the agent that is actually behind", async () => {
  vi.mocked(api.checkAgentVersions).mockResolvedValue({
    checkedAt: Date.now(),
    agents: [
      versionStatus({ installedVersion: "2.0.0", latestVersion: "2.1.0", updateAvailable: true }),
      // 没装的那条只报上游版本，本机没有可比较的基准，不能写成「有新版本」
      versionStatus({
        id: "pi",
        displayName: "Pi Coding Agent",
        cliFound: false,
        installedVersion: null,
        latestVersion: "0.9.0",
      }),
    ],
  });
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));

  expect(await screen.findByText("2.0.0 → 2.1.0")).toBeInTheDocument();
  expect(screen.getByText("未找到命令")).toBeInTheDocument();
  expect(screen.queryByText(/→ 0.9.0/)).not.toBeInTheDocument();
  expect(screen.getByText(/Claude Code 2\.0\.0 → 2\.1\.0/)).toBeInTheDocument();
  expect(screen.getByText(/检查于/)).toBeInTheDocument();
});

test("an up-to-date agent shows a plain version with no warning", async () => {
  vi.mocked(api.checkAgentVersions).mockResolvedValue({
    checkedAt: Date.now(),
    agents: [versionStatus()],
  });
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));

  await screen.findByText("2.0.0");
  expect(screen.queryByText(/→/)).not.toBeInTheDocument();
  expect(screen.queryByText(/不代跑安装命令/)).not.toBeInTheDocument();
});

test("a CLI found but yielding no version is reported as unreadable", async () => {
  vi.mocked(api.checkAgentVersions).mockResolvedValue({
    checkedAt: Date.now(),
    agents: [versionStatus({ installedVersion: null, latestVersion: null })],
  });
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));

  // 「找到命令但读不出版本」是探测失败，不能和「没装」混成一句
  expect(await screen.findByText("版本读不出")).toBeInTheDocument();
});

test("a failed version check says why instead of showing nothing", async () => {
  vi.mocked(api.checkAgentVersions).mockRejectedValue("网络请求失败：超时");
  render(<AgentsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));

  expect(await screen.findByText(/网络请求失败：超时/)).toBeInTheDocument();
});

test("copying an install command writes the clipboard and runs nothing", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
  render(<AgentsPanel />);

  const triggers = await screen.findAllByRole("button", { name: "复制安装 / 升级命令" });
  fireEvent.click(triggers[0]);
  fireEvent.click(await screen.findByRole("menuitem", { name: "npm 全局安装 / 升级" }));

  await waitFor(() =>
    expect(writeText).toHaveBeenCalledWith(
      "npm i -g @anthropic-ai/claude-code@latest --ignore-scripts=false --include=optional --allow-scripts=@anthropic-ai/claude-code",
    ),
  );
  expect(await screen.findByText(/已复制/)).toBeInTheDocument();
  // 复制的串不回头变成任何一次后端调用
  expect(api.setEnabledAgents).not.toHaveBeenCalled();
  expect(api.setAgentOrder).not.toHaveBeenCalled();
});

test("an agent without a verified install entry exposes no copy button", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ id: "qoder-cn", displayName: "Qoder CN", cliProgram: "qoderclicn" }),
  ]);
  render(<AgentsPanel />);

  await screen.findByText("Qoder CN");
  expect(screen.queryByRole("button", { name: "复制安装 / 升级命令" })).not.toBeInTheDocument();
});
