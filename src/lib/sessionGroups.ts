import type { SessionSummary } from "../types";

export type SessionTimeBucket = "今天" | "昨天" | "本周" | "更早";

export interface SessionBucket {
  label: SessionTimeBucket;
  sessions: SessionSummary[];
}

export interface SessionGroup {
  project: string;
  latest: number;
  buckets: SessionBucket[];
}

const BUCKET_ORDER: SessionTimeBucket[] = ["今天", "昨天", "本周", "更早"];

/** 按项目归组，再按今天/昨天/本周/更早分桶；空桶不出现。 */
export function groupSessions(
  sessions: SessionSummary[],
  now: Date | number,
): SessionGroup[] {
  const nowDate = typeof now === "number" ? new Date(now) : now;
  const today = startOfDay(nowDate);
  const yesterday = addDays(today, -1);
  const week = addDays(today, -((today.getDay() + 6) % 7));

  const groups = new Map<string, Map<SessionTimeBucket, SessionSummary[]>>();
  for (const session of sessions) {
    const project = session.projectPath?.trim() ? session.projectPath : "未知项目";
    const bucket = bucketFor(session.updatedAt, today, yesterday, week);
    const buckets = groups.get(project) ?? new Map<SessionTimeBucket, SessionSummary[]>();
    const items = buckets.get(bucket) ?? [];
    items.push(session);
    buckets.set(bucket, items);
    groups.set(project, buckets);
  }

  return [...groups.entries()]
    .map(([project, buckets]) => {
      const ordered = BUCKET_ORDER.flatMap((label) => {
        const items = buckets.get(label);
        if (!items || items.length === 0) {
          return [];
        }
        items.sort((left, right) => right.updatedAt - left.updatedAt);
        return [{ label, sessions: items }];
      });
      const latest = Math.max(...ordered.flatMap((bucket) => bucket.sessions.map((item) => item.updatedAt)));
      return { project, latest, buckets: ordered };
    })
    .sort((left, right) => right.latest - left.latest);
}

function bucketFor(
  timestamp: number,
  today: Date,
  yesterday: Date,
  week: Date,
): SessionTimeBucket {
  if (timestamp >= today.getTime()) {
    return "今天";
  }
  if (timestamp >= yesterday.getTime()) {
    return "昨天";
  }
  if (timestamp >= week.getTime()) {
    return "本周";
  }
  return "更早";
}

function startOfDay(date: Date): Date {
  const value = new Date(date);
  value.setHours(0, 0, 0, 0);
  return value;
}

function addDays(date: Date, days: number): Date {
  const value = new Date(date);
  value.setDate(value.getDate() + days);
  return value;
}