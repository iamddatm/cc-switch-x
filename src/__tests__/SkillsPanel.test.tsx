import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";
import { SkillsPanel } from "../panels/SkillsPanel";
import { api } from "../lib/api";
import { resetSkillUpdateCheck } from "../hooks/useSkillUpdateCheck";
import { agentStatus } from "../test/agents";
import type { SkillCheckReport } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    importSkillDir: vi.fn(),
    listAgents: vi.fn(),
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
    installDiscoveredSkill: vi.fn(),
    searchSkillsSh: vi.fn(),
    listSkillsShTrending: vi.fn(),
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

test("shows the skills.sh trending list by default", async () => {
  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  await screen.findByText("pdf");

  fireEvent.click(screen.getByRole("button", { name: "skills.sh" }));
  expect(await screen.findByText(/skills\.sh 趋势榜 · 近 24 小时/)).toBeInTheDocument();
  expect(await screen.findByText("trending-demo")).toBeInTheDocument();
  expect(api.listSkillsShTrending).toHaveBeenCalledWith(0);

  // 清空搜索关键词回到趋势榜。
  fireEvent.change(screen.getByLabelText("搜索可发现的 Skill"), {
    target: { value: "pdf" },
  });
  fireEvent.keyDown(screen.getByLabelText("搜索可发现的 Skill"), { key: "Enter" });
  expect(await screen.findByText("skills.sh · pdf")).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("搜索可发现的 Skill"), {
    target: { value: "" },
  });
  expect(await screen.findByText("trending-demo")).toBeInTheDocument();
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

test("discovery repo filter includes repos only known from installed skills", async () => {
  vi.mocked(api.listAssets).mockResolvedValue([
    {
      assetType: "skill",
      id: "ask-ui",
      name: "ask-ui",
      updatedAt: 1,
      hash: "h",
      origin: { kind: "github", repo: "iamddatm/skills", updateAvailable: false, updatable: true },
    },
  ]);
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
    repos: [
      { owner: "acme", name: "skills", branch: "main", enabled: true },
      { owner: "iamddatm", name: "skills", branch: "main", enabled: true },
    ],
  });

  render(<SkillsPanel />);
  fireEvent.click(await screen.findByRole("tab", { name: /发现/ }));
  expect(await screen.findByRole("button", { name: "仓库 · 2" })).toBeInTheDocument();
  fireEvent.click(await screen.findByRole("button", { name: "仓库：2/2" }));
  expect(await screen.findByRole("checkbox", { name: "iamddatm/skills" })).toBeInTheDocument();
  // 已安装页仍挂载在隐藏容器里，同名 Skill 会出现两次。
  expect(screen.getAllByText("ask-ui").length).toBeGreaterThan(0);
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
