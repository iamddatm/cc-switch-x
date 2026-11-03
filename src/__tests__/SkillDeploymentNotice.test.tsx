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

const vanished: SkillDeploymentIssue = {
  ...hijacked,
  skillId: "tdd",
  status: "missing",
  detail: "落点上已经没有这个目录（被外部删掉，或从未部署成功）",
};

test("勾选与落点一致时什么都不说", () => {
  render(
    <SkillDeploymentNotice issues={[]} busy={false} onRepair={vi.fn()} onRescan={vi.fn()} />,
  );
  expect(screen.queryByText(/个 Skill 已勾选/)).not.toBeInTheDocument();
});

test("报出被别处接走的落点，并说清一键处理会做什么", () => {
  render(
    <SkillDeploymentNotice
      issues={[hijacked, vanished]}
      busy={false}
      onRepair={vi.fn()}
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
  // 横幅只说会做什么，「怎么做的」交给确认框；抢不回来的必须原样保留并说明原因。
  expect(screen.getByText(/抢不回来的原样保留并逐条说明原因/)).toBeInTheDocument();
});

test("一键处理把处置交给面板，不在横幅里自己动盘", () => {
  const onRepair = vi.fn();
  render(
    <SkillDeploymentNotice issues={[hijacked]} busy={false} onRepair={onRepair} onRescan={vi.fn()} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "一键处理" }));
  expect(onRepair).toHaveBeenCalledTimes(1);
});

test("重新检查把重扫交给面板，不在横幅里自己扫一遍", () => {
  const onRescan = vi.fn();
  render(
    <SkillDeploymentNotice issues={[hijacked]} busy={false} onRepair={vi.fn()} onRescan={onRescan} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "重新检查" }));
  expect(onRescan).toHaveBeenCalledTimes(1);
});

test("处置进行中两个入口都锁住，避免重复批处理", () => {
  render(
    <SkillDeploymentNotice issues={[hijacked]} busy onRepair={vi.fn()} onRescan={vi.fn()} />,
  );
  expect(screen.getByRole("button", { name: "一键处理" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "重新检查" })).toBeDisabled();
});
