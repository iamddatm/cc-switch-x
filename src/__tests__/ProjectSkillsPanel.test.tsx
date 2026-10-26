import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { ProjectSkillsPanel } from "../panels/ProjectSkillsPanel";
import { api } from "../lib/api";
import type { ProjectSkillView, ProjectSkillsView, ProjectView } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listProjectSkills: vi.fn(),
    revealProjectFolder: vi.fn(),
    setProjectSkillTargets: vi.fn(),
    syncProjectSkills: vi.fn(),
    removeProjectSkillCopy: vi.fn(),
    uninstallProjectSkill: vi.fn(),
    setProjectAutoSync: vi.fn(),
    migrateProjectSkills: vi.fn(),
    adoptProjectSkill: vi.fn(),
    readProjectSkillBody: vi.fn(),
    writeProjectSkillBody: vi.fn(),
  },
}));

function projectView(overrides: Partial<ProjectView> = {}): ProjectView {
  return {
    id: "p1",
    path: "C:/work/demo",
    name: "demo",
    addedAt: "2026-10-01T00:00:00Z",
    lastOpenedAt: "2026-10-02T00:00:00Z",
    autoSync: true,
    exists: true,
    ...overrides,
  };
}

const CLAUDE_DIR = {
  key: "claude",
  label: "Claude Code",
  relativePath: ".claude/skills",
  agents: ["claude"],
  verified: true,
} as const;

const QODER_DIR = {
  key: "qoder",
  label: "Qoder",
  relativePath: ".qoder/skills",
  agents: ["qoder"],
  verified: true,
} as const;

function viewWithOneSkill(
  skill: Partial<ProjectSkillView> = {},
  project: Partial<ProjectView> = {},
): ProjectSkillsView {
  return {
    project: projectView(project),
    dirs: [
      { ...CLAUDE_DIR, agents: ["claude"] },
      { ...QODER_DIR, agents: ["qoder"] },
    ],
    skills: [
      {
        id: "a",
        name: "a",
        description: null,
        targets: [],
        dirs: {
          claude: { ...CLAUDE_DIR, agents: ["claude"], state: "present" },
          qoder: { ...QODER_DIR, agents: ["qoder"], state: "absent" },
        },
        source: null,
        lastOp: null,
        canonical: true,
        canonicalMissing: false,
        ...skill,
      },
    ],
    conflictCount: 0,
    migration: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithOneSkill());
  vi.mocked(api.setProjectSkillTargets).mockResolvedValue(viewWithOneSkill().skills[0]);
  vi.mocked(api.syncProjectSkills).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });
  vi.mocked(api.removeProjectSkillCopy).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });
  vi.mocked(api.uninstallProjectSkill).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });
  vi.mocked(api.setProjectAutoSync).mockResolvedValue(projectView());
});

test("renders project skill matrix with per-directory state", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project: projectView(),
    dirs: [
      {
        key: "claude",
        label: "Claude Code",
        relativePath: ".claude/skills",
        agents: ["claude"],
        verified: true,
      },
      {
        key: "qoder",
        label: "Qoder",
        relativePath: ".qoder/skills",
        agents: ["qoder"],
        verified: true,
      },
    ],
    skills: [
      {
        id: "code-review",
        name: "code-review",
        description: "审查代码",
        targets: [],
        dirs: {
          claude: {
            key: "claude",
            label: "Claude Code",
            relativePath: ".claude/skills",
            agents: ["claude"],
            verified: true,
            state: "present",
          },
          qoder: {
            key: "qoder",
            label: "Qoder",
            relativePath: ".qoder/skills",
            agents: ["qoder"],
            verified: true,
            state: "absent",
          },
        },
        source: null,
        lastOp: null,
        canonical: true,
        canonicalMissing: false,
      },
    ],
    conflictCount: 0,
    migration: null,
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("code-review")).toBeInTheDocument();
  expect(screen.getByText("已同步")).toBeInTheDocument();
  expect(screen.getByText("无")).toBeInTheDocument();
});

test("shows empty state when project has no skills", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project: projectView(),
    dirs: [],
    skills: [],
    conflictCount: 0,
    migration: null,
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("项目里还没有 Skills")).toBeInTheDocument();
});

test("checking a target calls setProjectSkillTargets and sync sends resolutions", async () => {
  render(<ProjectSkillsPanel project={projectView()} />);
  const qoder = await screen.findByRole("checkbox", { name: /Qoder/ });
  fireEvent.click(qoder);
  await waitFor(() =>
    expect(api.setProjectSkillTargets).toHaveBeenCalledWith("p1", "a", ["qoder"]),
  );

  fireEvent.click(screen.getByRole("button", { name: "立即同步" }));
  await waitFor(() => expect(api.syncProjectSkills).toHaveBeenCalledWith("p1", []));
});

test("sync conflict opens dialog and resolves with chosen directory", async () => {
  vi.mocked(api.syncProjectSkills)
    .mockResolvedValueOnce({
      outcomes: [],
      conflicts: [{ skillId: "a", dirs: ["claude", "qoder"], recommended: "claude", kind: "targetVsCanonical" }],
      warnings: [],
    })
    .mockResolvedValueOnce({ outcomes: [], conflicts: [], warnings: [] });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "立即同步" }));
  expect(await screen.findByText("需要处理冲突")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "以 Claude Code 为真源" }));
  await waitFor(() =>
    expect(api.syncProjectSkills).toHaveBeenLastCalledWith("p1", [
      { skillId: "a", dir: "claude", action: "use" },
    ]),
  );
});

test("both-kept resolution asks the backend to preserve the losing copy", async () => {
  vi.mocked(api.syncProjectSkills)
    .mockResolvedValueOnce({
      outcomes: [],
      conflicts: [{ skillId: "a", dirs: ["claude", "qoder"], recommended: "claude", kind: "targetVsCanonical" }],
      warnings: [],
    })
    .mockResolvedValueOnce({ outcomes: [], conflicts: [], warnings: [] });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "立即同步" }));
  fireEvent.click(await screen.findByRole("button", { name: "两边都保留" }));
  await waitFor(() =>
    expect(api.syncProjectSkills).toHaveBeenLastCalledWith("p1", [
      { skillId: "a", dir: "claude", action: "keepBoth" },
    ]),
  );
});

test("row menu removes a single copy without touching the other target", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    skills: [
      {
        ...viewWithOneSkill().skills[0],
        targets: ["claude", "qoder"],
        dirs: {
          claude: { ...CLAUDE_DIR, agents: ["claude"], state: "present" },
          qoder: { ...QODER_DIR, agents: ["qoder"], state: "present" },
        },
      },
    ],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "a 更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "移除 Qoder 副本" }));
  fireEvent.click(await screen.findByRole("button", { name: "确认移除" }));
  await waitFor(() => expect(api.removeProjectSkillCopy).toHaveBeenCalledWith("p1", "a", "qoder"));
});

test("uninstall needs an explicit confirmation before deleting every copy", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    skills: [
      {
        ...viewWithOneSkill().skills[0],
        targets: ["claude", "qoder"],
        dirs: {
          claude: { ...CLAUDE_DIR, agents: ["claude"], state: "present" },
          qoder: { ...QODER_DIR, agents: ["qoder"], state: "present" },
        },
      },
    ],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "a 更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "卸载" }));
  expect(api.uninstallProjectSkill).not.toHaveBeenCalled();
  expect(await screen.findByText(/claude\/skills、\.qoder\/skills/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "确认卸载" }));
  await waitFor(() => expect(api.uninstallProjectSkill).toHaveBeenCalledWith("p1", "a"));
});

test("sync is disabled and explained while no project directory is verified", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    dirs: [
      { ...CLAUDE_DIR, agents: ["claude"], verified: false },
      { ...QODER_DIR, agents: ["qoder"], verified: false },
    ],
    skills: [
      {
        ...viewWithOneSkill().skills[0],
        dirs: {
          claude: { ...CLAUDE_DIR, agents: ["claude"], verified: false, state: "present" },
          qoder: { ...QODER_DIR, agents: ["qoder"], verified: false, state: "absent" },
        },
      },
    ],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText(/尚未在真机验证/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "立即同步" })).toBeDisabled();
  expect(screen.queryByRole("checkbox", { name: /Claude/ })).not.toBeInTheDocument();
});

test("a failed sync reports the backend reason instead of failing silently", async () => {
  vi.mocked(api.syncProjectSkills).mockRejectedValue("同步失败，回滚出现问题：permission denied");

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "立即同步" }));
  expect(await screen.findByText(/回滚出现问题/)).toBeInTheDocument();
});

test("auto sync runs once on mount when enabled and targets exist", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(
    viewWithOneSkill({ targets: ["claude"] }, { autoSync: true }),
  );
  vi.mocked(api.syncProjectSkills).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });

  render(<ProjectSkillsPanel project={projectView({ autoSync: true })} />);
  await screen.findByRole("checkbox", { name: "同步到 Claude Code" });
  await waitFor(() => expect(api.syncProjectSkills).toHaveBeenCalledTimes(1));
});

test("auto sync stays silent on first add and reports conflicts as an entry", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(
    viewWithOneSkill({ targets: ["claude"] }, { autoSync: true }),
  );
  vi.mocked(api.syncProjectSkills).mockResolvedValue({
    outcomes: [],
    conflicts: [{ skillId: "a", dirs: ["claude", "qoder"], recommended: "claude", kind: "targetVsCanonical" }],
    warnings: [],
  });

  render(<ProjectSkillsPanel project={projectView({ autoSync: true })} />);
  // 自动同步不弹对话框，只留入口；点入口才展开。
  const entry = await screen.findByRole("button", { name: /1 个冲突待处理/ });
  expect(screen.queryByText("需要处理冲突")).not.toBeInTheDocument();
  fireEvent.click(entry);
  expect(await screen.findByText("需要处理冲突")).toBeInTheDocument();
});

test("toggling auto sync persists through setProjectAutoSync", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(
    viewWithOneSkill({}, { autoSync: false }),
  );
  vi.mocked(api.setProjectAutoSync).mockResolvedValue(projectView({ autoSync: true }));

  render(<ProjectSkillsPanel project={projectView({ autoSync: false })} />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "自动同步" }));
  await waitFor(() => expect(api.setProjectAutoSync).toHaveBeenCalledWith("p1", true));
});

const migrationPlan = {
  adopts: [{ skillId: "a", fromDir: "claude", files: [".ccswitchx/skills/a/SKILL.md"] }],
  proposals: [
    {
      skillId: "b",
      fromDir: "claude",
      files: [".ccswitchx/skills/b/SKILL.md"],
      candidates: ["claude"],
      defaultAdopt: false,
      reason: "该 skill 还没被勾选为渲染目标",
    },
  ],
  conflicts: [
    { skillId: "c", dirs: ["claude", "qoder"], recommended: "qoder", kind: "targetsDiffer" },
  ],
  drops: ["gone"],
  warnings: [],
};

async function openMigrationDialog() {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    migration: migrationPlan,
  });
  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "查看迁移方案" }));
  await screen.findByText("把项目 skill 收进项目真源");
}

test("旧台账显示迁移横幅，冲突未裁决时不能提交", async () => {
  await openMigrationDialog();

  // 安全性质必须写在界面上：迁移只新建真源，不碰 agent 目录。
  expect(screen.getByText(/不会改动任何 agent 目录/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "确认迁移" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "确认迁移" })).toHaveAttribute(
    "title",
    "还有 1 个冲突需要你裁决",
  );
});

test("迁移清单只列会真正新建的文件", async () => {
  await openMigrationDialog();

  expect(screen.getByText(".ccswitchx/skills/a/SKILL.md")).toBeInTheDocument();
  expect(screen.getByText(".ccswitchx/skills.json")).toBeInTheDocument();
  expect(screen.queryByText(".ccswitchx/skills/b/SKILL.md")).not.toBeInTheDocument();
  expect(screen.getByText(/台账里有 1 个条目在磁盘上已经不存在/)).toBeInTheDocument();
});

test("确认迁移把候选与冲突裁决一起提交", async () => {
  vi.mocked(api.migrateProjectSkills).mockResolvedValue({
    adopted: ["a"],
    skipped: ["b"],
    dropped: ["gone"],
    warnings: [],
    version: 2,
  });
  await openMigrationDialog();

  fireEvent.click(screen.getByRole("button", { name: "以 Qoder 为真源" }));
  const confirm = screen.getByRole("button", { name: "确认迁移" });
  expect(confirm).toBeEnabled();
  fireEvent.click(confirm);

  await waitFor(() =>
    expect(api.migrateProjectSkills).toHaveBeenCalledWith("p1", [
      { skillId: "b", dir: "" },
      { skillId: "c", dir: "qoder" },
    ]),
  );
  expect(await screen.findByText(/迁移完成：收进真源 1 个/)).toBeInTheDocument();
});

test("stale 与 edited 两种状态分得开", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    skills: [
      {
        ...viewWithOneSkill().skills[0],
        targets: ["claude", "qoder"],
        dirs: {
          claude: { ...CLAUDE_DIR, agents: ["claude"], state: "stale" },
          qoder: { ...QODER_DIR, agents: ["qoder"], state: "edited" },
        },
      },
    ],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("待更新")).toBeInTheDocument();
  expect(screen.getByText("已改")).toBeInTheDocument();
  // 悬停提示必须说明「已改」不会被自动覆盖，这是用户最容易被误导的一格。
  expect(
    screen.getByRole("checkbox", { name: "同步到 Qoder" }),
  ).toHaveAttribute("title", expect.stringContaining("需要你裁决"));
});

test("还没进真源的副本提供「收进真源」", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    skills: [{ ...viewWithOneSkill().skills[0], canonical: false }],
  });
  vi.mocked(api.adoptProjectSkill).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "a 更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "把 Claude Code 的副本收进真源" }));
  await waitFor(() => expect(api.adoptProjectSkill).toHaveBeenCalledWith("p1", "a", "claude"));
});

test("真源丢失时只能显式确认「删除全部副本」", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    ...viewWithOneSkill(),
    skills: [{ ...viewWithOneSkill().skills[0], canonicalMissing: true }],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "a 更多操作" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "真源已丢失：删除全部副本" }));
  // 菜单项只打开确认框，不直接删。
  fireEvent.click(screen.getByRole("button", { name: "确认删除全部副本" }));
  await waitFor(() =>
    expect(api.syncProjectSkills).toHaveBeenLastCalledWith("p1", [
      { skillId: "a", dir: "", action: "purge" },
    ]),
  );
});
