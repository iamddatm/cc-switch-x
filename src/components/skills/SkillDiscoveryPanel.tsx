import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { ChevronDown, Download, RefreshCw, Sparkles } from "lucide-react";
import { api } from "../../lib/api";
import type {
  AgentStatus,
  AssetSummary,
  DiscoverableSkill,
  SkillDiscoveryReport,
  SkillRepo,
  SkillsShSkill,
} from "../../types";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { EmptyState } from "../ui/EmptyState";
import { IconButton } from "../ui/IconButton";
import { Notice } from "../ui/Notice";
import { SearchField } from "../ui/SearchField";
import { SegmentedControl } from "../ui/SegmentedControl";
import { RepoManagerSheet } from "./RepoManagerSheet";

type Source = "repos" | "skillssh";

function repoKey(repo: SkillRepo): string {
  return `${repo.owner}/${repo.name}`.toLowerCase();
}

function formatInstalls(count: number): string {
  if (count >= 1000) return `${(count / 1000).toFixed(1)}k`;
  return String(count);
}

/** 多选筛选的下拉层；用原生 details，点选不会像菜单那样立即收起。 */
function CheckPopover({ label, children }: { label: string; children: ReactNode }) {
  return (
    <details className="relative shrink-0">
      <summary className="flex h-8 cursor-pointer list-none items-center gap-1.5 rounded-control border border-border bg-surface px-2.5 text-caption text-fg-2 transition-colors hover:bg-subtle hover:text-fg-1 [&::-webkit-details-marker]:hidden">
        {label}
        <ChevronDown size={12} aria-hidden />
      </summary>
      <div className="absolute end-0 z-30 mt-1 max-h-72 w-64 overflow-auto rounded-panel border border-border bg-surface p-2 shadow-v7-md">
        {children}
      </div>
    </details>
  );
}

export function SkillDiscoveryPanel({
  installedItems,
  agents,
  onInstalled,
  onShowInstalled,
}: {
  installedItems: AssetSummary[];
  agents: AgentStatus[];
  onInstalled: () => void;
  onShowInstalled: () => void;
}) {
  const [source, setSource] = useState<Source>("repos");
  const [repos, setRepos] = useState<SkillRepo[]>([]);
  const [reposLoading, setReposLoading] = useState(true);
  const [reposError, setReposError] = useState<string | null>(null);
  const [deselectedRepos, setDeselectedRepos] = useState<Set<string>>(new Set());
  const [deselectedTargets, setDeselectedTargets] = useState<Set<string>>(new Set());
  const [report, setReport] = useState<SkillDiscoveryReport | null>(null);
  const [discoveryLoading, setDiscoveryLoading] = useState(false);
  const [discoveryError, setDiscoveryError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [onlyUninstalled, setOnlyUninstalled] = useState(false);
  const [installingKey, setInstallingKey] = useState<string | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [repoManagerOpen, setRepoManagerOpen] = useState(false);

  const [shResults, setShResults] = useState<SkillsShSkill[]>([]);
  // skills.sh 的 count 是本次返回条数，不是总数；用「本页是否装满」判断还有没有下一页。
  const [shHasMore, setShHasMore] = useState(false);
  const [shOffset, setShOffset] = useState(0);
  const [shLoading, setShLoading] = useState(false);
  const [shError, setShError] = useState<string | null>(null);
  const [shQueryDone, setShQueryDone] = useState<string | null>(null);
  const [shInstalled, setShInstalled] = useState<Set<string>>(new Set());

  const loadRepos = useCallback(async () => {
    setReposLoading(true);
    setReposError(null);
    try {
      setRepos(await api.getSkillRepos());
    } catch (e) {
      setReposError(String(e));
    } finally {
      setReposLoading(false);
    }
  }, []);

  const refreshDiscovery = useCallback(async () => {
    setDiscoveryLoading(true);
    setDiscoveryError(null);
    try {
      setReport(await api.discoverSkills());
    } catch (e) {
      setDiscoveryError(String(e));
    } finally {
      setDiscoveryLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadRepos();
    void refreshDiscovery();
  }, [loadRepos, refreshDiscovery]);

  const enabledRepos = repos.filter((repo) => repo.enabled);
  const selectedRepoKeys = useMemo(
    () =>
      new Set(
        enabledRepos
          .filter((repo) => !deselectedRepos.has(repoKey(repo)))
          .map(repoKey),
      ),
    [enabledRepos, deselectedRepos],
  );
  const targetAgents = agents.filter((agent) => !deselectedTargets.has(agent.id));
  const targetLabel =
    agents.length === 0
      ? "未启用 Agent"
      : targetAgents.length === agents.length
        ? `全部 Agent（${agents.length}）`
        : targetAgents.length === 0
          ? "未选择 Agent"
          : targetAgents.map((agent) => agent.displayName).join("、");

  const shIsInstalled = (skill: SkillsShSkill) => {
    if (shInstalled.has(skill.key)) return true;
    const repo = `${skill.repoOwner}/${skill.repoName}`.toLowerCase();
    const name = skill.name.toLowerCase();
    // skills.sh 的结果没有本地候选 ID，只能按「同仓库 + 同名」判断；
    // 没有来源元数据的老 Skill 才退回到目录名匹配。
    return installedItems.some((item) => {
      if (item.name.toLowerCase() !== name) return false;
      const originRepo = item.origin?.repo?.toLowerCase();
      return originRepo
        ? originRepo === repo
        : item.id.toLowerCase() === skill.directory.toLowerCase();
    });
  };

  const normalizedQuery = query.trim().toLowerCase();
  const visibleSkills = (report?.skills ?? []).filter((skill) => {
    if (!selectedRepoKeys.has(skill.repo.toLowerCase())) return false;
    if (onlyUninstalled && skill.installed) return false;
    if (!normalizedQuery) return true;
    return [skill.name, skill.description ?? "", skill.relativePath, skill.repo].some(
      (value) => value.toLowerCase().includes(normalizedQuery),
    );
  });
  const visibleSh = shResults.filter(
    (skill) => !onlyUninstalled || !shIsInstalled(skill),
  );
  const failures = report?.failures ?? [];

  const submitSh = async (offset: number) => {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
      setShError("至少输入 2 个字符再搜索");
      return;
    }
    const pageSize = 20;
    setShLoading(true);
    setShError(null);
    setMessage(null);
    try {
      const result = await api.searchSkillsSh(trimmed, pageSize, offset);
      setShOffset(offset);
      setShResults((previous) => {
        if (offset === 0) return result.skills;
        const seen = new Set(previous.map((item) => item.key));
        return [...previous, ...result.skills.filter((item) => !seen.has(item.key))];
      });
      setShHasMore(result.skills.length >= pageSize);
      setShQueryDone(trimmed);
    } catch (e) {
      setShError(String(e));
    } finally {
      setShLoading(false);
    }
  };

  const runInstall = async (
    skill: { key: string; name: string },
    input: {
      repo: string;
      branch?: string | null;
      relativePath?: string | null;
      directory?: string | null;
    },
    markInstalled: () => void,
  ) => {
    setInstallingKey(skill.key);
    setError(null);
    setMessage(null);
    setRowErrors((previous) => {
      const next = { ...previous };
      delete next[skill.key];
      return next;
    });
    try {
      const outcome = await api.installDiscoveredSkill(
        { ...input, name: skill.name },
        "skip",
      );
      if (outcome.status === "failed") {
        throw new Error(outcome.error ?? "安装失败");
      }
      const failures: string[] = [];
      for (const agent of targetAgents) {
        const applied = await api.setSkillsEnabled([outcome.id], agent.id, true);
        failures.push(
          ...applied.failed.map((item) => `${agent.displayName}：${item.error}`),
        );
      }
      markInstalled();
      onInstalled();
      const targetText =
        targetAgents.length > 0
          ? `，已启用给 ${targetAgents.map((agent) => agent.displayName).join("、")}`
          : "，未启用给任何 Agent";
      setMessage(
        `已安装 ${skill.name}${outcome.status === "overwritten" ? "（覆盖旧版本）" : ""}${targetText}。`,
      );
      if (failures.length > 0) setError(failures.join("；"));
    } catch (e) {
      setRowErrors((previous) => ({ ...previous, [skill.key]: String(e) }));
    } finally {
      setInstallingKey(null);
    }
  };

  const installRepoSkill = (skill: DiscoverableSkill) =>
    runInstall(
      skill,
      {
        repo: skill.repo,
        branch: skill.branch,
        relativePath: skill.relativePath,
      },
      () =>
        setReport((previous) =>
          previous
            ? {
                ...previous,
                skills: previous.skills.map((item) =>
                  item.key === skill.key ? { ...item, installed: true } : item,
                ),
              }
            : previous,
        ),
    );

  const installShSkill = (skill: SkillsShSkill) =>
    runInstall(
      skill,
      {
        repo: `${skill.repoOwner}/${skill.repoName}`,
        branch: skill.repoBranch,
        directory: skill.directory,
      },
      () => setShInstalled((previous) => new Set(previous).add(skill.key)),
    );

  const toggleRepo = (key: string, checked: boolean) => {
    setDeselectedRepos((previous) => {
      const next = new Set(previous);
      if (checked) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleAllRepos = (checked: boolean) => {
    setDeselectedRepos(checked ? new Set() : new Set(enabledRepos.map(repoKey)));
  };

  const toggleTarget = (id: string, checked: boolean) => {
    setDeselectedTargets((previous) => {
      const next = new Set(previous);
      if (checked) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const repoFailuresText = failures
    .map((failure) => `${failure.repo}@${failure.branch}：${failure.error}`)
    .join("；");

  const renderRepoRows = () => {
    if (visibleSkills.length === 0) {
      if (discoveryLoading && !report) {
        return <p className="py-12 text-center text-caption text-fg-3">正在读取仓库…</p>;
      }
      if (discoveryError) {
        return (
          <EmptyState
            icon={Sparkles}
            title="仓库读取失败"
            description={discoveryError}
            actions={
              <>
                <Button onClick={() => void refreshDiscovery()}>重试</Button>
                <Button onClick={() => setRepoManagerOpen(true)}>管理仓库</Button>
              </>
            }
          />
        );
      }
      if (failures.length > 0) {
        return (
          <EmptyState
            icon={Sparkles}
            title="仓库读取失败"
            description={`${failures.length} 个仓库没有读到：${repoFailuresText}`}
            actions={
              <>
                <Button onClick={() => void refreshDiscovery()}>重试</Button>
                <Button onClick={() => setRepoManagerOpen(true)}>管理仓库</Button>
              </>
            }
          />
        );
      }
      if (repos.length === 0 || enabledRepos.length === 0) {
        return (
          <EmptyState
            icon={Sparkles}
            title="还没有可用的仓库"
            description="添加包含 SKILL.md 的 GitHub 仓库，或直接去 skills.sh 搜索。"
            actions={
              <>
                <Button onClick={() => setRepoManagerOpen(true)}>管理仓库</Button>
                <Button onClick={() => setSource("skillssh")}>去 skills.sh 搜索</Button>
              </>
            }
          />
        );
      }
      return (
        <EmptyState
          icon={Sparkles}
          title={onlyUninstalled ? "全都装好了" : "没有匹配的 Skill"}
          description={
            onlyUninstalled
              ? "当前筛选下没有未安装的 Skill。"
              : "换个关键词或筛选条件，也可以刷新仓库。"
          }
          actions={
            onlyUninstalled ? (
              <Button onClick={() => setOnlyUninstalled(false)}>显示全部</Button>
            ) : undefined
          }
        />
      );
    }
    return (
      <ul className="divide-y divide-border">
        {visibleSkills.map((skill) => {
          const rowError = rowErrors[skill.key];
          return (
            <li key={skill.key} className="flex items-start gap-3 px-4 py-3">
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-body font-medium text-fg-1">
                    {skill.name}
                  </span>
                  <Badge mono className="max-w-[200px]" title={skill.repo}>
                    {skill.repo}
                  </Badge>
                  {skill.relativePath !== "." && skill.name !== skill.relativePath && (
                    <span className="truncate font-mono text-caption text-fg-3">
                      {skill.relativePath}
                    </span>
                  )}
                </div>
                <p
                  className="truncate text-caption text-fg-2"
                  title={skill.description ?? undefined}
                >
                  {skill.description || "没有描述"}
                </p>
                {rowError && (
                  <p className="mt-1 text-caption text-danger-text">{rowError}</p>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {skill.installed ? (
                  <Button variant="ghost" size="sm" onClick={onShowInstalled}>
                    已安装
                  </Button>
                ) : (
                  <Button
                    size="sm"
                    variant="primary"
                    loading={installingKey === skill.key}
                    disabled={installingKey !== null && installingKey !== skill.key}
                    onClick={() => void installRepoSkill(skill)}
                  >
                    安装
                  </Button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    );
  };

  const renderShRows = () => {
    if (shLoading && shResults.length === 0) {
      return <p className="py-12 text-center text-caption text-fg-3">正在搜索 skills.sh…</p>;
    }
    if (shError) {
      return (
        <EmptyState
          icon={Sparkles}
          title="skills.sh 搜索失败"
          description={shError}
          actions={
            <Button onClick={() => void submitSh(shOffset)}>重试</Button>
          }
        />
      );
    }
    if (shQueryDone === null) {
      return (
        <EmptyState
          icon={Sparkles}
          title="搜索 skills.sh"
          description="输入至少 2 个字符，按回车或点击搜索，从公共目录查找 Skill。"
        />
      );
    }
    if (visibleSh.length === 0) {
      return (
        <EmptyState
          icon={Sparkles}
          title={onlyUninstalled ? "没有未安装的结果" : `没有找到「${shQueryDone}」`}
          description={
            onlyUninstalled ? "当前结果都已安装，取消筛选可以看到全部。" : "换个关键词再试。"
          }
          actions={
            onlyUninstalled ? (
              <Button onClick={() => setOnlyUninstalled(false)}>显示全部</Button>
            ) : undefined
          }
        />
      );
    }
    return (
      <>
        <ul className="divide-y divide-border">
          {visibleSh.map((skill) => {
            const installed = shIsInstalled(skill);
            const rowError = rowErrors[skill.key];
            return (
              <li key={skill.key} className="flex items-start gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="truncate text-body font-medium text-fg-1">
                      {skill.name}
                    </span>
                    <Badge mono className="max-w-[200px]">
                      {skill.repoOwner}/{skill.repoName}
                    </Badge>
                    <span className="inline-flex items-center gap-0.5 text-caption text-fg-3">
                      <Download size={12} aria-hidden />
                      {formatInstalls(skill.installs)}
                    </span>
                  </div>
                  <p className="truncate text-caption text-fg-2">
                    skills.sh · {skill.directory}
                  </p>
                  {rowError && (
                    <p className="mt-1 text-caption text-danger-text">{rowError}</p>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  {installed ? (
                    <Button variant="ghost" size="sm" onClick={onShowInstalled}>
                      已安装
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      variant="primary"
                      loading={installingKey === skill.key}
                      disabled={installingKey !== null && installingKey !== skill.key}
                      onClick={() => void installShSkill(skill)}
                    >
                      安装
                    </Button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
        {shHasMore && (
          <div className="border-t border-border p-3 text-center">
            <Button
              size="sm"
              loading={shLoading}
              onClick={() => void submitSh(shOffset + 20)}
            >
              加载更多（已加载 {shResults.length}）
            </Button>
          </div>
        )}
      </>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 px-6 py-2">
        <SegmentedControl<Source>
          ariaLabel="Skill 发现来源"
          size="sm"
          value={source}
          onValueChange={setSource}
          items={[
            { value: "repos", label: `仓库 · ${enabledRepos.length}` },
            { value: "skillssh", label: "skills.sh" },
          ]}
        />
        {source === "repos" && (
          <>
            <CheckPopover
              label={`仓库：${selectedRepoKeys.size}/${enabledRepos.length}`}
            >
              <label className="flex items-center gap-2 rounded-control px-1.5 py-1 text-caption text-fg-2">
                <Checkbox
                  checked={
                    enabledRepos.length > 0 &&
                    selectedRepoKeys.size === enabledRepos.length
                  }
                  onChange={(event) => toggleAllRepos(event.target.checked)}
                />
                全部启用仓库
              </label>
              {enabledRepos.map((repo) => {
                const key = repoKey(repo);
                return (
                  <label
                    key={key}
                    className="flex items-center gap-2 rounded-control px-1.5 py-1 text-caption text-fg-2"
                  >
                    <Checkbox
                      checked={selectedRepoKeys.has(key)}
                      onChange={(event) => toggleRepo(key, event.target.checked)}
                    />
                    <span className="truncate font-mono">
                      {repo.owner}/{repo.name}
                    </span>
                  </label>
                );
              })}
            </CheckPopover>
            <IconButton
              label="刷新发现"
              size="md"
              disabled={discoveryLoading}
              onClick={() => void refreshDiscovery()}
            >
              <RefreshCw
                size={15}
                className={discoveryLoading ? "animate-spin" : undefined}
                aria-hidden
              />
            </IconButton>
          </>
        )}
        <label className="flex h-8 shrink-0 cursor-pointer items-center gap-2 rounded-control px-2 text-caption text-fg-2 hover:bg-subtle">
          <Checkbox
            checked={onlyUninstalled}
            onChange={(event) => setOnlyUninstalled(event.target.checked)}
          />
          只看未安装
        </label>
        <div className="flex-1" />
        <CheckPopover label={`安装到：${targetLabel}`}>
          {agents.length === 0 ? (
            <p className="px-1.5 py-1 text-caption text-fg-3">
              没有启用支持 Skills 的 Agent。
            </p>
          ) : (
            agents.map((agent) => (
              <label
                key={agent.id}
                className="flex items-center gap-2 rounded-control px-1.5 py-1 text-caption text-fg-2"
              >
                <Checkbox
                  checked={!deselectedTargets.has(agent.id)}
                  onChange={(event) => toggleTarget(agent.id, event.target.checked)}
                />
                {agent.displayName}
              </label>
            ))
          )}
        </CheckPopover>
        <SearchField
          className="w-[240px] shrink-0"
          value={query}
          onValueChange={setQuery}
          placeholder={
            source === "skillssh" ? "搜索 skills.sh…" : "搜索 Skill 或仓库…"
          }
          ariaLabel="搜索可发现的 Skill"
          onKeyDown={(event) => {
            if (event.key === "Enter" && source === "skillssh") {
              void submitSh(0);
            }
          }}
        />
        {source === "skillssh" ? (
          <Button size="sm" loading={shLoading} onClick={() => void submitSh(0)}>
            搜索
          </Button>
        ) : (
          <Button size="sm" onClick={() => setRepoManagerOpen(true)}>
            管理仓库
          </Button>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
        {reposError && (
          <Notice tone="danger" className="mb-3">
            {reposError}
          </Notice>
        )}
        {message && (
          <Notice tone="success" className="mb-3">
            {message}
          </Notice>
        )}
        {error && (
          <Notice tone="danger" className="mb-3">
            {error}
          </Notice>
        )}
        {source === "repos" && discoveryLoading && report && (
          <Notice tone="info" className="mb-3">
            正在刷新仓库；已显示上次结果。
          </Notice>
        )}
        {source === "repos" && failures.length > 0 && visibleSkills.length > 0 && (
          <Notice tone="warning" className="mb-3">
            <p>
              {failures.length} 个仓库没有读到：{repoFailuresText}
            </p>
            <div className="mt-2 flex gap-2">
              <Button size="sm" onClick={() => void refreshDiscovery()}>
                重试
              </Button>
              <Button size="sm" onClick={() => setRepoManagerOpen(true)}>
                管理仓库
              </Button>
            </div>
          </Notice>
        )}

        <div className="overflow-hidden rounded-panel border border-border bg-surface">
          {reposLoading && source === "repos" && !report ? (
            <p className="py-12 text-center text-caption text-fg-3">正在读取仓库配置…</p>
          ) : source === "repos" ? (
            renderRepoRows()
          ) : (
            renderShRows()
          )}
        </div>
      </div>

      {repoManagerOpen && (
        <RepoManagerSheet
          repos={repos}
          skills={report?.skills ?? []}
          onSaved={(saved) => {
            setRepos(saved);
            setDeselectedRepos(new Set());
          }}
          onClose={() => {
            setRepoManagerOpen(false);
            void refreshDiscovery();
          }}
        />
      )}
    </div>
  );
}
