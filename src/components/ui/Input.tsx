import type { InputHTMLAttributes, TextareaHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

const FIELD_CLASS =
  "w-full rounded-control border border-border bg-surface px-2.5 text-body text-fg-1 " +
  "placeholder:text-fg-3 transition-colors duration-150 " +
  "focus:border-action focus:outline-none disabled:opacity-45";

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...rest} className={cn(FIELD_CLASS, "h-8", className)} />;
}

export function Textarea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...rest} className={cn(FIELD_CLASS, "py-2 leading-5", className)} />;
}