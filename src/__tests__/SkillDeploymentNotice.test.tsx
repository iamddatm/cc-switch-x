import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { SkillDeploymentNotice } from "../components/skills/SkillDeploymentNotice";
import type { SkillDeploymentIssue } from "../types";

const hijacked: SkillDeploymentIssue = {
  agentId: "claude",
  skillId: "brainstorming",
  status: "foreign",
  targetPath: "C:/Users/me/.claude/skills/brainstorming",
  detail: "落点现在是指向 C:/Users/me/.cc-switch/skills/brainstorming 的链接，不再是我们的真源",
};

test("勾选与落点一致时什么都不说", () => {
  render(<SkillDeploymentNotice issues={[]} busy={false} onRescan={vi.fn()} />);
  expect(screen.queryByText(/个 Skill 已勾选/)).not.toBeInTheDocument();
});

test("报出被别处接走的落点，并给出两条处置方向", () => {
  render(
    <SkillDeploymentNotice
      issues={[hijacked, { ...hijacked, skillId: "tdd", status: "missing", detail: "落点上已经没有这个目录（被外部删掉，或从未部署成功）" }]}
      busy={false}
      onRescan={vi.fn()}
    />,
  );

  expect(
    screen.getByText(/2 个 Skill 已勾选，但落点上的内容不是本工具部署的/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/claude \/ brainstorming：落点现在是指向 .*\.cc-switch\/skills\/brainstorming/),
  ).toBeInTheDocument();
  expect(screen.getByText(/claude \/ tdd：落点上已经没有这个目录/)).toBeInTheDocument();
  expect(screen.getByText(/先在对侧工具里取消同名条目/)).toBeInTheDocument();
  expect(screen.getByText(/盘上别人的文件不会被删/)).toBeInTheDocument();
});

test("重新检查把重扫交给面板，不在横幅里自己扫一遍", () => {
  const onRescan = vi.fn();
  render(<SkillDeploymentNotice issues={[hijacked]} busy={false} onRescan={onRescan} />);
  fireEvent.click(screen.getByRole("button", { name: "重新检查" }));
  expect(onRescan).toHaveBeenCalledTimes(1);
});
