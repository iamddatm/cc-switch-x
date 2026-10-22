import type { SessionMessage } from "../../types";
import { SessionMessage as SessionMessageView } from "./SessionMessage";

export interface SessionTimelineProps {
  messages: SessionMessage[];
  total: number;
  loading: boolean;
  highlightedSeq: number | null;
  query: string;
  onLoadMore: () => void;
}

export function SessionTimeline({
  messages,
  total,
  loading,
  highlightedSeq,
  query,
  onLoadMore,
}: SessionTimelineProps) {
  return (
    <div className="space-y-2">
      {messages.length === 0 && (
        <p className="text-sm text-slate-500">这个会话没有可读消息。</p>
      )}
      {messages.map((message) => (
        <SessionMessageView
          key={message.seq}
          message={message}
          highlighted={highlightedSeq === message.seq}
          query={query}
        />
      ))}
      <div className="flex flex-wrap items-center gap-3 pt-2 text-sm text-slate-500">
        {messages.length < total && (
          <button
            type="button"
            className="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
            disabled={loading}
            onClick={onLoadMore}
          >
            {loading ? "载入中…" : "载入更多"}
          </button>
        )}
        <span>
          已载入 {messages.length} / 共 {total} 条
        </span>
      </div>
    </div>
  );
}