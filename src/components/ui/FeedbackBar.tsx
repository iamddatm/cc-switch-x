import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

/**
 * 动作反馈条：放在滚动区之外的固定位置，列表滚到哪，操作结果都看得见。
 *
 * 面板里列表往往比一屏长，行内按钮、页底表单点完把提示写在滚动内容顶部，
 * 用户正看着页底就看不到结果 —— 这类提示一律走这里，状态类/报告类提示仍留在内容里。
 */
export function FeedbackBar({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("shrink-0 space-y-2 px-6 pb-2", className)}>{children}</div>;
}
