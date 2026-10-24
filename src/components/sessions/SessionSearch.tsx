import { useState, type FormEvent } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { api } from "../../lib/api";
import type { SessionMatch } from "../../types";

export interface SessionSearchProps {
  agentId: string;
  sessionId: string;
  onSelect: (match: SessionMatch, query: string) => void;
}

export function SessionSearch({
  agentId,
  sessionId,
  onSelect,
}: SessionSearchProps) {
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<SessionMatch[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const trimmed = query.trim();
    if (!trimmed) {
      setMatches(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setMatches(await api.searchInSession(agentId, sessionId, trimmed, 50));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-2">
      <form className="flex items-center gap-2" onSubmit={submit}>
        <Input
          aria-label="在会话内查找"
          className="min-w-48 flex-1"
          placeholder="在会话内查找"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <Button type="submit" variant="secondary" disabled={loading}>
          {loading ? "查找中…" : "查找"}
        </Button>
      </form>
      {error && (
        <p role="alert" className="text-body text-danger-text">
          会话内查找失败：{error}
        </p>
      )}
      {matches !== null && (
        <div className="space-y-1">
          {matches.length === 0 ? (
            <p className="text-body text-fg-3">没有匹配的消息。</p>
          ) : (
            <ul className="space-y-1">
              {matches.map((match) => (
                <li key={`${match.seq}:${match.index}`}>
                  <button
                    type="button"
                    className="w-full rounded-panel border border-border bg-surface px-2 py-1 text-left text-body text-fg-1 hover:bg-subtle"
                    onClick={() => onSelect(match, query.trim())}
                  >
                    <span className="mr-2 text-caption text-fg-3">
                      第 {match.index + 1} 条
                    </span>
                    <span>{match.snippet}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}