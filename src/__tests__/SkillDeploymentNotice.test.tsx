import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { SkillDeploymentNotice } from "../components/skills/SkillDeploymentNotice";
import { api } from "../lib/api";
import type { SkillDeploymentIssue } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listDrifts: vi.fn().mockResolvedValue([]),
    rescanDrift: vi.fn().mockResolvedValue([]),
    verifySkillDeployments: vi.fn().mockResolvedValue([]),
    resolveDrift: vi.fn(),
  },
}));

const hijacked: SkillDeploymentIssue = {
  agentId: "claude",
  skillId: "brainstorming",
  status: "foreign",
  targetPath: "C:/Users/me/.claude/skills/brainstorming",
  detail: "落点现在是指向 C:/Users/me/.cc-switch/skills/brainstorming 的链接，不再是我们的真源",
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listDrifts).mockResolvedValue([]);
  vi.mocked(api.rescanDrift).mockResolvedValue([]);
  vi.mocked(api.verifySkillDeployments).mockResolvedValue([]);
});

test("勾选与落点一致时什么都不说", async () => {
  render(<SkillDeploymentNotice />);
  await screen.findByText(() => true, { selector: ".max-h-32" }).catch(() => null);
  expect(document.querySelector("ul")).toBeNull();
});

test("报出被别处接走的落点，并给出恢复方向", async () => {
  vi.mocked(api.verifySkillDeployments).mockResolvedValue([hijacked]);
  render(<SkillDeploymentNotice />);

  expect(await screen.findByText(/1 个 Skill 已勾选，但落点上的内容不是本工具部署的/)).toBeInTheDocument();
  expect(
    await screen.findByText(/claude \/ brainstorming：落点现在是指向 .*\.cc-switch\/skills\/brainstorming/),
  ).toBeInTheDocument();
  expect(screen.getByText(/先在对侧工具里取消同名条目/)).toBeInTheDocument();
});

test("重新检查真的再去看一次盘", async () => {
  vi.mocked(api.verifySkillDeployments).mockResolvedValue([hijacked]);
  render(<SkillDeploymentNotice />);
  await screen.findByText(/1 个 Skill 已勾选/);

  fireEvent.click(screen.getByRole("button", { name: "重新检查" }));
  await vi.waitFor(() => expect(api.rescanDrift).toHaveBeenCalled());
  expect(api.verifySkillDeployments).toHaveBeenCalledTimes(2);
});
