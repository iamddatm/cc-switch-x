import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { McpPanel } from "../panels/McpPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
    listAgents: vi.fn(),
    setMcpEnabled: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "mcp", id: "pw", name: "Playwright", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue(
    '{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}',
  );
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "mcp",
    id: "pw",
    name: "Playwright",
    updatedAt: 2,
    hash: "h2",
  });
  vi.mocked(api.deleteAsset).mockResolvedValue(true);
  // 面板现在还会读 agent 列表并下发开关，默认给空实现，个别用例再覆盖
  vi.mocked(api.listAgents).mockResolvedValue([]);
  vi.mocked(api.setMcpEnabled).mockResolvedValue({
    written: 1,
    skipped: 0,
    ok: true,
    backupDir: "C:/x/backups/1",
    error: null,
    methodUsed: "file",
    note: null,
  });
});

test("lists, edits and saves an MCP server", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  const editor = await screen.findByRole("textbox", { name: "JSON 编辑器" });
  fireEvent.change(editor, {
    target: { value: '{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}' },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith("mcp", "pw", expect.any(String)),
  );
});

test("shows an error for invalid JSON", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  fireEvent.change(await screen.findByRole("textbox", { name: "JSON 编辑器" }), {
    target: { value: "{" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByText("JSON 无效")).toBeInTheDocument();
  expect(api.writeAsset).not.toHaveBeenCalled();
});

test("creates a new MCP entry", async () => {
  render(<McpPanel />);
  fireEvent.change(await screen.findByLabelText("新 MCP ID"), { target: { value: "ctx" } });
  fireEvent.click(screen.getByRole("button", { name: "新建" }));
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith("mcp", "ctx", expect.stringContaining('"id": "ctx"')),
  );
});

test("deletes the selected MCP entry after confirm", async () => {
  vi.spyOn(window, "confirm").mockReturnValue(true);
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  await screen.findByRole("textbox", { name: "JSON 编辑器" });
  fireEvent.click(screen.getByRole("button", { name: "删除" }));
  await waitFor(() => expect(api.deleteAsset).toHaveBeenCalledWith("mcp", "pw"));
  expect(await screen.findByText("选择左侧 MCP 服务器进行编辑。")).toBeInTheDocument();
});

test("keeps the entry when the delete is cancelled", async () => {
  vi.spyOn(window, "confirm").mockReturnValue(false);
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  await screen.findByRole("textbox", { name: "JSON 编辑器" });
  fireEvent.click(screen.getByRole("button", { name: "删除" }));
  await sleep();
  expect(api.deleteAsset).not.toHaveBeenCalled();
  expect(screen.getByRole("textbox", { name: "JSON 编辑器" })).toBeInTheDocument();
});

test("toggling an agent switch calls setMcpEnabled", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([agentStatus()]);
  vi.mocked(api.setMcpEnabled).mockResolvedValue({
    written: 1,
    skipped: 0,
    ok: true,
    backupDir: "C:/x/backups/1",
    error: null,
    methodUsed: "file",
    note: null,
  });
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  const checkbox = await screen.findByRole("checkbox", { name: "Claude Code 启用 pw" });
  fireEvent.click(checkbox);
  await waitFor(() => expect(api.setMcpEnabled).toHaveBeenCalledWith("pw", "claude", true));
});

test("an unverified capability shows a note instead of a switch", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus(),
    agentStatus({
      id: "qoder",
      displayName: "Qoder",
      capabilities: capabilities({ mcp: "unverified" }),
    }),
  ]);
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  expect(await screen.findByText(/Qoder：未实测，暂不提供开关/)).toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Qoder 启用 pw" })).not.toBeInTheDocument();
});

test("an agent without that capability at all does not appear in the row", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus(),
    agentStatus({ id: "gemini", displayName: "Gemini CLI", capabilities: capabilities({}) }),
  ]);
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  await screen.findByRole("checkbox", { name: "Claude Code 启用 pw" });
  expect(screen.queryByText(/Gemini CLI/)).not.toBeInTheDocument();
});

test("an agent with unmeasured restart semantics still warns about restart", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ mcpNeedsRestart: null, enabledMcp: ["pw"] }),
  ]);
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  await screen.findByRole("checkbox", { name: "Claude Code 启用 pw" });
  // 已勾选 + 没有实测依据 → 提示需要重启，不能当成已生效
  expect(screen.getByText("需重启或 reload")).toBeInTheDocument();
});

async function sleep() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}
