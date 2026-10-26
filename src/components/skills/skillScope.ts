// Skill 组件的作用域适配器：同一套抽屉、市场、详情组件，装的东西去向不同。
// 全局 = 中央库 `assets/skills/<id>`（来源在 `.meta.json`，启用 = 部署到 agent 目录）；
// 项目 = `<project>/.ccswitchx/skills/<id>`（来源在台账 `skills.json`，部署 = 渲染到勾选的目录）。
// 组件只认这个接口，不在内部写「是全局还是项目」的分支。
import { api } from "../../lib/api";
import type {
  AssetSummary,
  ConflictPolicy,
  DiscoveredSkillInstallInput,
  InstallSelection,
  ProjectSkillSource,
  ProjectSkillView,
  SkillCheckReport,
  SkillInstallOutcome,
  SkillOrigin,
  SkillSourceInput,
  SkillSourcePreview,
  SkillSourceKind,
  SkillUpdateReport,
} from "../../types";

export interface SkillDeployTarget {
  key: string;
  label: string;
}

export interface SkillScope {
  kind: "store" | "project";
  /** 抽屉与提示里对目的端的称呼，让用户知道文件会写到哪。 */
  label: string;
  /** 覆盖真源之外还能一步铺开的落点：全局是 agent，项目是渲染目录。 */
  deployTargets: SkillDeployTarget[];
  previewSource: (input: SkillSourceInput) => Promise<SkillSourcePreview>;
  installSource: (
    input: SkillSourceInput,
    stagingId: string | null,
    selections: InstallSelection[],
    policy: ConflictPolicy,
  ) => Promise<SkillInstallOutcome[]>;
  installDiscovered: (
    input: DiscoveredSkillInstallInput,
    policy: ConflictPolicy,
  ) => Promise<SkillInstallOutcome>;
  /** 把一个 skill 铺到落点，返回失败说明；空数组代表全部成功。 */
  deploy: (id: string, targets: string[]) => Promise<string[]>;
  importDir: (path: string) => Promise<SkillInstallOutcome>;
  /** `creating` 为真时走新建，否则只替换真源的 SKILL.md。 */
  saveSkill: (id: string, content: string, creating: boolean) => Promise<void>;
  readBody: (id: string) => Promise<string>;
  checkUpdates: (ids?: string[]) => Promise<SkillCheckReport>;
  updateSkills: (ids: string[]) => Promise<SkillUpdateReport>;
}

export function storeScope(agents: { id: string; displayName: string }[]): SkillScope {
  return {
    kind: "store",
    label: "中央库真源",
    deployTargets: agents.map((agent) => ({ key: agent.id, label: agent.displayName })),
    previewSource: (input) => api.previewSkillSource(input),
    installSource: async (input, stagingId, selections, policy) =>
      (await api.installSkill(input, stagingId, selections, policy)).outcomes,
    installDiscovered: (input, policy) => api.installDiscoveredSkill(input, policy),
    deploy: async (id, targets) => {
      const failures: string[] = [];
      for (const agentId of targets) {
        const applied = await api.setSkillsEnabled([id], agentId, true);
        failures.push(...applied.failed.map((item) => `${agentId}：${item.error}`));
      }
      return failures;
    },
    importDir: async (path) => {
      const item = await api.importSkillDir(path);
      return { id: item.id, name: item.name, status: "installed", warnings: [] };
    },
    saveSkill: (id, content) => api.writeAsset("skill", id, content).then(() => undefined),
    readBody: (id) => api.readAsset("skill", id),
    checkUpdates: (ids) => api.checkSkillUpdates(ids),
    updateSkills: (ids) => api.updateSkills(ids),
  };
}

/**
 * 项目作用域。`existingIds` 用来把预览候选上的「已存在」标成项目真源的情况：
 * 预览命令只看中央库，不换算的话会把「项目里已经有了」显示错。
 */
export function projectScope(
  projectId: string,
  existingIds: string[],
  dirs: SkillDeployTarget[],
): SkillScope {
  const previewSource = async (input: SkillSourceInput) => {
    const preview = await api.previewSkillSource(input);
    const existing = new Set(existingIds);
    return {
      ...preview,
      candidates: preview.candidates.map((candidate) => ({
        ...candidate,
        exists: existing.has(candidate.suggestedId),
      })),
    };
  };
  return {
    kind: "project",
    label: "项目真源",
    deployTargets: dirs,
    previewSource,
    // 安装只落真源，不带渲染目标：往仓库的 agent 目录里写文件必须由「勾选 + 同步」明确发生。
    installSource: async (input, stagingId, selections, policy) =>
      (await api.installProjectSkill(projectId, input, stagingId, selections, policy, []))
        .installed,
    installDiscovered: async (input, policy) =>
      (
        await api.installDiscoveredProjectSkill(
          projectId,
          {
            repo: input.repo,
            branch: input.branch,
            relativePath: input.relativePath,
            directory: input.directory,
            name: input.name,
          },
          policy,
          [],
        )
      ).installed[0],
    deploy: async (id, targets) => {
      await api.setProjectSkillTargets(projectId, id, targets);
      const report = await api.syncProjectSkills(projectId);
      return report.outcomes
        .filter((outcome) => outcome.status !== "ok")
        .map((outcome) => `${outcome.dir}：${outcome.error ?? outcome.action}`);
    },
    importDir: async (path) => {
      const report = await api.importProjectSkillDir(projectId, path, []);
      const outcome = report.installed[0];
      if (!outcome) throw new Error("导入没有返回结果");
      return outcome;
    },
    saveSkill: async (id, content, creating) => {
      if (creating) {
        await api.createProjectSkill(projectId, id, content, []);
        return;
      }
      await api.writeProjectSkillBody(projectId, id, content);
    },
    readBody: (id) => api.readProjectSkillBody(projectId, id),
    checkUpdates: (ids) => api.checkProjectSkillUpdates(projectId, ids),
    updateSkills: (ids) => api.updateProjectSkills(projectId, ids),
  };
}

/** GitHub 来源的 README 链接：全局存在 `.meta.json`，项目按来源三要素折算。 */
function readmeUrlOf(source: ProjectSkillSource): string | null {
  if (source.kind !== "github" || !source.repo || !source.branch) return null;
  const subpath = source.subpath && source.subpath !== "." ? `${source.subpath}/` : "";
  return `https://github.com/${source.repo}/blob/${source.branch}/${subpath}SKILL.md`;
}

/**
 * 项目行折算成 `AssetSummary` 形状，好让全局的行组件、来源徽标与筛选谓词原样复用。
 * `mtime`/`hash` 项目视图里没有，留空；行组件不显示它们。
 */
export function projectRow(skill: ProjectSkillView): AssetSummary {
  const origin: SkillOrigin | null = skill.source
    ? {
        kind: skill.source.kind as SkillSourceKind,
        repo: skill.source.repo ?? null,
        branch: skill.source.branch ?? null,
        sourceUrl: null,
        readmeUrl: readmeUrlOf(skill.source),
        installedAt: null,
        updatedAt: null,
        lastCheckedAt: skill.check?.lastCheckedAt ?? null,
        updateAvailable: skill.check?.updateAvailable ?? false,
        lastCheckError: skill.check?.lastCheckError ?? null,
        updatable: skill.source.kind === "github" && Boolean(skill.source.repo),
      }
    : null;
  return {
    assetType: "skill",
    id: skill.id,
    name: skill.name,
    updatedAt: 0,
    hash: "",
    description: skill.description,
    mtime: null,
    origin,
  };
}
