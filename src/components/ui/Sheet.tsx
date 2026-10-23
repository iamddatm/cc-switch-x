import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { IconButton } from "./IconButton";

export interface SheetProps {
  title: string;
  onBack: () => void;
  backLabel?: string;
  actions?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
}

/**
 * 盖住内容区的编辑面板（不是居中弹窗）。父容器必须有 relative，
 * 这样侧栏仍可切换，返回时列表状态也还在。
 */
export function Sheet({ title, onBack, backLabel = "返回", actions, footer, children }: SheetProps) {
  return (
    <div className="absolute inset-0 z-40 flex flex-col bg-app">
      <header className="flex h-[52px] shrink-0 items-center gap-2 border-b border-border px-4">
        <IconButton label={backLabel} onClick={onBack}>
          <ArrowLeft size={16} aria-hidden />
        </IconButton>
        <h2 className="truncate text-title text-fg-1">{title}</h2>
        {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
      </header>
      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">{children}</div>
      {footer && (
        <footer className="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-border px-6">
          {footer}
        </footer>
      )}
    </div>
  );
}