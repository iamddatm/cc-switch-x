import { useEffect, useState } from "react";
import { Folder, History, Search, Users } from "lucide-react";
import { AgentGlyph } from "../components/agents/AgentGlyph";
import { SessionGroupHeader } from "../components/sessions/SessionGroupHeader";
import { SessionReader } from "../components/sessions/SessionReader";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import { EmptyState } from "../components/ui/EmptyState";
import { Menu } from "../components/ui/Menu";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { SearchField } from "../components/ui/SearchField";
import { SegmentedControl } from "../components/ui/SegmentedControl";
import { Select } from "../components/ui/Select";
import { useAgents } from "../hooks/useAgents";
import { ALL_SESSIONS_LIMIT, SESSION_SEARCH_LIMIT, useSessions } from "../hooks/useSessions";
import { formatRelativeTime } from "../lib/format";
import {
  groupSessionsByProject,
  groupSessionsByTime,
  isSessionInProject,
  projectName,
  projectOf,
} from "../lib/sessionGroups";
import type {
  ProjectView,
  SessionProviderStatus,
  SessionSearchHit,
  SessionSummary,
} from "../types";

type GroupMode = "time" | "project";

/**
 * `project` 非空时（侧栏处于项目范围）只展示归属到该项目及其子目录的会话；
 * 无归属（缺少 cwd）的会话只在全局范围出现。
 */
export function SessionsPanel({ project = null }: { project?: ProjectView | null }) {
  const {
    sessions,
    providers,
    hits,
    loading,
    scanning,
    scanReport,
    error,
    scan,
    reload,
    search,
    clearSearch,
  } = useSessions();
  const { enabledAgents } = useAgents();
  const [query, setQuery] = useState("");
  const [selectedAgent, setSelectedAgent] = useState("all");
  const [groupMode, setGroupMode] = useState<GroupMode>("project");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [openSession, setOpenSession] = useState<SessionSummary | null>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);

  const enabledIds = new Set(enabledAgents.map((agent) => agent.id));
  const inProject = (session: SessionSummary) =>
    !project || isSessionInProject(session, project.path);
  const projectSessions = project ? sessions.filter(inProject) : sessions;
  // 项目范围下每个应用的数量必须按项目集合算，否则下拉写「Claude (300)」而列表只有 3 条。
  const countByAgent = new Map<string, number>();
  if (project) {
    for (const session of projectSessions) {
      countByAgent.set(session.agentId, (countByAgent.get(session.agentId) ?? 0) + 1);
    }
  }
  const countFor = (provider: SessionProviderStatus) =>
    project ? (countByAgent.get(provider.agentId) ?? 0) : provider.sessionCount;
  const presentProviders = providers.filter(
    (provider) => enabledIds.has(provider.agentId) && countFor(provider) > 0,
  );
  const providerFor = (agentId: string) =>
    providers.find((provider) => provider.agentId === agentId);

  useEffect(() => {
    if (
      selectedAgent !== "all" &&
      !presentProviders.some((provider) => provider.agentId === selectedAgent)
    ) {
      setSelectedAgent("all");
    }
  }, [presentProviders, selectedAgent]);

  const matchesAgent = (agentId: string) =>
    (selectedAgent === "all" || agentId === selectedAgent) && enabledIds.has(agentId);

  const visibleSessions = projectSessions.filter((session) => matchesAgent(session.agentId));
  const visibleHits =
    hits?.filter((hit) => matchesAgent(hit.session.agentId) && inProject(hit.session)) ?? null;
  const totalCount = presentProviders.reduce((sum, provider) => sum + countFor(provider), 0);
  const allAgents = selectedAgent === "all";
  // 项目范围下只剩一个项目，按项目分组等于「一个默认折叠的组 + 一片空白」，直接按时间平铺。
  const byProject = !project && groupMode === "project";
  // 过滤会静默剔除没有 cwd 的会话，必须把数量说出来，否则用户以为会话丢了。
  const hiddenUnattributed = project
    ? sessions.filter(
        (session) => matchesAgent(session.agentId) && !session.projectPath?.trim(),
      ).length
    : 0;
  const truncated = sessions.length >= ALL_SESSIONS_LIMIT;

  const toggleGroup = (groupKey: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(groupKey)) next.delete(groupKey);
      else next.add(groupKey);
      return next;
    });
  };

  if (openSession) {
    return (
      <div className="h-full overflow-auto px-6 py-5">
        <SessionReader
          session={openSession}
          agentName={providerFor(openSession.agentId)?.displayName ?? openSession.agentId}
          onBack={() => setOpenSession(null)}
        />
      </div>
    );
  }

  const projectGroups = groupSessionsByProject(visibleSessions);
  const timeGroups = groupSessionsByTime(visibleSessions, Date.now());

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader
        icon={History}
        title="会话"
        description={project ? `${project.name} · ${project.path}` : undefined}
        actions={
          <Menu
            label="会话的更多操作"
            items={[
              {
                label: scanning
                  ? "正在扫描…"
                  : sessions.length === 0
                    ? "扫描本机会话"
                    : "重新扫描",
                disabled: scanning,
                onSelect: () => void scan(),
              },
              { label: "会话记录在哪里", onSelect: () => setSourcesOpen(true) },
              { label: "刷新列表", onSelect: () => void reload() },
            ]}
          />
        }
      />

      <div className="flex flex-wrap items-center gap-2 px-6 py-3">
        <div className="flex items-center gap-2">
          {selectedAgent === "all" ? (
            <span className="flex h-[18px] w-[18px] items-center justify-center text-fg-3">
              <Users size={16} aria-hidden />
            </span>
          ) : (
            <AgentGlyph
              agentId={selectedAgent}
              label={providerFor(selectedAgent)?.displayName}
              size={18}
            />
          )}
          <Select
            aria-label="按应用筛选"
            value={selectedAgent}
            onChange={(event) => setSelectedAgent(event.target.value)}
          >
            <option value="all">全部应用 ({totalCount})</option>
            {presentProviders.map((provider) => (
              <option key={provider.agentId} value={provider.agentId}>
                {provider.displayName} ({countFor(provider)})
              </option>
            ))}
          </Select>
        </div>

        <SearchField
          className="min-w-[220px] flex-1"
          value={query}
          onValueChange={(value) => {
            setQuery(value);
            if (!value.trim()) clearSearch();
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void search(query);
            }
          }}
          placeholder={
            project
              ? `在 ${project.name} 内搜索标题、目录、首条消息或会话 ID…`
              : "搜索标题、目录、首条消息或会话 ID…"
          }
          ariaLabel="搜索会话内容"
        />

        {/* 项目范围下只有一个项目，按项目分组没有信息量，直接按时间平铺。 */}
        {!project && (
          <SegmentedControl
            ariaLabel="会话分组方式"
            value={groupMode}
            onValueChange={setGroupMode}
            items={[
              { value: "time", label: "按时间" },
              { value: "project", label: "按项目" },
            ]}
          />
        )}
        <span className="w-[92px] text-right text-caption text-fg-3">
          {visibleHits !== null ? `${visibleHits.length} 个匹配` : `${visibleSessions.length} 个会话`}
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
        {error && (
          <Notice tone="danger" className="mb-3">
            读取会话失败：{error}
          </Notice>
        )}

        {scanReport && (
          <Notice
            tone={
              scanReport.failures.length > 0 || scanReport.warnings.length > 0
                ? "warning"
                : "success"
            }
            className="mb-3"
          >
            <p>
              新增 {scanReport.added}，更新 {scanReport.updated}，移除 {scanReport.removed}，未变{" "}
              {scanReport.unchanged}
            </p>
            {(scanReport.failures.length > 0 || scanReport.warnings.length > 0) && (
              <details className="mt-1">
                <summary className="cursor-pointer">查看详情</summary>
                <ul className="mt-1 space-y-0.5">
                  {scanReport.failures.map((failure) => (
                    <li key={`${failure.agentId}:${failure.file}`}>
                      {failure.file}：{failure.reason}
                    </li>
                  ))}
                  {scanReport.warnings.map((warning, index) => (
                    <li key={`${warning.agentId}:${warning.file}:${warning.line}:${index}`}>
                      {warning.file}:{warning.line}：{warning.message}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </Notice>
        )}

        {project && hiddenUnattributed > 0 && (
          <p className="mb-3 text-caption text-fg-3">
            另有 {hiddenUnattributed} 个会话没有记录项目目录，无法判断归属，只在「全局配置」里列出。
          </p>
        )}
        {project && truncated && (
          <p className="mb-3 text-caption text-fg-3">
            本机已索引的会话超过 {ALL_SESSIONS_LIMIT} 条，这里只在这部分最近活跃的会话里按项目筛选。
          </p>
        )}

        {loading && sessions.length === 0 ? (
          <p className="py-10 text-center text-caption text-fg-3">正在读取会话索引…</p>
        ) : sessions.length === 0 ? (
          <>
            <EmptyState
              icon={History}
              title="还没有会话索引"
              description="首次扫描会读取本机全部会话，可能需要一些时间；之后只增量解析变化的文件。"
              actions={
                <>
                  <Button variant="primary" loading={scanning} onClick={() => void scan()}>
                    扫描本机会话
                  </Button>
                  <Button onClick={() => setSourcesOpen(true)}>会话记录在哪里</Button>
                </>
              }
            />
            <div className="mx-auto max-w-2xl space-y-1">
              {providers
                .filter((provider) => enabledIds.has(provider.agentId))
                .map((provider) => (
                  <div
                    key={provider.agentId}
                    className="flex items-center gap-2 rounded-control border border-border px-3 py-2 text-caption"
                  >
                    <AgentGlyph
                      agentId={provider.agentId}
                      label={provider.displayName}
                      size={16}
                    />
                    <span className="font-medium text-fg-1">{provider.displayName}</span>
                    <span className="text-fg-3">{providerReason(provider)}</span>
                    <span className="ml-auto min-w-0 truncate font-mono text-fg-3">
                      {provider.roots.join("、")}
                    </span>
                  </div>
                ))}
            </div>
          </>
        ) : visibleHits !== null ? (
          visibleHits.length === 0 ? (
            <EmptyState
              icon={Search}
              title="没有匹配的会话"
              description={
                project
                  ? `当前项目内没有匹配的结果；搜索先在全库取最近 ${SESSION_SEARCH_LIMIT} 条命中，再按项目筛选。`
                  : "换个关键词试试。"
              }
            />
          ) : (
            <div className="overflow-hidden rounded-panel border border-border bg-surface">
              <ul>
                {visibleHits.map((hit) => (
                  <SearchHitRow
                    key={`${hit.session.agentId}:${hit.session.sessionId}`}
                    hit={hit}
                    provider={providerFor(hit.session.agentId)}
                    onOpen={setOpenSession}
                  />
                ))}
              </ul>
            </div>
          )
        ) : visibleSessions.length === 0 ? (
          project ? (
            <EmptyState
              icon={History}
              title="这个项目还没有会话"
              description={`只统计项目目录 ${project.path} 及其子目录里的会话；切到「全局配置」可查看全部。`}
            />
          ) : (
            <EmptyState icon={History} title="没有匹配的会话" description="换个应用或关键词试试。" />
          )
        ) : (
          <div className="overflow-hidden rounded-panel border border-border bg-surface">
            {byProject ? (
              projectGroups.map((group) => {
                const isOpen = expanded.has(group.project);
                return (
                  <section key={group.project} aria-label={`项目 ${group.project}`}>
                    <SessionGroupHeader
                      project={group.project}
                      count={group.sessions.length}
                      latest={group.latest}
                      expanded={isOpen}
                      onToggle={() => toggleGroup(group.project)}
                    />
                    {isOpen && (
                      <ul>
                        {group.sessions.map((session) => (
                          <SessionRow
                            key={`${session.agentId}:${session.sessionId}`}
                            session={session}
                            provider={providerFor(session.agentId)}
                            showAgent={allAgents}
                            showProject={false}
                            onOpen={setOpenSession}
                          />
                        ))}
                      </ul>
                    )}
                  </section>
                );
              })
            ) : (
              timeGroups.map((group) => (
                <section key={group.label} aria-label={`时间 ${group.label}`}>
                  <h3 className="border-t border-border bg-subtle px-4 py-1 text-caption font-semibold text-fg-2 first:border-t-0">
                    {group.label}
                  </h3>
                  <ul>
                    {group.sessions.map((session) => (
                      <SessionRow
                        key={`${session.agentId}:${session.sessionId}`}
                        session={session}
                        provider={providerFor(session.agentId)}
                        showAgent={allAgents}
                        showProject={!project}
                        onOpen={setOpenSession}
                      />
                    ))}
                  </ul>
                </section>
              ))
            )}
          </div>
        )}
      </div>

      <Dialog
        open={sourcesOpen}
        onClose={() => setSourcesOpen(false)}
        title="会话记录在哪里"
        description="本应用从这些位置读取会话；文件只读，不会修改。"
        footer={<Button onClick={() => setSourcesOpen(false)}>关闭</Button>}
      >
        <ul className="space-y-2">
          {providers.map((provider) => (
            <li key={provider.agentId} className="rounded-control border border-border p-2.5">
              <div className="flex items-center gap-2">
                <AgentGlyph agentId={provider.agentId} label={provider.displayName} size={18} />
                <span className="text-body font-medium text-fg-1">{provider.displayName}</span>
                <Badge tone={provider.verified ? "neutral" : "warning"}>
                  {provider.verified ? "已实测" : "解析器未实测"}
                </Badge>
                <span className="ml-auto text-caption text-fg-3">
                  {providerReason(provider)}
                </span>
              </div>
              <p className="mt-1 break-all font-mono text-caption text-fg-3">
                {provider.roots.join("、") || "无声明根目录"}
              </p>
            </li>
          ))}
        </ul>
      </Dialog>
    </div>
  );
}

function SessionRow({
  session,
  provider,
  showAgent,
  showProject,
  onOpen,
}: {
  session: SessionSummary;
  provider?: SessionProviderStatus;
  showAgent: boolean;
  showProject: boolean;
  onOpen: (session: SessionSummary) => void;
}) {
  return (
    <li className="border-t border-border first:border-t-0">
      <button
        type="button"
        onClick={() => onOpen(session)}
        className="flex min-h-14 w-full items-center gap-3 px-4 py-2 text-left transition-colors duration-150 hover:bg-subtle"
      >
        {showAgent && (
          <AgentGlyph
            agentId={session.agentId}
            label={provider?.displayName ?? session.agentId}
            size={20}
          />
        )}
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2">
            <span className="truncate text-body font-medium text-fg-1">{session.title}</span>
            {provider && !provider.verified && <Badge tone="warning">解析器未实测</Badge>}
            {session.parseError && (
              <span title={session.parseError}>
                <Badge tone="danger">解析失败</Badge>
              </span>
            )}
          </span>
          <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-caption text-fg-2">
            {showProject && (
              <>
                <Folder size={12} className="shrink-0 text-fg-3" aria-hidden />
                <span className="max-w-[180px] truncate">
                  {projectName(projectOf(session))}
                </span>
                <span className="text-fg-3">·</span>
              </>
            )}
            {session.preview && session.preview !== session.title ? (
              <span className="truncate">最后：{session.preview}</span>
            ) : (
              <span className="text-fg-3">{session.messageCount} 条消息</span>
            )}
          </span>
        </span>
        <span className="shrink-0 text-caption tabular-nums text-fg-3">
          {formatRelativeTime(session.updatedAt)}
        </span>
      </button>
    </li>
  );
}

function SearchHitRow({
  hit,
  provider,
  onOpen,
}: {
  hit: SessionSearchHit;
  provider?: SessionProviderStatus;
  onOpen: (session: SessionSummary) => void;
}) {
  return (
    <li className="border-t border-border first:border-t-0">
      <button
        type="button"
        onClick={() => onOpen(hit.session)}
        className="flex w-full items-start gap-3 px-4 py-2.5 text-left transition-colors duration-150 hover:bg-subtle"
      >
        <AgentGlyph
          agentId={hit.session.agentId}
          label={provider?.displayName ?? hit.session.agentId}
          size={20}
        />
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2">
            <Badge tone="brand">命中</Badge>
            <span className="truncate text-body font-medium text-fg-1">{hit.session.title}</span>
            <span className="text-caption text-fg-3">
              {provider?.displayName ?? hit.session.agentId}
            </span>
          </span>
          <span className="mt-0.5 block line-clamp-2 text-caption text-fg-2">{hit.snippet}</span>
        </span>
        <span className="shrink-0 text-caption tabular-nums text-fg-3">
          {formatRelativeTime(hit.session.updatedAt)}
        </span>
      </button>
    </li>
  );
}

function providerReason(provider: SessionProviderStatus): string {
  if (!provider.parseReady) {
    return "解析器未接入";
  }
  if (provider.sessionCount > 0) {
    return `已索引 ${provider.sessionCount} 个会话`;
  }
  if (provider.foundFiles > 0) {
    return "已发现文件，尚未解析出会话";
  }
  if (!provider.rootExists) {
    return "目录不存在";
  }
  return "未发现文件";
}