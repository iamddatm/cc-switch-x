import { act, renderHook } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { NO_REPO_KEY } from "../components/skills/skillFilters";
import { useSkillFilters } from "../components/skills/useSkillFilters";
import type { AssetSummary, SkillOrigin } from "../types";

function skill(id: string, origin?: Partial<SkillOrigin>): AssetSummary {
  return {
    assetType: "skill",
    id,
    name: id,
    updatedAt: 1,
    hash: `h-${id}`,
    origin: origin ? ({ kind: "github", ...origin } as SkillOrigin) : undefined,
  };
}

// 带一个附加字段，验证面板行的自有字段不会在过滤中被抹掉
const rows = [
  { ...skill("acme-a", { repo: "acme/ai" }), extra: 1 },
  { ...skill("beta-b", { repo: "beta/pack" }), extra: 2 },
  { ...skill("local-c"), extra: 3 },
];

/** 每次重新取 result.current：钩子每轮渲染都返回新对象，缓存住旧对象会读到过期值。 */
function setup() {
  const { result } = renderHook(() => useSkillFilters(rows));
  return {
    get view() {
      return result.current;
    },
    act: (run: () => void) => act(run),
  };
}

describe("useSkillFilters", () => {
  test("matched 只收搜索 / 来源 / 仓库三轴，保留行上的附加字段", () => {
    const hooks = setup();
    expect(hooks.view.matched.map((row) => row.id)).toEqual(["acme-a", "beta-b", "local-c"]);
    expect(hooks.view.matched[0].extra).toBe(1);

    hooks.act(() => hooks.view.setQuery("beta"));
    expect(hooks.view.matched.map((row) => row.id)).toEqual(["beta-b"]);
  });

  test("取消勾选仓库是把它记入排除集，生效集随之收窄", () => {
    const hooks = setup();
    expect(hooks.view.repoKeys).toEqual(new Set(["acme/ai", "beta/pack", NO_REPO_KEY]));

    hooks.act(() => hooks.view.toggleRepo("acme/ai", false));
    expect(hooks.view.repoKeys).toEqual(new Set(["beta/pack", NO_REPO_KEY]));
    expect(hooks.view.matched.map((row) => row.id)).toEqual(["beta-b", "local-c"]);

    hooks.act(() => hooks.view.toggleRepo("acme/ai", true));
    expect(hooks.view.allReposSelected).toBe(true);
  });

  test("「全部仓库」一次清空或一次全选", () => {
    const hooks = setup();
    hooks.act(() => hooks.view.toggleAllRepos(false));
    expect(hooks.view.repoKeys.size).toBe(0);
    expect(hooks.view.matched).toEqual([]);

    hooks.act(() => hooks.view.toggleAllRepos(true));
    expect(hooks.view.allReposSelected).toBe(true);
    expect(hooks.view.matched).toHaveLength(3);
  });

  test("表头全选只影响传入的可见行，再点一次清空这批", () => {
    const hooks = setup();
    const visible = rows.slice(0, 2);

    hooks.act(() => hooks.view.toggleAll(visible));
    expect(hooks.view.selectedIds).toEqual(["acme-a", "beta-b"]);
    expect(hooks.view.allVisibleSelected(visible)).toBe(true);
    expect(hooks.view.allVisibleSelected(rows)).toBe(false);

    hooks.act(() => hooks.view.toggleAll(visible));
    expect(hooks.view.selected.size).toBe(0);
  });

  test("单选增删与 drop / clear 都返回新集合", () => {
    const hooks = setup();
    hooks.act(() => hooks.view.toggleOne("acme-a", true));
    hooks.act(() => hooks.view.toggleOne("beta-b", true));
    expect(hooks.view.selectedItems.map((row) => row.id)).toEqual(["acme-a", "beta-b"]);

    hooks.act(() => hooks.view.drop(["acme-a"]));
    expect(hooks.view.selectedIds).toEqual(["beta-b"]);

    hooks.act(() => hooks.view.clear());
    expect(hooks.view.selectedIds).toEqual([]);
  });
});
