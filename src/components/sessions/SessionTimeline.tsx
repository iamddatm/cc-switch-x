import { Button } from "../ui/Button";
import type { SessionMessage } from "../../types";
import { SessionMessage as SessionMessageView } from "./SessionMessage";

export interface SessionTimelineProps {
  messages: SessionMessage[];
  total: number;
  loading: boolean;
  highlightedSeq: number | null;
  query: string;
  emptyHint?: string;
  onLoadMore: () => void;
}

export function SessionTimeline({
  messages,
  total,
  loading,
  highlightedSeq,
  query,
  emptyHint,
  onLoadMore,
}: SessionTimelineProps) {
  return (
    <div className="space-y-2">
      {messages.length === 0 && (
        <p className="text-body text-fg-3">
          {emptyHint ?? "这个会话没有可读消息。"}
        </p>
      )}
      {messages.map((message) => (
        <SessionMessageView
          key={message.seq}
          message={message}
          highlighted={highlightedSeq === message.seq}
          query={query}
        />
      ))}
      <div className="flex flex-wrap items-center gap-3 pt-2 text-body text-fg-3">
        {messages.length < total && (
          <Button variant="secondary" disabled={loading} onClick={onLoadMore}>
            {loading ? "载入中…" : "载入更多"}
          </Button>
        )}
        <span>
          已载入 {messages.length} / 共 {total} 条
        </span>
      </div>
    </div>
  );
}