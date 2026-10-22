import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { ImportDialog } from "../components/ImportDialog";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    importPreview: vi.fn(),
    importApply: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.importPreview).mockResolvedValue({
    mcpCount: 5,
    promptsCount: 5,
    skillsCount: 81,
    ignoredColumns: ["opencode"],
    missingSkillDirs: ["gone"],
    conflicts: ["mcp/pw"],
  });
  vi.mocked(api.importApply).mockResolvedValue({
    mcpImported: 5,
    promptsImported: 4,
    skillsImported: 81,
    unchanged: 0,
    skipped: 0,
    failed: ["prompt-x"],
    missingSkillDirs: ["gone"],
    ambiguousActive: ["claude"],
    preview: {
      mcpCount: 5,
      promptsCount: 5,
      skillsCount: 81,
      ignoredColumns: [],
      missingSkillDirs: [],
      conflicts: [],
    },
  });
});

test("previews counts and applies with the chosen policy", async () => {
  render(<ImportDialog onClose={vi.fn()} />);
  expect(await screen.findByText("MCP 5 条")).toBeInTheDocument();
  expect(screen.getByText("Skills 81 条")).toBeInTheDocument();
  expect(screen.getByText(/有记录但文件缺失的 skill：gone/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "开始导入" }));
  await waitFor(() => expect(api.importApply).toHaveBeenCalledWith("skip"));
});

test("tells the user cc-switch data was not modified", async () => {
  render(<ImportDialog onClose={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "开始导入" }));
  expect(await screen.findByText(/cc-switch 数据保持原样/)).toBeInTheDocument();
  // 失败与歧义必须摊开说，不能只报「成功」
  expect(screen.getByText(/失败：prompt-x/)).toBeInTheDocument();
  expect(screen.getByText(/这些 agent 有多条启用中的 prompt/)).toBeInTheDocument();
});

test("surfaces a preview failure instead of an empty dialog", async () => {
  vi.mocked(api.importPreview).mockRejectedValue("没找到 ~/.cc-switch/cc-switch.db");
  render(<ImportDialog onClose={vi.fn()} />);
  expect(await screen.findByText(/没找到/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "开始导入" })).toBeDisabled();
});
