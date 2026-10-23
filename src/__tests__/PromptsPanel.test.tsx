import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { PromptsPanel } from "../panels/PromptsPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    listAgents: vi.fn(),
    activatePrompt: vi.fn(),
    deactivatePrompt: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
  },
}));

const claude = agentStatus();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "prompt", id: "claude-team", name: "team", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue("---\ntitle: team\n---\n\n正文\n");
  vi.mocked(api.listAgents).mockResolvedValue([claude]);
  vi.mocked(api.deactivatePrompt).mockResolvedValue(true);
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "prompt",
    id: "team",
    name: "团队",
    updatedAt: 2,
    hash: "h2",
  });
});

test("creates a prompt from the form", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([]);
  render(<PromptsPanel />);
  const [add] = await screen.findAllByRole("button", { name: "添加提示词" });
  fireEvent.click(add);
  fireEvent.change(await screen.findByLabelText(/^ID/), {
    target: { value: "team" },
  });
  fireEvent.change(screen.getByLabelText(/^名称/), {
    target: { value: "团队" },
  });
  fireEvent.change(screen.getByLabelText(/^正文/), {
    target: { value: "# 团队\n\n正文\n" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith(
      "prompt",
      "team",
      expect.stringContaining("title: 团队"),
    ),
  );
});

test("foreign target asks for a decision instead of overwriting", async () => {
  vi.mocked(api.activatePrompt).mockResolvedValue({
    kind: "needsConfirmation",
    path: "C:/x/.claude/CLAUDE.md",
  });
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "启用" }));

  expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "确认覆盖" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "把现有内容另存为一条 prompt" })).toBeInTheDocument();
  // 只问了一次；没确认前不能再发写请求
  expect(api.activatePrompt).toHaveBeenCalledTimes(1);
  expect(api.activatePrompt).toHaveBeenCalledWith("claude-team", "claude");
});

test("confirming sends an explicit overwrite", async () => {
  vi.mocked(api.activatePrompt)
    .mockResolvedValueOnce({ kind: "needsConfirmation", path: "C:/x/.claude/CLAUDE.md" })
    .mockResolvedValueOnce({ kind: "activated", path: "C:/x/.claude/CLAUDE.md" });
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "启用" }));
  fireEvent.click(await screen.findByRole("button", { name: "确认覆盖" }));

  await waitFor(() =>
    expect(api.activatePrompt).toHaveBeenLastCalledWith("claude-team", "claude", {
      confirm: true,
    }),
  );
  expect(await screen.findByText(/已写入/)).toBeInTheDocument();
});

test("salvaging stores the old content under a new prompt id", async () => {
  vi.mocked(api.activatePrompt)
    .mockResolvedValueOnce({ kind: "needsConfirmation", path: "C:/x/.claude/CLAUDE.md" })
    .mockResolvedValueOnce({ kind: "activated", path: "C:/x/.claude/CLAUDE.md" });
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "启用" }));
  fireEvent.click(
    await screen.findByRole("button", { name: "把现有内容另存为一条 prompt" }),
  );

  await waitFor(() => expect(api.activatePrompt).toHaveBeenCalledTimes(2));
  const options = vi.mocked(api.activatePrompt).mock.calls[1][2];
  expect(options?.saveExistingAs).toMatch(/^salvaged-/);
});

test("an unverified prompt target is named but not activatable", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    claude,
    agentStatus({
      id: "qoder",
      displayName: "Qoder",
      capabilities: capabilities({ prompt: "unverified", rule: "unverified" }),
    }),
  ]);
  render(<PromptsPanel />);
  fireEvent.change(await screen.findByLabelText("选择 Agent"), {
    target: { value: "qoder" },
  });

  expect(
    await screen.findByText(/Qoder 的提示词落点未实测/),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "启用" })).toBeDisabled();
  expect(api.activatePrompt).not.toHaveBeenCalled();
});