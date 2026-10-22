/**
 * 明示「这个文件里还有 N 条不是本工具写的 hook，我们不动它们」。
 * hooks 与 MCP 同住 settings.json，用户最容易担心的就是被顺手清掉，所以这句要显眼。
 *
 * 数量按 agent 分开报：每个 agent 的 hook 文件是不同的一份（Claude 的 settings.json、
 * Codex 的 hooks.json…），加成一个总数会让人以为说的是同一个文件。
 */
export function HooksSafety({
  counts,
  gate,
}: {
  counts: { agent: string; count: number }[];
  gate: { file: string; why: string } | null;
}) {
  return (
    <div className="mb-3 space-y-2 text-sm">
      {gate && (
        <p
          role="alert"
          className="rounded border border-red-300 bg-red-50 p-2 text-red-800 dark:border-red-700 dark:bg-red-950 dark:text-red-300"
        >
          {gate.why}
        </p>
      )}
      {counts
        .filter((entry) => entry.count > 0)
        .map((entry) => (
          <p key={entry.agent} className="text-slate-600 dark:text-slate-300">
            {entry.agent} 的 hook 文件里还有 {entry.count} 条不是本工具写的 hook，我们不会动它们。
          </p>
        ))}
    </div>
  );
}
