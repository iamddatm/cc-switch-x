import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";
import { SkillInstallSheet } from "../components/skills/SkillInstallSheet";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    previewSkillSource: vi.fn(),
    installSkill: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(open).mockResolvedValue(null);
  vi.mocked(api.previewSkillSource).mockResolvedValue({
    kind: "local",
    stagingId: null,
    candidates: [],
    warnings: [],
  });
  vi.mocked(api.installSkill).mockResolvedValue({ outcomes: [] });
});

test("previews a local directory and installs all selected candidates", async () => {
  vi.mocked(open).mockResolvedValue("D:/skills");
  vi.mocked(api.previewSkillSource).mockResolvedValue({
    kind: "local",
    stagingId: null,
    warnings: [],
    candidates: [
      { relativePath: ".", name: "Demo", suggestedId: "demo", exists: false },
      {
        relativePath: "nested",
        name: "Nested",
        suggestedId: "nested",
        exists: true,
        description: "子技能",
      },
    ],
  });
  vi.mocked(api.installSkill).mockResolvedValue({
    outcomes: [{ id: "demo", name: ".", status: "installed", warnings: [] }],
  });
  const onInstalled = vi.fn();

  render(<SkillInstallSheet onClose={vi.fn()} onInstalled={onInstalled} />);
  fireEvent.click(screen.getByRole("button", { name: "本地目录" }));
  fireEvent.click(screen.getByRole("button", { name: "选择目录" }));
  await waitFor(() => expect(screen.getByLabelText("本地目录")).toHaveValue("D:/skills"));

  fireEvent.click(screen.getByRole("button", { name: "扫描来源" }));
  expect(await screen.findByText("Demo")).toBeInTheDocument();
  expect(screen.getByText("真源已存在")).toBeInTheDocument();
  expect(api.previewSkillSource).toHaveBeenCalledWith({
    kind: "directory",
    path: "D:/skills",
  });

  fireEvent.click(screen.getByRole("button", { name: "安装选中 (2)" }));
  await waitFor(() =>
    expect(api.installSkill).toHaveBeenCalledWith(
      { kind: "directory", path: "D:/skills" },
      null,
      [
        { relativePath: ".", id: "demo" },
        { relativePath: "nested", id: "nested" },
      ],
      "skip",
    ),
  );
  expect(onInstalled).toHaveBeenCalled();
  expect(await screen.findByText("安装结果")).toBeInTheDocument();
  expect(screen.getByText("已安装")).toBeInTheDocument();
});
