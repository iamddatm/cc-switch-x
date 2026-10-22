import { useCallback, useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../../lib/api";
import type {
  ResumeInfo,
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
  const [windowStart, setWindowStart] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [highlightedSeq, setHighlightedSeq] = useState<number | null>(null);
  const [highlightQuery, setHighlightQuery] = useState("");
  const [resume, setResume] = useState<ResumeInfo | null>(null);
  const [resumeUnavailable, setResumeUnavailable] = useState(false);
  const [copied, setCopied] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportMessage, setExportMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setHighlightedSeq(null);
    setHighlightQuery("");
    setWindowStart(0);
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
        windowStart + messages.length,
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
  }, [messages.length, session.agentId, session.sessionId, windowStart]);

  const jumpTo = useCallback(
    async (match: SessionMatch, query: string) => {
      setHighlightQuery(query);
      setHighlightedSeq(match.seq);
      setWindowStart(Math.max(0, match.index - JUMP_CONTEXT));
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
    let cancelled = false;
    setResume(null);
    setResumeUnavailable(false);
    api
      .sessionResume(session.agentId, session.sessionId)
      .then((info) => {
        if (!cancelled) {
          setResume(info);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setResumeUnavailable(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [session.agentId, session.sessionId]);

  useEffect(() => {
    if (highlightedSeq === null) {
      return;
    }
    const target = document.getElementById(
      `session-message-${highlightedSeq}`,
    );
    target?.scrollIntoView?.({ block: "center" });
  }, [highlightedSeq, messages]);

  const copyResume = async () => {
    if (!resume) {
      return;
    }
    setActionError(null);
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("当前环境不支持剪贴板");
      }
      await navigator.clipboard.writeText(resume.command);
      setCopied(true);
    } catch (cause) {
      setActionError(String(cause));
    }
  };

  const exportMarkdown = async () => {
    setExporting(true);
    setExportMessage(null);
    setActionError(null);
    try {
      const dest = await save({
        defaultPath: exportFileName(session.title),
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!dest) {
        return;
      }
      const path = await api.exportSessionMarkdown(
        session.agentId,
        session.sessionId,
        dest,
      );
      setExportMessage(`已导出到 ${path}`);
    } catch (cause) {
      setActionError(String(cause));
    } finally {
      setExporting(false);
    }
  };

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
        <div className="flex flex-wrap items-center gap-2 pt-1 text-sm">
          {resume ? (
            <>
              <button
                type="button"
                className="rounded border border-slate-300 px-3 py-1.5 hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                onClick={() => void copyResume()}
              >
                复制恢复命令
              </button>
              {copied && <span className="text-emerald-700">已复制</span>}
            </>
          ) : resumeUnavailable ? (
            <span className="text-xs text-slate-500">
              该会话没有可用的恢复命令。
            </span>
          ) : null}
          <button
            type="button"
            className="rounded border border-slate-300 px-3 py-1.5 hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
            disabled={exporting}
            onClick={() => void exportMarkdown()}
          >
            {exporting ? "导出中…" : "导出 Markdown"}
          </button>
        </div>
        {resume && (
          <code className="block break-all rounded bg-slate-100 px-2 py-1 text-xs dark:bg-slate-800">
            {resume.command}
          </code>
        )}
        <p className="text-xs text-amber-700">
          会话可能包含密钥与个人信息，导出文件不受本项目保护。
        </p>
        {exportMessage && (
          <p className="text-sm text-emerald-700">{exportMessage}</p>
        )}
        {actionError && (
          <p role="alert" className="text-sm text-red-700">
            操作失败：{actionError}
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
          emptyHint={
            session.agentId === "agy" && total === 0
              ? "该会话只有摘要索引，原始对话记录已不存在。"
              : undefined
          }
          onLoadMore={() => void loadMore()}
        />
      )}
    </section>
  );
}

function exportFileName(title: string): string {
  const safe = title.replace(/[\\/:*?"<>|]/g, "_").trim();
  return `${safe || "session"}.md`;
}

function formatTime(timestamp: number): string {
  const date = new Date(timestamp);
  const month = date.getMonth() + 1;
  const day = String(date.getDate()).padStart(2, "0");
  const hour = String(date.getHours()).padStart(2, "0");
  const minute = String(date.getMinutes()).padStart(2, "0");
  return `${month}-${day} ${hour}:${minute}`;
}