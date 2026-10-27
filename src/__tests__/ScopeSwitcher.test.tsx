import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import App from "../App";
import { api } from "../lib/api";
import { agentStatus } from "../test/agents";
import type { ProjectView, SessionSummary } from "../types";

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
    revealProjectFolder: vi.fn().mockResolvedValue(undefined),
    sessionProviders: vi.fn().mockResolvedValue([]),
    listSessions: vi.fn().mockResolvedValue([]),
    listDrifts: vi.fn().mockResolvedValue([]),
    rescanDrift: vi.fn().mockResolvedValue([]),
  },
}));

const project: ProjectView = {
  id: "pdd-1a2b3c4d5e6f",
  path: "D:\\repos\\ai\\PddWorker",
  name: "PddWorker",
  addedAt: "2026-10-26T00:00:00Z",
  lastOpenedAt: "2026-10-26T00:00:00Z",
  autoSync: true,
  exists: true,
};

function sessionRow(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    agentId: "claude",
    sessionId: "s1",
    title: "会话标题",
    projectPath: project.path,
    sourcePath: `${project.path}/s1.jsonl`,
    createdAt: null,
    updatedAt: Date.now(),
    messageCount: 1,
    parseError: null,
    preview: null,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.mocked(api.listProjects).mockResolvedValue([]);
  vi.mocked(api.listAgents).mockResolvedValue([]);
  vi.mocked(api.listSessions).mockResolvedValue([]);
  vi.mocked(api.touchProject).mockResolvedValue(project);
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project,
    dirs: [],
    skills: [],
    conflictCount: 0,
    migration: null,
  });
});

function openSwitcher() {
  fireEvent.click(screen.getByRole("button", { name: /^配置范围：/ }));
}

/** 触发器与项目行都带项目名，按路径取才只命中项目行。 */
async function pickProject() {
  openSwitcher();
  fireEvent.click(await screen.findByRole("button", { name: /D:\\repos/ }));
}

test("下拉里不再出现「全局配置 / 项目」切换按钮", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  render(<App />);
  openSwitcher();

  expect(await screen.findByText("PddWorker")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "项目" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "添加项目…" })).toBeInTheDocument();
});

test("全局与项目同时只有一项处于选中态", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  render(<App />);
  const pressed = () =>
    screen
      .getAllByRole("button")
      .filter((button) => button.getAttribute("aria-pressed") === "true")
      .map((button) => button.textContent?.trim());

  openSwitcher();
  await screen.findByRole("button", { name: /^全局配置$/ });
  expect(pressed()).toEqual(["全局配置"]);

  fireEvent.keyDown(window.document, { key: "Escape" });
  await pickProject();
  openSwitcher();
  await screen.findByRole("button", { name: /^全局配置$/ });
  expect(pressed()).toEqual(["PddWorkerD:\\repos\\ai\\PddWorker"]);
});

test("选中项目即切到项目范围，其他资产不可点", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  render(<App />);
  await pickProject();

  const mcp = screen.getByRole("button", { name: "MCP" });
  expect(mcp).toBeDisabled();
  expect(mcp).toHaveAttribute("title", "仅支持全局配置");
  expect(screen.getByRole("button", { name: "Skills" })).toBeEnabled();
  expect(localStorage.getItem("ccswitchx.scope")).toBe("project");
  await waitFor(() => expect(api.touchProject).toHaveBeenCalledWith(project.id));
});

test("选中项目时从 MCP 自动跳到项目 Skills", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  render(<App />);
  await pickProject();

  expect(await screen.findByRole("heading", { name: "Skills" })).toBeInTheDocument();
  expect(await screen.findByText(/PddWorker · D:\\repos\\ai\\PddWorker/)).toBeInTheDocument();
});

test("选「全局配置」切回全局范围", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  render(<App />);
  await pickProject();

  fireEvent.click(screen.getByRole("button", { name: /^配置范围：/ }));
  fireEvent.click(screen.getByRole("button", { name: "全局配置" }));

  expect(localStorage.getItem("ccswitchx.scope")).toBe("global");
  expect(screen.getByRole("button", { name: "MCP" })).toBeEnabled();
});

/**
 * 菜单弹层是 portal 到 body 的：浏览器里 pointerdown 先于 click，
 * 若「点外面关闭」在 pointerdown 阶段就把下拉卸载掉，菜单项的 click 永远收不到。
 */
test("下拉里的 ⋯ 菜单项点得动", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  render(<App />);
  openSwitcher();
  fireEvent.click(await screen.findByRole("button", { name: "PddWorker 更多操作" }));

  const item = await screen.findByRole("menuitem", { name: "在文件夹中显示" });
  fireEvent.pointerDown(item);
  fireEvent.click(item);

  await waitFor(() => expect(api.revealProjectFolder).toHaveBeenCalledWith(project.id));
});

test("注册表读不出来时必须说明原因，不能当成没有项目", async () => {
  vi.mocked(api.listProjects).mockRejectedValue("项目注册表格式损坏：expected value at line 1");

  render(<App />);
  expect(await screen.findByText(/格式损坏/)).toBeInTheDocument();
});

test("选中项目后会话自动只看这个项目的记录", async () => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  vi.mocked(api.listAgents).mockResolvedValue([agentStatus()]);
  vi.mocked(api.listSessions).mockResolvedValue([
    sessionRow({ sessionId: "in", title: "本项目会话" }),
    sessionRow({ sessionId: "sub", title: "子目录会话", projectPath: `${project.path}\\web` }),
    sessionRow({ sessionId: "out", title: "其他项目会话", projectPath: "D:\\repos\\other" }),
  ]);

  render(<App />);
  await pickProject();
  fireEvent.click(screen.getByRole("button", { name: "会话" }));

  expect(await screen.findByText("本项目会话")).toBeInTheDocument();
  expect(screen.getByText("子目录会话")).toBeInTheDocument();
  expect(screen.queryByText("其他项目会话")).not.toBeInTheDocument();
  expect(screen.getByText("PddWorker · D:\\repos\\ai\\PddWorker")).toBeInTheDocument();
});
