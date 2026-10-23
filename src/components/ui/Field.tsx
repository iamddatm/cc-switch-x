import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

export interface FieldProps {
  label: string;
  htmlFor?: string;
  hint?: string;
  error?: string;
  required?: boolean;
  children: ReactNode;
  className?: string;
}

/** 表单行：标签 + 控件 + 错误/提示，错误优先显示。 */
export function Field({ label, htmlFor, hint, error, required, children, className }: FieldProps) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <label htmlFor={htmlFor} className="block text-caption font-medium text-fg-2">
        {label}
        {required && (
          <span aria-hidden className="ml-0.5 text-danger-text">
            *
          </span>
        )}
      </label>
      {children}
      {error ? (
        <p role="alert" className="text-caption text-danger-text">
          {error}
        </p>
      ) : (
        hint && <p className="text-caption text-fg-3">{hint}</p>
      )}
    </div>
  );
}