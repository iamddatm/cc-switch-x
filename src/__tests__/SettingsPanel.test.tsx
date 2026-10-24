import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { SettingsPanel } from "../panels/SettingsPanel";
import { api } from "../lib/api";
import { makeUpdateController } from "../test/update";

vi.mock("../lib/api", () => ({
  api: {
    getSettings: vi.fn(),
    setTheme: vi.fn(),
    setStoreRoot: vi.fn(),
    setCloseToTray: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSettings).mockResolvedValue({
    storeRoot: "C:/Users/me/.ccswitchx",
    theme: "system",
    enabledAgents: null,
    closeToTray: true,
    proxy: { url: null, username: null },
    skillRepos: null,
      backup: { intervalHours: 24, retainCount: 10, git: { enabled: false, autoCommit: true, autoPull: true, remote: null, branch: "main" } },
  });
  vi.mocked(api.setTheme).mockResolvedValue({
    storeRoot: "C:/Users/me/.ccswitchx",
    theme: "dark",
    enabledAgents: null,
    closeToTray: true,
    proxy: { url: null, username: null },
    skillRepos: null,
      backup: { intervalHours: 24, retainCount: 10, git: { enabled: false, autoCommit: true, autoPull: true, remote: null, branch: "main" } },
  });
  vi.mocked(api.setStoreRoot).mockResolvedValue(true);
});

test("changes theme and persists it", async () => {
  render(
    <SettingsPanel
      theme="system"
      onThemeChange={() => {}}
      update={makeUpdateController()}
    />,
  );
  const select = await screen.findByLabelText("主题");
  fireEvent.change(select, { target: { value: "dark" } });
  await waitFor(() => expect(api.setTheme).toHaveBeenCalledWith("dark"));
});

test("changes store root and warns about restart", async () => {
  render(
    <SettingsPanel
      theme="system"
      onThemeChange={() => {}}
      update={makeUpdateController()}
    />,
  );
  const input = await screen.findByLabelText("存储位置");
  fireEvent.change(input, { target: { value: "D:/data/ccswitchx" } });
  fireEvent.click(screen.getByRole("button", { name: "保存存储位置" }));
  await waitFor(() => expect(api.setStoreRoot).toHaveBeenCalledWith("D:/data/ccswitchx"));
  expect(await screen.findByText("存储位置已保存，重启应用后生效。")).toBeInTheDocument();
});
