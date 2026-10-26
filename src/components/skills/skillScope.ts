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
  /** 落点动作的措辞。组件不判断作用域，文案由这里给，两个面板才对得上。 */
  wording: {
    /** 批量条与市场的落点选择器标题，例如「启用给」「渲染到」。 */
    targets: string;
    /** 成功提示里「已启用给 X」/「已渲染到 X」。 */
    deployed: string;
    /** 一个落点都没选时的收尾话术。 */
    noneDeployed: string;
  };
  /** 安装抽屉末尾的去向说明：装完会不会自动铺开，两个作用域不一样。 */
  installHint: string;
  /** 预览候选已在目的端存在时的徽标文案。 */
  existsLabel: string;
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

export function storeScope(
  agents: { id: string; displayName: string }[] = [],
): SkillScope {
  const targets: SkillDeployTarget[] = agents.map((agent) => ({
    key: agent.id,
    label: agent.displayName,
  }));
  return {
    kind: "store",
    label: "中央库真源",
    deployTargets: targets,
    wording: {
      targets: "启用给",
      deployed: "已启用给",
      noneDeployed: "未启用给任何 Agent",
    },
    installHint: "安装只写入中央库真源，不会自动启用给 agent。",
    existsLabel: "真源已存在",
    previewSource: (input) => api.previewSkillSource(input),
    installSource: async (input, stagingId, selections, policy) =>
      (await api.installSkill(input, stagingId, selections, policy)).outcomes,
    installDiscovered: (input, policy) => api.installDiscoveredSkill(input, policy),
    deploy: async (id, targetsToEnable) => {
      const failures: string[] = [];
      for (const key of targetsToEnable) {
        const label = targets.find((target) => target.key === key)?.label ?? key;
        const applied = await api.setSkillsEnabled([id], key, true);
        failures.push(...applied.failed.map((item) => `${label}：${item.error}`));
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
    wording: {
      targets: "渲染到",
      deployed: "已渲染到",
      noneDeployed: "未渲染到任何目录",
    },
    installHint:
      "安装只写入项目真源 .ccswitchx/skills/，要铺开还得勾选渲染目录后同步。",
    existsLabel: "项目真源里已存在",
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
