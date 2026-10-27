// Skill 列表的筛选判定：全局作用域与项目作用域共用同一套搜索、来源、仓库维度。
// 「启用/部署状态」这一轴在两个作用域里取值不同，留给各自的面板。
import type { AssetSummary, SkillCheckReport } from "../../types";

/** 仓库多选里「没有来源仓库」的桶：本地目录、无法识别来源的旧数据。 */
export const NO_REPO_KEY = "__no_repo__";

/** 来源筛选的取值；旧数据没有 `origin` 时按 source 字段推断。 */
export function sourceKey(item: AssetSummary): string {
  return item.origin?.kind ?? (item.source ? "github" : "local");
}

export function repoLabelOf(item: AssetSummary): string | null {
  return item.origin?.repo ?? item.source ?? null;
}

export function repoKeyOf(item: AssetSummary): string {
  return repoLabelOf(item)?.toLowerCase() ?? NO_REPO_KEY;
}

/** 多选集合的增删：`present` 为真时加入，返回新 Set 便于直接放进 state。 */
export function toggled(set: Set<string>, value: string, present: boolean): Set<string> {
  const next = new Set(set);
  if (present) next.add(value);
  else next.delete(value);
  return next;
}

export interface RepoOptions {
  /** 按显示名排序的仓库候选；key 小写用于匹配，label 保留原样用于展示。 */
  repos: { key: string; label: string }[];
  hasNoRepo: boolean;
  /** 可勾选的全部 key（含 `NO_REPO_KEY`）；为空表示没有仓库维度可筛。 */
  allKeys: string[];
}

export function repoOptions(items: AssetSummary[]): RepoOptions {
  const map = new Map<string, string>();
  let hasNoRepo = false;
  for (const item of items) {
    const repo = repoLabelOf(item);
    if (repo) map.set(repo.toLowerCase(), repo);
    else hasNoRepo = true;
  }
  const repos = [...map.entries()]
    .sort((left, right) => left[1].localeCompare(right[1]))
    .map(([key, label]) => ({ key, label }));
  return {
    repos,
    hasNoRepo,
    allKeys: [...repos.map((repo) => repo.key), ...(hasNoRepo ? [NO_REPO_KEY] : [])],
  };
}

/** UI 存的是「已排除」集合（默认全不选 = 全部生效），这里折算成生效的 key。 */
export function selectedRepoKeys(allKeys: string[], deselected: Set<string>): Set<string> {
  return new Set(allKeys.filter((key) => !deselected.has(key)));
}

export interface CommonFilters {
  /** 原始输入；内部 trim + 小写，空串等于不按搜索过滤。 */
  query: string;
  /** 来源筛选值；`"all"` 等于不过滤。 */
  sourceFilter: string;
  /** 生效的仓库 key；不在集合里的条目一律排除。 */
  repoKeys: Set<string>;
}

export function filterByCommonFilters<T extends AssetSummary>(
  items: T[],
  { query, sourceFilter, repoKeys }: CommonFilters,
): T[] {
  const normalized = query.trim().toLowerCase();
  return items.filter((item) => {
    if (
      normalized &&
      ![item.name, item.id, item.description ?? "", item.source ?? "", item.origin?.repo ?? ""]
        .join(" ")
        .toLowerCase()
        .includes(normalized)
    ) {
      return false;
    }
    if (sourceFilter !== "all" && sourceKey(item) !== sourceFilter) return false;
    if (!repoKeys.has(repoKeyOf(item))) return false;
    return true;
  });
}

/** 检查报告的失败行：仓库级与单条级并成一串，两版面板的展示口径一致。 */
export function checkFailureLines(report: SkillCheckReport | null): string[] {
  if (!report) return [];
  return [
    ...report.repoFailures.map((failure) => `${failure.repo}：${failure.error}`),
    ...report.skillFailures.map((failure) => `${failure.id}：${failure.error}`),
  ];
}
