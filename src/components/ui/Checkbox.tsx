import type { InputHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export interface CheckboxProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
}

/** 表单用原生 checkbox（键盘与读屏行为最稳）；矩阵单元格另有紧凑实现。 */
export function Checkbox({ label, className, id, ...rest }: CheckboxProps) {
  const input = (
    <input
      id={id}
      type="checkbox"
      {...rest}
      className={cn("h-4 w-4 shrink-0 accent-action", className)}
    />
  );
  if (!label) return input;
  return (
    <label htmlFor={id} className="inline-flex items-center gap-2 text-body text-fg-1">
      {input}
      <span>{label}</span>
    </label>
  );
}