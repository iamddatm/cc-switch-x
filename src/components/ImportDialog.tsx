import { useEffect, useState } from "react";
import { Button } from "./ui/Button";
import { Select } from "./ui/Select";
import { api } from "../lib/api";
import type { ConflictPolicy, ImportPreview, ImportSummary } from "../types";

const POLICIES: { value: ConflictPolicy; label: string }[] = [
  { value: "skip", label: "跳过已存在的" },
  { value: "overwrite", label: "覆盖已存在的" },
  { value: "keepBoth", label: "两条都留（加序号）" },
];

/** 从 cc-switch 导入：先预览计数，确认后执行。复制不移动。 */
export function ImportDialog({ onClose }: { onClose: () => void }) {
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const [policy, setPolicy] = useState<ConflictPolicy>("skip");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api
      .importPreview()
      .then(setPreview)
      .catch((e) => setError(String(e)));
  }, []);

  const apply = async () => {
    try {
      setSummary(await api.importApply(policy));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div
      role="dialog"
      aria-label="从 cc-switch 导入"
      className="fixed inset-0 flex items-center justify-center bg-overlay p-6"
    >
      <div className="max-h-full w-full max-w-xl overflow-auto rounded-dialog border border-border bg-surface p-6 shadow-v7-lg">
        <h2 className="mb-4 text-title text-fg-1">从 cc-switch 导入</h2>
        {error && <p className="mb-3 text-body text-danger-text">{error}</p>}

        {!preview && !error && <p className="text-body text-fg-3">正在读取 cc-switch…</p>}

        {preview && !summary && (
          <div className="space-y-3 text-body">
            <p>MCP {preview.mcpCount} 条</p>
            <p>Prompts {preview.promptsCount} 条</p>
            <p>Skills {preview.skillsCount} 条</p>
            {preview.conflicts.length > 0 && (
              <p className="text-warning-text">
                已有同名条目 {preview.conflicts.length} 个：{preview.conflicts.join("、")}
              </p>
            )}
            {preview.missingSkillDirs.length > 0 && (
              <p className="text-warning-text">
                有记录但文件缺失的 skill：{preview.missingSkillDirs.join("、")}
              </p>
            )}
            {preview.ignoredColumns.length > 0 && (
              <p className="text-fg-3">
                忽略的启用列：{preview.ignoredColumns.join("、")}
              </p>
            )}
            <label className="block">
              冲突处理
              <Select
                className="ml-2"
                value={policy}
                onChange={(e) => setPolicy(e.target.value as ConflictPolicy)}
              >
                {POLICIES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </Select>
            </label>
          </div>
        )}

        {summary && (
          <div className="space-y-2 text-body">
            <p>
              导入完成：MCP {summary.mcpImported} 条、Prompts {summary.promptsImported} 条、Skills{" "}
              {summary.skillsImported} 条
            </p>
            {summary.unchanged > 0 && <p>未变更 {summary.unchanged} 条</p>}
            {summary.failed.length > 0 && (
              <p className="text-danger-text">失败：{summary.failed.join("、")}</p>
            )}
            {summary.ambiguousActive.length > 0 && (
              <p className="text-warning-text">
                这些 agent 有多条启用中的 prompt，已取最新：{summary.ambiguousActive.join("、")}
              </p>
            )}
            <p className="text-success-text">
              复制不移动：cc-switch 数据保持原样，可按需自行清理。
            </p>
          </div>
        )}

        <div className="mt-6 flex gap-2">
          {!summary && (
            <Button variant="primary" disabled={!preview} onClick={() => void apply()}>
              开始导入
            </Button>
          )}
          <Button variant="secondary" onClick={onClose}>
            关闭
          </Button>
        </div>
      </div>
    </div>
  );
}
