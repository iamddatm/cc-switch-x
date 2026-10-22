import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { SkillsPanel } from "../panels/SkillsPanel";
import { api } from "../lib/api";
import { agentStatus } from "../test/agents";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    listAgents: vi.fn(),
    setSkillEnabled: vi.fn(),
  },
}));

const claude = agentStatus();

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "skill", id: "demo", name: "demo", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue("---\nname: demo\n---\n\n正文\n");
  vi.mocked(api.listAgents).mockResolvedValue([claude]);
  vi.mocked(api.setSkillEnabled).mockResolvedValue("junction");
});

test("shows the link kind the backend actually used", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByText("demo"));
  fireEvent.click(await screen.findByRole("checkbox", { name: "Claude Code 启用 demo" }));
  expect(await screen.findByText("落地方式：junction")).toBeInTheDocument();
  expect(api.setSkillEnabled).toHaveBeenCalledWith("demo", "claude", true);
  // 点一次只该发一次请求：控件嵌在 <label> 里时真实浏览器会触发两次 change
  expect(api.setSkillEnabled).toHaveBeenCalledTimes(1);
});

test("a rejected deployment shows the reason instead of a fake success", async () => {
  vi.mocked(api.setSkillEnabled).mockRejectedValue(
    "非法输入：claude 的 skills/demo 位置上是别人建的目录，未做任何改动",
  );
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByText("demo"));
  fireEvent.click(await screen.findByRole("checkbox", { name: "Claude Code 启用 demo" }));
  expect(await screen.findByText(/别人建的目录/)).toBeInTheDocument();
  // 只查动态的那条「落地方式：xxx」，面板底部的静态说明文字不算
  expect(screen.queryByText(/^落地方式：/)).not.toBeInTheDocument();
});
