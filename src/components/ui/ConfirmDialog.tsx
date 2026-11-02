import type { ReactNode } from "react";
import { Button } from "./Button";
import { Dialog } from "./Dialog";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  body?: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  pending?: boolean;
  /** 只锁确认按钮、仍允许取消：用于打开弹框后还要先做一次网络确认的场景。 */
  confirmDisabled?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

/** 不可撤销操作前的二次确认；取消按钮默认聚焦（cc-switch 同款）。 */
export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel = "确认",
  cancelLabel = "取消",
  danger = false,
  pending = false,
  confirmDisabled = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      role="alertdialog"
      title={title}
      footer={
        <>
          <Button variant="secondary" onClick={onCancel} disabled={pending}>
            {cancelLabel}
          </Button>
          <Button
            variant={danger ? "dangerSolid" : "primary"}
            onClick={onConfirm}
            loading={pending}
            disabled={pending || confirmDisabled}
          >
            {confirmLabel}
          </Button>
        </>
      }
    >
      {body && <div className="text-body text-fg-2">{body}</div>}
    </Dialog>
  );
}