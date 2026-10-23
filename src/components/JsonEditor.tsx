export function JsonEditor({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <textarea
      aria-label="JSON 编辑器"
      className="min-h-[320px] w-full rounded-control border border-border bg-surface p-3 font-mono text-caption text-fg-1 transition-colors duration-150 focus:border-action focus:outline-none"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}