/**
 * 明示「这个文件里还有 N 条不是本工具写的 hook，我们不动它们」。
 * hooks 与 MCP 同住 settings.json，用户最容易担心的就是被顺手清掉，所以这句要显眼。
 */
export function HooksSafety({
  foreignCount,
  gate,
}: {
  foreignCount: number | null;
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
      {foreignCount !== null && foreignCount > 0 && (
        <p className="text-slate-600 dark:text-slate-300">
          该文件里还有 {foreignCount} 条不是本工具写的 hook，我们不会动它们。
        </p>
      )}
    </div>
  );
}
