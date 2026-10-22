import { useState, type ReactNode } from "react";
import type { SessionMessage as SessionMessageData } from "../../types";

export interface SessionMessageProps {
  message: SessionMessageData;
  highlighted: boolean;
  query: string;
}

export function SessionMessage({
  message,
  highlighted,
  query,
}: SessionMessageProps) {
  const content = renderHighlighted(message.content, query);
  const toolName = message.toolName ?? "未知工具";
  return (
    <article
      id={`session-message-${message.seq}`}
      data-highlighted={highlighted ? "true" : undefined}
      className={`rounded border px-3 py-2 ${
        highlighted
          ? "border-amber-400 bg-amber-50 dark:bg-amber-950"
          : "border-slate-200 dark:border-slate-700"
      }`}
    >
      <div className="mb-1 flex items-center gap-2 text-xs text-slate-500">
        <span className="rounded bg-slate-100 px-1 dark:bg-slate-700">
          {roleLabel(message.role)}
        </span>
        <span>{kindLabel(message.kind)}</span>
      </div>
      {message.kind === "thinking" && (
        <details>
          <summary className="cursor-pointer text-sm font-medium">思考</summary>
          <div className="mt-1 whitespace-pre-wrap text-sm text-slate-600 dark:text-slate-300">
            {content}
          </div>
        </details>
      )}
      {message.kind === "tool_call" && (
        <details>
          <summary className="cursor-pointer text-sm font-medium">
            工具调用 · {toolName}
          </summary>
          <pre className="mt-1 overflow-auto whitespace-pre-wrap text-xs">
            {content}
          </pre>
        </details>
      )}
      {message.kind === "tool_result" && (
        <details>
          <summary className="cursor-pointer text-sm font-medium">
            工具结果 · {toolName}
          </summary>
          <pre className="mt-1 overflow-auto whitespace-pre-wrap text-xs">
            {content}
          </pre>
        </details>
      )}
      {message.kind === "image" && <ImageBlock content={message.content} />}
      {message.kind === "event" && (
        <div className="whitespace-pre-wrap text-sm text-slate-500">{content}</div>
      )}
      {message.kind === "text" && (
        <div className="whitespace-pre-wrap text-sm">{content}</div>
      )}
    </article>
  );
}

function ImageBlock({ content }: { content: string }) {
  const [broken, setBroken] = useState(false);
  if (!content.startsWith("data:") || broken) {
    return (
      <p className="text-sm text-amber-700">
        图片无法显示：
        {content.startsWith("data:") ? "内联图片已损坏" : content}
      </p>
    );
  }
  return (
    <img
      src={content}
      alt="会话图片"
      className="max-h-96 max-w-full rounded"
      onError={() => setBroken(true)}
    />
  );
}

function roleLabel(role: SessionMessageData["role"]): string {
  switch (role) {
    case "user":
      return "用户";
    case "assistant":
      return "助手";
    case "tool":
      return "工具";
    default:
      return "系统";
  }
}

function kindLabel(kind: SessionMessageData["kind"]): string {
  switch (kind) {
    case "thinking":
      return "推理";
    case "tool_call":
      return "调用";
    case "tool_result":
      return "结果";
    case "image":
      return "图片";
    case "event":
      return "事件";
    default:
      return "正文";
  }
}

/**
 * 只做文本切分，不拼 HTML：消息正文里的 `<script>` 只会作为文字显示。
 */
function renderHighlighted(text: string, query: string): ReactNode {
  const needle = query.trim();
  if (!needle) {
    return text;
  }
  const lower = text.toLocaleLowerCase();
  const lowerNeedle = needle.toLocaleLowerCase();
  const parts: ReactNode[] = [];
  let start = 0;
  let index = lower.indexOf(lowerNeedle, start);
  let key = 0;
  while (index >= 0) {
    if (index > start) {
      parts.push(text.slice(start, index));
    }
    parts.push(<mark key={key++}>{text.slice(index, index + needle.length)}</mark>);
    start = index + needle.length;
    index = lower.indexOf(lowerNeedle, start);
  }
  if (parts.length === 0) {
    return text;
  }
  if (start < text.length) {
    parts.push(text.slice(start));
  }
  return parts;
}