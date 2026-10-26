import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
    {
      assetType: "mcp",
      id: "pw",
      name: "Playwright",
      updatedAt: 1,
      hash: "h",
      description: "npx -y playwright",
      transport: "stdio",
    },
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

test("edits and saves an MCP server through the JSON view", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "编辑 Playwright" }));
  fireEvent.click(await screen.findByRole("button", { name: "JSON" }));
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
  fireEvent.click(await screen.findByRole("button", { name: "编辑 Playwright" }));
  fireEvent.click(await screen.findByRole("button", { name: "JSON" }));
  fireEvent.change(await screen.findByRole("textbox", { name: "JSON 编辑器" }), {
    target: { value: "{" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByText("JSON 无效")).toBeInTheDocument();
  expect(api.writeAsset).not.toHaveBeenCalled();
});

test("creates a new MCP entry from the form", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "添加 MCP" }));
  fireEvent.change(await screen.findByLabelText(/^ID/), { target: { value: "ctx" } });
  fireEvent.change(screen.getByLabelText(/^显示名称/), { target: { value: "Context7" } });
  fireEvent.change(screen.getByLabelText(/^命令/), { target: { value: "npx" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith(
      "mcp",
      "ctx",
      expect.stringContaining('"id": "ctx"'),
    ),
  );
});

test("deletes the MCP entry after confirm", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "Playwright 的更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "删除" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "删除 MCP" }));
  await waitFor(() => expect(api.deleteAsset).toHaveBeenCalledWith("mcp", "pw"));
});

test("keeps the entry when the delete is cancelled", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "Playwright 的更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "删除" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "取消" }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(api.deleteAsset).not.toHaveBeenCalled();
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

test("toggling an agent switch calls setMcpEnabled", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([agentStatus()]);
  render(<McpPanel />);
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
  expect(
    await screen.findByLabelText("Qoder：未实测，暂不提供开关"),
  ).toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Qoder 启用 pw" })).not.toBeInTheDocument();
});

test("an agent without that capability at all does not appear in the row", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus(),
    agentStatus({ id: "gemini", displayName: "Gemini CLI", capabilities: capabilities({}) }),
  ]);
  render(<McpPanel />);
  await screen.findByRole("checkbox", { name: "Claude Code 启用 pw" });
  expect(screen.queryByText(/Gemini CLI/)).not.toBeInTheDocument();
});

