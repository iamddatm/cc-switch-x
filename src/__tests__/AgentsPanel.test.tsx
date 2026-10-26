import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { AgentsPanel } from "../panels/AgentsPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";

vi.mock("../lib/api", () => ({
  api: {
    listAgents: vi.fn(),
    setEnabledAgents: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
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
  vi.mocked(api.setEnabledAgents).mockResolvedValue({
    storeRoot: "C:/Users/me/.ccswitchx",
    theme: "system",
    enabledAgents: ["claude", "pi"],
    closeToTray: true,
    proxy: { url: null, username: null },
    skillRepos: null,
      backup: { intervalHours: 24, retainCount: 10, git: { enabled: false, autoCommit: true, autoPull: true, remote: null, branch: "main" } },
      cloud: { autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx" },
  });
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