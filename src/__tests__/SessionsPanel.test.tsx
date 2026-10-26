import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SessionsPanel } from "../panels/SessionsPanel";
import { api } from "../lib/api";
import { agentStatus } from "../test/agents";
import type { ProjectView, SessionProviderStatus, SessionSummary } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAgents: vi.fn(),
    listSessions: vi.fn(),
    sessionProviders: vi.fn(),
    scanSessions: vi.fn(),
    searchSessions: vi.fn(),
    getSession: vi.fn(),
    searchInSession: vi.fn(),
    sessionResume: vi.fn(),
    exportSessionMarkdown: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

function provider(
  overrides: Partial<SessionProviderStatus> = {},
): SessionProviderStatus {
  return {
    agentId: "claude",
    displayName: "Claude Code",
    verified: true,
    parseReady: true,
    roots: ["C:/Users/me/.claude/projects"],
    rootExists: true,
    foundFiles: 1,
    sessionCount: 1,
    ...overrides,
  };
}

function session(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    agentId: "claude",
    sessionId: "s1",
    title: "会话标题",
    projectPath: "D:/repo",
    sourcePath: "D:/repo/s1.jsonl",
    createdAt: null,
    updatedAt: Date.now(),
    messageCount: 2,
    parseError: null,
    preview: null,
    ...overrides,
  };
}

function agent(id: string, displayName: string) {
  return agentStatus({ id, displayName });
}

/** 与注册表一致的项目条目：路径是 Rust `PathBuf` 序列化出的 Windows 写法。 */
function project(overrides: Partial<ProjectView> = {}): ProjectView {
  return {
    id: "app-1a2b3c4d5e6f",
    path: "D:\\repos\\ai\\App",
    name: "App",
    addedAt: "2026-10-26T00:00:00Z",
    lastOpenedAt: "2026-10-26T00:00:00Z",
    autoSync: true,
    exists: true,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAgents).mockResolvedValue([agent("claude", "Claude Code")]);
  vi.mocked(api.listSessions).mockResolvedValue([]);
  vi.mocked(api.sessionProviders).mockResolvedValue([]);
  vi.mocked(api.scanSessions).mockResolvedValue({
    added: 0,
    updated: 0,
    removed: 0,
    unchanged: 0,
    failures: [],
    warnings: [],
  });
  vi.mocked(api.searchSessions).mockResolvedValue([]);
  vi.mocked(api.getSession).mockResolvedValue({
    session: session(),
    messages: [],
    total: 0,
  });
  vi.mocked(api.searchInSession).mockResolvedValue([]);
  vi.mocked(api.sessionResume).mockResolvedValue({
    agentId: "claude",
    sessionId: "s1",
    cwd: null,
    command: "claude --resume s1",
  });
  vi.mocked(api.exportSessionMarkdown).mockResolvedValue("");
});

afterEach(cleanup);

test("first_run_offers_a_manual_scan_and_does_not_autoscan", async () => {
  render(<SessionsPanel />);

  const scan = await screen.findByRole("button", { name: "扫描本机会话" });
  expect(scan).toBeEnabled();
  expect(api.listSessions).toHaveBeenCalled();
  expect(api.scanSessions).not.toHaveBeenCalled();
});

test("empty_state_names_each_provider_and_why_it_has_no_sessions", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agent("claude", "Claude Code"),
    agent("pi", "Pi Coding Agent"),
    agent("gemini", "Gemini CLI"),
  ]);
  vi.mocked(api.sessionProviders).mockResolvedValue([
    provider({
      agentId: "claude",
      displayName: "Claude Code",
      foundFiles: 0,
      sessionCount: 0,
      roots: ["C:/Users/me/.claude/projects"],
    }),
    provider({
      agentId: "pi",
      displayName: "Pi Coding Agent",
      parseReady: false,
      foundFiles: 5,
      sessionCount: 0,
      roots: ["C:/Users/me/.pi/agent/sessions"],
    }),
    provider({
      agentId: "gemini",
      displayName: "Gemini CLI",
      rootExists: false,
      foundFiles: 0,
      sessionCount: 0,
      roots: ["C:/Users/me/.gemini/tmp"],
    }),
  ]);

  render(<SessionsPanel />);

  await screen.findByText("Claude Code");
  expect(screen.getByText("未发现文件")).toBeInTheDocument();
  expect(screen.getByText("解析器未接入")).toBeInTheDocument();
  expect(screen.getByText("目录不存在")).toBeInTheDocument();
  expect(screen.getByText(/\.claude\/projects/)).toBeInTheDocument();
});

test("files_found_through_an_env_root_are_not_reported_as_missing", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([agent("opencode", "OpenCode")]);
  vi.mocked(api.sessionProviders).mockResolvedValue([
    provider({
      agentId: "opencode",
      displayName: "OpenCode",
      roots: ["C:/Users/me/.local/share/opencode"],
      rootExists: false,
      foundFiles: 1,
      sessionCount: 0,
    }),
  ]);

  render(<SessionsPanel />);

  await screen.findByText("OpenCode");
  expect(screen.getByText("已发现文件，尚未解析出会话")).toBeInTheDocument();
  expect(screen.queryByText("目录不存在")).not.toBeInTheDocument();
});

test("unverified_provider_sessions_carry_a_warning_badge", async () => {
  vi.mocked(api.listSessions).mockResolvedValue([
    session({ title: "未实测会话" }),
  ]);
  vi.mocked(api.sessionProviders).mockResolvedValue([
    provider({ verified: false, sessionCount: 1 }),
  ]);

  render(<SessionsPanel />);

  fireEvent.click(await screen.findByRole("button", { name: /repo/ }));
  await screen.findByText("未实测会话");
  expect(screen.getAllByText("解析器未实测").length).toBeGreaterThan(0);
});

test("search_shows_the_matching_snippet_and_session", async () => {
  const target = session({ title: "登录问题" });
  vi.mocked(api.listSessions).mockResolvedValue([target]);
  vi.mocked(api.sessionProviders).mockResolvedValue([provider()]);
  vi.mocked(api.searchSessions).mockResolvedValue([
    { session: target, index: 2, seq: 2, snippet: "这里包含关键字片段" },
  ]);

  render(<SessionsPanel />);

  const input = await screen.findByLabelText("搜索会话内容");
  fireEvent.change(input, { target: { value: "关键字" } });
  fireEvent.keyDown(input, { key: "Enter" });

  expect(await screen.findByText("这里包含关键字片段")).toBeInTheDocument();
  expect(screen.getByText("登录问题")).toBeInTheDocument();
  expect(screen.getByText("命中")).toBeInTheDocument();
  expect(api.searchSessions).toHaveBeenCalledWith("关键字", 500);
});

test("loads_all_sessions_in_one_request_for_complete_project_groups", async () => {
  const target = session({ sessionId: "s50", title: "第 51 个会话" });
  vi.mocked(api.listSessions).mockResolvedValue([target]);
  vi.mocked(api.sessionProviders).mockResolvedValue([provider()]);

  render(<SessionsPanel />);

  fireEvent.click(await screen.findByRole("button", { name: /repo/ }));
  expect(await screen.findByText("第 51 个会话")).toBeInTheDocument();
  expect(api.listSessions).toHaveBeenCalledWith(undefined, undefined, 0, 2000);
  expect(screen.queryByRole("button", { name: "载入更多" })).not.toBeInTheDocument();
});

test("opening_a_session_and_returning_keeps_the_search_term", async () => {
  const target = session({ title: "登录问题" });
  vi.mocked(api.listSessions).mockResolvedValue([target]);
  vi.mocked(api.sessionProviders).mockResolvedValue([provider()]);
  vi.mocked(api.searchSessions).mockResolvedValue([
    { session: target, index: 0, seq: 0, snippet: "关键字片段" },
  ]);
  vi.mocked(api.getSession).mockResolvedValue({
    session: target,
    messages: [
      { seq: 0, role: "user", kind: "text", content: "会话正文", toolName: null },
    ],
    total: 1,
  });

  render(<SessionsPanel />);

  const input = await screen.findByLabelText("搜索会话内容");
  fireEvent.change(input, { target: { value: "关键字" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await screen.findByText("关键字片段");
  fireEvent.click(screen.getByRole("button", { name: /登录问题/ }));
  expect(await screen.findByText("会话正文")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "返回列表" }));
  expect(screen.getByLabelText("搜索会话内容")).toHaveValue("关键字");
  expect(screen.getByText("关键字片段")).toBeInTheDocument();
});

test("scan_warnings_show_the_file_and_line", async () => {
  vi.mocked(api.scanSessions).mockResolvedValue({
    added: 0,
    updated: 0,
    removed: 0,
    unchanged: 0,
    failures: [],
    warnings: [
      {
        agentId: "claude",
        file: "C:/bad.jsonl",
        line: 4,
        message: "JSON 解析失败：expected value",
      },
    ],
  });

  render(<SessionsPanel />);

  fireEvent.click(await screen.findByRole("button", { name: "扫描本机会话" }));

  expect(await screen.findByText(/bad\.jsonl:4/)).toBeInTheDocument();
  expect(screen.getByText(/expected value/)).toBeInTheDocument();
});

test("scan_failures_are_listed_not_hidden", async () => {
  vi.mocked(api.scanSessions).mockResolvedValue({
    added: 0,
    updated: 0,
    removed: 0,
    unchanged: 0,
    failures: [
      {
        agentId: "claude",
        file: "C:/bad.jsonl",
        reason: "JSON 解析失败",
      },
    ],
    warnings: [],
  });

  render(<SessionsPanel />);

  fireEvent.click(await screen.findByRole("button", { name: "扫描本机会话" }));

  expect(await screen.findByText(/bad\.jsonl/)).toBeInTheDocument();
  expect(screen.getByText(/JSON 解析失败/)).toBeInTheDocument();
});

test("project_scope_keeps_only_that_project_including_subdirectories", async () => {
  vi.mocked(api.listSessions).mockResolvedValue([
    session({ sessionId: "in", title: "项目根会话", projectPath: "D:\\repos\\ai\\App" }),
    session({ sessionId: "sub", title: "子目录会话", projectPath: "D:/repos/ai/App/web" }),
    session({ sessionId: "sibling", title: "同名兄弟目录会话", projectPath: "D:\\repos\\ai\\Appweb" }),
    session({ sessionId: "other", title: "别的项目会话", projectPath: "D:\\repos\\other" }),
    session({ sessionId: "none", title: "无归属会话", projectPath: null }),
  ]);
  vi.mocked(api.sessionProviders).mockResolvedValue([provider({ sessionCount: 5 })]);

  render(<SessionsPanel project={project()} />);

  expect(await screen.findByText("项目根会话")).toBeInTheDocument();
  expect(screen.getByText("子目录会话")).toBeInTheDocument();
  expect(screen.queryByText("同名兄弟目录会话")).not.toBeInTheDocument();
  expect(screen.queryByText("别的项目会话")).not.toBeInTheDocument();
  expect(screen.queryByText("无归属会话")).not.toBeInTheDocument();
  expect(screen.getByText("App · D:\\repos\\ai\\App")).toBeInTheDocument();
  expect(screen.getByText("2 个会话")).toBeInTheDocument();
  // 没有 cwd 的会话被过滤掉了，必须说出来，否则用户以为会话丢了。
  expect(screen.getByText(/另有 1 个会话没有记录项目目录/)).toBeInTheDocument();
});

test("project_scope_counts_agents_within_the_project", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agent("claude", "Claude Code"),
    agent("pi", "Pi"),
  ]);
  vi.mocked(api.listSessions).mockResolvedValue([
    session({ sessionId: "c1", title: "本项目 Claude 会话", projectPath: "D:/repos/ai/App" }),
    session({
      sessionId: "p1",
      agentId: "pi",
      title: "本项目 Pi 会话",
      projectPath: "D:/repos/ai/App/web",
    }),
    session({ sessionId: "c2", title: "别的项目 Claude 会话", projectPath: "D:/other" }),
  ]);
  vi.mocked(api.sessionProviders).mockResolvedValue([
    provider({ sessionCount: 300 }),
    provider({ agentId: "pi", displayName: "Pi", sessionCount: 5 }),
  ]);

  render(<SessionsPanel project={project()} />);

  const select = await screen.findByLabelText("按应用筛选");
  expect(select).toHaveTextContent("全部应用 (2)");
  expect(select).toHaveTextContent("Claude Code (1)");
  expect(select).toHaveTextContent("Pi (1)");
});

test("project_scope_without_sessions_explains_the_scope_instead_of_offering_a_full_scan", async () => {
  vi.mocked(api.listSessions).mockResolvedValue([
    session({ title: "别的项目会话", projectPath: "D:/other" }),
  ]);
  vi.mocked(api.sessionProviders).mockResolvedValue([provider({ sessionCount: 1 })]);

  render(<SessionsPanel project={project()} />);

  expect(await screen.findByText("这个项目还没有会话")).toBeInTheDocument();
  expect(screen.getByText(/D:\\repos\\ai\\App 及其子目录/)).toBeInTheDocument();
  // 索引本身非空：不能退化成「还没有会话索引」诱导用户跑分钟级全量扫描。
  expect(screen.queryByRole("button", { name: "扫描本机会话" })).not.toBeInTheDocument();
});

test("project_scope_search_hits_are_filtered_to_the_project", async () => {
  const inside = session({ sessionId: "in", title: "项目内命中", projectPath: "D:/repos/ai/App" });
  const outside = session({ sessionId: "out", title: "项目外命中", projectPath: "D:/other" });
  vi.mocked(api.listSessions).mockResolvedValue([inside, outside]);
  vi.mocked(api.sessionProviders).mockResolvedValue([provider({ sessionCount: 2 })]);
  vi.mocked(api.searchSessions).mockResolvedValue([
    { session: inside, index: 1, seq: 1, snippet: "项目内片段" },
    { session: outside, index: 1, seq: 1, snippet: "项目外片段" },
  ]);

  render(<SessionsPanel project={project()} />);

  const input = await screen.findByLabelText("搜索会话内容");
  fireEvent.change(input, { target: { value: "命中" } });
  fireEvent.keyDown(input, { key: "Enter" });

  expect(await screen.findByText("项目内片段")).toBeInTheDocument();
  expect(screen.queryByText("项目外片段")).not.toBeInTheDocument();
});