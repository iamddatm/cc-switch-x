import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { McpPanel } from "../panels/McpPanel";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
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
