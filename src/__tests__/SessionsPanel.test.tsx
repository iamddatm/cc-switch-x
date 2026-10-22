import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SessionsPanel } from "../panels/SessionsPanel";
import { api } from "../lib/api";
import type { SessionProviderStatus, SessionSummary } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listSessions: vi.fn(),
    sessionProviders: vi.fn(),
    scanSessions: vi.fn(),
    searchSessions: vi.fn(),
    getSession: vi.fn(),
    searchInSession: vi.fn(),
  },
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
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
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

test("unverified_provider_sessions_carry_a_warning_badge", async () => {
  vi.mocked(api.listSessions).mockResolvedValue([
    session({ title: "未实测会话" }),
  ]);
  vi.mocked(api.sessionProviders).mockResolvedValue([
    provider({ verified: false, sessionCount: 1 }),
  ]);

  render(<SessionsPanel />);

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

  const input = await screen.findByPlaceholderText("搜索会话内容");
  fireEvent.change(input, { target: { value: "关键字" } });
  fireEvent.click(screen.getByRole("button", { name: "搜索" }));

  expect(await screen.findByText("这里包含关键字片段")).toBeInTheDocument();
  expect(screen.getByText("登录问题")).toBeInTheDocument();
  expect(screen.getByText("会话内命中")).toBeInTheDocument();
  expect(api.searchSessions).toHaveBeenCalledWith("关键字");
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

  const input = await screen.findByPlaceholderText("搜索会话内容");
  fireEvent.change(input, { target: { value: "关键字" } });
  fireEvent.click(screen.getByRole("button", { name: "搜索" }));
  await screen.findByText("关键字片段");
  fireEvent.click(screen.getByRole("button", { name: /登录问题/ }));
  expect(await screen.findByText("会话正文")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "返回列表" }));
  expect(screen.getByPlaceholderText("搜索会话内容")).toHaveValue("关键字");
  expect(screen.getByText("关键字片段")).toBeInTheDocument();
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