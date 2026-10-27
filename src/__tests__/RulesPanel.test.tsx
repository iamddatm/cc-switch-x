import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { RulesPanel } from "../panels/RulesPanel";
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
    setRuleEnabled: vi.fn(),
    scanRules: vi.fn(),
    listDrifts: vi.fn(),
    rescanDrift: vi.fn(),
    resolveDrift: vi.fn(),
  },
}));

const RULE = "---\nname: style\ndescription: 写法\npaths:\n- src/**\n---\n\n正文\n";

const drift: DriftRecord = {
  id: 7,
  assetType: "rule",
  assetId: "style",
  agentId: "qoder-cn",
  kind: "modified",
  detectedAt: 1,
  targetPath: "C:/x/.qoder-cn/rules/style.md",
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "rule", id: "style", name: "style", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue(RULE);
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ capabilities: capabilities({ rule: "verified", mcp: "verified" }) }),
    agentStatus({
      id: "qoder-cn",
      displayName: "Qoder CN",
      capabilities: capabilities({ rule: "unverified", mcp: "verified" }),
    }),
    agentStatus({ id: "gemini", displayName: "Gemini CLI", capabilities: capabilities({ mcp: "verified" }) }),
  ]);
  vi.mocked(api.listDrifts).mockResolvedValue([]);
  vi.mocked(api.rescanDrift).mockResolvedValue([]);
});

afterEach(cleanup);

async function openRule() {
  render(<RulesPanel />);
  fireEvent.click(await screen.findByText("style"));
  await screen.findByLabelText("规则正文");
}

test("drift_banner_offers_both_resolutions_and_no_silent_write", async () => {
  vi.mocked(api.listDrifts).mockResolvedValue([drift]);
  await openRule();

  const region = await screen.findByRole("region", { name: "落盘漂移" });
  expect(region).toHaveTextContent("磁盘与真源不一致");
  expect(region).toHaveTextContent("C:/x/.qoder-cn/rules/style.md");
  // 处置只有两条路，没有「忽略 / 强制覆盖」这种第三个选项
  expect(screen.getByRole("button", { name: "采纳磁盘" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "重刷真源" })).toBeEnabled();
  expect(screen.queryByRole("button", { name: /忽略|强制|知道了/ })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "采纳磁盘" }));
  await vi.waitFor(() =>
    expect(api.resolveDrift).toHaveBeenCalledWith(7, "adoptDisk", "qoder-cn"),
  );
  // 面板自己没有再发一次写请求：覆盖只能发生在被选中的那一条路径里
  expect(api.writeAsset).not.toHaveBeenCalled();
  expect(api.setRuleEnabled).not.toHaveBeenCalled();
});

test("repushing_from_the_banner_goes_through_the_write_pipeline", async () => {
  vi.mocked(api.listDrifts).mockResolvedValue([drift]);
  vi.mocked(api.resolveDrift).mockResolvedValue({
    kind: "repushed",
    path: "C:/x/.qoder-cn/rules/style.md",
    written: 1,
    skipped: 0,
  });
  await openRule();
  fireEvent.click(await screen.findByRole("button", { name: "重刷真源" }));
  await vi.waitFor(() =>
    expect(api.resolveDrift).toHaveBeenCalledWith(7, "repushSource", "qoder-cn"),
  );
  expect(api.rescanDrift).toHaveBeenCalled();
});

test("a_missing_drift_hides_adopt_disk_because_there_is_nothing_to_adopt", async () => {
  vi.mocked(api.listDrifts).mockResolvedValue([{ ...drift, kind: "missing" }]);
  await openRule();
  expect(screen.getByRole("button", { name: "采纳磁盘" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "重刷真源" })).toBeEnabled();
});

test("unverified_capability_shows_no_switch_but_says_why", async () => {
  await openRule();
  expect(screen.getByText("Qoder CN：未实测，暂不提供开关")).toBeInTheDocument();
  expect(screen.queryByLabelText("Qoder CN 启用 style")).not.toBeInTheDocument();
  // 没有 rules 落点的 agent（supported=false）整行不出现，而不是给一个禁用开关
  expect(screen.queryByText(/Gemini CLI/)).not.toBeInTheDocument();
  expect(screen.queryByText("Gemini CLI：未实测，暂不提供开关")).not.toBeInTheDocument();
});

test("a_drift_error_from_the_backend_is_shown_when_toggling", async () => {
  await openRule();
  vi.mocked(api.setRuleEnabled).mockRejectedValue(
    new Error("有未解决的漂移：qoder-cn 的 rule/style 有未解决的漂移：先选「采纳磁盘」或「重刷真源」"),
  );
  const box = screen.getByRole("checkbox", { name: "Claude Code 启用 style" });
  fireEvent.click(box);
  expect(await screen.findByText(/有未解决的漂移/)).toBeInTheDocument();
  // 被拒之后没有偷偷再发一次
  expect(api.setRuleEnabled).toHaveBeenCalledTimes(1);
});

test("saving_sends_the_canonical_rule_file", async () => {
  await openRule();
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "rule",
    id: "style",
    name: "style",
    updatedAt: 2,
    hash: "h2",
  });
  fireEvent.change(screen.getByLabelText("规则生效范围"), { target: { value: "src/**, docs/**" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await vi.waitFor(() => expect(api.writeAsset).toHaveBeenCalled());
  const content = vi.mocked(api.writeAsset).mock.calls[0][2];
  expect(content).toBe(
    "---\nname: style\ndescription: 写法\npaths:\n- src/**\n- docs/**\n---\n\n正文\n",
  );
});

test("scanning_adds_up_every_agent_and_skips_the_unsupported_one", async () => {
  await openRule();
  vi.mocked(api.scanRules)
    .mockResolvedValueOnce({
      imported: 1,
      updated: 0,
      unchanged: 2,
      skipped: [{ file: "好.md", reason: "文件名推导不出合法 ID" }],
    })
    .mockResolvedValueOnce({
      imported: 2,
      updated: 1,
      unchanged: 0,
      skipped: [{ file: "dup.md", reason: "已有同名真源" }],
    });
  fireEvent.click(screen.getByRole("button", { name: "从 agent 目录扫描导入" }));
  const line = await screen.findByText(/合计/);
  // 每个 agent 的结果相加，而不是最后一个把前面的挤掉
  expect(line).toHaveTextContent("新增 3、更新 1、未变 2、跳过 2");
  expect(line).toHaveTextContent("好.md");
  expect(line).toHaveTextContent("dup.md");
  // gemini 没有 rule 落点，不该被拿去扫
  expect(api.scanRules).toHaveBeenCalledTimes(2);
});
