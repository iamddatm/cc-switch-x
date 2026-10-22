import { useCallback, useEffect, useState } from "react";
import { api } from "../../lib/api";
import type {
  SessionMatch,
  SessionMessage,
  SessionSummary,
} from "../../types";
import { SessionSearch } from "./SessionSearch";
import { SessionTimeline } from "./SessionTimeline";

const PAGE_SIZE = 200;
const JUMP_CONTEXT = 20;
const JUMP_PAGE_SIZE = 50;

export interface SessionReaderProps {
  session: SessionSummary;
  agentName: string;
  onBack: () => void;
}

export function SessionReader({
  session,
  agentName,
  onBack,
}: SessionReaderProps) {
  const [messages, setMessages] = useState<SessionMessage[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [highlightedSeq, setHighlightedSeq] = useState<number | null>(null);
  const [highlightQuery, setHighlightQuery] = useState("");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setHighlightedSeq(null);
    setHighlightQuery("");
    api
      .getSession(session.agentId, session.sessionId, 0, PAGE_SIZE)
      .then((detail) => {
        if (cancelled) {
          return;
        }
        setMessages(detail.messages);
        setTotal(detail.total);
      })
      .catch((cause) => {
        if (!cancelled) {
          setError(String(cause));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [session.agentId, session.sessionId]);

  const loadMore = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const detail = await api.getSession(
        session.agentId,
        session.sessionId,
        messages.length,
        PAGE_SIZE,
      );
      setTotal(detail.total);
      setMessages((current) => {
        const seen = new Set(current.map((message) => message.seq));
        return [
          ...current,
          ...detail.messages.filter((message) => !seen.has(message.seq)),
        ];
      });
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, [messages.length, session.agentId, session.sessionId]);

  const jumpTo = useCallback(
    async (match: SessionMatch, query: string) => {
      setHighlightQuery(query);
      setHighlightedSeq(match.seq);
      setLoading(true);
      setError(null);
      try {
        const detail = await api.getSession(
          session.agentId,
          session.sessionId,
          Math.max(0, match.index - JUMP_CONTEXT),
          JUMP_PAGE_SIZE,
        );
        setMessages(detail.messages);
        setTotal(detail.total);
      } catch (cause) {
        setError(String(cause));
      } finally {
        setLoading(false);
      }
    },
    [session.agentId, session.sessionId],
  );

  useEffect(() => {
    if (highlightedSeq === null) {
      return;
    }
    const target = document.getElementById(
      `session-message-${highlightedSeq}`,
    );
    target?.scrollIntoView?.({ block: "center" });
  }, [highlightedSeq, messages]);

  return (
    <section className="space-y-4">
      <button
        type="button"
        className="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
        onClick={onBack}
      >
        返回列表
      </button>

      <header className="space-y-1">
        <h2 className="text-xl font-semibold">{session.title}</h2>
        <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-2">
          <div>
            <dt className="inline text-slate-500">工具：</dt>
            <dd className="inline">{agentName}</dd>
          </div>
          <div>
            <dt className="inline text-slate-500">项目：</dt>
            <dd className="inline break-all">
              {session.projectPath ?? "未知项目"}
            </dd>
          </div>
          <div className="sm:col-span-2">
            <dt className="inline text-slate-500">源文件：</dt>
            <dd className="inline break-all">{session.sourcePath}</dd>
          </div>
          <div>
            <dt className="inline text-slate-500">更新时间：</dt>
            <dd className="inline">{formatTime(session.updatedAt)}</dd>
          </div>
        </dl>
        {session.parseError && (
          <p className="rounded border border-red-300 bg-red-50 p-2 text-sm text-red-700">
            上次解析失败：{session.parseError}
          </p>
        )}
      </header>

      <SessionSearch
        agentId={session.agentId}
        sessionId={session.sessionId}
        onSelect={(match, query) => void jumpTo(match, query)}
      />

      {error && (
        <p role="alert" className="rounded border border-red-300 bg-red-50 p-2 text-sm text-red-700">
          读取会话失败：{error}
        </p>
      )}

      {loading && messages.length === 0 ? (
        <p className="text-sm text-slate-500">正在载入会话…</p>
      ) : (
        <SessionTimeline
          messages={messages}
          total={total}
          loading={loading}
          highlightedSeq={highlightedSeq}
          query={highlightQuery}
          onLoadMore={() => void loadMore()}
        />
      )}
    </section>
  );
}

function formatTime(timestamp: number): string {
  const date = new Date(timestamp);
  const month = date.getMonth() + 1;
  const day = String(date.getDate()).padStart(2, "0");
  const hour = String(date.getHours()).padStart(2, "0");
  const minute = String(date.getMinutes()).padStart(2, "0");
  return `${month}-${day} ${hour}:${minute}`;
}