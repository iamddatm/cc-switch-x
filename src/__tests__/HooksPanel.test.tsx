import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { HooksPanel } from "../panels/HooksPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";
import { HOOK_EVENTS } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
    listAgents: vi.fn(),
    setHookEnabled: vi.fn(),
    foreignHookCount: vi.fn(),
    hooksGateStatus: vi.fn(),
  },
}));

const YAML = [
  "name: fmt",
  "event: PreToolUse",
  "matcher: Bash",
  "command: cargo fmt --all && echo done",
  "command_windows: null",
  "timeout: 30",
  "",
].join("\n");

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "hook", id: "fmt", name: "fmt", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue(YAML);
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ capabilities: capabilities({ hook: "verified", mcp: "verified" }) }),
    agentStatus({
      id: "qoder",
      displayName: "Qoder",
      capabilities: capabilities({ hook: "unverified" }),
    }),
    agentStatus({ id: "gemini", displayName: "Gemini CLI", capabilities: capabilities({}) }),
  ]);
  vi.mocked(api.foreignHookCount).mockResolvedValue(3);
  vi.mocked(api.hooksGateStatus).mockResolvedValue(null);
});

async function openFmt() {
  render(<HooksPanel />);
  fireEvent.click(await screen.findByText("fmt"));
  await screen.findByLabelText("hook 命令");
}

test("hooks_panel_shows_the_full_command_without_truncation", async () => {
  await openFmt();
  // spec 4.2：hook 是可执行内容，UI 必须展示完整命令，不许省略号截断
  const shown = screen.getByTestId("hook-command");
  expect(shown.textContent).toContain("cargo fmt --all && echo done");
  expect(shown.textContent).not.toContain("…");
  expect(shown.textContent).not.toContain("...");
  expect(window.getComputedStyle(shown).textOverflow).not.toBe("ellipsis");
  // 表单里也是全文，不是只读摘要
  expect(screen.getByLabelText("hook 命令")).toHaveValue("cargo fmt --all && echo done");
});

test("event_dropdown_only_offers_known_events", async () => {
  await openFmt();
  const dropdown = screen.getByLabelText("hook 事件");
  const options = Array.from(dropdown.querySelectorAll("option")).map((node) => node.value);
  expect(options).toEqual(HOOK_EVENTS);
  expect(options).toHaveLength(28);
  // 拼错的事件名根本选不出来
  expect(options).not.toContain("PreTool");
  expect(dropdown).toHaveValue("PreToolUse");
});

test("unverified_capability_shows_no_switch_but_says_why", async () => {
  await openFmt();
  expect(screen.getByText("Qoder：未实测，暂不提供开关")).toBeInTheDocument();
  expect(screen.queryByLabelText("Qoder 启用 fmt")).not.toBeInTheDocument();
  // 没有这一类落点的 agent 整行不出现
  expect(screen.queryByText(/Gemini CLI/)).not.toBeInTheDocument();
  const checkbox = screen.getByRole("checkbox", { name: "Claude Code 启用 fmt" });
  expect(checkbox).toBeEnabled();
});

test("foreign_hooks_are_named_per_file_and_the_gate_is_shown", async () => {
  await openFmt();
  // 数量按 agent 分开报：每个 agent 的 hook 文件是不同的一份，加成总数会指错文件
  expect(
    screen.getByText("Claude Code 的 hook 文件里还有 3 条不是本工具写的 hook，我们不会动它们。"),
  ).toBeInTheDocument();

  vi.mocked(api.hooksGateStatus).mockResolvedValue({
    file: "C:/x/.codex/config.toml",
    why: "请在该文件里把 [features].hooks 设为 true（我们不代改这个开关）",
  });
  render(<HooksPanel />);
  fireEvent.click(await screen.findByText("fmt"));
  expect(await screen.findByRole("alert")).toHaveTextContent("[features].hooks");
});

test("saving_writes_the_canonical_yaml_not_a_reshaped_document", async () => {
  await openFmt();
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "hook",
    id: "fmt",
    name: "fmt",
    updatedAt: 2,
    hash: "h2",
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await vi.waitFor(() => expect(api.writeAsset).toHaveBeenCalled());
  const [kind, id, content] = vi.mocked(api.writeAsset).mock.calls[0];
  expect(kind).toBe("hook");
  expect(id).toBe("fmt");
  expect(content).toContain("event: PreToolUse");
  expect(content).toContain("timeout: 30");
  expect(content).toContain("command_windows: null");
});
