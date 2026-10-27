import type { ReactNode } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import type { AssetSummary } from "../../types";
import type { SourceScanState } from "./useSourceScan";

/**
 * 整文件资产（Rules / Subagents / Hooks）编辑面板共用的骨架。
 * 三者只有「表单字段、脚注、有没有扫描 / 漂移」不同，容器与左右两栏的形状是一样的；
 * 把形状收在这里，改布局时只动一处，也不会再出现同一个 bug 修两遍。
 */
export function AssetEditorLayout({ children }: { children: ReactNode }) {
  return (
    <div className="grid h-full grid-cols-[240px_1fr] gap-6 overflow-auto px-6 py-5">{children}</div>
  );
}

/** 左栏：真源列表 + 新建 + 扫描导入结果。 */
export function AssetSourceList({
  title,
  items,
  selected,
  onSelect,
  listError,
  emptyHint,
  newIdLabel,
  newIdValue,
  onNewIdChange,
  onCreate,
  scan,
}: {
  title: string;
  items: AssetSummary[];
  selected: string | null;
  onSelect: (id: string) => void;
  listError?: string | null;
  emptyHint?: string;
  newIdLabel: string;
  newIdValue: string;
  onNewIdChange: (value: string) => void;
  onCreate: () => void;
  scan?: SourceScanState;
}) {
  return (
    <section>
      <h2 className="mb-3 text-page text-fg-1">{title}</h2>
      {listError && <p className="text-caption text-danger-text">{listError}</p>}
      <ul className="space-y-0.5">
        {items.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              className={`w-full rounded-control px-2.5 py-1.5 text-left text-body transition-colors duration-150 ${
                selected === item.id
                  ? "bg-selected text-fg-1"
                  : "text-fg-2 hover:bg-subtle hover:text-fg-1"
              }`}
              onClick={() => onSelect(item.id)}
            >
              {item.name}
            </button>
          </li>
        ))}
      </ul>
      {items.length === 0 && emptyHint && (
        <p className="mt-2 text-caption text-fg-3">{emptyHint}</p>
      )}
      <div className="mt-4 flex gap-2">
        <Input
          aria-label={newIdLabel}
          className="min-w-0 flex-1"
          placeholder={newIdLabel}
          value={newIdValue}
          onChange={(e) => onNewIdChange(e.target.value)}
        />
        <Button variant="secondary" onClick={onCreate}>
          新建
        </Button>
      </div>
      {scan && (
        <>
          <Button
            variant="secondary"
            className="mt-2 w-full"
            loading={scan.running}
            onClick={() => void scan.run()}
          >
            从 agent 目录扫描导入
          </Button>
          {scan.report && (
            <p className="mt-2 text-caption text-fg-3">
              合计：新增 {scan.report.imported}、更新 {scan.report.updated}、未变{" "}
              {scan.report.unchanged}
              {scan.report.skipped.length > 0 &&
                `、跳过 ${scan.report.skipped.length}（${scan.report.skipped
                  .map((s) => s.file)
                  .join("、")}）`}
            </p>
          )}
          {scan.error && (
            <p className="mt-2 text-caption text-danger-text">{scan.error}</p>
          )}
        </>
      )}
    </section>
  );
}

/** 右栏：处置区（漂移横幅等）+ 保存 / 删除 + 表单错误，剩下的表单由面板自己给。 */
export function AssetEditorPane({
  selected,
  header,
  formError,
  onSave,
  onDelete,
  notSelectedHint,
  children,
}: {
  selected: string | null;
  header?: ReactNode;
  formError?: string | null;
  onSave: () => void;
  onDelete: () => void;
  notSelectedHint: string;
  children: ReactNode;
}) {
  return (
    <section>
      {header}
      {selected ? (
        <>
          <div className="mb-3 flex gap-2">
            <Button variant="primary" onClick={onSave}>
              保存
            </Button>
            <Button variant="danger" onClick={onDelete}>
              删除
            </Button>
          </div>
          {formError && <p className="mb-2 text-caption text-danger-text">{formError}</p>}
          {children}
        </>
      ) : (
        <p className="text-body text-fg-3">{notSelectedHint}</p>
      )}
    </section>
  );
}
