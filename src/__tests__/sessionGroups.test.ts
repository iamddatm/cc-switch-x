import { expect, test } from "vitest";
import { groupSessions } from "../lib/sessionGroups";
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