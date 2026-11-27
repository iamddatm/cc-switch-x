import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { CommandsPanel } from "../panels/CommandsPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
    listAgents: vi.fn(),
    setCommandEnabled: vi.fn(),
    scanCommands: vi.fn(),
    listDrifts: vi.fn(),
    rescanDrift: vi.fn(),
    verifySkillDeployments: vi.fn().mockResolvedValue([]),
    resolveDrift: vi.fn(),
  },
}));

const COMMAND = "---\ndescription: 审查并合并\nallowed-tools: Bash\n---\n\n按步骤审查。\n";

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "command", id: "review", name: "review", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue(COMMAND);
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ capabilities: capabilities({ mcp: "verified", command: "verified" }) }),
    agentStatus({
      id: "qoder-cn",
      displayName: "Qoder CN",
      capabilities: capabilities({ mcp: "verified" }),
    }),
  ]);
  vi.mocked(api.listDrifts).mockResolvedValue([]);
  vi.mocked(api.rescanDrift).mockResolvedValue([]);
  vi.mocked(api.setCommandEnabled).mockResolvedValue("written");
});

afterEach(cleanup);

async function openCommand() {
  render(<CommandsPanel />);
  fireEvent.click(await screen.findByText("review"));
  return screen.findByLabelText("命令内容");
}

test("the editor round-trips the markdown verbatim", async () => {
  const editor = await openCommand();
  expect(editor).toHaveValue(COMMAND);

  fireEvent.change(editor, { target: { value: `${COMMAND}补充一句。\n` } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith("command", "review", `${COMMAND}补充一句。\n`),
  );
});

test("only agents with a verified command target get a switch", async () => {
  await openCommand();

  const claude = await screen.findByRole("checkbox", { name: "Claude Code 启用 review" });
  expect(claude).not.toBeChecked();
  // Qoder CN 注册表里没有命令落点：整行不出现，而不是给一个灰开关
  expect(screen.queryByText("Qoder CN")).not.toBeInTheDocument();

  fireEvent.click(claude);
  await waitFor(() => expect(api.setCommandEnabled).toHaveBeenCalledWith("review", "claude", true));
});

test("scanning asks the supported agent for its commands directory", async () => {
  vi.mocked(api.scanCommands).mockResolvedValue({
    imported: 1,
    updated: 0,
    unchanged: 0,
    skipped: [],
  });
  await openCommand();

  fireEvent.click(screen.getByRole("button", { name: "从 agent 目录扫描导入" }));

  await waitFor(() => expect(api.scanCommands).toHaveBeenCalledTimes(1));
  expect(api.scanCommands).toHaveBeenCalledWith("claude");
  await screen.findByText(/新增 1/);
});
