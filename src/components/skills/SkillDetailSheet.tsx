import { Pencil } from "lucide-react";
import type { ReactNode } from "react";
import type { SkillForm } from "../../lib/canonical";
import { formatRelativeTime } from "../../lib/format";
import type { AssetSummary } from "../../types";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Sheet } from "../ui/Sheet";
import { SkillSourceBadge } from "./SkillSourceBadge";

/** 只读详情：先看清 SKILL.md 与来源，再决定是否进入编辑。 */
export function SkillDetailSheet({
  item,
  form,
  onEdit,
  onClose,
  extra,
}: {
  item: AssetSummary;
  form: SkillForm;
  onEdit: () => void;
  onClose: () => void;
  /** 作用域专属的信息（例如项目里的渲染目录），放在来源行下面。 */
  extra?: ReactNode;
}) {
  const checkedAt = item.origin?.lastCheckedAt
    ? `检查于 ${formatRelativeTime(item.origin.lastCheckedAt)}`
    : null;
  const updatedAt = item.mtime ? `更新于 ${formatRelativeTime(item.mtime)}` : null;

  return (
    <Sheet
      title={form.name || item.name}
      onBack={onClose}
      backLabel="返回 Skills"
      actions={
        <Button size="sm" onClick={onEdit}>
          <Pencil size={14} aria-hidden />
          编辑
        </Button>
      }
      footer={<Button onClick={onClose}>关闭</Button>}
    >
      <div className="mx-auto max-w-3xl space-y-4">
        <div className="flex flex-wrap items-center gap-2 text-caption text-fg-3">
          <span className="font-mono">{item.id}</span>
          <SkillSourceBadge item={item} />
          {item.origin?.updateAvailable && <Badge tone="warning">可更新</Badge>}
          {updatedAt && <span>{updatedAt}</span>}
          {checkedAt && <span>{checkedAt}</span>}
        </div>

        {extra}

        {form.description && (
          <p className="text-body text-fg-2">{form.description}</p>
        )}

        {item.origin?.lastCheckError && (
          <p className="text-caption text-danger-text">
            上次检查失败：{item.origin.lastCheckError}
          </p>
        )}

        <section className="space-y-2">
          <h3 className="text-caption font-semibold text-fg-2">SKILL.md 正文</h3>
          <pre className="max-h-[60vh] overflow-auto whitespace-pre-wrap break-words rounded-panel border border-border bg-subtle p-4 font-mono text-caption leading-relaxed text-fg-1">
            {form.body || "（正文为空）"}
          </pre>
        </section>

        {form.extraFrontmatter.trim() && (
          <details className="rounded-panel border border-border bg-surface p-3">
            <summary className="cursor-pointer text-caption font-medium text-fg-2">
              其他 frontmatter
            </summary>
            <pre className="mt-2 overflow-auto whitespace-pre-wrap break-words font-mono text-caption text-fg-2">
              {form.extraFrontmatter}
            </pre>
          </details>
        )}
      </div>
    </Sheet>
  );
}
