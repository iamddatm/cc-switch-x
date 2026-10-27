import { useState } from "react";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Dialog } from "../ui/Dialog";
import { Notice } from "../ui/Notice";
import type {
  MigrationDecision,
  ProjectDirMeta,
  ProjectMigrationPlan,
} from "../../types";

function labelOf(dirs: ProjectDirMeta[], key: string): string {
  return dirs.find((dir) => dir.key === key)?.label || key;
}

/**
 * v1「对等副本」台账迁到 v2「项目真源」的确认对话框。
 *
 * 必须让用户看清两件事：会在客户端数据目录里新建哪些文件，以及**不会**改动任何
 * agent 目录的字节。冲突条目一律要人工裁决（含显式「跳过」），否则不开放提交 ——
 * 后端同样会整体拒绝，这里只是把失败提前到点按钮之前。
 */
export function ProjectMigrationDialog({
  plan,
  dirs,
  busy,
  onCancel,
  onConfirm,
}: {
  plan: ProjectMigrationPlan;
  dirs: ProjectDirMeta[];
  busy: boolean;
  onCancel: () => void;
  onConfirm: (decisions: MigrationDecision[]) => void;
}) {
  const [adopted, setAdopted] = useState<Record<string, boolean>>(() =>
    Object.fromEntries(plan.proposals.map((item) => [item.skillId, item.defaultAdopt])),
  );
  const [winners, setWinners] = useState<Record<string, string>>({});

  const writes = [
    ...plan.adopts.flatMap((item) => item.files),
    ...plan.proposals
      .filter((item) => adopted[item.skillId])
      .flatMap((item) => item.files),
    "skills.json",
  ].sort();
  const undecided = plan.conflicts.filter((item) => winners[item.skillId] === undefined);

  const submit = () => {
    const decisions: MigrationDecision[] = [
      // 候选也显式提交，避免前后端各自理解「默认」的含义。
      ...plan.proposals.map((item) => ({
        skillId: item.skillId,
        dir: adopted[item.skillId] ? item.fromDir : "",
      })),
      ...plan.conflicts.map((item) => ({
        skillId: item.skillId,
        dir: winners[item.skillId] ?? "",
      })),
    ];
    onConfirm(decisions);
  };

  return (
    <Dialog
      open
      onClose={onCancel}
      role="alertdialog"
      title="把项目 skill 收进项目真源"
      description="项目级 Skills 改为在客户端数据目录里保留唯一可编辑真源，.claude / .qoder / .agents 目录变成它的渲染产物。迁移只在客户端新建真源文件，不会改动任何 agent 目录的字节。"
      width={620}
      footer={
        <>
          <Button onClick={onCancel} disabled={busy}>
            先不迁移
          </Button>
          <Button
            variant="primary"
            loading={busy}
            disabled={undecided.length > 0}
            title={
              undecided.length > 0
                ? `还有 ${undecided.length} 个冲突需要你裁决`
                : undefined
            }
            onClick={submit}
          >
            确认迁移
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <Notice tone="info">
          将在你仓库里新建 {writes.length} 个文件（含台账本身）：
          <ul className="mt-1 max-h-32 space-y-0.5 overflow-auto font-mono text-badge text-fg-2">
            {writes.map((file) => (
              <li key={file}>{file}</li>
            ))}
          </ul>
        </Notice>

        {plan.adopts.length > 0 && (
          <section>
            <p className="text-body font-medium text-fg-1">
              收进真源（{plan.adopts.length} 个，各渲染目标内容一致）
            </p>
            <ul className="mt-1 space-y-0.5 text-caption text-fg-2">
              {plan.adopts.map((item) => (
                <li key={item.skillId}>
                  <span className="font-mono">{item.skillId}</span> · 取自 {labelOf(dirs, item.fromDir)}
                </li>
              ))}
            </ul>
          </section>
        )}

        {plan.proposals.length > 0 && (
          <section>
            <p className="text-body font-medium text-fg-1">以下副本还没被勾选为目标</p>
            <ul className="mt-1 space-y-1">
              {plan.proposals.map((item) => (
                <li key={item.skillId} className="flex items-start gap-2">
                  <Checkbox
                    checked={adopted[item.skillId] ?? false}
                    onChange={(event) =>
                      setAdopted((prev) => ({
                        ...prev,
                        [item.skillId]: event.target.checked,
                      }))
                    }
                    aria-label={`把 ${item.skillId} 收进项目真源`}
                    label={`${item.skillId}（取自 ${labelOf(dirs, item.fromDir)}）`}
                  />
                  <span className="text-caption text-fg-3">{item.reason}</span>
                </li>
              ))}
            </ul>
          </section>
        )}

        {plan.conflicts.length > 0 && (
          <section>
            <p className="text-body font-medium text-danger-text">
              需要你裁决（{plan.conflicts.length} 个）
            </p>
            <ul className="mt-1 space-y-2">
              {plan.conflicts.map((conflict) => (
                <li key={conflict.skillId} className="rounded-panel border border-border p-2">
                  <p className="text-caption text-fg-1">
                    <span className="font-mono">{conflict.skillId}</span>
                    <span className="ml-2 text-fg-3">
                      {conflict.kind === "unresolvable"
                        ? "有目录的状态取不到（符号链接等），不能自动判定"
                        : "各目录内容不一致，不能自动判定谁是真源"}
                    </span>
                  </p>
                  <div className="mt-1 flex flex-wrap gap-2">
                    {conflict.dirs.map((dir) => (
                      <Button
                        key={dir}
                        size="sm"
                        variant={winners[conflict.skillId] === dir ? "primary" : "secondary"}
                        disabled={busy}
                        onClick={() => setWinners((prev) => ({ ...prev, [conflict.skillId]: dir }))}
                      >
                        以 {labelOf(dirs, dir)} 为真源
                      </Button>
                    ))}
                    <Button
                      size="sm"
                      variant={winners[conflict.skillId] === "" ? "primary" : "secondary"}
                      disabled={busy}
                      onClick={() => setWinners((prev) => ({ ...prev, [conflict.skillId]: "" }))}
                    >
                      跳过这条
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          </section>
        )}

        {plan.drops.length > 0 && (
          <p className="text-caption text-fg-3">
            台账里有 {plan.drops.length} 个条目在磁盘上已经不存在，只清理条目，不写任何文件。
          </p>
        )}
        {plan.warnings.length > 0 && (
          <Notice tone="warning">
            <ul className="space-y-0.5">
              {plan.warnings.map((warning) => (
                <li key={warning}>{warning}</li>
              ))}
            </ul>
          </Notice>
        )}
      </div>
    </Dialog>
  );
}
