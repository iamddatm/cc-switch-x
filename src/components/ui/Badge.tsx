import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

export type BadgeTone = "neutral" | "success" | "warning" | "danger" | "brand";

const TONES: Record<BadgeTone, string> = {
  neutral: "border-border text-fg-2",
  success: "border-success bg-success-soft text-success-text",
  warning: "border-warning bg-warning-soft text-warning-text",
  danger: "border-danger bg-danger-soft text-danger-text",
  brand: "border-action bg-action-soft text-action-text",
};

export interface BadgeProps {
  tone?: BadgeTone;
  mono?: boolean;
  className?: string;
  title?: string;
  children: ReactNode;
}

/** 18px 胶囊标签，和 cc-switch 的 badge 密度一致。 */
export function Badge({ tone = "neutral", mono = false, className, title, children }: BadgeProps) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-[18px] max-w-full items-center rounded-full border px-2 text-badge leading-none",
        mono && "font-mono",
        TONES[tone],
        className,
      )}
    >
      <span className="min-w-0 truncate">{children}</span>
    </span>
  );
}