import { useState, type FormEvent } from "react";
import { useSessions } from "../hooks/useSessions";
import { groupSessions } from "../lib/sessionGroups";
import type {
  SessionProviderStatus,
  SessionSearchHit,
  SessionSummary,
} from "../types";

export function SessionsPanel() {
  const {
    sessions,
    providers,
    hits,
    loading,
    scanning,
    scanReport,
    error,
    scan,
    search,
    clearSearch,
  } = useSessions();
  const [query, setQuery] = useState("");
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);

  const providerFor = (agentId: string) =>
    providers.find((provider) => provider.agentId === agentId);
  const presentProviders = providers.filter((provider) =>
    sessions.some((session) => session.agentId === provider.agentId),
  );
  const visibleSessions = sessions.filter(
    (session) => !selectedAgent || session.agentId === selectedAgent,
  );
  const visibleHits = hits?.filter(
    (hit) => !selectedAgent || hit.session.agentId === selectedAgent,
  );
  const groups = groupSessions(visibleSessions, new Date());

  const submitSearch = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void search(query);
  };
  const clear = () => {
    setQuery("");
    clearSearch();
  };

  return (
    <section className="space-y-4">
      <h2 className="text-xl font-semibold">Sessions</h2>

      <form className="flex flex-wrap items-center gap-2" onSubmit={submitSearch}>
        <input
          aria-label="搜索会话内容"
          className="min-w-64 flex-1 rounded border border-slate-300 px-3 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-800"
          placeholder="搜索会话内容"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <button
          type="submit"
          className="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
        >
          搜索
        </button>
        {hits !== null && (
          <button
            type="button"
            className="rounded px-3 py-1.5 text-sm text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700"
            onClick={clear}
          >
            清除
          </button>
        )}
      </form>

      {error && (
        <p role="alert" className="rounded border border-red-300 bg-red-50 p-2 text-sm text-red-700">
          读取会话失败：{error}
        </p>
      )}

      {loading && sessions.length === 0 ? (
        <p className="text-sm text-slate-500">正在读取会话索引…</p>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <button
            type="button"
            className="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
            disabled={scanning}
            onClick={() => void scan()}
          >
            {scanning
              ? "正在扫描…"
              : sessions.length === 0
                ? "扫描本机会话"
                : "重新扫描"}
          </button>
          {sessions.length === 0 && (
            <p className="text-xs text-slate-500">
              首次扫描会读取本机全部会话；Claude 主会话可能约 710 MB，请耐心等待。
            </p>
          )}
        </div>
      )}

      {scanReport && (
        <section
          aria-label="扫描结果"
          className="rounded border border-slate-200 p-3 text-sm dark:border-slate-700"
        >
          <p>
            新增 {scanReport.added}，更新 {scanReport.updated}，移除{" "}
            {scanReport.removed}，未变 {scanReport.unchanged}
          </p>
          {scanReport.failures.length > 0 && (
            <ul className="mt-2 space-y-1 text-red-700">
              {scanReport.failures.map((failure) => (
                <li key={`${failure.agentId}:${failure.file}`}>
                  <span>{failure.file}</span>：<span>{failure.reason}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}

      {sessions.length === 0 ? (
        <section aria-label="会话来源" className="space-y-2 text-sm">
          <p className="text-slate-500">还没有会话索引，先扫描一次。</p>
          <ul className="space-y-1">
            {providers.map((provider) => (
              <li key={provider.agentId} className="rounded border border-slate-200 p-2 dark:border-slate-700">
                <span className="font-medium">{provider.displayName}</span>
                <span className="ml-2 text-slate-500">{providerReason(provider)}</span>
                <span className="ml-2 block text-xs text-slate-400">
                  {provider.roots.join("、")}
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2 text-sm" aria-label="按工具筛选">
            <button
              type="button"
              aria-pressed={selectedAgent === null}
              className="rounded border border-slate-300 px-2 py-1 aria-pressed:bg-slate-200 dark:border-slate-600 dark:aria-pressed:bg-slate-700"
              onClick={() => setSelectedAgent(null)}
            >
              全部
            </button>
            {presentProviders.map((provider) => (
              <button
                key={provider.agentId}
                type="button"
                aria-pressed={selectedAgent === provider.agentId}
                className="rounded border border-slate-300 px-2 py-1 aria-pressed:bg-slate-200 dark:border-slate-600 dark:aria-pressed:bg-slate-700"
                onClick={() => setSelectedAgent(provider.agentId)}
              >
                <span>{provider.displayName}</span>
                {!provider.verified && (
                  <span className="ml-1 rounded bg-amber-100 px-1 text-xs text-amber-800">
                    解析器未实测
                  </span>
                )}
              </button>
            ))}
          </div>

          {hits !== null ? (
            <section aria-label="搜索结果" className="space-y-2">
              {visibleHits && visibleHits.length > 0 ? (
                <ul className="space-y-2">
                  {visibleHits.map((hit) => (
                    <SearchHitRow
                      key={`${hit.session.agentId}:${hit.session.sessionId}`}
                      hit={hit}
                      provider={providerFor(hit.session.agentId)}
                    />
                  ))}
                </ul>
              ) : (
                <p className="text-sm text-slate-500">没有匹配的会话。</p>
              )}
            </section>
          ) : groups.length === 0 ? (
            <p className="text-sm text-slate-500">没有匹配的会话。</p>
          ) : (
            <div className="space-y-5">
              {groups.map((group) => (
                <section key={group.project} aria-label={`项目 ${group.project}`}>
                  <h3 className="mb-2 break-all text-sm font-semibold text-slate-700 dark:text-slate-200">
                    {group.project}
                  </h3>
                  <div className="space-y-3">
                    {group.buckets.map((bucket) => (
                      <div key={bucket.label}>
                        <h4 className="mb-1 text-xs font-medium text-slate-500">
                          {bucket.label}
                        </h4>
                        <ul className="space-y-1">
                          {bucket.sessions.map((session) => (
                            <li key={`${session.agentId}:${session.sessionId}`}>
                              <SessionRow
                                session={session}
                                provider={providerFor(session.agentId)}
                              />
                            </li>
                          ))}
                        </ul>
                      </div>
                    ))}
                  </div>
                </section>
              ))}
            </div>
          )}
        </>
      )}
    </section>
  );
}

function SessionRow({
  session,
  provider,
}: {
  session: SessionSummary;
  provider?: SessionProviderStatus;
}) {
  return (
    <button
      type="button"
      className="w-full rounded border border-slate-200 px-3 py-2 text-left text-sm hover:bg-slate-50 dark:border-slate-700 dark:hover:bg-slate-800"
    >
      <span className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{session.title}</span>
        {provider && !provider.verified && (
          <span className="rounded bg-amber-100 px-1 text-xs text-amber-800">
            解析器未实测
          </span>
        )}
        {session.parseError && (
          <span
            className="rounded bg-red-100 px-1 text-xs text-red-700"
            title={session.parseError}
          >
            解析失败
          </span>
        )}
      </span>
      <span className="mt-1 block text-xs text-slate-500">
        {provider?.displayName ?? session.agentId} · {session.messageCount} 条 ·{" "}
        {formatTime(session.updatedAt)}
      </span>
    </button>
  );
}

function SearchHitRow({
  hit,
  provider,
}: {
  hit: SessionSearchHit;
  provider?: SessionProviderStatus;
}) {
  return (
    <li className="rounded border border-slate-200 px-3 py-2 text-sm dark:border-slate-700">
      <div className="flex flex-wrap items-center gap-2">
        <span className="rounded bg-slate-100 px-1 text-xs text-slate-600 dark:bg-slate-700 dark:text-slate-200">
          会话内命中
        </span>
        <span className="font-medium">{hit.session.title}</span>
        <span className="text-xs text-slate-500">
          {provider?.displayName ?? hit.session.agentId}
        </span>
        {provider && !provider.verified && (
          <span className="rounded bg-amber-100 px-1 text-xs text-amber-800">
            解析器未实测
          </span>
        )}
      </div>
      <p className="mt-1 text-slate-600 dark:text-slate-300">{hit.snippet}</p>
    </li>
  );
}

function providerReason(provider: SessionProviderStatus): string {
  if (!provider.parseReady) {
    return "解析器未接入";
  }
  if (!provider.rootExists) {
    return "目录不存在";
  }
  if (provider.foundFiles === 0) {
    return "未发现文件";
  }
  if (provider.sessionCount === 0) {
    return "已发现文件，尚未解析出会话";
  }
  return `已索引 ${provider.sessionCount} 个会话`;
}

function formatTime(timestamp: number): string {
  const date = new Date(timestamp);
  const month = date.getMonth() + 1;
  const day = String(date.getDate()).padStart(2, "0");
  const hour = String(date.getHours()).padStart(2, "0");
  const minute = String(date.getMinutes()).padStart(2, "0");
  return `${month}-${day} ${hour}:${minute}`;
}