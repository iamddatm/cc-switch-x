import { useMemo, useState } from "react";
import { FolderSearch, RefreshCw } from "lucide-react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { EmptyState } from "../ui/EmptyState";
import { Notice } from "../ui/Notice";
import { SearchField } from "../ui/SearchField";
import { api } from "../../lib/api";
import type {
  FoundLocation,
  UnmanagedImportReport,
  UnmanagedSkill,
  UnmanagedSkillReport,
} from "../../types";

const STATUS_LABELS: Record<string, string> = {
  installed: "已收进真源",
  overwritten: "已覆盖真源里的同名条目",
  skipped: "已跳过",
  failed: "失败",
  linked: "已接管",
  already: "本来就是我们落的",
  refused: "未接管",
};

function statusLabel(status: string): string {
  return STATUS_LABELS[status] ?? status;
}

/** 候选里可以接管的位置：共享目录（`sourceId` 为空）与未实测落点都不算。 */
function takeoverLocations(items: UnmanagedSkill[]): FoundLocation[] {
  const seen = new Map<string, FoundLocation>();
  for (const item of items) {
    for (const found of item.foundIn) {
      if (found.sourceId && found.takeoverPossible) seen.set(found.sourceId, found);
    }
  }
  return [...seen.values()];
}

/** 导入结果折算成几条提示；失败与接管细节必须原样可见，不能压成一句「完成」。 */
function summarize(
  report: UnmanagedImportReport | null,
): { key: string; tone: "success" | "warning" | "danger"; text: string }[] {
  if (!report) return [];
  return report.outcomes.map((outcome, index) => {
    const details = [
      ...outcome.takenOver.map(
        (item) =>
          `${item.label}：${statusLabel(item.status)}${item.detail ? `（${item.detail}）` : ""}` +
          (item.backupPath ? `，留底 ${item.backupPath}` : ""),
      ),
      ...outcome.warnings,
    ];
    if (outcome.error) details.push(outcome.error);
    return {
      key: `outcome-${index}`,
      tone: outcome.error
        ? "danger"
        : details.length > 0
          ? "warning"
          : "success",
      text: [`${outcome.directory || outcome.id}：${statusLabel(outcome.status)}`, ...details].join(
        "；",
      ) as string,
    };
  });
}

/**
 * 「未纳管」视图：列出各 agent 的 skills 落点里**不是我们落的** Skill，查看并导入。
 *
 * 扫描是只读的、由父面板持有（页签计数与横幅要和这里共用一份结果）；本组件只管查看与导入。
 * 导入的落盘口径见 `src-tauri/src/services/skill_takeover.rs`：先收进中央库真源，
 * 再按勾选把盘上那份换成指向真源的部署（真目录先整份留底，链接只解链不删真身）。
 */
export function SkillUnmanagedPanel({
  report,
  scanning,
  error,
  onRescan,
  onImported,
}: {
  report: UnmanagedSkillReport | null;
  scanning: boolean;
  error: string | null;
  onRescan: () => void;
  onImported: () => void;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [pending, setPending] = useState<UnmanagedSkill[] | null>(null);
  const [takeOver, setTakeOver] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<UnmanagedImportReport | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  const skills = useMemo(() => {
    const items = report?.skills ?? [];
    const needle = query.trim().toLowerCase();
    if (!needle) return items;
    return items.filter((item) =>
      [
        item.name,
        item.directory,
        item.suggestedId,
        item.description ?? "",
        item.path,
        ...item.foundIn.map((found) => found.label),
      ]
        .join("\n")
        .toLowerCase()
        .includes(needle),
    );
  }, [query, report]);
  const conflicts = useMemo(() => {
    const items = report?.conflicts ?? [];
    const needle = query.trim().toLowerCase();
    if (!needle) return items;
    return items.filter((item) =>
      `${item.name}\n${item.directory}\n${item.path}`.toLowerCase().includes(needle),
    );
  }, [query, report]);
  const locations = useMemo(() => takeoverLocations(pending ?? []), [pending]);
  const outcomeLines = useMemo(() => summarize(result), [result]);

  const toggleOne = (directory: string, checked: boolean) => {
    setSelected((current) => {
      const next = new Set(current);
      if (checked) next.add(directory);
      else next.delete(directory);
      return next;
    });
  };

  const askImport = (items: UnmanagedSkill[]) => {
    if (items.length === 0) return;
    setFailure(null);
    setResult(null);
    setPending(items);
    // 默认接管「已实测可写」的发现位置，不留一条启不起来的死路；不想要就取消勾选。
    setTakeOver(new Set(takeoverLocations(items).map((found) => found.sourceId as string)));
  };

  const runImport = async () => {
    if (!pending) return;
    setBusy(true);
    setFailure(null);
    try {
      const imported = await api.importUnmanagedSkills(
        pending.map((item) => ({
          directory: item.directory,
          takeOver: item.foundIn
            .map((found) => found.sourceId)
            .filter((id): id is string => id !== null && takeOver.has(id)),
        })),
        "skip",
      );
      setResult(imported);
      setSelected(new Set());
      onImported();
    } catch (reason) {
      setFailure(String(reason));
    } finally {
      setBusy(false);
      setPending(null);
    }
  };

  /** 冲突项只另存为新 ID：接管要删的那份内容并不在真源里，那条路不通。 */
  const runSaveAsNew = async (item: UnmanagedSkill) => {
    setBusy(true);
    setFailure(null);
    setResult(null);
    try {
      const imported = await api.importUnmanagedSkills(
        [{ directory: item.directory, takeOver: [] }],
        "keepBoth",
      );
      setResult(imported);
      onImported();
    } catch (reason) {
      setFailure(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const nothingToDo = (report?.skills.length ?? 0) === 0 && conflicts.length === 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 px-6">
        <SearchField
          className="w-[280px]"
          value={query}
          onValueChange={setQuery}
          placeholder="搜索名称、目录、描述或路径…"
          ariaLabel="搜索未纳管 Skill"
        />
        <Button onClick={onRescan} loading={scanning} disabled={busy}>
          <RefreshCw size={14} aria-hidden />
          重新扫描
        </Button>
        <span className="ml-auto text-caption text-fg-3">
          待处理 {(report?.skills.length ?? 0) + (report?.conflicts.length ?? 0)} 项
        </span>
      </div>

      {(error || failure || outcomeLines.length > 0) && (
        <div className="space-y-2 px-6 pb-3">
          {(error || failure) && <Notice tone="danger">{(error ?? failure) as string}</Notice>}
          {outcomeLines.map((line) => (
            <Notice key={line.key} tone={line.tone}>
              {line.text}
            </Notice>
          ))}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-auto px-6 pb-5">
        {report && report.warnings.length > 0 && (
          <div className="mb-3 space-y-2">
            {report.warnings.map((warning) => (
              <Notice key={warning} tone="warning">
                {warning}
              </Notice>
            ))}
          </div>
        )}

        {scanning && !report ? (
          <p className="py-10 text-center text-caption text-fg-3">正在读取各 agent 的 skills 目录…</p>
        ) : !report ? (
          // 扫描没成功时不能说「没有未纳管」：那是不知道，不是没有。
          <EmptyState
            icon={FolderSearch}
            title="还没有扫描结果"
            description="重新扫描一次才能确认各 agent 的 skills 落点里有没有不归我们管的内容。"
            actions={
              <Button onClick={onRescan} loading={scanning}>
                重新扫描
              </Button>
            }
          />
        ) : nothingToDo ? (
          <EmptyState
            icon={FolderSearch}
            title="没有未纳管的 Skill"
            description="各 agent 的 skills 目录里，含 SKILL.md 的目录都已经由中央库真源管着。"
            actions={
              <Button onClick={onRescan} loading={scanning}>
                重新扫描
              </Button>
            }
          />
        ) : (
          <>
            {selected.size > 0 && (
              <div className="mb-3 flex items-center gap-2 rounded-panel border border-border bg-subtle px-3 py-2">
                <span className="text-caption text-fg-2">已选 {selected.size} 个</span>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={busy}
                  onClick={() => askImport(skills.filter((item) => selected.has(item.directory)))}
                >
                  导入所选并接管
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
                  清空选择
                </Button>
              </div>
            )}

            {skills.length > 0 && (
              <div className="mb-5 overflow-hidden rounded-panel border border-border bg-surface">
                <div className="flex h-11 items-center bg-subtle ps-4 pe-4">
                  <span className="flex w-6 shrink-0 items-center">
                    <Checkbox
                      aria-label="全选未纳管列表"
                      checked={skills.length > 0 && skills.every((item) => selected.has(item.directory))}
                      onChange={() =>
                        setSelected(
                          skills.every((item) => selected.has(item.directory))
                            ? new Set()
                            : new Set(skills.map((item) => item.directory)),
                        )
                      }
                    />
                  </span>
                  <span className="flex-1 text-caption font-semibold text-fg-2">可导入</span>
                </div>
                <ul>
                  {skills.map((item) => (
                    <SkillRow
                      key={item.directory}
                      item={item}
                      checked={selected.has(item.directory)}
                      busy={busy}
                      onChecked={(checked) => toggleOne(item.directory, checked)}
                      onImport={() => askImport([item])}
                    />
                  ))}
                </ul>
              </div>
            )}

            {conflicts.length > 0 && (
              <div className="overflow-hidden rounded-panel border border-border bg-surface">
                <div className="flex h-11 items-center bg-subtle ps-4 pe-4">
                  <span className="flex-1 text-caption font-semibold text-fg-2">
                    真源里已有同名条目
                  </span>
                </div>
                <div className="px-4 pb-2">
                  <Notice tone="warning">
                    这些目录与真源里的某个 ID 同名，但盘上那份不是我们落的。接管会删掉那份内容，
                    而它并不在真源里，所以这里只提供「另存为新 ID」，不动盘。
                  </Notice>
                </div>
                <ul>
                  {conflicts.map((item) => (
                    <SkillRow
                      key={item.directory}
                      item={item}
                      checked={false}
                      busy={busy}
                      selectable={false}
                      importLabel="另存为新 ID"
                      onChecked={() => undefined}
                      onImport={() => void runSaveAsNew(item)}
                    />
                  ))}
                </ul>
              </div>
            )}

            {report && (report.roots.length > 0 || report.skipped.length > 0) && (
              <details className="mt-4 text-caption text-fg-3">
                <summary className="cursor-pointer select-none">
                  扫描明细：{report.roots.length} 个落点，{report.skipped.length} 项未当作候选
                </summary>
                <ul className="mt-2 space-y-1">
                  {report.roots.map((root) => (
                    <li key={`${root.sourceId ?? "shared"}-${root.path}`}>
                      {root.label}：{root.path}
                      {root.exists ? "" : "（目录不存在）"}
                      {root.sandboxed ? "（baseOverride 沙箱）" : ""}
                    </li>
                  ))}
                  {report.skipped.map((skip) => (
                    <li key={skip.dir}>
                      {skip.dir}：{skip.reason}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </>
        )}
      </div>

      <ConfirmDialog
        open={pending !== null}
        title={`导入 ${pending?.length ?? 0} 个 Skill？`}
        pending={busy}
        confirmLabel="导入并接管"
        body={
          <div className="space-y-3">
            <p>
              会把选中的目录整份复制进中央库真源（来源记为本地目录），复制不移动：原目录的内容仍在。
            </p>
            {locations.length > 0 ? (
              <div className="space-y-2">
                <p className="font-medium text-fg-1">接管这些位置的原目录（真目录先整份备份到 backups/skills，链接只解链）</p>
                <ul className="space-y-1">
                  {locations.map((found) => (
                    <li key={`${found.sourceId}-locations`} className="flex items-center gap-2">
                      <Checkbox
                        aria-label={`接管 ${found.label}`}
                        checked={takeOver.has(found.sourceId as string)}
                        onChange={(event) =>
                          setTakeOver((current) => {
                            const next = new Set(current);
                            const key = found.sourceId as string;
                            if (event.target.checked) next.add(key);
                            else next.delete(key);
                            return next;
                          })
                        }
                      />
                      <span className="min-w-0 truncate text-body">{found.label}</span>
                      <span className="truncate font-mono text-caption text-fg-3">{found.path}</span>
                    </li>
                  ))}
                </ul>
                <p className="text-caption text-fg-3">
                  未实测的落点不在列表里：那些位置只收进真源，不改盘。
                </p>
              </div>
            ) : (
              <p className="text-caption text-fg-3">
                这些候选都没有可接管的位置，只写入中央库真源。
              </p>
            )}
            <ul className="max-h-40 space-y-1 overflow-y-auto">
              {(pending ?? []).map((item) => (
                <li key={item.directory} className="truncate text-body">
                  {item.name} <span className="font-mono text-caption text-fg-3">{item.suggestedId}</span>
                </li>
              ))}
            </ul>
          </div>
        }
        onConfirm={() => void runImport()}
        onCancel={() => setPending(null)}
      />
    </div>
  );
}

function SkillRow({
  item,
  checked,
  busy,
  selectable = true,
  importLabel = "导入并接管",
  onChecked,
  onImport,
}: {
  item: UnmanagedSkill;
  checked: boolean;
  busy: boolean;
  selectable?: boolean;
  importLabel?: string;
  onChecked: (checked: boolean) => void;
  onImport: () => void;
}) {
  const detail = [
    item.description,
    item.idRenamed ? `导入后 ID 会变成 ${item.suggestedId}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <li className="flex items-center gap-3 border-b border-border px-4 py-3 transition-colors duration-150 hover:bg-subtle last:border-b-0">
      <span className="flex w-6 shrink-0 items-center">
        {selectable ? (
          <Checkbox
            aria-label={`选择 ${item.name}`}
            checked={checked}
            onChange={(event) => onChecked(event.target.checked)}
          />
        ) : null}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-body font-medium text-fg-1">{item.name}</span>
          <span className="shrink-0 font-mono text-caption text-fg-3">{item.directory}</span>
        </div>
        {detail && <p className="truncate text-caption text-fg-2">{detail}</p>}
        <p className="truncate font-mono text-caption text-fg-3" title={item.path}>
          {item.path}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {item.foundIn.map((found) => (
          <Badge
            key={`${found.sourceId ?? "shared"}-${found.path}`}
            tone={found.takeoverPossible ? "neutral" : "warning"}
            title={
              found.linkTarget
                ? `链接指向 ${found.linkTarget}`
                : found.takeoverPossible
                  ? found.path
                  : "落点未实测，只收进真源"
            }
          >
            {found.label}
          </Badge>
        ))}
        <Button size="sm" disabled={busy} onClick={onImport}>
          {importLabel}
        </Button>
      </div>
    </li>
  );
}
