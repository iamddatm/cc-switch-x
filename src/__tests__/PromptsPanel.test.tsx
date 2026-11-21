import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { PromptsPanel } from "../panels/PromptsPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";
import type { DriftRecord } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    listAgents: vi.fn(),
    activatePrompt: vi.fn(),
    deactivatePrompt: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
    listDrifts: vi.fn(),
    rescanDrift: vi.fn(),
    verifySkillDeployments: vi.fn().mockResolvedValue([]),
    resolveDrift: vi.fn(),
    readPromptTarget: vi.fn().mockResolvedValue(null),
    openPromptTarget: vi.fn().mockResolvedValue(undefined),
    readPiPromptFile: vi.fn(),
    replacePiPromptFile: vi.fn(),
    deletePiPromptFile: vi.fn(),
    listPiPromptTemplates: vi.fn(),
    savePiPromptTemplate: vi.fn(),
    deletePiPromptTemplate: vi.fn(),
  },
}));

const claude = agentStatus();

const drift: DriftRecord = {
  id: 5,
  assetType: "prompt",
  assetId: "claude-team",
  agentId: "claude",
  kind: "modified",
  detectedAt: 1,
  targetPath: "C:/x/.claude/CLAUDE.md",
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "prompt", id: "claude-team", name: "team", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue("---\ntitle: team\n---\n\n正文\n");
  vi.mocked(api.listAgents).mockResolvedValue([claude]);
  vi.mocked(api.deactivatePrompt).mockResolvedValue(true);
  vi.mocked(api.listDrifts).mockResolvedValue([]);
  vi.mocked(api.rescanDrift).mockResolvedValue([]);
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "prompt",
    id: "team",
    name: "团队",
    updatedAt: 2,
    hash: "h2",
  });
});

test("creates a prompt with a generated id", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([]);
  render(<PromptsPanel />);
  const [add] = await screen.findAllByRole("button", { name: "添加提示词" });
  fireEvent.click(add);
  fireEvent.change(await screen.findByLabelText(/^名称/), {
    target: { value: "团队" },
  });
  fireEvent.change(screen.getByLabelText(/^正文/), {
    target: { value: "# 团队\n\n正文\n" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith(
      "prompt",
      "prompt",
      expect.stringContaining("title: 团队"),
    ),
  );
});

test("shows the config's last updated time, not the file mtime", async () => {
  const now = Date.now();
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "prompt",
      id: "claude-team",
      name: "team",
      updatedAt: now - 2 * 24 * 60 * 60 * 1000,
      mtime: now,
      hash: "h",
    },
  ]);
  render(<PromptsPanel />);
  expect(await screen.findByText(/更新于 2 天前/)).toBeInTheDocument();
});

test("opens the selected agent's prompt target file", async () => {
  render(<PromptsPanel />);
  const target = await screen.findByRole("button", {
    name: "目标文件：C:/x/.claude/CLAUDE.md",
  });
  fireEvent.click(target);
  await waitFor(() => expect(api.openPromptTarget).toHaveBeenCalledWith("claude"));
});

test("imports a prompt from an agent target file", async () => {
  vi.mocked(api.readPromptTarget).mockResolvedValue({
    path: "C:/x/.claude/CLAUDE.md",
    content: "现有规范\n",
  });
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "提示词的更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "从 Agent 目标文件导入" }));

  const targetPath = await screen.findByRole("button", { name: "C:/x/.claude/CLAUDE.md" });
  fireEvent.click(targetPath);
  await waitFor(() => expect(api.openPromptTarget).toHaveBeenCalledWith("claude"));

  fireEvent.click(screen.getByRole("button", { name: /导入选中/ }));

  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith(
      "prompt",
      "claude-code",
      expect.stringContaining("现有规范"),
    ),
  );
  await waitFor(() =>
    expect(api.activatePrompt).toHaveBeenCalledWith("claude-code", "claude", {
      confirm: true,
    }),
  );
});

test("the editor leaves target-file differences to the drift banner", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ activePrompt: "claude-team" }),
  ]);
  vi.mocked(api.listDrifts).mockResolvedValue([drift]);
  render(<PromptsPanel />);

  // 面板先报漂移，编辑表单里不再重复提示、也不给第三条替换路径
  const region = await screen.findByRole("region", { name: "落盘漂移" });
  expect(region).toHaveTextContent("文件被改过");
  const [edit] = await screen.findAllByRole("button", { name: "编辑 team" });
  fireEvent.click(edit);
  await screen.findByLabelText(/^正文/);
  expect(screen.queryByRole("button", { name: "替换成目标文件" })).not.toBeInTheDocument();
  // 打开编辑器不需要去读目标文件
  expect(api.readPromptTarget).not.toHaveBeenCalled();
});

test("pi splits prompts into library, system files and templates", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({
      id: "pi",
      displayName: "pi",
      promptPath: "C:/Users/me/.pi/agent/AGENTS.md",
    }),
  ]);
  vi.mocked(api.readPiPromptFile).mockImplementation(async (kind) =>
    kind === "system_append"
      ? { exists: true, revision: "rev-append", content: "旧约定\n" }
      : { exists: false, revision: "missing", content: "" },
  );
  vi.mocked(api.replacePiPromptFile).mockResolvedValue({
    exists: true,
    revision: "rev-new",
    content: "新约定\n",
  });
  render(<PromptsPanel />);

  // 三个区块都在；提示库仍是默认视图
  expect(await screen.findByRole("button", { name: "提示库" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "系统提示" }));

  expect(await screen.findByText("追加系统提示")).toBeInTheDocument();
  expect(screen.getByText("替换系统提示")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "编辑 APPEND_SYSTEM.md" }));

  const body = await screen.findByLabelText(/^正文/);
  expect(body).toHaveValue("旧约定\n");
  fireEvent.change(body, { target: { value: "新约定\n" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(api.replacePiPromptFile).toHaveBeenCalledWith(
      "system_append",
      "rev-append",
      "新约定\n",
    ),
  );
});

test("pi templates can be created from the templates section", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ id: "pi", displayName: "pi" }),
  ]);
  vi.mocked(api.listPiPromptTemplates).mockResolvedValue([]);
  vi.mocked(api.savePiPromptTemplate).mockResolvedValue({
    slug: "review",
    content: "检查\n",
    revision: "r1",
  });
  render(<PromptsPanel />);

  fireEvent.click(await screen.findByRole("button", { name: "模板" }));
  const [createTemplate] = await screen.findAllByRole("button", { name: "新建模板" });
  fireEvent.click(createTemplate);

  fireEvent.change(screen.getByLabelText(/^名称/), { target: { value: "review" } });
  fireEvent.change(screen.getByLabelText(/^模板正文/), { target: { value: "检查\n" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));

  await waitFor(() =>
    expect(api.savePiPromptTemplate).toHaveBeenCalledWith(
      "review",
      undefined,
      "missing",
      "检查\n",
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

test("a_blocked_activation_surfaces_the_drift_banner_instead_of_a_bare_error", async () => {
  vi.mocked(api.activatePrompt).mockRejectedValue(
    new Error(
      "有未解决的漂移：claude 的 prompt/claude-team 有未解决的漂移：先选「采纳磁盘」或「重刷真源」",
    ),
  );
  vi.mocked(api.rescanDrift).mockResolvedValue([drift]);
  render(<PromptsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "启用" }));

  const region = await screen.findByRole("region", { name: "落盘漂移" });
  expect(region).toHaveTextContent("C:/x/.claude/CLAUDE.md");
  expect(screen.getByRole("button", { name: "采纳磁盘" })).toBeEnabled();
  // 被挡住之后面板自己没有再发写请求，覆盖只能走横幅里的那两条路
  expect(api.activatePrompt).toHaveBeenCalledTimes(1);

  fireEvent.click(screen.getByRole("button", { name: "采纳磁盘" }));
  await waitFor(() =>
    expect(api.resolveDrift).toHaveBeenCalledWith(5, "adoptDisk", "claude"),
  );
});