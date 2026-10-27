import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SubagentsPanel } from "../panels/SubagentsPanel";
import { api } from "../lib/api";
import { agentStatus, capabilities } from "../test/agents";
import type { DriftRecord } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
    listAgents: vi.fn(),
    setSubagentEnabled: vi.fn(),
    scanSubagents: vi.fn(),
    listDrifts: vi.fn(),
    rescanDrift: vi.fn(),
    resolveDrift: vi.fn(),
  },
}));

/** 真源就是后端 `render_subagent` 会写出的形状，用来锁住 parse↔render 的往返恒等。 */
const SUBAGENT = [
  "name: reviewer",
  "description: 只看代码不改",
  "instructions: |-",
  "  第一步：读 diff",
  "  第二步：列风险",
  "model: sonnet",
  "tools:",
  "- Read",
  "- Grep",
  "tool_extras: {}",
  "targets: []",
  "",
].join("\n");

const drift: DriftRecord = {
  id: 11,
  assetType: "subagent",
  assetId: "reviewer",
  agentId: "qoder-cn",
  kind: "modified",
  detectedAt: 1,
  targetPath: "C:/x/.qoder-cn/agents/reviewer.md",
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "subagent", id: "reviewer", name: "reviewer", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue(SUBAGENT);
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ capabilities: capabilities({ subagent: "verified", mcp: "verified" }) }),
    agentStatus({
      id: "qoder-cn",
      displayName: "Qoder CN",
      capabilities: capabilities({ subagent: "unverified", mcp: "verified" }),
    }),
    agentStatus({
      id: "gemini",
      displayName: "Gemini CLI",
      capabilities: capabilities({ mcp: "verified" }),
    }),
  ]);
  vi.mocked(api.listDrifts).mockResolvedValue([]);
  vi.mocked(api.rescanDrift).mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

async function openSubagent() {
  render(<SubagentsPanel />);
  fireEvent.click(await screen.findByText("reviewer"));
  await screen.findByLabelText("子代理指令");
}

test("saving_without_editing_writes_the_same_bytes_back", async () => {
  await openSubagent();
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "subagent",
    id: "reviewer",
    name: "reviewer",
    updatedAt: 2,
    hash: "h2",
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await vi.waitFor(() => expect(api.writeAsset).toHaveBeenCalled());
  const [type, id, content] = vi.mocked(api.writeAsset).mock.calls[0];
  expect(type).toBe("subagent");
  expect(id).toBe("reviewer");
  // 只改一行提示词不该把整块正文压成转义长行，也不该丢掉空集合的显式写法
  expect(content).toBe(SUBAGENT);
});

test("editing_the_tool_allowlist_renders_a_yaml_sequence", async () => {
  await openSubagent();
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "subagent",
    id: "reviewer",
    name: "reviewer",
    updatedAt: 2,
    hash: "h2",
  });
  fireEvent.change(screen.getByLabelText("子代理工具白名单"), {
    target: { value: "Read, Grep, Bash" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await vi.waitFor(() => expect(api.writeAsset).toHaveBeenCalled());
  expect(vi.mocked(api.writeAsset).mock.calls[0][2]).toBe(
    SUBAGENT.replace("- Grep\n", "- Grep\n- Bash\n"),
  );
});

test("drift_banner_offers_both_resolutions_and_no_silent_write", async () => {
  vi.mocked(api.listDrifts).mockResolvedValue([drift]);
  await openSubagent();

  const region = await screen.findByRole("region", { name: "落盘漂移" });
  expect(region).toHaveTextContent("磁盘与真源不一致");
  expect(region).toHaveTextContent("C:/x/.qoder-cn/agents/reviewer.md");
  expect(screen.getByRole("button", { name: "采纳磁盘" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "重刷真源" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: /忽略|强制|知道了/ })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "采纳磁盘" }));
  await vi.waitFor(() =>
    expect(api.resolveDrift).toHaveBeenCalledWith(11, "adoptDisk", "qoder-cn"),
  );
  expect(api.writeAsset).not.toHaveBeenCalled();
  expect(api.setSubagentEnabled).not.toHaveBeenCalled();
});

test("rescanning_from_the_banner_rechecks_the_disk", async () => {
  vi.mocked(api.listDrifts).mockResolvedValue([drift]);
  await openSubagent();
  fireEvent.click(await screen.findByRole("button", { name: "重新检查" }));
  await vi.waitFor(() => expect(api.rescanDrift).toHaveBeenCalled());
  expect(api.listDrifts).toHaveBeenCalledTimes(1);
});

test("deleting_the_source_file_needs_an_explicit_confirmation", async () => {
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
  await openSubagent();
  fireEvent.click(screen.getByRole("button", { name: "删除" }));
  await vi.waitFor(() => expect(confirm).toHaveBeenCalled());
  expect(api.deleteAsset).not.toHaveBeenCalled();
  expect(screen.queryByLabelText("子代理指令")).toBeInTheDocument();

  confirm.mockReturnValue(true);
  fireEvent.click(screen.getByRole("button", { name: "删除" }));
  await vi.waitFor(() => expect(api.deleteAsset).toHaveBeenCalledWith("subagent", "reviewer"));
  expect(await screen.findByText("选择左侧子代理进行编辑。")).toBeInTheDocument();
});

test("unverified_capability_shows_no_switch_but_says_why", async () => {
  await openSubagent();
  expect(screen.getByText("Qoder CN：未实测，暂不提供开关")).toBeInTheDocument();
  expect(screen.queryByLabelText("Qoder CN 启用 reviewer")).not.toBeInTheDocument();
  // 没有 subagent 落点的 agent（supported=false）整行不出现，而不是给一个禁用开关
  expect(screen.queryByText(/Gemini CLI/)).not.toBeInTheDocument();
  expect(screen.queryByText("Gemini CLI：未实测，暂不提供开关")).not.toBeInTheDocument();
});

test("a_drift_error_from_the_backend_is_shown_when_toggling", async () => {
  await openSubagent();
  vi.mocked(api.setSubagentEnabled).mockRejectedValue(
    new Error("有未解决的漂移：qoder-cn 的 subagent/reviewer 有未解决的漂移：先选「采纳磁盘」或「重刷真源」"),
  );
  const box = screen.getByRole("checkbox", { name: "Claude Code 启用 reviewer" });
  fireEvent.click(box);
  expect(await screen.findByText(/有未解决的漂移/)).toBeInTheDocument();
  expect(api.setSubagentEnabled).toHaveBeenCalledTimes(1);
});

test("scanning_adds_up_every_agent_it_can_reach", async () => {
  await openSubagent();
  vi.mocked(api.scanSubagents)
    .mockResolvedValueOnce({
      imported: 1,
      updated: 0,
      unchanged: 2,
      skipped: [{ file: "old.md", reason: "已存在同名真源" }],
    })
    .mockResolvedValueOnce({
      imported: 0,
      updated: 2,
      unchanged: 1,
      skipped: [],
    });
  fireEvent.click(screen.getByRole("button", { name: "从 agent 目录扫描导入" }));
  const line = await screen.findByText(/合计/);
  expect(line).toHaveTextContent("新增 1、更新 2、未变 3、跳过 1");
  // gemini 没有 subagent 落点，不该被拿去扫
  expect(api.scanSubagents).toHaveBeenCalledTimes(2);
});
