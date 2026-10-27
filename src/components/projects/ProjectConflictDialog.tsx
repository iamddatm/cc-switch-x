import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Notice } from "../ui/Notice";
import type { ProjectConflict, ProjectConflictResolution, ProjectDirMeta } from "../../types";

function labelOf(dirs: ProjectDirMeta[], key: string): string {
  return dirs.find((dir) => dir.key === key)?.label || key;
}

/**
 * 冲突一律人工裁决。两种情形：
 * - `targetVsCanonical`：副本被外部改过。选项是「以某目录为真源」（先收进真源再渲染其余）
 *   或「两边都保留」（落败的手改另存为未跟踪副本）。
 * - `canonicalMissing`：账本记着有真源、磁盘上却没有。可以拿某个副本重建真源，
 *   也可以显式删除全部副本 —— 自动同步永远不会替用户做这个决定。
 */
export function ProjectConflictDialog({
  conflicts,
  dirs,
  busy,
  onCancel,
  onResolve,
}: {
  conflicts: ProjectConflict[];
  dirs: ProjectDirMeta[];
  busy: boolean;
  onCancel: () => void;
  onResolve: (resolutions: ProjectConflictResolution[]) => void;
}) {
  return (
    <Dialog
      open
      onClose={onCancel}
      role="alertdialog"
      title="需要处理冲突"
      description="项目真源与某些副本对不上，不能自动判定谁更新。请选择保留哪一边；「两边都保留」会把落败的一边另存为未跟踪副本，不会丢内容。"
      width={560}
      footer={
        <Button onClick={onCancel} disabled={busy}>
          先不同步
        </Button>
      }
    >
      <div className="space-y-4">
        {conflicts.map((conflict) => {
          const missingCanonical = conflict.kind === "canonicalMissing";
          const winner = conflict.recommended ?? conflict.dirs[0];
          return (
            <div key={conflict.skillId} className="rounded-panel border border-border p-3">
              <p className="text-body text-fg-1">{conflict.skillId}</p>
              <p className="mt-0.5 text-caption text-fg-3">
                {missingCanonical
                  ? "账本记着有项目真源，但客户端的项目数据目录里找不到它。"
                  : "这些目录里的副本被外部改过，与真源和基线都不一致。"}
              </p>
              <ul className="mt-1 space-y-0.5 text-caption text-fg-2">
                {conflict.dirs.map((dir) => (
                  <li key={dir} className="flex items-center gap-2">
                    <span className="font-mono text-fg-3">{labelOf(dirs, dir)}</span>
                    {dir === conflict.recommended && <span className="text-fg-3">· 修改时间最新</span>}
                  </li>
                ))}
              </ul>
              <div className="mt-2 flex flex-wrap gap-2">
                {conflict.dirs.map((dir) => (
                  <Button
                    key={dir}
                    size="sm"
                    variant={dir === winner ? "primary" : "secondary"}
                    disabled={busy}
                    onClick={() =>
                      onResolve([{ skillId: conflict.skillId, dir, action: "use" }])
                    }
                  >
                    {missingCanonical ? "用它重建真源" : `以 ${labelOf(dirs, dir)} 为真源`}
                  </Button>
                ))}
                {!missingCanonical && (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() =>
                      onResolve([{ skillId: conflict.skillId, dir: winner, action: "keepBoth" }])
                    }
                  >
                    两边都保留
                  </Button>
                )}
                {missingCanonical && (
                  <Button
                    size="sm"
                    variant="danger"
                    disabled={busy}
                    onClick={() =>
                      onResolve([{ skillId: conflict.skillId, dir: "", action: "purge" }])
                    }
                  >
                    删除全部副本
                  </Button>
                )}
              </div>
            </div>
          );
        })}
        <Notice tone="info">删除或替换前都会先备份到备份目录，失败会整批回滚。</Notice>
      </div>
    </Dialog>
  );
}
