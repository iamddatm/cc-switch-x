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
});

test("foreign target asks for a decision instead of overwriting", async () => {
  vi.mocked(api.activatePrompt).mockResolvedValue({
    kind: "needsConfirmation",
    path: "C:/x/.claude/CLAUDE.md",
  });
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByText("team"));
  fireEvent.click(await screen.findByRole("button", { name: "激活到 Claude Code" }));

  expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "确认覆盖" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "把现有内容另存为一条 prompt" })).toBeInTheDocument();
  // 只问了一次；没确认前不能再发写请求
  expect(api.activatePrompt).toHaveBeenCalledTimes(1);
  expect(api.activatePrompt).toHaveBeenCalledWith("claude-team", "claude", undefined);
});

test("confirming sends an explicit overwrite", async () => {
  vi.mocked(api.activatePrompt)
    .mockResolvedValueOnce({ kind: "needsConfirmation", path: "C:/x/.claude/CLAUDE.md" })
    .mockResolvedValueOnce({ kind: "activated", path: "C:/x/.claude/CLAUDE.md" });
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByText("team"));
  fireEvent.click(await screen.findByRole("button", { name: "激活到 Claude Code" }));
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
  fireEvent.click(await screen.findByText("team"));
  fireEvent.click(await screen.findByRole("button", { name: "激活到 Claude Code" }));
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
  fireEvent.click(await screen.findByText("team"));

  expect(await screen.findByText(/Qoder：未实测，暂不提供激活/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "激活到 Qoder" })).not.toBeInTheDocument();
  expect(api.activatePrompt).not.toHaveBeenCalled();
});
