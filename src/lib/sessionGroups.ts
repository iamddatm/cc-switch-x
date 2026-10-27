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

export interface ProjectGroup {
  project: string;
  latest: number;
  sessions: SessionSummary[];
}

export interface TimeGroup {
  label: SessionTimeBucket;
  sessions: SessionSummary[];
}

/** 按项目分组并保持「最近活跃在前」；项目名取末尾目录。 */
export function groupSessionsByProject(sessions: SessionSummary[]): ProjectGroup[] {
  const groups = new Map<string, SessionSummary[]>();
  for (const session of sessions) {
    const project = projectOf(session);
    const items = groups.get(project) ?? [];
    items.push(session);
    groups.set(project, items);
  }
  return [...groups.entries()]
    .map(([project, items]) => {
      items.sort((left, right) => right.updatedAt - left.updatedAt);
      return { project, latest: items[0]?.updatedAt ?? 0, sessions: items };
    })
    .sort((left, right) => right.latest - left.latest);
}

/** 今天 / 昨天 / 本周 / 更早四组；空组不出现。 */
export function groupSessionsByTime(
  sessions: SessionSummary[],
  now: Date | number,
): TimeGroup[] {
  const nowDate = typeof now === "number" ? new Date(now) : now;
  const today = startOfDay(nowDate);
  const yesterday = addDays(today, -1);
  const week = addDays(today, -((today.getDay() + 6) % 7));
  const groups = new Map<SessionTimeBucket, SessionSummary[]>();
  for (const session of sessions) {
    const bucket = bucketFor(session.updatedAt, today, yesterday, week);
    const items = groups.get(bucket) ?? [];
    items.push(session);
    groups.set(bucket, items);
  }
  return BUCKET_ORDER.flatMap((label) => {
    const items = groups.get(label);
    if (!items || items.length === 0) return [];
    items.sort((left, right) => right.updatedAt - left.updatedAt);
    return [{ label, sessions: items }];
  });
}

export function projectOf(session: SessionSummary): string {
  return session.projectPath?.trim() ? session.projectPath : "未知项目";
}

/**
 * 项目路径归一化，用于跨来源比较。
 *
 * 注册表里存的是 Rust `PathBuf` 的 Windows 可读写法（`D:\repos\ai\App`），而会话的
 * `projectPath` 是各 provider 从会话文件的 `cwd` / `directory` 原样取来的字符串，
 * 分隔符与盘符大小写都不保证一致；Windows 路径本身大小写不敏感，因此统一成
 * 正斜杠 + 小写 + 无尾部斜杠后再比。长度 > 1 才去尾部斜杠，避免把根 `/` 变成空串。
 */
export function normalizeProjectPath(path: string): string {
  const collapsed = path.trim().replace(/[\\/]+/g, "/").toLowerCase();
  return collapsed.length > 1 ? collapsed.replace(/\/+$/, "") : collapsed;
}

/** 会话是否属于该项目：项目根本身或其下任意子目录；缺少 cwd 的会话不属于任何项目。 */
export function isSessionInProject(session: SessionSummary, projectPath: string): boolean {
  const root = normalizeProjectPath(projectPath);
  if (!root || !session.projectPath?.trim()) return false;
  const cwd = normalizeProjectPath(session.projectPath);
  return Boolean(cwd) && (cwd === root || cwd.startsWith(`${root}/`));
}

/** 路径末尾的目录名；「未知项目」原样返回。 */
export function projectName(project: string): string {
  if (project === "未知项目") return project;
  const parts = project.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? project;
}

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