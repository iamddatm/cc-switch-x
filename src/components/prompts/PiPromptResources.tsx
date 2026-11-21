import { useState } from "react";
import { Trash2 } from "lucide-react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { EmptyState } from "../ui/EmptyState";
import { Field } from "../ui/Field";
import { Input, Textarea } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { Sheet } from "../ui/Sheet";
import { api } from "../../lib/api";
import { formatBytes } from "../../lib/format";
import type { PiPromptFileKind, PiPromptFileSnapshot, PiPromptTemplate } from "../../types";

export interface PiPromptFileSpec {
  kind: PiPromptFileKind;
  filename: string;
  title: string;
  description: string;
  recommended?: boolean;
}

/** pi 只认这两个固定文件；存在就生效，没有「启用」这一步。 */
export const PI_PROMPT_FILES: PiPromptFileSpec[] = [
  {
    kind: "system_append",
    filename: "APPEND_SYSTEM.md",
    title: "追加系统提示",
    description: "接在 pi 自带系统提示后面，不改动原提示。",
    recommended: true,
  },
  {
    kind: "system_override",
    filename: "SYSTEM.md",
    title: "替换系统提示",
    description: "整份换掉 pi 自带的系统提示，会影响所有会话。",
  },
];

const TEMPLATE_PATTERN = /^[A-Za-z0-9._-]+$/;

function byteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

function firstLine(content: string): string {
  return content.split(/\r?\n/).find((line) => line.trim()) ?? "";
}

export function PiPromptFilesList({
  directory,
  snapshots,
  loading,
  onEdit,
}: {
  directory: string;
  snapshots: Partial<Record<PiPromptFileKind, PiPromptFileSnapshot>>;
  loading: boolean;
  onEdit: (kind: PiPromptFileKind) => void;
}) {
  return (
    <div className="overflow-hidden rounded-panel border border-border bg-surface">
      <div className="flex h-11 items-center bg-subtle px-4">
        <span className="flex-1 text-caption font-semibold text-fg-2">系统提示</span>
        <span className="text-caption text-fg-3">{directory}</span>
      </div>
      {loading && (
        <p className="px-4 py-6 text-caption text-fg-3">正在读取系统提示…</p>
      )}
      {!loading && (
        <ul>
          {PI_PROMPT_FILES.map((file) => {
            const snapshot = snapshots[file.kind];
            return (
              <li
                key={file.kind}
                className="flex min-h-14 items-center gap-3 border-t border-border px-4 first:border-t-0"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="truncate text-body font-medium text-fg-1">
                      {file.title}
                    </span>
                    <code className="shrink-0 font-mono text-caption text-fg-3">
                      {file.filename}
                    </code>
                    {file.recommended && <Badge tone="brand">推荐</Badge>}
                  </div>
                  <p className="truncate text-caption text-fg-2">{file.description}</p>
                </div>
                <span className="shrink-0 text-caption text-fg-3">
                  {!snapshot
                    ? "读取失败"
                    : snapshot.exists
                      ? `已创建 · ${formatBytes(byteLength(snapshot.content))}`
                      : "未创建"}
                </span>
                <Button
                  size="sm"
                  variant="secondary"
                  aria-label={`${snapshot?.exists ? "编辑" : "创建"} ${file.filename}`}
                  disabled={!snapshot}
                  onClick={() => onEdit(file.kind)}
                >
                  {snapshot?.exists ? "编辑" : "创建"}
                </Button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

export function PiPromptFileSheet({
  kind,
  snapshot,
  onClose,
  onSaved,
}: {
  kind: PiPromptFileKind;
  snapshot: PiPromptFileSnapshot;
  onClose: () => void;
  onSaved: () => void;
}) {
  const spec = PI_PROMPT_FILES.find((file) => file.kind === kind)!;
  const [body, setBody] = useState(snapshot.content);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const save = async () => {
    if (!body.trim()) {
      setError("内容不能为空；不需要时请删除文件");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await api.replacePiPromptFile(kind, snapshot.revision, body);
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    setDeleting(true);
    setError(null);
    try {
      await api.deletePiPromptFile(kind, snapshot.revision);
      onSaved();
    } catch (e) {
      setError(String(e));
      setConfirmingDelete(false);
    } finally {
      setDeleting(false);
    }
  };

  return (
    <>
      <Sheet
        title={`${snapshot.exists ? "编辑" : "新建"} ${spec.filename}`}
        onBack={onClose}
        footer={
          <>
            {snapshot.exists && (
              <Button
                variant="danger"
                className="mr-auto"
                disabled={saving || deleting}
                onClick={() => setConfirmingDelete(true)}
              >
                <Trash2 size={14} aria-hidden />
                删除文件
              </Button>
            )}
            <Button onClick={onClose} disabled={saving || deleting}>
              取消
            </Button>
            <Button
              variant="primary"
              loading={saving}
              disabled={deleting}
              onClick={() => void save()}
            >
              保存
            </Button>
          </>
        }
      >
        <div className="mx-auto max-w-3xl space-y-4">
          {error && <Notice tone="danger">{error}</Notice>}
          {kind === "system_override" && (
            <Notice tone="warning">
              SYSTEM.md 会整份替换 pi 自带的系统提示，只影响能读到这个文件的环境。
            </Notice>
          )}
          <Field
            label="正文"
            htmlFor="pi-prompt-file-body"
            hint="保存后立刻生效，无需启用；pi 重启或新开会话时读取。"
          >
            <Textarea
              id="pi-prompt-file-body"
              value={body}
              onChange={(event) => setBody(event.target.value)}
              className="min-h-[360px] font-mono text-caption"
              placeholder={"用 Markdown 写需要 pi 长期遵守的约定…"}
            />
          </Field>
          <p className="text-right text-caption text-fg-3">
            {body.length} 个字符 · {formatBytes(byteLength(body))}
          </p>
        </div>
      </Sheet>
      <ConfirmDialog
        open={confirmingDelete}
        title={`删除 ${spec.filename}？`}
        body="删除后这个文件里的内容会从 pi 的提示词里消失；原文件已留底在备份目录。"
        confirmLabel="删除"
        danger
        pending={deleting}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmingDelete(false)}
      />
    </>
  );
}

export function PiPromptTemplatesList({
  templates,
  loading,
  onEdit,
  onCreate,
}: {
  templates: PiPromptTemplate[];
  loading: boolean;
  onEdit: (template: PiPromptTemplate) => void;
  onCreate: () => void;
}) {
  if (!loading && templates.length === 0) {
    return (
      <EmptyState
        title="还没有模板"
        description="模板会作为斜杠命令出现在 pi 里，例如 /review。"
        actions={
          <Button variant="primary" onClick={onCreate}>
            新建模板
          </Button>
        }
      />
    );
  }
  return (
    <div className="overflow-hidden rounded-panel border border-border bg-surface">
      <div className="flex h-11 items-center bg-subtle px-4">
        <span className="flex-1 text-caption font-semibold text-fg-2">模板</span>
        <span className="text-caption text-fg-3">prompts/</span>
      </div>
      {loading ? (
        <p className="px-4 py-6 text-caption text-fg-3">正在读取模板…</p>
      ) : (
        <ul>
          {templates.map((template) => (
            <li
              key={template.slug}
              className="flex min-h-14 items-center gap-3 border-t border-border px-4 first:border-t-0"
            >
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-body font-medium text-fg-1">
                    /{template.slug}
                  </span>
                  <code className="shrink-0 font-mono text-caption text-fg-3">
                    {template.slug}.md
                  </code>
                </div>
                <p className="truncate font-mono text-caption text-fg-2">
                  {firstLine(template.content) || "空模板"}
                </p>
              </div>
              <Button
                size="sm"
                variant="secondary"
                aria-label={`编辑 /${template.slug}`}
                onClick={() => onEdit(template)}
              >
                编辑
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function PiPromptTemplateSheet({
  template,
  existingSlugs,
  onClose,
  onSaved,
}: {
  template: PiPromptTemplate | null;
  existingSlugs: string[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const [slug, setSlug] = useState(template?.slug ?? "");
  const [content, setContent] = useState(template?.content ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const save = async () => {
    const nextSlug = slug.trim();
    if (!nextSlug) {
      setError("名称不能为空");
      return;
    }
    if (!TEMPLATE_PATTERN.test(nextSlug)) {
      setError("名称只能包含字母、数字、点、下划线或连字符");
      return;
    }
    const taken = existingSlugs.some(
      (existing) => existing === nextSlug && existing !== template?.slug,
    );
    if (taken) {
      setError(`已经有名为 ${nextSlug} 的模板`);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await api.savePiPromptTemplate(
        nextSlug,
        template?.slug,
        template?.revision ?? "missing",
        content,
      );
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    if (!template) return;
    setDeleting(true);
    setError(null);
    try {
      await api.deletePiPromptTemplate(template.slug, template.revision);
      onSaved();
    } catch (e) {
      setError(String(e));
      setConfirmingDelete(false);
    } finally {
      setDeleting(false);
    }
  };

  return (
    <>
      <Sheet
        title={template ? `编辑 /${template.slug}` : "新建模板"}
        onBack={onClose}
        footer={
          <>
            {template && (
              <Button
                variant="danger"
                className="mr-auto"
                disabled={saving || deleting}
                onClick={() => setConfirmingDelete(true)}
              >
                <Trash2 size={14} aria-hidden />
                删除模板
              </Button>
            )}
            <Button onClick={onClose} disabled={saving || deleting}>
              取消
            </Button>
            <Button
              variant="primary"
              loading={saving}
              disabled={deleting}
              onClick={() => void save()}
            >
              保存
            </Button>
          </>
        }
      >
        <div className="mx-auto max-w-3xl space-y-4">
          {error && <Notice tone="danger">{error}</Notice>}
          <Field
            label="名称"
            htmlFor="pi-template-slug"
            required
            hint="保存后在 pi 里用 /名称 调用；改名会同时移动文件。"
          >
            <Input
              id="pi-template-slug"
              value={slug}
              onChange={(event) => setSlug(event.target.value)}
              placeholder="例如 review"
            />
          </Field>
          <Field
            label="模板正文"
            htmlFor="pi-template-content"
            hint="模板内容会和命令一起提交给 pi。"
          >
            <Textarea
              id="pi-template-content"
              value={content}
              onChange={(event) => setContent(event.target.value)}
              className="min-h-[320px] font-mono text-caption"
              placeholder={"回顾这次改动，先列风险点，再给结论。\n\n$ARGUMENTS"}
            />
          </Field>
          <p className="text-right text-caption text-fg-3">
            {content.length} 个字符 · {formatBytes(byteLength(content))}
          </p>
        </div>
      </Sheet>
      <ConfirmDialog
        open={confirmingDelete}
        title={`删除模板 /${template?.slug ?? ""}？`}
        body="删除后这个斜杠命令会从 pi 里消失；原文件已留底在备份目录。"
        confirmLabel="删除"
        danger
        pending={deleting}
        onConfirm={() => void remove()}
        onCancel={() => setConfirmingDelete(false)}
      />
    </>
  );
}
