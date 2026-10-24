import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import App from "../App";

vi.mock("../lib/api", () => ({
  api: {
    getSettings: vi.fn().mockResolvedValue({
      storeRoot: "C:/Users/me/.ccswitchx",
      theme: "system",
      enabledAgents: null,
      closeToTray: true,
      proxy: { url: null, username: null },
      skillRepos: null,
      backup: { intervalHours: 24, retainCount: 10, git: { enabled: false, autoCommit: true, autoPull: true, remote: null, branch: "main" } },
    }),
    listAgents: vi.fn().mockResolvedValue([]),
    listAssets: vi.fn().mockResolvedValue([]),
    sessionProviders: vi.fn().mockResolvedValue([]),
    listSessions: vi.fn().mockResolvedValue([]),
  },
}));

test("renders all sidebar entries and switches panes", () => {
  render(<App />);
  for (const label of [
    "MCP",
    "Skills",
    "提示词",
    "Rules",
    "Subagents",
    "Hooks",
    "会话",
    "Agents",
    "网络",
    "备份",
    "Settings",
  ]) {
    expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
  }
  fireEvent.click(screen.getByRole("button", { name: "Rules" }));
  expect(screen.getByRole("heading", { name: "Rules" })).toBeInTheDocument();
});