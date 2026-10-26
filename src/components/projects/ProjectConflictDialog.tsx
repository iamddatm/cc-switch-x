import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Notice } from "../ui/Notice";
import type { ProjectConflict, ProjectConflictResolution, ProjectDirMeta } from "../../types";

function labelOf(dirs: ProjectDirMeta[], key: string): string {
  return dirs.find((dir) => dir.key === key)?.label || key;
}

/**
 * 冲突一律人工裁决：每个冲突给出「保留 X」和「两边都保留」。
 * 一次只提交一条处置，后端按它写盘并重新报告剩余冲突，界面因此会多次打开。
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
      description="同一个 skill 在多个目录里都有改动，不能自动判定谁更新。请选择保留哪一边；「两边都保留」会把落败的一边另存为未跟踪副本，不会丢内容。"
      width={560}
      footer={<Button onClick={onCancel} disabled={busy}>先不同步</Button>}
    >
      <div className="space-y-4">
        {conflicts.map((conflict) => {
          const winner = conflict.recommended ?? conflict.dirs[0];
          return (
            <div key={conflict.skillId} className="rounded-panel border border-border p-3">
              <p className="text-body text-fg-1">{conflict.skillId}</p>
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
                    保留 {labelOf(dirs, dir)}
                  </Button>
                ))}
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() =>
                    onResolve([{ skillId: conflict.skillId, dir: winner, action: "keepBoth" }])
                  }
                >
                  两边都保留
                </Button>
              </div>
            </div>
          );
        })}
        <Notice tone="info">
          删除或替换前都会先备份到备份目录，失败会整批回滚。
        </Notice>
      </div>
    </Dialog>
  );
}
