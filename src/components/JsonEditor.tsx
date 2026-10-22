export function JsonEditor({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <textarea
      aria-label="JSON 编辑器"
      className="h-64 w-full rounded border border-slate-300 bg-white p-3 font-mono text-sm text-slate-900 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
