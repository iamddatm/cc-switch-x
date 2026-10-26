import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";
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
    installProjectSkill: vi.fn(),
    importProjectSkillDir: vi.fn(),
    createProjectSkill: vi.fn(),
    checkProjectSkillUpdates: vi.fn(),
    updateProjectSkills: vi.fn(),
    previewSkillSource: vi.fn(),
    writeAsset: vi.fn(),
    getSkillRepos: vi.fn(),
    discoverSkills: vi.fn(),
    listSkillsShTrending: vi.fn(),
    searchSkillsSh: vi.fn(),
    installDiscoveredProjectSkill: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
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
  vi.mocked(api.getSkillRepos).mockResolvedValue([]);
  vi.mocked(api.discoverSkills).mockResolvedValue({ skills: [], failures: [], repos: [] });
  vi.mocked(api.checkProjectSkillUpdates).mockResolvedValue({
    checkedAt: 0,
    updates: [],
    upToDate: [],
    skipped: [],
    repoFailures: [],
    skillFailures: [],
  });
  vi.mocked(open).mockResolvedValue(null);
});

/** 三行不同状态的清单，用来证明搜索与筛选真的在收窄列表。 */
function viewWithRows(): ProjectSkillsView {
  const base = viewWithOneSkill();
  const row = (
    id: string,
    description: string | null,
    states: Record<string, ProjectSkillView["dirs"][string]["state"]>,
    overrides: Partial<ProjectSkillView> = {},
  ): ProjectSkillView => ({
    ...base.skills[0],
    id,
    name: id,
    description,
    targets: Object.keys(states),
    dirs: Object.fromEntries(
      Object.entries(states).map(([key, state]) => [
        key,
        {
          ...(key === "claude" ? CLAUDE_DIR : QODER_DIR),
          agents: [key],
          state,
        },
      ]),
    ),
    ...overrides,
  });
  return {
    ...base,
    skills: [
      row("alpha", "代码审查", { claude: "present" }),
      row("beta", "部署清单", { claude: "present", qoder: "edited" }),
      row(
        "gamma",
        null,
        { qoder: "absent" },
        {
          source: { kind: "github", repo: "acme/skills", branch: "main", subpath: "gamma" },
          check: { updateAvailable: true, lastCheckedAt: 1, updatable: true },
        },
      ),
    ],
  };
}

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
  // 状态文字要看的是格子里那份，筛选下拉里也有同名选项。
  expect(screen.getByRole("checkbox", { name: "同步到 Claude Code" })).toHaveTextContent("已同步");
  expect(screen.getByRole("checkbox", { name: "同步到 Qoder" })).toHaveTextContent("无");
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

test("搜索、渲染目录与部署状态筛选收窄列表并给出计数", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithRows());
  render(<ProjectSkillsPanel project={projectView()} />);
  await screen.findByText("alpha");
  expect(screen.getByText("3 / 3 个 Skill")).toBeInTheDocument();

  fireEvent.change(screen.getByLabelText("搜索 Skill"), { target: { value: "部署" } });
  expect(screen.getByText("1 / 3 个 Skill")).toBeInTheDocument();
  expect(screen.getByText("beta")).toBeInTheDocument();
  expect(screen.queryByText("alpha")).not.toBeInTheDocument();

  fireEvent.change(screen.getByLabelText("搜索 Skill"), { target: { value: "" } });
  fireEvent.change(screen.getByLabelText("按部署状态筛选"), { target: { value: "edited" } });
  expect(screen.getByText("1 / 3 个 Skill")).toBeInTheDocument();
  expect(screen.queryByText("alpha")).not.toBeInTheDocument();
  expect(screen.queryByText("gamma")).not.toBeInTheDocument();

  // 目录筛选改的是「看哪个目录的状态」：beta 被改过的那格在 qoder，换到 claude 就不再命中。
  fireEvent.change(screen.getByLabelText("按渲染目录筛选"), { target: { value: "claude" } });
  expect(screen.getByText("0 / 3 个 Skill")).toBeInTheDocument();
  // 同一目录上 alpha 与 beta 都是已同步，说明目录筛选本身不额外删行。
  fireEvent.change(screen.getByLabelText("按部署状态筛选"), { target: { value: "present" } });
  expect(screen.getByText("2 / 3 个 Skill")).toBeInTheDocument();
});

test("批量渲染把选中的 Skill 铺到目录并立即同步", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithRows());
  render(<ProjectSkillsPanel project={projectView()} />);
  await screen.findByText("alpha");

  fireEvent.click(screen.getByRole("checkbox", { name: "全选当前列表" }));
  expect(screen.getByText("已选 3 个")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "批量渲染到" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "Qoder" }));

  await waitFor(() =>
    expect(api.setProjectSkillTargets).toHaveBeenCalledWith("p1", "alpha", ["claude", "qoder"]),
  );
  expect(api.setProjectSkillTargets).toHaveBeenCalledWith("p1", "beta", ["claude", "qoder"]);
  expect(api.setProjectSkillTargets).toHaveBeenCalledWith("p1", "gamma", ["qoder"]);
  await waitFor(() => expect(api.syncProjectSkills).toHaveBeenCalledWith("p1"));
});

test("安装抽屉装进项目真源，且不带任何渲染目标", async () => {
  vi.mocked(api.previewSkillSource).mockResolvedValue({
    kind: "local",
    stagingId: null,
    warnings: [],
    candidates: [{ relativePath: ".", name: "Demo", suggestedId: "demo", exists: false }],
  });
  vi.mocked(api.installProjectSkill).mockResolvedValue({
    installed: [{ id: "demo", name: ".", status: "installed", warnings: [] }],
    render: { outcomes: [], conflicts: [], warnings: [] },
    warnings: [],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "添加 Skill" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "从来源安装…" }));
  // 去向说明必须是项目真源，不是中央库。
  expect(await screen.findByText(/安装只写入项目真源/)).toBeInTheDocument();

  vi.mocked(open).mockResolvedValue("D:/skills");
  fireEvent.click(screen.getByRole("button", { name: "本地目录" }));
  fireEvent.click(screen.getByRole("button", { name: "选择目录" }));
  await waitFor(() => expect(screen.getByLabelText("本地目录")).toHaveValue("D:/skills"));
  fireEvent.click(screen.getByRole("button", { name: "扫描来源" }));
  await screen.findByText("Demo");
  fireEvent.click(screen.getByRole("button", { name: "安装选中 (1)" }));

  await waitFor(() =>
    expect(api.installProjectSkill).toHaveBeenCalledWith(
      "p1",
      { kind: "directory", path: "D:/skills" },
      null,
      [{ relativePath: ".", id: "demo" }],
      "skip",
      [],
    ),
  );
});

test("详情与编辑读写项目真源，不碰中央库", async () => {
  vi.mocked(api.readProjectSkillBody).mockResolvedValue("---\nname: a\ndescription: 演示\n---\n\n# 正文\n");
  vi.mocked(api.writeProjectSkillBody).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "查看 a" }));
  expect(await screen.findByText("# 正文")).toBeInTheDocument();
  // 项目专属的那一段：真源路径 + 已经渲染到哪些目录。
  expect(screen.getByText(/\.ccswitchx\/skills\/a/)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  fireEvent.change(screen.getByLabelText(/正文/), { target: { value: "# 新正文\n" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeProjectSkillBody).toHaveBeenCalledWith(
      "p1",
      "a",
      expect.stringContaining("# 新正文"),
    ),
  );
  expect(api.writeAsset).not.toHaveBeenCalled();
});

test("项目里没有 cc-switch 导入这条来源", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithRows());
  render(<ProjectSkillsPanel project={projectView()} />);
  await screen.findByText("alpha");

  fireEvent.click(screen.getByRole("button", { name: "添加 Skill" }));
  expect(await screen.findByRole("menuitem", { name: "从文件夹导入" })).toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: /cc-switch/i })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "项目 Skills 的更多操作" }));
  expect(await screen.findByRole("menuitem", { name: "打开文件夹" })).toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: /cc-switch/i })).not.toBeInTheDocument();

  expect(screen.getByLabelText("按来源筛选")).not.toHaveTextContent(/cc-switch/);
});

test("检查更新与更新走项目命令：只换真源，渲染留给同步", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithRows());
  vi.mocked(api.checkProjectSkillUpdates).mockResolvedValue({
    checkedAt: 1,
    updates: [{ id: "gamma", name: "gamma", remoteHash: "h2", missingLocal: false }],
    upToDate: [],
    skipped: [],
    repoFailures: [],
    skillFailures: [],
  });
  vi.mocked(api.updateProjectSkills).mockResolvedValue({
    outcomes: [{ id: "gamma", name: "gamma", status: "updated", warnings: [] }],
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  await screen.findByText("alpha");
  // 本机检查状态折进行数据：gamma 带「可更新」徽标。
  expect(screen.getByText("可更新")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
  expect(await screen.findByText(/发现 1 个可更新/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "全部更新（1 个）" }));
  fireEvent.click(await screen.findByRole("button", { name: "更新 1 个" }));
  await waitFor(() => expect(api.checkProjectSkillUpdates).toHaveBeenCalledWith("p1", undefined));
  await waitFor(() => expect(api.updateProjectSkills).toHaveBeenCalledWith("p1", ["gamma"]));
  expect(await screen.findByText(/只换了项目真源/)).toBeInTheDocument();
});

test("发现 tab 复用同一个市场面板，落点选择器给的是项目目录", async () => {
  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  // 落点措辞来自作用域适配器：项目说「目录」，全局说「Agent」。
  expect(await screen.findByText("安装到：全部目录（2）")).toBeInTheDocument();
});
