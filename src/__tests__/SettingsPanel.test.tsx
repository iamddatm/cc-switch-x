import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { SettingsPanel } from "../panels/SettingsPanel";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    getSettings: vi.fn(),
    setTheme: vi.fn(),
    setStoreRoot: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSettings).mockResolvedValue({
    storeRoot: "C:/Users/me/.ccswitchx",
    theme: "system",
    enabledAgents: null,
  });
  vi.mocked(api.setTheme).mockResolvedValue({
    storeRoot: "C:/Users/me/.ccswitchx",
    theme: "dark",
    enabledAgents: null,
  });
  vi.mocked(api.setStoreRoot).mockResolvedValue(true);
});

test("changes theme and persists it", async () => {
  render(<SettingsPanel theme="system" onThemeChange={() => {}} />);
  const select = await screen.findByLabelText("主题");
  fireEvent.change(select, { target: { value: "dark" } });
  await waitFor(() => expect(api.setTheme).toHaveBeenCalledWith("dark"));
});

test("changes store root and warns about restart", async () => {
  render(<SettingsPanel theme="system" onThemeChange={() => {}} />);
  const input = await screen.findByLabelText("存储位置");
  fireEvent.change(input, { target: { value: "D:/data/ccswitchx" } });
  fireEvent.click(screen.getByRole("button", { name: "保存存储位置" }));
  await waitFor(() => expect(api.setStoreRoot).toHaveBeenCalledWith("D:/data/ccswitchx"));
  expect(await screen.findByText("存储位置已保存，重启应用后生效。")).toBeInTheDocument();
});
