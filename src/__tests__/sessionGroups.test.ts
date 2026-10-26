import { expect, test } from "vitest";
import {
  groupSessions,
  groupSessionsByProject,
  groupSessionsByTime,
  isSessionInProject,
  normalizeProjectPath,
  projectName,
} from "../lib/sessionGroups";
import type { SessionSummary } from "../types";

const NOW = new Date("2026-10-22T12:00:00");

function session(overrides: Partial<SessionSummary>): SessionSummary {
  return {
    agentId: "claude",
    sessionId: "s",
    title: "标题",
    projectPath: "D:/repo",
    sourcePath: "D:/repo/s.jsonl",
    createdAt: null,
    updatedAt: new Date("2026-10-22T10:00:00").getTime(),
    messageCount: 1,
    parseError: null,
    ...overrides,
  };
}

test("sessions_are_grouped_by_project_and_time_then_sorted_by_recent_activity", () => {
  const groups = groupSessions(
    [
      session({
        sessionId: "a-old",
        title: "A 昨天",
        projectPath: "D:/repo-a",
        updatedAt: new Date("2026-10-21T09:00:00").getTime(),
      }),
      session({
        sessionId: "a-new",
        title: "A 今天",
        projectPath: "D:/repo-a",
        updatedAt: new Date("2026-10-22T11:00:00").getTime(),
      }),
      session({
        sessionId: "b-old",
        title: "B 更早",
        projectPath: "D:/repo-b",
        updatedAt: new Date("2026-09-01T10:00:00").getTime(),
      }),
    ],
    NOW,
  );

  expect(groups.map((group) => group.project)).toEqual(["D:/repo-a", "D:/repo-b"]);
  expect(groups[0].buckets.map((bucket) => bucket.label)).toEqual(["今天", "昨天"]);
  expect(groups[0].buckets[0].sessions.map((item) => item.title)).toEqual(["A 今天"]);
  expect(groups[0].buckets[1].sessions.map((item) => item.title)).toEqual(["A 昨天"]);
  expect(groups[1].buckets.map((bucket) => bucket.label)).toEqual(["更早"]);
});

test("missing_project_and_this_week_bucket_are_handled", () => {
  const groups = groupSessions(
    [
      session({
        sessionId: "unknown",
        title: "未知项目会话",
        projectPath: null,
        updatedAt: new Date("2026-10-20T10:00:00").getTime(),
      }),
    ],
    NOW,
  );

  expect(groups).toHaveLength(1);
  expect(groups[0].project).toBe("未知项目");
  expect(groups[0].buckets.map((bucket) => bucket.label)).toEqual(["本周"]);
  expect(groups[0].buckets[0].sessions[0].title).toBe("未知项目会话");
});

test("project_view_keeps_all_sessions_of_a_project_together", () => {
  const groups = groupSessionsByProject([
    session({ title: "A 早", projectPath: "D:/repo-a", updatedAt: 1 }),
    session({ title: "B 新", projectPath: "D:/repo-b", updatedAt: 4 }),
    session({ title: "A 新", projectPath: "D:/repo-a", updatedAt: 3 }),
  ]);

  expect(groups.map((group) => projectName(group.project))).toEqual(["repo-b", "repo-a"]);
  expect(groups[1].sessions.map((item) => item.title)).toEqual(["A 新", "A 早"]);
});

test("time_view_returns_only_non_empty_buckets_in_order", () => {
  const groups = groupSessionsByTime(
    [
      session({ title: "今天", updatedAt: new Date("2026-10-22T11:00:00").getTime() }),
      session({ title: "更早", updatedAt: new Date("2026-09-01T10:00:00").getTime() }),
    ],
    NOW,
  );
  expect(groups.map((group) => group.label)).toEqual(["今天", "更早"]);
});

test("sessions_inside_one_bucket_are_sorted_by_updated_at_desc", () => {
  const groups = groupSessions(
    [
      session({ sessionId: "early", title: "早", updatedAt: new Date("2026-10-22T08:00:00").getTime() }),
      session({ sessionId: "late", title: "晚", updatedAt: new Date("2026-10-22T09:00:00").getTime() }),
    ],
    NOW,
  );

  expect(groups[0].buckets[0].sessions.map((item) => item.title)).toEqual(["晚", "早"]);
});

test("normalize_project_path_unifies_separators_case_and_trailing_slash", () => {
  expect(normalizeProjectPath("D:\\repos\\ai\\PddWorker")).toBe("d:/repos/ai/pddworker");
  expect(normalizeProjectPath("D:/repos/ai/PddWorker/")).toBe("d:/repos/ai/pddworker");
  expect(normalizeProjectPath("  D:\\repos\\\\ai\\PddWorker  ")).toBe("d:/repos/ai/pddworker");
  expect(normalizeProjectPath("D:\\")).toBe("d:");
  expect(normalizeProjectPath("/")).toBe("/");
  expect(normalizeProjectPath("")).toBe("");
});

test("session_belongs_to_the_project_root_or_any_subdirectory", () => {
  const root = "D:\\repos\\ai\\PddWorker";
  expect(isSessionInProject(session({ projectPath: "D:/repos/ai/PddWorker" }), root)).toBe(true);
  expect(isSessionInProject(session({ projectPath: "d:\\repos\\ai\\pddworker\\" }), root)).toBe(true);
  expect(isSessionInProject(session({ projectPath: "D:\\repos\\ai\\PddWorker\\web\\app" }), root)).toBe(
    true,
  );
});

test("sibling_and_unattributed_sessions_do_not_belong_to_the_project", () => {
  const root = "D:\\repos\\ai\\PddWorker";
  expect(isSessionInProject(session({ projectPath: "D:\\repos\\ai\\PddWorkerX" }), root)).toBe(false);
  expect(isSessionInProject(session({ projectPath: "D:\\repos\\ai" }), root)).toBe(false);
  expect(isSessionInProject(session({ projectPath: null }), root)).toBe(false);
  expect(isSessionInProject(session({ projectPath: "   " }), root)).toBe(false);
  expect(isSessionInProject(session({ projectPath: "D:\\repos\\ai\\PddWorker" }), "")).toBe(false);
});