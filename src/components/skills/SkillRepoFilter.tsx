import { NO_REPO_KEY, type RepoOptions } from "./skillFilters";
import { CheckOption, CheckPopover } from "../ui/CheckPopover";

/**
 * Skill 列表的「仓库」多选气泡。用户级与项目级面板在这一块逐字相同，
 * 状态都由 `useSkillFilters` 提供，所以形状也收在一处，改文案只动一遍。
 */
export function SkillRepoFilter({
  repoChoices,
  repoKeys,
  allReposSelected,
  onToggleRepo,
  onToggleAllRepos,
}: {
  repoChoices: RepoOptions;
  repoKeys: Set<string>;
  allReposSelected: boolean;
  onToggleRepo: (key: string, checked: boolean) => void;
  onToggleAllRepos: (checked: boolean) => void;
}) {
  if (repoChoices.allKeys.length === 0) return null;
  return (
    <CheckPopover
      label={
        allReposSelected
          ? `仓库：全部（${repoChoices.allKeys.length}）`
          : `仓库：${repoKeys.size}/${repoChoices.allKeys.length}`
      }
    >
      <CheckOption label="全部仓库" checked={allReposSelected} onChange={onToggleAllRepos} />
      {repoChoices.repos.map((repo) => (
        <CheckOption
          key={repo.key}
          mono
          label={repo.label}
          checked={repoKeys.has(repo.key)}
          onChange={(checked) => onToggleRepo(repo.key, checked)}
        />
      ))}
      {repoChoices.hasNoRepo && (
        <CheckOption
          label="无来源仓库"
          checked={repoKeys.has(NO_REPO_KEY)}
          onChange={(checked) => onToggleRepo(NO_REPO_KEY, checked)}
        />
      )}
    </CheckPopover>
  );
}
