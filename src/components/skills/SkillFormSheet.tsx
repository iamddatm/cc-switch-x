import { useState } from "react";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { Input, Textarea } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { Sheet } from "../ui/Sheet";
import { renderSkill, type SkillForm } from "../../lib/canonical";
import type { SkillScope } from "./skillScope";

export interface SkillFormSheetProps {
  mode: "create" | "edit";
  id: string;
  initial: SkillForm;
  /** 写到哪一份真源：中央库还是项目。 */
  scope: SkillScope;
  onClose: () => void;
  onSaved: () => void;
}

const ID_PATTERN = /^[A-Za-z0-9._-]+$/;

export function SkillFormSheet({
  mode,
  id: initialId,
  initial,
  scope,
  onClose,
  onSaved,
}: SkillFormSheetProps) {
  const [id, setId] = useState(initialId);
  const [form, setForm] = useState<SkillForm>(initial);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = <K extends keyof SkillForm>(key: K, value: SkillForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  const save = async () => {
    const trimmedId = id.trim();
    if (!ID_PATTERN.test(trimmedId) || trimmedId === "." || trimmedId === "..") {
      setError("ID 只能包含字母、数字、点、下划线和短横线");
      return;
    }
    if (!form.name.trim()) {
      setError("名称不能为空");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await scope.saveSkill(trimmedId, renderSkill(form), mode === "create");
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet
      title={mode === "create" ? "新建 Skill" : `编辑 ${initial.name || initialId}`}
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
        <Field
          label="ID（目录名）"
          htmlFor="skill-id"
          required={mode === "create"}
          hint={mode === "edit" ? "ID 即真源目录名，编辑时不能改；需要改名请新建后删除旧项。" : "只能使用 slug 字符，例如 lark-wiki。"}
        >
          <Input
            id="skill-id"
            value={id}
            readOnly={mode === "edit"}
            onChange={(e) => setId(e.target.value)}
            className="font-mono"
          />
        </Field>
        <Field label="名称" htmlFor="skill-name" required>
          <Input
            id="skill-name"
            value={form.name}
            onChange={(e) => set("name", e.target.value)}
            placeholder="例如 Lark 知识库"
          />
        </Field>
        <Field label="描述" htmlFor="skill-description" hint="显示在列表行，也用于搜索。">
          <Textarea
            id="skill-description"
            value={form.description}
            onChange={(e) => set("description", e.target.value)}
            className="min-h-[64px]"
            placeholder="一句话说明这个 Skill 做什么"
          />
        </Field>
        <Field label="正文（SKILL.md）" htmlFor="skill-body" hint="Markdown 正文；agent 实际读取的内容。">
          <Textarea
            id="skill-body"
            value={form.body}
            onChange={(e) => set("body", e.target.value)}
            className="min-h-[280px] font-mono text-caption"
          />
        </Field>
        <details className="rounded-panel border border-border bg-surface p-3">
          <summary className="cursor-pointer text-caption font-medium text-fg-2">
            其他 frontmatter（原样保留）
          </summary>
          <p className="mt-1 text-caption text-fg-3">
            这里只放 name/description 以外的字段；保存时按原样写回，不会被丢弃。
          </p>
          <Textarea
            aria-label="其他 frontmatter"
            value={form.extraFrontmatter}
            onChange={(e) => set("extraFrontmatter", e.target.value)}
            className="mt-2 min-h-[100px] font-mono text-caption"
          />
        </details>
      </div>
    </Sheet>
  );
}