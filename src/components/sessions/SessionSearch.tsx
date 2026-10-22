import { useState, type FormEvent } from "react";
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
        <input
          aria-label="在会话内查找"
          className="min-w-48 flex-1 rounded border border-slate-300 px-3 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-800"
          placeholder="在会话内查找"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <button
          type="submit"
          className="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
          disabled={loading}
        >
          {loading ? "查找中…" : "查找"}
        </button>
      </form>
      {error && (
        <p role="alert" className="text-sm text-red-700">
          会话内查找失败：{error}
        </p>
      )}
      {matches !== null && (
        <div className="space-y-1">
          {matches.length === 0 ? (
            <p className="text-sm text-slate-500">没有匹配的消息。</p>
          ) : (
            <ul className="space-y-1">
              {matches.map((match) => (
                <li key={`${match.seq}:${match.index}`}>
                  <button
                    type="button"
                    className="w-full rounded border border-slate-200 px-2 py-1 text-left text-sm hover:bg-slate-50 dark:border-slate-700 dark:hover:bg-slate-800"
                    onClick={() => onSelect(match, query.trim())}
                  >
                    <span className="mr-2 text-xs text-slate-500">
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