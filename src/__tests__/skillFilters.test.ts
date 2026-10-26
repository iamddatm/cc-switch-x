import { describe, expect, test } from "vitest";
import {
  NO_REPO_KEY,
  filterByCommonFilters,
  repoOptions,
  selectedRepoKeys,
  sourceKey,
  toggled,
} from "../components/skills/skillFilters";
import type { AssetSummary, SkillOrigin } from "../types";

function skill(id: string, origin?: Partial<SkillOrigin>, extra: Partial<AssetSummary> = {}): AssetSummary {
  return {
    assetType: "skill",
    id,
    name: id,
    updatedAt: 1,
    hash: `h-${id}`,
    origin: origin ? ({ kind: "github", ...origin } as SkillOrigin) : undefined,
    ...extra,
  };
}

const items: AssetSummary[] = [
  skill("acme-ai-sql", { repo: "acme/ai" }, { description: "写 SQL" }),
  skill("Beta-Pack", { repo: "beta/pack" }),
  skill("local-note", undefined, { source: "D:/skills/local-note" }),
  skill("no-source"),
];

const ALL_KEYS = repoOptions(items).allKeys;

describe("skillFilters", () => {
  test("仓库候选大小写归一并按显示名排序，无仓库的条目单独成桶", () => {
    const options = repoOptions([
      skill("b", { repo: "Zeta/Repo" }),
      skill("a", { repo: "alpha/repo" }),
      skill("c"),
    ]);
    expect(options.repos.map((repo) => repo.label)).toEqual(["alpha/repo", "Zeta/Repo"]);
    expect(options.repos.map((repo) => repo.key)).toEqual(["alpha/repo", "zeta/repo"]);
    expect(options.hasNoRepo).toBe(true);
    expect(options.allKeys).toEqual(["alpha/repo", "zeta/repo", NO_REPO_KEY]);
  });

  test("同一仓库只出现一次候选", () => {
    const options = repoOptions([
      skill("one", { repo: "acme/ai" }),
      skill("two", { repo: "acme/ai" }),
    ]);
    expect(options.repos).toHaveLength(1);
    expect(options.hasNoRepo).toBe(false);
  });

  test("本地目录来源也进仓库候选，只有完全没来源的才落进无仓库桶", () => {
    expect(repoOptions(items).allKeys).toEqual([
      "acme/ai",
      "beta/pack",
      "d:/skills/local-note",
      NO_REPO_KEY,
    ]);
    expect(repoOptions(items).hasNoRepo).toBe(true);
  });

  test("UI 存的是排除集，selectedRepoKeys 折算成生效集", () => {
    expect(selectedRepoKeys(ALL_KEYS, new Set())).toEqual(new Set(ALL_KEYS));
    expect(selectedRepoKeys(ALL_KEYS, new Set(["beta/pack"]))).toEqual(
      new Set(["acme/ai", "d:/skills/local-note", NO_REPO_KEY]),
    );
  });

  test("搜索命中名称、ID、描述、来源与仓库", () => {
    const filter = (query: string, repoKeys = new Set(ALL_KEYS)) =>
      filterByCommonFilters(items, { query, sourceFilter: "all", repoKeys }).map(
        (item) => item.id,
      );
    expect(filter("")).toEqual(["acme-ai-sql", "Beta-Pack", "local-note", "no-source"]);
    expect(filter("SQL")).toEqual(["acme-ai-sql"]);
    expect(filter("beta-pack")).toEqual(["Beta-Pack"]);
    expect(filter("ACME/AI")).toEqual(["acme-ai-sql"]);
    expect(filter("d:/skills")).toEqual(["local-note"]);
    expect(filter("  ")).toEqual(["acme-ai-sql", "Beta-Pack", "local-note", "no-source"]);
    expect(filter("不存在")).toEqual([]);
  });

  test("没有 origin 的旧数据按 source 字段推断来源", () => {
    expect(sourceKey(skill("x", { repo: "acme/ai" }))).toBe("github");
    expect(sourceKey(skill("y", undefined, { source: "D:/y" }))).toBe("github");
    expect(sourceKey(skill("z"))).toBe("local");
  });

  test("来源筛选与仓库多选叠加", () => {
    expect(
      filterByCommonFilters(items, {
        query: "",
        sourceFilter: "local",
        repoKeys: new Set(ALL_KEYS),
      }).map((item) => item.id),
    ).toEqual(["no-source"]);

    expect(
      filterByCommonFilters(items, {
        query: "",
        sourceFilter: "all",
        repoKeys: new Set(["acme/ai"]),
      }).map((item) => item.id),
    ).toEqual(["acme-ai-sql"]);
  });

  test("toggled 返回新集合，不改原集合", () => {
    const before = new Set(["a"]);
    const added = toggled(before, "b", true);
    const removed = toggled(before, "a", false);
    expect(before).toEqual(new Set(["a"]));
    expect(added).toEqual(new Set(["a", "b"]));
    expect(removed).toEqual(new Set());
  });
});
