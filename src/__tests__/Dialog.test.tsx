import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Dialog } from "../components/ui/Dialog";

test("dialog closes on Escape and on an overlay click", () => {
  const onClose = vi.fn();
  const { rerender } = render(
    <Dialog open onClose={onClose} title="测试对话框">
      <p>内容</p>
    </Dialog>,
  );

  const dialog = screen.getByRole("dialog", { name: "测试对话框" });
  expect(dialog).toBeInTheDocument();

  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalledTimes(1);

  fireEvent.mouseDown(dialog.parentElement as HTMLElement);
  expect(onClose).toHaveBeenCalledTimes(2);

  rerender(
    <Dialog open={false} onClose={onClose} title="测试对话框">
      <p>内容</p>
    </Dialog>,
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("confirm dialog exposes both actions and a danger role", () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <ConfirmDialog
      open
      title="删除这条记录？"
      body="不可撤销"
      confirmLabel="删除"
      danger
      onConfirm={onConfirm}
      onCancel={onCancel}
    />,
  );

  expect(screen.getByRole("alertdialog", { name: "删除这条记录？" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  expect(onCancel).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole("button", { name: "删除" }));
  expect(onConfirm).toHaveBeenCalledTimes(1);
});