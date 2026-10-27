import { useMemo, useState } from "react";
import {
  filterByCommonFilters,
  repoOptions,
  selectedRepoKeys as toRepoKeys,
  toggled,
} from "./skillFilters";
import type { AssetSummary } from "../../types";

/**
 * Skill 列表的筛选与勾选状态。用户级面板与项目级面板在这一块是逐字相同的：
 * 搜索、来源、仓库三轴的谓词完全共用（`skillFilters`），「启用 / 部署状态」那一轴
 * 取值不同，所以只把前三轴收在这里，第四轴留给面板自己 filter。
 *
 * 行数据用泛型：项目级把每行折算成 `AssetSummary & { skill }` 形状，
 * 复用同一套谓词而不丢自己的附加字段。
 */
export function useSkillFilters<T extends AssetSummary>(items: T[]) {
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState("all");
  const [deselectedRepos, setDeselectedRepos] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const repoChoices = useMemo(() => repoOptions(items), [items]);
  const repoKeys = useMemo(
    () => toRepoKeys(repoChoices.allKeys, deselectedRepos),
    [repoChoices, deselectedRepos],
  );
  const allReposSelected =
    repoChoices.allKeys.length > 0 && repoKeys.size === repoChoices.allKeys.length;

  /** 前三轴已收窄完的候选；面板再按自己的状态轴过滤出 visible。 */
  const matched = useMemo(
    () => filterByCommonFilters(items, { query, sourceFilter, repoKeys }),
    [items, query, sourceFilter, repoKeys],
  );

  const toggleRepo = (key: string, checked: boolean) => {
    // 存的是「已排除」集合，勾选对应移出。
    setDeselectedRepos((previous) => toggled(previous, key, !checked));
  };

  const toggleAllRepos = (checked: boolean) => {
    setDeselectedRepos(checked ? new Set() : new Set(repoChoices.allKeys));
  };

  const toggleOne = (id: string, checked: boolean) => {
    setSelected((previous) => toggled(previous, id, checked));
  };

  const allVisibleSelected = (visible: readonly T[]) =>
    visible.length > 0 && visible.every((item) => selected.has(item.id));

  /** 表头全选：以当前可见行为准，已全选则清空。 */
  const toggleAll = (visible: readonly T[]) => {
    setSelected((previous) => {
      const next = new Set(previous);
      if (allVisibleSelected(visible)) {
        visible.forEach((item) => next.delete(item.id));
      } else {
        visible.forEach((item) => next.add(item.id));
      }
      return next;
    });
  };

  const drop = (ids: readonly string[]) => {
    setSelected((previous) => {
      const next = new Set(previous);
      ids.forEach((id) => next.delete(id));
      return next;
    });
  };

  const clear = () => setSelected(new Set());

  const selectedItems = items.filter((item) => selected.has(item.id));
  const selectedIds = selectedItems.map((item) => item.id);

  return {
    query,
    setQuery,
    sourceFilter,
    setSourceFilter,
    repoChoices,
    repoKeys,
    allReposSelected,
    matched,
    selected,
    selectedItems,
    selectedIds,
    toggleRepo,
    toggleAllRepos,
    toggleOne,
    toggleAll,
    allVisibleSelected,
    drop,
    clear,
  };
}
