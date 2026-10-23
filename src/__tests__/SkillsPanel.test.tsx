import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";
import { SkillsPanel } from "../panels/SkillsPanel";
import { api } from "../lib/api";
import { agentStatus } from "../test/agents";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    importSkillDir: vi.fn(),
    listAgents: vi.fn(),
    setSkillEnabled: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
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
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "skill",
    id: "team-skill",
    name: "团队规范",
    updatedAt: 2,
    hash: "h2",
  });
  vi.mocked(api.importSkillDir).mockResolvedValue({
    assetType: "skill",
    id: "my-skill",
    name: "My Skill",
    updatedAt: 2,
    hash: "h2",
  });
  vi.mocked(open).mockResolvedValue(null);
});

test("shows the link kind the backend actually used", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "Claude Code 启用 demo" }));
  expect(await screen.findByText("实测：junction")).toBeInTheDocument();
  expect(api.setSkillEnabled).toHaveBeenCalledWith("demo", "claude", true);
  // 点一次只该发一次请求：控件嵌在 <label> 里时真实浏览器会触发两次 change
  expect(api.setSkillEnabled).toHaveBeenCalledTimes(1);
});

test("creates a skill from the form", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([]);
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "新建 Skill" }));
  fireEvent.change(await screen.findByLabelText(/ID（目录名）/), {
    target: { value: "team-skill" },
  });
  fireEvent.change(screen.getByLabelText(/^名称/), {
    target: { value: "团队规范" },
  });
  fireEvent.change(screen.getByLabelText(/正文（SKILL.md）/), {
    target: { value: "# 团队规范\n" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith(
      "skill",
      "team-skill",
      expect.stringContaining("name: 团队规范"),
    ),
  );
});

test("imports a skill directory picked through the native dialog", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([]);
  vi.mocked(open).mockResolvedValue("D:/skills/my-skill");
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "从文件夹导入" }));
  await waitFor(() => expect(api.importSkillDir).toHaveBeenCalledWith("D:/skills/my-skill"));
});

test("only enabled agents get a checkbox column", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    claude,
    agentStatus({ id: "pi", displayName: "Pi Coding Agent", enabled: false }),
  ]);
  render(<SkillsPanel />);
  expect(
    await screen.findByRole("checkbox", { name: "Claude Code 启用 demo" }),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("checkbox", { name: "Pi Coding Agent 启用 demo" }),
  ).not.toBeInTheDocument();
});

test("the unassigned filter ignores deployments of disabled agents", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({
      id: "pi",
      displayName: "Pi Coding Agent",
      enabled: false,
      enabledSkills: ["demo"],
    }),
  ]);
  render(<SkillsPanel />);
  await screen.findByText(/还没有启用任何 Agent/);
  fireEvent.change(screen.getByLabelText("按 Agent 筛选"), { target: { value: "off" } });
  expect(screen.getByRole("button", { name: "编辑 demo" })).toBeInTheDocument();
});

test("a rejected deployment shows the reason instead of a fake success", async () => {
  vi.mocked(api.setSkillEnabled).mockRejectedValue(
    "非法输入：claude 的 skills/demo 位置上是别人建的目录，未做任何改动",
  );
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "Claude Code 启用 demo" }));
  expect(await screen.findByText(/别人建的目录/)).toBeInTheDocument();
  // 失败时不能再显示一条假的「实测：xxx」
  expect(screen.queryByText(/^实测：/)).not.toBeInTheDocument();
});
