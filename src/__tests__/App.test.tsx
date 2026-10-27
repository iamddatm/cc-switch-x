import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import App from "../App";
import { api } from "../lib/api";
import type { DriftRecord } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    getSettings: vi.fn().mockResolvedValue({
      storeRoot: "C:/Users/me/.ccswitchx",
      theme: "system",
      enabledAgents: null,
      closeToTray: true,
      proxy: { url: null, username: null },
      skillRepos: null,
      backup: { intervalHours: 24, retainCount: 10, git: { enabled: false, autoCommit: true, autoPull: true, remote: null, branch: "main" } },
      cloud: { autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx" },
    }),
    listAgents: vi.fn().mockResolvedValue([]),
    listAssets: vi.fn().mockResolvedValue([]),
    listProjects: vi.fn().mockResolvedValue([]),
    sessionProviders: vi.fn().mockResolvedValue([]),
    listSessions: vi.fn().mockResolvedValue([]),
    listDrifts: vi.fn().mockResolvedValue([]),
    rescanDrift: vi.fn().mockResolvedValue([]),
  },
}));

test("renders all sidebar entries and switches panes", () => {
  render(<App />);
  for (const label of [
    "MCP",
    "Skills",
    "提示词",
    "Rules",
    "Subagents",
    "Hooks",
    "会话",
    "Agents",
    "网络",
    "备份",
    "Settings",
  ]) {
    expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
  }

  const nav = screen.getByRole("navigation", { name: "主导航" });
  expect(within(nav).getByText("资产")).toBeInTheDocument();
  expect(within(nav).getByText("工具")).toBeInTheDocument();
  expect(within(nav).getByText("管理")).toBeInTheDocument();
  expect(within(nav).getAllByRole("button").map((item) => item.textContent)).toEqual([
    "全局配置",
    "Skills",
    "MCP",
    "提示词",
    "Rules",
    "Subagents",
    "Hooks",
    "会话",
    "Agents",
    "网络",
    "备份",
    "Settings",
  ]);

  fireEvent.click(screen.getByRole("button", { name: "Rules" }));
  expect(screen.getByRole("heading", { name: "Rules" })).toBeInTheDocument();
});

test("落盘状态条常驻，重扫后数字跟着磁盘结果变", async () => {
  const drift: DriftRecord = {
    id: 1,
    assetType: "rule",
    assetId: "style",
    agentId: "claude",
    kind: "modified",
    detectedAt: 1,
    targetPath: "C:/x/.claude/rules/style.md",
  };
  render(<App />);
  expect(await screen.findByText("落盘与真源一致")).toBeInTheDocument();

  vi.mocked(api.rescanDrift).mockResolvedValue([drift]);
  fireEvent.click(screen.getByRole("button", { name: "重扫落盘" }));

  expect(await screen.findByText(/落盘有 1 处与真源不一致/)).toBeInTheDocument();
  expect(api.rescanDrift).toHaveBeenCalledTimes(1);
});