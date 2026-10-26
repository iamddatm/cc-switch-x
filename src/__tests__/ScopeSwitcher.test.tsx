import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import App from "../App";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    getSettings: vi.fn().mockResolvedValue({ theme: "system" }),
    listAgents: vi.fn().mockResolvedValue([]),
    listAssets: vi.fn().mockResolvedValue([]),
    listProjects: vi.fn().mockResolvedValue([]),
    touchProject: vi.fn(),
    addProject: vi.fn(),
    removeProject: vi.fn(),
    setProjectAutoSync: vi.fn(),
    listProjectSkills: vi.fn(),
    revealProjectFolder: vi.fn(),
    sessionProviders: vi.fn().mockResolvedValue([]),
    listSessions: vi.fn().mockResolvedValue([]),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.mocked(api.listProjects).mockResolvedValue([]);
});

test("项目范围只允许 Skills，其他资产不可点", async () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /全局配置/ }));
  fireEvent.click(screen.getByRole("button", { name: "项目" }));

  const mcp = screen.getByRole("button", { name: "MCP" });
  expect(mcp).toBeDisabled();
  expect(mcp).toHaveAttribute("title", "仅支持全局配置");
  expect(screen.getByRole("button", { name: "Skills" })).toBeEnabled();
});

test("切到项目范围时从 MCP 自动跳到 Skills", async () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /全局配置/ }));
  fireEvent.click(screen.getByRole("button", { name: "项目" }));

  expect(await screen.findByRole("heading", { name: "Skills" })).toBeInTheDocument();
});
