import { useState } from "react";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { Input, Textarea } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { Sheet } from "../ui/Sheet";
import { api } from "../../lib/api";
import { generatePromptId, renderPrompt, type PromptForm } from "../../lib/canonical";
import { formatBytes } from "../../lib/format";

export interface PromptFormSheetProps {
  mode: "create" | "edit";
  id: string;
  initial: PromptForm;
  /** 已有的提示词 ID：新建时用来避免生成重名。 */
  existingIds: string[];
  onClose: () => void;
  onSaved: () => void;
}

export function PromptFormSheet({
  mode,
  id: initialId,
  initial,
  existingIds,
  onClose,
  onSaved,
}: PromptFormSheetProps) {
  const [form, setForm] = useState<PromptForm>(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const bytes = new TextEncoder().encode(form.body).length;

  const set = <K extends keyof PromptForm>(key: K, value: PromptForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  const save = async () => {
    if (!form.title.trim()) {
      setError("名称不能为空");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      // ID 只是真源文件名，用户填完名称后自动生成；编辑时沿用原 ID。
      const finalId = mode === "edit" ? initialId : generatePromptId(form.title, existingIds);
      await api.writeAsset("prompt", finalId, renderPrompt(form));
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet
      title={mode === "create" ? "添加提示词" : `编辑 ${initial.title || initialId}`}
      onBack={onClose}
      footer={
        <>
          <Button onClick={onClose} disabled={saving}>
            取消
          </Button>
          <Button variant="primary" loading={saving} onClick={() => void save()}>
            保存
          </Button>
        </>
      }
    >
      <div className="mx-auto max-w-3xl space-y-4">
        {error && <Notice tone="danger">{error}</Notice>}
        <Field label="名称" htmlFor="prompt-title" required>
          <Input
            id="prompt-title"
            value={form.title}
            onChange={(e) => set("title", e.target.value)}
            placeholder="例如 团队编码规范"
          />
        </Field>
        <Field label="说明" htmlFor="prompt-description" hint="只在客户端显示，不写进目标文件。">
          <Textarea
            id="prompt-description"
            value={form.description}
            onChange={(e) => set("description", e.target.value)}
            className="min-h-[64px]"
            placeholder="可选，一句话说明这条提示词的用途"
          />
        </Field>
        <Field label="正文" htmlFor="prompt-body" hint="启用时会整份写入目标文件，frontmatter 不会混进去。">
          <Textarea
            id="prompt-body"
            value={form.body}
            onChange={(e) => set("body", e.target.value)}
            className="min-h-[320px] font-mono text-caption"
            placeholder={"# 团队编码规范\n\n用 Markdown 写长期说明…"}
          />
        </Field>
        <p className="text-right text-caption text-fg-3">
          {form.body.length} 个字符 · {formatBytes(bytes)}
        </p>
      </div>
    </Sheet>
  );
}
