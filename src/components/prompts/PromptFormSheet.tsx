import { useState } from "react";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { Input, Textarea } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { Sheet } from "../ui/Sheet";
import { api } from "../../lib/api";
import { renderPrompt, type PromptForm } from "../../lib/canonical";
import { formatBytes } from "../../lib/format";

export interface PromptFormSheetProps {
  mode: "create" | "edit";
  id: string;
  initial: PromptForm;
  onClose: () => void;
  onSaved: () => void;
}

const ID_PATTERN = /^[A-Za-z0-9._-]+$/;

export function PromptFormSheet({
  mode,
  id: initialId,
  initial,
  onClose,
  onSaved,
}: PromptFormSheetProps) {
  const [id, setId] = useState(initialId);
  const [form, setForm] = useState<PromptForm>(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const bytes = new TextEncoder().encode(form.body).length;

  const set = <K extends keyof PromptForm>(key: K, value: PromptForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  const save = async () => {
    const trimmedId = id.trim();
    if (!ID_PATTERN.test(trimmedId) || trimmedId === "." || trimmedId === "..") {
      setError("ID 只能包含字母、数字、点、下划线和短横线");
      return;
    }
    if (!form.title.trim()) {
      setError("名称不能为空");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await api.writeAsset("prompt", trimmedId, renderPrompt(form));
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
        <div className="grid grid-cols-2 gap-4">
          <Field
            label="ID"
            htmlFor="prompt-id"
            required={mode === "create"}
            hint={mode === "edit" ? "ID 即真源文件名，编辑时不能改。" : "只能使用 slug 字符，例如 team-style。"}
          >
            <Input
              id="prompt-id"
              className="font-mono"
              value={id}
              readOnly={mode === "edit"}
              onChange={(e) => setId(e.target.value)}
            />
          </Field>
          <Field label="名称" htmlFor="prompt-title" required>
            <Input
              id="prompt-title"
              value={form.title}
              onChange={(e) => set("title", e.target.value)}
              placeholder="例如 团队编码规范"
            />
          </Field>
        </div>
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