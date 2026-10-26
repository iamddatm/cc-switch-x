import { beforeEach, expect, test, vi } from "vitest";
import { projectRow, projectScope, storeScope } from "../components/skills/skillScope";
import { api } from "../lib/api";
import type { ProjectSkillView } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    previewSkillSource: vi.fn(),
    installSkill: vi.fn(),
    installDiscoveredSkill: vi.fn(),
    setSkillsEnabled: vi.fn(),
    importSkillDir: vi.fn(),
    writeAsset: vi.fn(),
    readAsset: vi.fn(),
    checkSkillUpdates: vi.fn(),
    updateSkills: vi.fn(),
    installProjectSkill: vi.fn(),
    installDiscoveredProjectSkill: vi.fn(),
    importProjectSkillDir: vi.fn(),
    createProjectSkill: vi.fn(),
    readProjectSkillBody: vi.fn(),
    writeProjectSkillBody: vi.fn(),
    setProjectSkillTargets: vi.fn(),
    syncProjectSkills: vi.fn(),
    checkProjectSkillUpdates: vi.fn(),
    updateProjectSkills: vi.fn(),
  },
}));

function projectSkill(overrides: Partial<ProjectSkillView> = {}): ProjectSkillView {
  return {
    id: "demo",
    name: "Demo",
    description: "演示",
    targets: ["claude"],
    dirs: {},
    source: {
      kind: "github",
      repo: "acme/demo",
      branch: "main",
      subpath: "skills/demo",
      treeHash: "remote-hash",
    },
    lastOp: null,
    canonical: true,
    canonicalMissing: false,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

test("store 适配器仍然逐字走全局那几条命令", async () => {
  vi.mocked(api.installSkill).mockResolvedValue({
    outcomes: [{ id: "demo", name: ".", status: "installed", warnings: [] }],
  });
  vi.mocked(api.setSkillsEnabled).mockResolvedValue({ ok: [], failed: [] });
  const scope = storeScope([{ id: "claude", displayName: "Claude Code" }]);

  await expect(
    scope.installSource({ kind: "zip", path: "a.zip" }, "preview-1", [], "skip"),
  ).resolves.toHaveLength(1);
  expect(api.installSkill).toHaveBeenCalledWith({ kind: "zip", path: "a.zip" }, "preview-1", [], "skip");

  await expect(scope.deploy("demo", ["claude"])).resolves.toEqual([]);
  expect(api.setSkillsEnabled).toHaveBeenCalledWith(["demo"], "claude", true);
});

test("项目适配器把安装指向项目真源，并且不带渲染目标", async () => {
  vi.mocked(api.installProjectSkill).mockResolvedValue({
    installed: [{ id: "demo", name: ".", status: "installed", warnings: [] }],
    render: { outcomes: [], conflicts: [], warnings: [] },
    warnings: [],
  });
  const scope = projectScope("app-1", [], [{ key: "claude", label: ".claude/skills" }]);

  const outcomes = await scope.installSource(
    { kind: "directory", path: "D:/incoming/demo" },
    null,
    [{ relativePath: ".", id: "demo" }],
    "skip",
  );

  expect(outcomes).toHaveLength(1);
  expect(api.installProjectSkill).toHaveBeenCalledWith(
    "app-1",
    { kind: "directory", path: "D:/incoming/demo" },
    null,
    [{ relativePath: ".", id: "demo" }],
    "skip",
    [],
  );
});

test("项目部署是设目标加同步两步，失败格子要回报", async () => {
  vi.mocked(api.setProjectSkillTargets).mockResolvedValue(projectSkill());
  vi.mocked(api.syncProjectSkills).mockResolvedValue({
    outcomes: [
      { skillId: "demo", dir: "claude", action: "render", status: "ok" },
      { skillId: "demo", dir: "qoder", action: "render", status: "failed", error: "回读不一致" },
    ],
    conflicts: [],
    warnings: [],
  });
  const scope = projectScope("app-1", [], []);

  await expect(scope.deploy("demo", ["claude", "qoder"])).resolves.toEqual([
    "qoder：回读不一致",
  ]);
  expect(api.setProjectSkillTargets).toHaveBeenCalledWith("app-1", "demo", [
    "claude",
    "qoder",
  ]);
});

test("预览的「已存在」按项目真源换算，不按中央库", async () => {
  vi.mocked(api.previewSkillSource).mockResolvedValue({
    kind: "github",
    repo: "acme/demo",
    branch: "main",
    stagingId: "preview-1",
    warnings: [],
    candidates: [
      { relativePath: "skills/demo", name: "Demo", suggestedId: "acme-demo-skills-demo", exists: false },
      { relativePath: "skills/other", name: "Other", suggestedId: "acme-demo-skills-other", exists: true },
    ],
  });
  const scope = projectScope("app-1", ["acme-demo-skills-other"], []);

  const preview = await scope.previewSource({ kind: "github", repo: "acme/demo", branch: null });

  expect(preview.candidates.map((item) => item.exists)).toEqual([false, true]);
});

test("新建与编辑分别走 create 与只换正文", async () => {
  vi.mocked(api.createProjectSkill).mockResolvedValue({
    installed: [],
    render: { outcomes: [], conflicts: [], warnings: [] },
    warnings: [],
  });
  vi.mocked(api.writeProjectSkillBody).mockResolvedValue({
    outcomes: [],
    conflicts: [],
    warnings: [],
  });
  const scope = projectScope("app-1", [], []);

  await scope.saveSkill("demo", "---\nname: Demo\n---\n", true);
  expect(api.createProjectSkill).toHaveBeenCalledWith("app-1", "demo", "---\nname: Demo\n---\n", []);
  expect(api.writeProjectSkillBody).not.toHaveBeenCalled();

  await scope.saveSkill("demo", "---\nname: Demo\n---\n改过\n", false);
  expect(api.writeProjectSkillBody).toHaveBeenCalledWith(
    "app-1",
    "demo",
    "---\nname: Demo\n---\n改过\n",
  );
  expect(api.createProjectSkill).toHaveBeenCalledTimes(1);
});

test("项目行折算成 AssetSummary：来源、可更新与检查状态都来自台账和本机缓存", () => {
  const row = projectRow(
    projectSkill({
      check: {
        updateAvailable: true,
        latestHash: "remote-hash",
        lastCheckedAt: 5,
        lastCheckError: null,
        updatable: true,
      },
    }),
  );

  expect(row.assetType).toBe("skill");
  expect(row.id).toBe("demo");
  expect(row.name).toBe("Demo");
  expect(row.description).toBe("演示");
  expect(row.origin).toMatchObject({
    kind: "github",
    repo: "acme/demo",
    branch: "main",
    updateAvailable: true,
    lastCheckedAt: 5,
    updatable: true,
    readmeUrl: "https://github.com/acme/demo/blob/main/skills/demo/SKILL.md",
  });
});

test("本地来源的项目行不谎称可更新", () => {
  const row = projectRow(
    projectSkill({
      source: { kind: "local", repo: null, branch: null, subpath: null, treeHash: null },
    }),
  );

  expect(row.origin?.updatable).toBe(false);
  expect(row.origin?.updateAvailable).toBe(false);
  expect(row.origin?.readmeUrl).toBeNull();
});
