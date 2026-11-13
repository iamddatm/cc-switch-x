import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";
import { SkillsPanel } from "../panels/SkillsPanel";
import { api } from "../lib/api";
import { resetSkillUpdateCheck } from "../hooks/useSkillUpdateCheck";
import { agentStatus, capabilities } from "../test/agents";
import type { SkillCheckReport, UnmanagedSkillReport } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    importSkillDir: vi.fn(),
    listAgents: vi.fn(),
    listDrifts: vi.fn().mockResolvedValue([]),
    rescanDrift: vi.fn().mockResolvedValue([]),
    verifySkillDeployments: vi.fn().mockResolvedValue([]),
    repairSkillDeployments: vi.fn().mockResolvedValue({ items: [] }),
    setSkillEnabled: vi.fn(),
    previewSkillSource: vi.fn(),
    installSkill: vi.fn(),
    checkSkillUpdates: vi.fn(),
    updateSkills: vi.fn(),
    uninstallSkills: vi.fn(),
    setSkillsEnabled: vi.fn(),
    getSkillRepos: vi.fn(),
    setSkillRepos: vi.fn(),
    discoverSkills: vi.fn(),
    openExternalUrl: vi.fn(),
    installDiscoveredSkill: vi.fn(),
    searchSkillsSh: vi.fn(),
    listSkillsShTrending: vi.fn(),
    scanUnmanagedSkills: vi.fn(),
    importUnmanagedSkills: vi.fn(),
    discardUnmanagedSkills: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

const claude = agentStatus();

beforeEach(() => {
  vi.clearAllMocks();
  resetSkillUpdateCheck();
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
  vi.mocked(api.checkSkillUpdates).mockResolvedValue({
    checkedAt: Date.now(),
    updates: [],
    upToDate: [],
    skipped: [],
    repoFailures: [],
    skillFailures: [],
  });
  vi.mocked(api.updateSkills).mockResolvedValue({ outcomes: [] });
  vi.mocked(api.uninstallSkills).mockResolvedValue({ outcomes: [] });
  vi.mocked(api.setSkillsEnabled).mockResolvedValue({ ok: [], failed: [] });
  vi.mocked(api.previewSkillSource).mockResolvedValue({
    kind: "local",
    stagingId: null,
    candidates: [],
    warnings: [],
  });
  vi.mocked(api.installSkill).mockResolvedValue({ outcomes: [] });
  vi.mocked(api.getSkillRepos).mockResolvedValue([
    { owner: "acme", name: "skills", branch: "main", enabled: true },
  ]);
  vi.mocked(api.setSkillRepos).mockImplementation(async (repos) => repos);
  vi.mocked(api.discoverSkills).mockResolvedValue({
    skills: [
      {
        key: "acme/skills@main:pdf",
        name: "pdf",
        description: "PDF 处理",
        relativePath: "pdf",
        suggestedId: "acme-skills-pdf",
        repo: "acme/skills",
        branch: "main",
        installed: false,
      },
    ],
    failures: [],
    repos: [{ owner: "acme", name: "skills", branch: "main", enabled: true }],
    installedRepos: [],
  });
  vi.mocked(api.installDiscoveredSkill).mockResolvedValue({
    id: "acme-skills-pdf",
    name: "pdf",
    status: "installed",
    warnings: [],
  });
  vi.mocked(api.searchSkillsSh).mockResolvedValue({
    skills: [
      {
        key: "anthropics/skills/pdf",
        name: "pdf",
        directory: "pdf",
        repoOwner: "anthropics",
        repoName: "skills",
        repoBranch: "main",
        installs: 1200,
        readmeUrl: "https://github.com/anthropics/skills",
      },
    ],
    totalCount: 1,
    query: "pdf",
  });
  vi.mocked(api.listSkillsShTrending).mockResolvedValue({
    skills: [
      {
        key: "vercel-labs/agent-skills:trending-demo",
        name: "trending-demo",
        directory: "trending-demo",
        repoOwner: "vercel-labs",
        repoName: "agent-skills",
        repoBranch: "main",
        installs: 20512,
        readmeUrl: "https://github.com/vercel-labs/agent-skills",
      },
    ],
    hasMore: false,
  });
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue({
    skills: [],
    conflicts: [],
    skipped: [],
    roots: [],
    warnings: [],
  });
  vi.mocked(api.importUnmanagedSkills).mockResolvedValue({ outcomes: [] });
  vi.mocked(api.discardUnmanagedSkills).mockResolvedValue({ outcomes: [] });
});

test("shows the link kind the backend actually used", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "Claude Code 启用 demo" }));
  expect(await screen.findByText("实测：junction")).toBeInTheDocument();
  expect(api.setSkillEnabled).toHaveBeenCalledWith("demo", "claude", true);
  // 点一次只该发一次请求：控件嵌在 <label> 里时真实浏览器会触发两次 change
  expect(api.setSkillEnabled).toHaveBeenCalledTimes(1);
  // 勾选动过盘，落点核对要跟着重读：挂载时一次，开关之后再一次，横幅不能停在旧计数上
  await vi.waitFor(() => expect(api.verifySkillDeployments).toHaveBeenCalledTimes(2));
});

test("一键处理先讲清要动什么，确认后才动盘并重读核对", async () => {
  vi.mocked(api.verifySkillDeployments).mockResolvedValue([
    {
      agentId: "claude",
      skillId: "demo",
      status: "missing",
      targetPath: "C:/Users/me/.claude/skills/demo",
      detail: "落点上已经没有这个目录（被外部删掉，或从未部署成功）",
    },
    {
      agentId: "claude",
      skillId: "other",
      status: "foreign",
      targetPath: "C:/Users/me/.claude/skills/other",
      detail: "落点现在是指向 C:/Users/me/.cc-switch/skills/other 的链接，不再是我们的真源",
    },
  ]);
  vi.mocked(api.repairSkillDeployments).mockResolvedValue({
    items: [
      {
        agentId: "claude",
        skillId: "demo",
        status: "redeployed",
        detail: "已重新部署到落点（junction）",
      },
      {
        agentId: "claude",
        skillId: "other",
        status: "refused",
        detail: "盘上这份与真源内容不一致，已保留原目录；请先比对清楚再处理",
      },
    ],
  });
  render(<SkillsPanel />);

  fireEvent.click(await screen.findByRole("button", { name: "一键处理" }));
  const dialog = await screen.findByRole("alertdialog");
  expect(within(dialog).getByText(/重刷 1 个空落点/)).toBeInTheDocument();
  expect(within(dialog).getByText(/抢回 1 个被别处接走的落点/)).toBeInTheDocument();
  // 确认框只是预览：没点确认就一个字节都不许动
  expect(api.repairSkillDeployments).not.toHaveBeenCalled();

  const readsBefore = vi.mocked(api.verifySkillDeployments).mock.calls.length;
  fireEvent.click(within(dialog).getByRole("button", { name: "确认处理" }));
  await vi.waitFor(() => expect(api.repairSkillDeployments).toHaveBeenCalledTimes(1));
  expect(await screen.findByText("处理完成：重刷落点 1 个、未处理 1 个。")).toBeInTheDocument();
  // 抢不回来的那条要逐条说原因，不能只报个数字
  expect(screen.getByText(/other：盘上这份与真源内容不一致/)).toBeInTheDocument();
  // 处置完以磁盘为准重读：横幅不能停在处理前的计数上
  await vi.waitFor(() =>
    expect(api.verifySkillDeployments).toHaveBeenCalledTimes(readsBefore + 1),
  );
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
  fireEvent.change(screen.getByLabelText("按启用状态筛选"), { target: { value: "off" } });
  expect(screen.getByRole("button", { name: "查看 demo" })).toBeInTheDocument();
});

test("splits the agent and status filters", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "skill", id: "demo", name: "demo", updatedAt: 1, hash: "h" },
    { assetType: "skill", id: "other", name: "other", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ enabledSkills: ["demo"] }),
    agentStatus({ id: "qoder", displayName: "Qoder CN", enabledSkills: ["other"] }),
  ]);
  render(<SkillsPanel />);
  await screen.findByRole("button", { name: "查看 demo" });
  expect(screen.getByRole("button", { name: "查看 other" })).toBeInTheDocument();

  // 选中具体 Agent + 已启用：只剩该 Agent 启用的项。
  fireEvent.change(screen.getByLabelText("按 Agent 筛选"), { target: { value: "qoder" } });
  fireEvent.change(screen.getByLabelText("按启用状态筛选"), { target: { value: "on" } });
  expect(screen.queryByRole("button", { name: "查看 demo" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "查看 other" })).toBeInTheDocument();

  // 该 Agent 的未启用项。
  fireEvent.change(screen.getByLabelText("按启用状态筛选"), { target: { value: "off" } });
  expect(screen.getByRole("button", { name: "查看 demo" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "查看 other" })).not.toBeInTheDocument();

  // 全部 Agent 的未启用项（当前两个都被某个 Agent 启用，应该都隐藏）。
  fireEvent.change(screen.getByLabelText("按 Agent 筛选"), { target: { value: "all" } });
  expect(screen.queryByRole("button", { name: "查看 demo" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "查看 other" })).not.toBeInTheDocument();

  // 默认就是「全部 Agent + 全部状态」，回到初始时两项都在。
  fireEvent.change(screen.getByLabelText("按启用状态筛选"), { target: { value: "all" } });
  expect(screen.getByRole("button", { name: "查看 demo" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "查看 other" })).toBeInTheDocument();
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

test("shows the source repo and update badge from metadata", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "skill",
      id: "demo",
      name: "demo",
      updatedAt: 1,
      hash: "h",
      mtime: Date.now() - 3 * 24 * 60 * 60 * 1000,
      origin: {
        kind: "cc-switch",
        repo: "acme/demo",
        branch: "main",
        updateAvailable: true,
        updatable: true,
        lastCheckedAt: Date.now(),
      },
    },
  ]);
  render(<SkillsPanel />);
  expect(await screen.findByText("更新于 3 天前")).toBeInTheDocument();
  expect(screen.getAllByText("acme/demo").length).toBeGreaterThan(0);
  expect(screen.queryByText("acme/demo@main")).not.toBeInTheDocument();
  expect(screen.getByText("可更新")).toBeInTheDocument();
  expect(screen.getByText(/检查于/)).toBeInTheDocument();
});

test("checks all skills and opens the update confirmation", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "skill",
      id: "demo",
      name: "demo",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "cc-switch", repo: "acme/demo", updateAvailable: false, updatable: true },
    },
  ]);
  vi.mocked(api.checkSkillUpdates).mockResolvedValue({
    checkedAt: Date.now(),
    updates: [
      { id: "demo", name: "demo", currentHash: "h", remoteHash: "h2", missingLocal: false },
    ],
    upToDate: [],
    skipped: [],
    repoFailures: [],
    skillFailures: [],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));
  expect(api.checkSkillUpdates).toHaveBeenCalledWith(undefined);

  // 对齐 cc-switch：查到更新就直接弹「全部更新」确认，不用再找按钮。
  const dialog = await screen.findByRole("alertdialog");
  expect(within(dialog).getByText("acme/demo")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "全部更新（1 个）" })).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole("button", { name: "更新 1 个" }));
  await waitFor(() => expect(api.updateSkills).toHaveBeenCalledWith(["demo"]));
});

test("keeps the update check running when the panel unmounts", async () => {
  let resolveCheck: (report: SkillCheckReport) => void = () => {};
  vi.mocked(api.checkSkillUpdates).mockReturnValue(
    new Promise<SkillCheckReport>((resolve) => {
      resolveCheck = resolve;
    }),
  );
  const first = render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "检查更新" }));
  first.unmount();

  // 切回来（重新挂载）时检查还在进行，结果不丢。
  render(<SkillsPanel />);
  expect(await screen.findByText(/正在读取上游仓库/)).toBeInTheDocument();

  resolveCheck({
    checkedAt: Date.now(),
    updates: [
      { id: "demo", name: "demo", currentHash: "h", remoteHash: "h2", missingLocal: false },
    ],
    upToDate: [],
    skipped: [],
    repoFailures: [],
    skillFailures: [],
  });
  expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
});

test("deleting a skill goes through uninstallSkills and mentions the backup", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "demo 的更多操作" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "删除" }));
  const dialog = await screen.findByRole("alertdialog");
  expect(within(dialog).getByText(/backups\/skills/)).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole("button", { name: "删除 Skill" }));
  await waitFor(() => expect(api.uninstallSkills).toHaveBeenCalledWith(["demo"]));
});

test("discovers repo skills and installs to all enabled agents", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));

  expect(await screen.findByText("pdf")).toBeInTheDocument();
  expect(api.discoverSkills).toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "安装" }));

  await waitFor(() =>
    expect(api.installDiscoveredSkill).toHaveBeenCalledWith(
      {
        repo: "acme/skills",
        branch: "main",
        relativePath: "pdf",
        name: "pdf",
      },
      "skip",
    ),
  );
  await waitFor(() =>
    expect(api.setSkillsEnabled).toHaveBeenCalledWith(
      ["acme-skills-pdf"],
      "claude",
      true,
    ),
  );
  expect(await screen.findByText(/已安装 pdf/)).toBeInTheDocument();
});

test("adds a repository from the manager sheet", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  await screen.findByText("pdf");
  fireEvent.click(screen.getByRole("button", { name: "管理仓库" }));

  fireEvent.change(await screen.findByLabelText("仓库地址"), {
    target: { value: "https://github.com/new/skills" },
  });
  fireEvent.click(screen.getByRole("button", { name: "添加仓库" }));

  await waitFor(() =>
    expect(api.setSkillRepos).toHaveBeenCalledWith([
      { owner: "acme", name: "skills", branch: "main", enabled: true },
      { owner: "new", name: "skills", branch: "main", enabled: true },
    ]),
  );
});

test("searches skills.sh and installs by skill id", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  await screen.findByText("pdf");

  fireEvent.click(screen.getByRole("button", { name: "skills.sh" }));
  fireEvent.change(screen.getByLabelText("搜索可发现的 Skill"), {
    target: { value: "pdf" },
  });
  fireEvent.keyDown(screen.getByLabelText("搜索可发现的 Skill"), {
    key: "Enter",
  });

  expect(await screen.findByText("skills.sh · pdf")).toBeInTheDocument();
  expect(api.searchSkillsSh).toHaveBeenCalledWith("pdf", 20, 0);
  fireEvent.click(screen.getByRole("button", { name: "安装" }));
  await waitFor(() =>
    expect(api.installDiscoveredSkill).toHaveBeenCalledWith(
      {
        repo: "anthropics/skills",
        branch: "main",
        directory: "pdf",
        name: "pdf",
      },
      "skip",
    ),
  );
});

test("点 skills.sh 只准备搜索，不抓趋势榜", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  await screen.findByText("pdf");

  fireEvent.click(screen.getByRole("button", { name: "skills.sh" }));
  expect(await screen.findByText("搜索 skills.sh")).toBeInTheDocument();
  expect(api.listSkillsShTrending).not.toHaveBeenCalled();

  // 搜索 → 清空关键词回到「请先搜索」，不再把趋势榜当兜底。
  const field = screen.getByLabelText("搜索可发现的 Skill");
  fireEvent.change(field, { target: { value: "pdf" } });
  fireEvent.keyDown(field, { key: "Enter" });
  expect(await screen.findByText("skills.sh · pdf")).toBeInTheDocument();
  fireEvent.change(field, { target: { value: "" } });
  expect(await screen.findByText("搜索 skills.sh")).toBeInTheDocument();
  expect(api.listSkillsShTrending).not.toHaveBeenCalled();
});

test("点「趋势」页签才请求趋势榜，且已有内容时不重复抓", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  await screen.findByText("pdf");
  expect(api.listSkillsShTrending).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "趋势" }));
  expect(await screen.findByText(/skills\.sh 趋势榜 · 近 24 小时/)).toBeInTheDocument();
  expect(await screen.findByText("trending-demo")).toBeInTheDocument();
  expect(api.listSkillsShTrending).toHaveBeenCalledWith(0);

  fireEvent.click(screen.getByRole("button", { name: "skills.sh" }));
  fireEvent.click(screen.getByRole("button", { name: "趋势" }));
  expect(api.listSkillsShTrending).toHaveBeenCalledTimes(1);

  // 趋势页签没有搜索框：它只做榜单浏览。
  expect(screen.queryByLabelText("搜索可发现的 Skill")).not.toBeInTheDocument();
});

test("selecting rows enables batch actions scoped to the selection", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "skill", id: "demo", name: "demo", updatedAt: 1, hash: "h" },
    { assetType: "skill", id: "other", name: "other", updatedAt: 1, hash: "h" },
  ]);
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "选择 demo" }));
  const bar = screen.getByText("已选 1 个").parentElement as HTMLElement;
  fireEvent.click(within(bar).getByRole("button", { name: "检查更新" }));
  await waitFor(() => expect(api.checkSkillUpdates).toHaveBeenCalledWith(["demo"]));
});

test("clicking a skill opens a read-only detail and can continue to edit", async () => {
  vi.mocked(api.readAsset).mockResolvedValue(
    "---\nname: demo\ndescription: 演示\n---\n\n# 正文\n",
  );
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "查看 demo" }));

  expect(await screen.findByRole("button", { name: "编辑" })).toBeInTheDocument();
  expect(screen.getByText("# 正文")).toBeInTheDocument();
  expect(screen.getByText("演示")).toBeInTheDocument();
  expect(api.writeAsset).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "编辑" }));
  expect(await screen.findByLabelText(/正文（SKILL.md）/)).toHaveValue("# 正文\n");
});

test("filters installed skills by multiple source repos", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "skill",
      id: "demo",
      name: "demo",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "github", repo: "acme/demo", updateAvailable: false, updatable: true },
    },
    {
      assetType: "skill",
      id: "other",
      name: "other",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "github", repo: "beta/other", updateAvailable: false, updatable: true },
    },
  ]);
  render(<SkillsPanel />);
  expect(await screen.findByRole("button", { name: "查看 demo" })).toBeInTheDocument();
  expect(screen.getByText("仓库：全部（2）")).toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "acme/demo" })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "仓库：全部（2）" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "acme/demo" }));
  expect(screen.queryByRole("button", { name: "查看 demo" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "查看 other" })).toBeInTheDocument();
  expect(screen.getByText("仓库：1/2")).toBeInTheDocument();

  // 连续勾选不收起，点外部才关闭。
  fireEvent.click(screen.getByRole("checkbox", { name: "全部仓库" }));
  expect(screen.getByRole("button", { name: "查看 demo" })).toBeInTheDocument();
  expect(screen.getByText("仓库：全部（2）")).toBeInTheDocument();

  fireEvent.pointerDown(document.body);
  expect(screen.queryByRole("checkbox", { name: "acme/demo" })).not.toBeInTheDocument();
});

test("installed-source repos are offered but only fetched when checked", async () => {
  vi.mocked(api.discoverSkills).mockResolvedValue({
    skills: [
      {
        key: "iamddatm/skills@main:ask-ui",
        name: "ask-ui",
        description: "提问 UI",
        relativePath: "skills/ask-ui",
        suggestedId: "iamddatm-skills-ask-ui",
        repo: "iamddatm/skills",
        branch: "main",
        installed: true,
      },
    ],
    failures: [],
    repos: [{ owner: "acme", name: "skills", branch: "main", enabled: true }],
    installedRepos: [{ owner: "iamddatm", name: "skills", branch: "main", enabled: true }],
  });

  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));

  // 默认只读配置仓库；已安装来源出现在候选列表里，但未勾选、未下载。
  expect(await screen.findByRole("button", { name: "仓库 · 1" })).toBeInTheDocument();
  fireEvent.click(await screen.findByRole("button", { name: "仓库：1/2" }));
  const checkbox = await screen.findByRole("checkbox", { name: "iamddatm/skills" });
  expect(checkbox).not.toBeChecked();
  expect(api.discoverSkills).toHaveBeenLastCalledWith({ force: false, extraRepos: [] });

  // 勾选后才把它作为额外仓库传回后端抓取。
  fireEvent.click(checkbox);
  await waitFor(() =>
    expect(api.discoverSkills).toHaveBeenLastCalledWith({
      force: false,
      extraRepos: [{ owner: "iamddatm", name: "skills", branch: "main", enabled: true }],
    }),
  );
  expect(screen.getByRole("button", { name: "仓库：2/2" })).toBeInTheDocument();
});

test("discovery rows can open the matching repo directory on github", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));

  const links = await screen.findAllByRole("button", { name: "在 GitHub 打开" });
  fireEvent.click(links[0]);

  await waitFor(() =>
    expect(api.openExternalUrl).toHaveBeenCalledWith(
      "https://github.com/acme/skills/tree/main/pdf",
    ),
  );
});

test("source filter hides skills from other origins", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "skill",
      id: "demo",
      name: "demo",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "zip", updateAvailable: false, updatable: false },
    },
    {
      assetType: "skill",
      id: "other",
      name: "other",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "local", updateAvailable: false, updatable: false },
    },
  ]);
  render(<SkillsPanel />);
  await screen.findByRole("button", { name: "查看 demo" });
  fireEvent.change(screen.getByLabelText("按来源筛选"), { target: { value: "zip" } });
  expect(screen.getByRole("button", { name: "查看 demo" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "查看 other" })).not.toBeInTheDocument();
});

test("updating clears both the selection and the discovered update list", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "skill",
      id: "demo",
      name: "demo",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "github", repo: "acme/demo", updateAvailable: true, updatable: true },
    },
  ]);
  vi.mocked(api.checkSkillUpdates).mockResolvedValue({
    checkedAt: Date.now(),
    updates: [
      { id: "demo", name: "demo", currentHash: "h", remoteHash: "h2", missingLocal: false },
    ],
    upToDate: [],
    skipped: [],
    repoFailures: [],
    skillFailures: [],
  });
  vi.mocked(api.updateSkills).mockResolvedValue({
    outcomes: [
      { id: "demo", name: "demo", status: "updated", warnings: [] },
    ],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "选择 demo" }));
  const bar = screen.getByText("已选 1 个").parentElement as HTMLElement;

  fireEvent.click(within(bar).getByRole("button", { name: "检查更新" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "更新 1 个" }));
  await waitFor(() => expect(api.updateSkills).toHaveBeenCalledWith(["demo"]));

  // 更新成功后两笔账都要清：勾选（批量条）与「发现 N 个可更新」（全部更新按钮）。
  expect(screen.queryByText("已选 1 个")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /^全部更新/ })).not.toBeInTheDocument();
});

test("deleting one selected skill keeps the others selected", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "skill", id: "demo", name: "demo", updatedAt: 1, hash: "h" },
    { assetType: "skill", id: "other", name: "other", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.uninstallSkills).mockResolvedValue({
    outcomes: [{ id: "demo", backupPath: null, disabledAgents: [], warnings: [] }],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "选择 demo" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "选择 other" }));
  expect(screen.getByText("已选 2 个")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "demo 的更多操作" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "删除" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "删除 Skill" }));
  await waitFor(() => expect(api.uninstallSkills).toHaveBeenCalledWith(["demo"]));

  // 只把删掉的那条从勾选里摘出去，剩下的还得能继续批量操作。
  expect(screen.getByText("已选 1 个")).toBeInTheDocument();
});

// ── 未纳管 Skill：发现 → 查看 → 导入接管 ──────────────────────────

function unmanagedFixture(overrides: Partial<UnmanagedSkillReport> = {}): UnmanagedSkillReport {
  return {
    skills: [
      {
        directory: "hand-made",
        suggestedId: "hand-made",
        name: "手工装的",
        description: "自己放的",
        path: "C:/Users/me/.claude/skills/hand-made",
        foundIn: [
          {
            sourceId: "claude",
            label: "Claude Code",
            path: "C:/Users/me/.claude/skills/hand-made",
            linkTarget: null,
            takeoverPossible: true,
          },
          {
            sourceId: "gemini",
            label: "Gemini CLI",
            path: "C:/Users/me/.gemini/skills/hand-made",
            linkTarget: null,
            takeoverPossible: false,
          },
        ],
        idRenamed: false,
      },
    ],
    conflicts: [],
    skipped: [],
    roots: [
      {
        sourceId: "claude",
        label: "Claude Code",
        path: "C:/Users/me/.claude/skills",
        exists: true,
        sandboxed: false,
      },
    ],
    warnings: [],
    ...overrides,
  };
}

test("the unmanaged tab counts the scan result and banners the installed view", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  render(<SkillsPanel />);

  const tab = await screen.findByRole("tab", { name: /未纳管/ });
  expect(tab).toHaveTextContent("1");
  expect(screen.getByText(/发现 1 个未纳管 Skill/)).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "知道了" }));
  expect(screen.queryByText(/发现 1 个未纳管 Skill/)).not.toBeInTheDocument();
});

test("the unmanaged banner stays out of an empty library", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([]);
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  render(<SkillsPanel />);

  expect(await screen.findByRole("tab", { name: /未纳管/ })).toBeInTheDocument();
  expect(screen.queryByText(/发现 1 个未纳管 Skill/)).not.toBeInTheDocument();
});

test("unmanaged rows show where each copy was found and which spots cannot be taken over", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  const row = (await screen.findByText("手工装的")).closest("li");
  expect(row).not.toBeNull();
  // 两个发现位置各有徽标，title 指到具体路径。
  expect(within(row as HTMLElement).getByText("Claude Code")).toBeInTheDocument();
  // 未实测的落点照样列出来，但标成「只收进真源」。
  expect(within(row as HTMLElement).getByTitle("落点未实测，只收进真源")).toBeInTheDocument();
});

test("importing takes over only the locations the user checked", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  vi.mocked(api.importUnmanagedSkills).mockResolvedValue({
    outcomes: [
      {
        directory: "hand-made",
        id: "hand-made",
        status: "installed",
        takenOver: [
          {
            sourceId: "claude",
            label: "Claude Code",
            status: "linked",
            detail: "已用指向真源的部署替换原目录；启用方式：junction",
            backupPath: "store/backups/skills/1_adopt_claude_hand-made",
          },
        ],
        warnings: [],
      },
    ],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  fireEvent.click(await screen.findByRole("checkbox", { name: "选择 手工装的" }));
  fireEvent.click(screen.getByRole("button", { name: "导入所选并接管" }));

  const dialog = await screen.findByRole("alertdialog");
  // 默认勾上已实测的落点；未实测的 Gemini 落点根本不该出现在接管列表里。
  expect(within(dialog).getByRole("checkbox", { name: "接管 Claude Code" })).toBeChecked();
  expect(
    within(dialog).queryByRole("checkbox", { name: "接管 Gemini CLI" }),
  ).not.toBeInTheDocument();

  fireEvent.click(within(dialog).getByRole("button", { name: "导入并接管" }));
  await waitFor(() =>
    expect(api.importUnmanagedSkills).toHaveBeenCalledWith(
      [{ directory: "hand-made", takeOver: ["claude"] }],
      "skip",
    ),
  );
  expect(await screen.findByText(/启用方式：junction/)).toBeInTheDocument();
  // 导入之后要重扫，这份内容已经不再是未纳管。
  await waitFor(() => expect(api.scanUnmanagedSkills).toHaveBeenCalledTimes(2));
});

test("a failed takeover is shown verbatim instead of a success line", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  vi.mocked(api.importUnmanagedSkills).mockResolvedValue({
    outcomes: [
      {
        directory: "hand-made",
        id: "hand-made",
        status: "failed",
        takenOver: [
          {
            sourceId: "claude",
            label: "Claude Code",
            status: "refused",
            detail: "盘上这份与真源内容不一致，已保留原目录",
          },
        ],
        warnings: [],
        error: "写入真源失败：磁盘满",
      },
    ],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  fireEvent.click(await screen.findByRole("button", { name: "导入并接管" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "导入并接管" }));

  expect(await screen.findByText(/写入真源失败：磁盘满/)).toBeInTheDocument();
  expect(screen.getByText(/已保留原目录/)).toBeInTheDocument();
  expect(screen.getByText(/^hand-made：失败/)).toBeInTheDocument();
});

test("deleting an unmanaged copy spells out what happens to each spot", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  vi.mocked(api.discardUnmanagedSkills).mockResolvedValue({
    outcomes: [
      {
        directory: "hand-made",
        deleted: [
          {
            sourceId: "claude",
            label: "Claude Code",
            path: "C:/Users/me/.claude/skills/hand-made",
            status: "removedDirectory",
            detail: "已删除原目录（删除前已整份留底）",
            backupPath: "store/backups/skills/1_adopt_claude_hand-made",
          },
        ],
      },
    ],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  fireEvent.click(await screen.findByRole("button", { name: "删除 手工装的" }));
  const dialog = await screen.findByRole("alertdialog");
  // 确认框必须把每一处副本和删除方式讲清楚：链接只解链，目录先留底。
  expect(within(dialog).getAllByText("目录，整份留底后删除").length).toBe(2);
  fireEvent.click(within(dialog).getByRole("button", { name: "删除副本" }));

  await waitFor(() => expect(api.discardUnmanagedSkills).toHaveBeenCalledWith(["hand-made"]));
  expect(await screen.findByText(/1_adopt_claude_hand-made/)).toBeInTheDocument();
  // 删完要重扫：这份内容已经不在盘上了。
  await waitFor(() => expect(api.scanUnmanagedSkills).toHaveBeenCalledTimes(2));
});

test("a refused delete keeps the reason instead of claiming success", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue(unmanagedFixture());
  vi.mocked(api.discardUnmanagedSkills).mockResolvedValue({
    outcomes: [
      {
        directory: "hand-made",
        deleted: [
          {
            sourceId: "claude",
            label: "Claude Code",
            path: "C:/Users/me/.claude/skills/hand-made",
            status: "refused",
            detail: "这一处归我们管，不走删除；请在已安装里停用或卸载",
          },
        ],
      },
    ],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  fireEvent.click(await screen.findByRole("button", { name: "删除 手工装的" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "删除副本" }));

  expect(await screen.findByText(/未删除/)).toBeInTheDocument();
  expect(screen.getByText(/请在已安装里停用或卸载/)).toBeInTheDocument();
});

test("conflicts only offer saving under a new id and never take over", async () => {
  const report = unmanagedFixture();
  vi.mocked(api.scanUnmanagedSkills).mockResolvedValue({
    ...report,
    skills: [],
    conflicts: report.skills,
  });
  vi.mocked(api.importUnmanagedSkills).mockResolvedValue({
    outcomes: [
      {
        directory: "hand-made",
        id: "hand-made-2",
        status: "installed",
        takenOver: [],
        warnings: [],
      },
    ],
  });
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  fireEvent.click(await screen.findByRole("button", { name: "另存为新 ID" }));
  await waitFor(() =>
    expect(api.importUnmanagedSkills).toHaveBeenCalledWith(
      [{ directory: "hand-made", takeOver: [] }],
      "keepBoth",
    ),
  );
});

test("a scan error is reported in the unmanaged view rather than shown as empty", async () => {
  vi.mocked(api.scanUnmanagedSkills).mockRejectedValue(new Error("读不了目录"));
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /未纳管/ }));

  expect(await screen.findByText(/读不了目录/)).toBeInTheDocument();
  // 扫描失败是「不知道」，不能报成「没有未纳管」。
  expect(screen.getByText("还没有扫描结果")).toBeInTheDocument();
  expect(screen.queryByText(/没有未纳管的 Skill/)).not.toBeInTheDocument();
});

// ── 列头批量开关 ─────────────────────────────────────────────────
// 列头点下去是给「整列」做批量，最容易出的事故是范围超出用户看见的那些，
// 所以这几条都盯着传给后端的 ids。

test("column header enables the rest of the list in one action", async () => {
  render(<SkillsPanel />);
  fireEvent.click(
    await screen.findByRole("button", { name: /Claude Code：当前列表已启用 0 \/ 1 个/ }),
  );
  fireEvent.click(screen.getByRole("menuitem", { name: "启用剩余 1 个" }));

  await waitFor(() =>
    expect(api.setSkillsEnabled).toHaveBeenCalledWith(["demo"], "claude", true),
  );
});

test("column header bulk stays inside the filtered rows", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "skill", id: "demo", name: "demo", updatedAt: 1, hash: "h" },
    { assetType: "skill", id: "other", name: "other", updatedAt: 1, hash: "h2" },
  ]);
  render(<SkillsPanel />);
  fireEvent.change(await screen.findByLabelText("搜索 Skill"), {
    target: { value: "demo" },
  });

  fireEvent.click(screen.getByRole("button", { name: /已启用 0 \/ 1 个，点击批量开关/ }));
  // 脚注要在菜单还开着的时候看：动作一执行菜单就收了
  expect(screen.getByText(/仅当前筛选结果 · 1 个/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("menuitem", { name: "启用剩余 1 个" }));

  await waitFor(() =>
    expect(api.setSkillsEnabled).toHaveBeenCalledWith(["demo"], "claude", true),
  );
  // 筛出来的「全部启用」不能顺手把看不见的 other 也铺上去
  expect(vi.mocked(api.setSkillsEnabled).mock.calls[0][0]).not.toContain("other");
});

test("column header bulk reports per-item failures", async () => {
  vi.mocked(api.setSkillsEnabled).mockResolvedValue({
    ok: [{ id: "demo", label: "junction" }],
    failed: [{ id: "ghost", error: "落点被别处占据" }],
  });
  render(<SkillsPanel />);
  fireEvent.click(
    await screen.findByRole("button", { name: /点击批量开关/ }),
  );
  fireEvent.click(screen.getByRole("menuitem", { name: "启用剩余 1 个" }));

  expect(await screen.findByText(/启用完成：成功 1 个，失败 1 个/)).toBeInTheDocument();
  expect(screen.getByText(/落点被别处占据/)).toBeInTheDocument();
});

test("an unverified skill column exposes no bulk action", async () => {
  vi.mocked(api.listAgents).mockResolvedValue([
    agentStatus({ capabilities: capabilities({ skill: "unverified" }) }),
  ]);
  render(<SkillsPanel />);

  await screen.findByText("demo");
  expect(screen.queryByRole("button", { name: /点击批量开关/ })).not.toBeInTheDocument();
});
