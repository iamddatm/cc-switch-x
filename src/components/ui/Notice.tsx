import type { ReactNode } from "react";
import { CircleAlert, CircleCheck, Info, TriangleAlert } from "lucide-react";
import { cn } from "../../lib/cn";

type Tone = "info" | "success" | "warning" | "danger";

const TONES: Record<Tone, { box: string; icon: typeof Info }> = {
  info: { box: "border-border bg-subtle text-fg-2", icon: Info },
  success: { box: "border-transparent bg-success-soft text-success-text", icon: CircleCheck },
  warning: { box: "border-transparent bg-warning-soft text-warning-text", icon: TriangleAlert },
  danger: { box: "border-transparent bg-danger-soft text-danger-text", icon: CircleAlert },
};

export function Notice({
  tone = "info",
  children,
  className,
}: {
  tone?: Tone;
  children: ReactNode;
  className?: string;
}) {
  const { box, icon: Icon } = TONES[tone];
  return (
    <div className={cn("flex items-start gap-2 rounded-control border px-3 py-2 text-caption", box, className)}>
      <Icon size={14} className="mt-0.5 shrink-0" aria-hidden />
      <div className="min-w-0">{children}</div>
    </div>
  );
}