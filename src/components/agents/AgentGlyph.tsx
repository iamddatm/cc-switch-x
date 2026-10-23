import { cn } from "../../lib/cn";
import { BRAND_MARKS } from "./brandMarks";

export interface AgentGlyphProps {
  agentId: string;
  label?: string;
  size?: number;
  className?: string;
}

/** 内置 agent 的品牌标记；未知 id 用首字母 monogram，绝不空白。 */
export function AgentGlyph({ agentId, label, size = 18, className }: AgentGlyphProps) {
  const mark = BRAND_MARKS[agentId];
  const glyphSize = Math.max(11, Math.round(size * 0.72));
  const radius = Math.max(4, Math.round(size * 0.26));

  return (
    <span
      role="img"
      aria-label={label ?? agentId}
      title={label ?? agentId}
      style={{
        width: size,
        height: size,
        color: mark?.color ?? "var(--fg-2)",
        background: mark ? `${mark.color}1f` : "var(--subtle)",
        borderRadius: radius,
      }}
      className={cn("inline-flex shrink-0 items-center justify-center", className)}
    >
      {mark ? (
        <svg
          viewBox={mark.viewBox ?? "0 0 24 24"}
          width={glyphSize}
          height={glyphSize}
          fill="currentColor"
          fillRule="evenodd"
          aria-hidden
          focusable="false"
        >
          {mark.paths.map((d, index) => (
            <path key={index} d={d} />
          ))}
        </svg>
      ) : (
        <span
          className="font-semibold leading-none"
          style={{ fontSize: Math.max(9, Math.round(size * 0.46)) }}
        >
          {agentId.slice(0, 1).toUpperCase()}
        </span>
      )}
    </span>
  );
}
