import { useState } from "react";
import { JsonEditor } from "../JsonEditor";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { Input, Textarea } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { SegmentedControl } from "../ui/SegmentedControl";
import { Select } from "../ui/Select";
import { Sheet } from "../ui/Sheet";
import { api } from "../../lib/api";
import { fromList, toList } from "../../lib/canonical";
import { errorText } from "../../lib/errors";
import {
  linesToRecord,
  parseMcp,
  recordToLines,
  renderMcp,
  validateMcp,
  type McpForm,
  type McpTransport,
} from "../../lib/mcp";

export interface McpFormSheetProps {
  mode: "create" | "edit";
  initial: McpForm;
  onClose: () => void;
  onSaved: () => void;
}

type View = "form" | "json";

export function McpFormSheet({ mode, initial, onClose, onSaved }: McpFormSheetProps) {
  const [form, setForm] = useState<McpForm>(initial);
  const [argsText, setArgsText] = useState(initial.args.join("\n"));
  const [envText, setEnvText] = useState(recordToLines(initial.env));
  const [headersText, setHeadersText] = useState(recordToLines(initial.headers));
  const [tagsText, setTagsText] = useState(fromList(initial.tags));
  const [view, setView] = useState<View>("form");
  const [raw, setRaw] = useState(renderMcp(initial));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = <K extends keyof McpForm>(key: K, value: McpForm[K]) =>
    setForm((current) => ({ ...current, [key]: value }));

  const compose = (): McpForm => ({
    ...form,
    args: argsText.split(/\r?\n/).filter((line) => line.length > 0),
    env: linesToRecord(envText),
    headers: linesToRecord(headersText),
    tags: toList(tagsText),
  });

  const changeView = (next: View) => {
    if (next === view) return;
    setError(null);
    if (next === "json") {
      setRaw(renderMcp(compose()));
    } else {
      try {
        applyParsed(parseMcp(raw));
      } catch (e) {
        setError(errorText(e));
        return;
      }
    }
    setView(next);
  };

  const applyParsed = (parsed: McpForm) => {
    setForm(parsed);
    setArgsText(parsed.args.join("\n"));
    setEnvText(recordToLines(parsed.env));
    setHeadersText(recordToLines(parsed.headers));
    setTagsText(fromList(parsed.tags));
  };

  const save = async () => {
    let current: McpForm;
    if (view === "json") {
      try {
        current = parseMcp(raw);
      } catch (e) {
        setError(errorText(e));
        return;
      }
      applyParsed(current);
    } else {
      current = compose();
    }
    const problem = validateMcp(current);
    if (problem) {
      setError(problem);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await api.writeAsset("mcp", current.id.trim(), renderMcp(current));
      onSaved();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet
      title={mode === "create" ? "添加 MCP" : `编辑 ${initial.name || initial.id}`}
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
        <div className="flex items-center justify-between">
          <SegmentedControl
            ariaLabel="编辑方式"
            value={view}
            onValueChange={changeView}
            items={[
              { value: "form", label: "表单" },
              { value: "json", label: "JSON" },
            ]}
          />
          <span className="text-caption text-fg-3">保存前由前端与后端各校验一次</span>
        </div>

        {error && <Notice tone="danger">{error}</Notice>}

        {view === "json" ? (
          <JsonEditor value={raw} onChange={setRaw} />
        ) : (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <Field
                label="ID"
                htmlFor="mcp-id"
                required={mode === "create"}
                hint={mode === "edit" ? "ID 即真源文件名，编辑时不能改。" : "只能使用 slug 字符，例如 playwright。"}
              >
                <Input
                  id="mcp-id"
                  className="font-mono"
                  value={form.id}
                  readOnly={mode === "edit"}
                  onChange={(e) => set("id", e.target.value)}
                />
              </Field>
              <Field label="显示名称" htmlFor="mcp-name" required>
                <Input
                  id="mcp-name"
                  value={form.name}
                  onChange={(e) => set("name", e.target.value)}
                  placeholder="例如 Playwright"
                />
              </Field>
            </div>

            <Field label="传输方式" htmlFor="mcp-transport">
              <Select
                id="mcp-transport"
                value={form.transport}
                onChange={(e) => set("transport", e.target.value as McpTransport)}
              >
                <option value="stdio">stdio</option>
                <option value="http">http</option>
                <option value="sse">sse</option>
                <option value="ws">ws</option>
              </Select>
            </Field>

            {form.transport === "stdio" ? (
              <>
                <Field label="命令" htmlFor="mcp-command" required>
                  <Input
                    id="mcp-command"
                    className="font-mono"
                    value={form.command}
                    onChange={(e) => set("command", e.target.value)}
                    placeholder="npx"
                  />
                </Field>
                <Field label="参数" htmlFor="mcp-args" hint="每行一个参数。">
                  <Textarea
                    id="mcp-args"
                    className="min-h-[90px] font-mono text-caption"
                    value={argsText}
                    onChange={(e) => setArgsText(e.target.value)}
                  />
                </Field>
                <Field label="环境变量" htmlFor="mcp-env" hint="每行 KEY=VALUE；值里可以再出现 =。">
                  <Textarea
                    id="mcp-env"
                    className="min-h-[90px] font-mono text-caption"
                    value={envText}
                    onChange={(e) => setEnvText(e.target.value)}
                  />
                </Field>
              </>
            ) : (
              <>
                <Field label="URL" htmlFor="mcp-url" required>
                  <Input
                    id="mcp-url"
                    className="font-mono"
                    value={form.url}
                    onChange={(e) => set("url", e.target.value)}
                    placeholder="https://example.com/mcp"
                  />
                </Field>
                <Field label="请求头" htmlFor="mcp-headers" hint="每行 KEY=VALUE。">
                  <Textarea
                    id="mcp-headers"
                    className="min-h-[90px] font-mono text-caption"
                    value={headersText}
                    onChange={(e) => setHeadersText(e.target.value)}
                  />
                </Field>
              </>
            )}

            <Field label="描述" htmlFor="mcp-description" hint="显示在列表行，也用于搜索。">
              <Textarea
                id="mcp-description"
                value={form.description}
                onChange={(e) => set("description", e.target.value)}
                className="min-h-[64px]"
                placeholder="一句话说明这个 MCP 提供什么"
              />
            </Field>

            <div className="grid grid-cols-2 gap-4">
              <Field label="主页" htmlFor="mcp-homepage">
                <Input
                  id="mcp-homepage"
                  className="font-mono"
                  value={form.homepage}
                  onChange={(e) => set("homepage", e.target.value)}
                  placeholder="https://example.com"
                />
              </Field>
              <Field label="文档" htmlFor="mcp-docs">
                <Input
                  id="mcp-docs"
                  className="font-mono"
                  value={form.docs}
                  onChange={(e) => set("docs", e.target.value)}
                  placeholder="https://example.com/docs"
                />
              </Field>
            </div>

            <Field label="标签" htmlFor="mcp-tags" hint="逗号分隔。">
              <Input
                id="mcp-tags"
                value={tagsText}
                onChange={(e) => setTagsText(e.target.value)}
                placeholder="stdio, time, utility"
              />
            </Field>
          </div>
        )}
      </div>
    </Sheet>
  );
}