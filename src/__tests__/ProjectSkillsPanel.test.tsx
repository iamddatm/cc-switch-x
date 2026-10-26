import { render, screen } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { ProjectSkillsPanel } from "../panels/ProjectSkillsPanel";
import { api } from "../lib/api";
import type { ProjectView } from "../types";

vi.mock("../lib/api", () => ({
  api: {
    listProjectSkills: vi.fn(),
    revealProjectFolder: vi.fn(),
  },
}));

function projectView(): ProjectView {
  return {
    id: "demo-abc123",
    path: "C:/work/demo",
    name: "demo",
    addedAt: "2026-10-01T00:00:00Z",
    lastOpenedAt: "2026-10-02T00:00:00Z",
    autoSync: true,
    exists: true,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

test("renders project skill matrix with per-directory state", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project: projectView(),
    dirs: [
      {
        key: "claude",
        label: "Claude Code",
        relativePath: ".claude/skills",
        agents: ["claude"],
        verified: true,
      },
      {
        key: "qoder",
        label: "Qoder",
        relativePath: ".qoder/skills",
        agents: ["qoder"],
        verified: true,
      },
    ],
    skills: [
      {
        id: "code-review",
        name: "code-review",
        description: "审查代码",
        targets: [],
        dirs: {
          claude: {
            key: "claude",
            label: "Claude Code",
            relativePath: ".claude/skills",
            agents: ["claude"],
            verified: true,
            state: "present",
          },
          qoder: {
            key: "qoder",
            label: "Qoder",
            relativePath: ".qoder/skills",
            agents: ["qoder"],
            verified: true,
            state: "absent",
          },
        },
        source: null,
        lastOp: null,
      },
    ],
    conflictCount: 0,
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("code-review")).toBeInTheDocument();
  expect(screen.getByText("有")).toBeInTheDocument();
  expect(screen.getByText("无")).toBeInTheDocument();
});

test("shows empty state when project has no skills", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project: projectView(),
    dirs: [],
    skills: [],
    conflictCount: 0,
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("项目里还没有 Skills")).toBeInTheDocument();
});
