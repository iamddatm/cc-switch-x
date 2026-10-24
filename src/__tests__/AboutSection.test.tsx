import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { AboutSection } from "../components/settings/AboutSection";
import { makeUpdateController } from "../test/update";

const updateInfo = {
  currentVersion: "0.1.0",
  version: "0.1.1",
  notes: "修复更新问题",
  pubDate: "2026-10-23T00:00:00Z",
};

test("shows current version and can trigger a manual check", () => {
  const check = vi.fn().mockResolvedValue(undefined);
  render(<AboutSection update={makeUpdateController({ check })} />);

  expect(screen.getByText("版本 0.1.0")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "检查更新" }));
  expect(check).toHaveBeenCalledTimes(1);
});

test("shows available update and confirms before installing", () => {
  const install = vi.fn().mockResolvedValue(undefined);
  render(
    <AboutSection
      update={makeUpdateController({
        available: updateInfo,
        hasUpdate: true,
        install,
      })}
    />,
  );

  expect(screen.getByText("发现新版本 v0.1.1")).toBeInTheDocument();
  expect(screen.getByText("修复更新问题")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "下载并安装 v0.1.1" }));
  expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  expect(install).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "下载并安装" }));
  expect(install).toHaveBeenCalledTimes(1);
});

test("shows download progress and install phase", () => {
  const { rerender } = render(
    <AboutSection
      update={makeUpdateController({
        available: updateInfo,
        hasUpdate: true,
        installing: true,
        progress: { downloaded: 5 * 1024 * 1024, total: 10 * 1024 * 1024 },
      })}
    />,
  );

  expect(screen.getByText(/正在下载更新… 50%/)).toBeInTheDocument();

  rerender(
    <AboutSection
      update={makeUpdateController({
        available: updateInfo,
        hasUpdate: true,
        installing: true,
        installStarted: true,
      })}
    />,
  );

  expect(screen.getByText(/正在安装更新，应用将自动重启/)).toBeInTheDocument();
});

test("can dismiss and restore the update prompt", () => {
  const dismiss = vi.fn();
  const resetDismiss = vi.fn();
  const { rerender } = render(
    <AboutSection
      update={makeUpdateController({
        available: updateInfo,
        hasUpdate: true,
        dismiss,
        resetDismiss,
      })}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "忽略此版本" }));
  expect(dismiss).toHaveBeenCalledTimes(1);

  rerender(
    <AboutSection
      update={makeUpdateController({
        available: updateInfo,
        hasUpdate: false,
        dismissed: true,
        dismiss,
        resetDismiss,
      })}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "恢复更新提示" }));
  expect(resetDismiss).toHaveBeenCalledTimes(1);
});

test("renders update errors", () => {
  render(
    <AboutSection
      update={makeUpdateController({ error: "检查更新失败：网络不可用" })}
    />,
  );

  expect(screen.getByText("检查更新失败：网络不可用")).toBeInTheDocument();
});
