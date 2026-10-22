import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { save } from "@tauri-apps/plugin-dialog";
import { SessionReader } from "../components/sessions/SessionReader";
import { api } from "../lib/api";
import type {
  SessionDetail,
  SessionMessage,
  SessionMessageKind,
  SessionSummary,
} from "../types";

vi.mock("../lib/api", () => ({
  api: {
    getSession: vi.fn(),
    searchInSession: vi.fn(),
    sessionResume: vi.fn(),
    exportSessionMarkdown: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

const SESSION: SessionSummary = {
  agentId: "claude",
  sessionId: "s1",
  title: "登录排查",
  projectPath: "D:/repo",
  sourcePath: "D:/repo/.claude/s1.jsonl",
  createdAt: null,
  updatedAt: new Date("2026-10-22T10:00:00").getTime(),
  messageCount: 4,
  parseError: null,
};

function message(
  seq: number,
  kind: SessionMessageKind,
  content: string,
  toolName: string | null = null,
): SessionMessage {
  return {
    seq,
    role: kind === "tool_result" ? "tool" : "assistant",
    kind,
    content,
    toolName,
  };
}

function detail(messages: SessionMessage[], total = messages.length): SessionDetail {
  return { session: SESSION, messages, total };
}

function renderReader() {
  return render(
    <SessionReader session={SESSION} agentName="Claude Code" onBack={() => {}} />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSession).mockResolvedValue(detail([]));
  vi.mocked(api.searchInSession).mockResolvedValue([]);
  vi.mocked(api.sessionResume).mockResolvedValue({
    agentId: "claude",
    sessionId: "s1",
    cwd: "D:/repo",
    command: 'cd "D:/repo" && claude --resume s1',
  });
  vi.mocked(api.exportSessionMarkdown).mockResolvedValue("D:/out/a.md");
  vi.mocked(save).mockResolvedValue(null);
});

test("reader_renders_thinking_tool_call_and_tool_result_as_distinct_blocks", async () => {
  vi.mocked(api.getSession).mockResolvedValue(
    detail([
      message(0, "text", "普通正文"),
      message(1, "thinking", "内部推理"),
      message(2, "tool_call", '{"command":"ls"}', "bash"),
      message(3, "tool_result", "a.rs", "bash"),
    ]),
  );

  const { container } = renderReader();

  await screen.findByText("普通正文");
  expect(screen.getByText("思考")).toBeInTheDocument();
  expect(screen.getByText("工具调用 · bash")).toBeInTheDocument();
  expect(screen.getByText("工具结果 · bash")).toBeInTheDocument();
  expect(screen.getByText("a.rs").tagName).toBe("PRE");
  expect(container.querySelector("pre")).not.toBeNull();
});

test("reader_paginates_without_losing_previously_loaded_messages", async () => {
  const first = Array.from({ length: 200 }, (_, index) =>
    message(index, "text", `消息${index}`),
  );
  const second = Array.from({ length: 100 }, (_, index) =>
    message(200 + index, "text", `消息${200 + index}`),
  );
  vi.mocked(api.getSession).mockImplementation(
    async (_agentId, _sessionId, offset = 0) => ({
      session: SESSION,
      messages: offset === 200 ? second : first,
      total: 300,
    }),
  );

  renderReader();

  await screen.findByText("消息0");
  fireEvent.click(screen.getByRole("button", { name: "载入更多" }));
  await screen.findByText("消息299");

  expect(screen.getByText("消息0")).toBeInTheDocument();
  expect(api.getSession).toHaveBeenLastCalledWith(
    SESSION.agentId,
    SESSION.sessionId,
    200,
    200,
  );
  expect(screen.getByText("已载入 300 / 共 300 条")).toBeInTheDocument();
});

test("in_session_search_jumps_to_the_matching_message", async () => {
  const initial = Array.from({ length: 5 }, (_, index) =>
    message(index, "text", `初始${index}`),
  );
  const jump = Array.from({ length: 50 }, (_, index) => {
    const seq = 100 + index;
    return message(seq, "text", seq === 120 ? "跳到这里" : `行${seq}`);
  });
  vi.mocked(api.getSession).mockImplementation(
    async (_agentId, _sessionId, offset = 0) => ({
      session: SESSION,
      messages: offset === 100 ? jump : initial,
      total: 130,
    }),
  );
  vi.mocked(api.searchInSession).mockResolvedValue([
    { index: 120, seq: 120, snippet: "命中片段" },
  ]);

  renderReader();

  await screen.findByText("初始0");
  fireEvent.change(screen.getByPlaceholderText("在会话内查找"), {
    target: { value: "命中" },
  });
  fireEvent.click(screen.getByRole("button", { name: "查找" }));
  fireEvent.click(await screen.findByText("命中片段"));

  await screen.findByText("跳到这里");
  expect(api.getSession).toHaveBeenLastCalledWith(
    SESSION.agentId,
    SESSION.sessionId,
    100,
    50,
  );
  expect(screen.getByText("跳到这里").closest("article")).toHaveAttribute(
    "data-highlighted",
    "true",
  );

  fireEvent.click(screen.getByRole("button", { name: "载入更多" }));
  await vi.waitFor(() =>
    expect(api.getSession).toHaveBeenLastCalledWith(
      SESSION.agentId,
      SESSION.sessionId,
      150,
      200,
    ),
  );
});

test("search_terms_are_rendered_as_text_not_html", async () => {
  vi.mocked(api.getSession).mockResolvedValue(
    detail([message(0, "text", "<script>alert(1)</script>")]),
  );

  const { container } = renderReader();

  expect(
    await screen.findByText("<script>alert(1)</script>"),
  ).toBeInTheDocument();
  expect(container.querySelector("script")).toBeNull();
});

test("truncated_message_shows_the_backend_marker", async () => {
  vi.mocked(api.getSession).mockResolvedValue(
    detail([message(0, "text", "正文\n…[已截断，原文 99999 字符]")]),
  );

  renderReader();

  expect(
    await screen.findByText(/已截断，原文 99999 字符/),
  ).toBeInTheDocument();
});

test("export_asks_for_a_destination_and_reports_the_path", async () => {
  vi.mocked(save).mockResolvedValue("D:/out/a.md");
  renderReader();

  fireEvent.click(await screen.findByRole("button", { name: "导出 Markdown" }));

  expect(await screen.findByText("已导出到 D:/out/a.md")).toBeInTheDocument();
  expect(api.exportSessionMarkdown).toHaveBeenCalledWith(
    "claude",
    "s1",
    "D:/out/a.md",
  );
});

test("cancelling_the_save_dialog_does_not_call_the_backend", async () => {
  vi.mocked(save).mockResolvedValue(null);
  renderReader();

  fireEvent.click(await screen.findByRole("button", { name: "导出 Markdown" }));
  await vi.waitFor(() => expect(save).toHaveBeenCalled());
  expect(api.exportSessionMarkdown).not.toHaveBeenCalled();
});

test("resume_command_is_copied_and_confirmed", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
  renderReader();

  fireEvent.click(
    await screen.findByRole("button", { name: "复制恢复命令" }),
  );

  await vi.waitFor(() =>
    expect(writeText).toHaveBeenCalledWith(
      'cd "D:/repo" && claude --resume s1',
    ),
  );
  expect(await screen.findByText("已复制")).toBeInTheDocument();
});

test("broken_image_falls_back_to_a_placeholder", async () => {
  vi.mocked(api.getSession).mockResolvedValue(
    detail([message(0, "image", "data:image/png;base64,bad")]),
  );

  renderReader();

  const image = await screen.findByRole("img");
  fireEvent.error(image);
  expect(await screen.findByText(/图片无法显示/)).toBeInTheDocument();
});