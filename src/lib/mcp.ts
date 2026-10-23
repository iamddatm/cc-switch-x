export type McpTransport = "stdio" | "http" | "sse" | "ws";

export interface McpForm {
  id: string;
  name: string;
  transport: McpTransport;
  command: string;
  args: string[];
  env: Record<string, string>;
  url: string;
  headers: Record<string, string>;
  description: string;
  homepage: string;
  docs: string;
  tags: string[];
}

const TRANSPORTS: McpTransport[] = ["stdio", "http", "sse", "ws"];

export function emptyMcpForm(): McpForm {
  return {
    id: "",
    name: "",
    transport: "stdio",
    command: "",
    args: [],
    env: {},
    url: "",
    headers: {},
    description: "",
    homepage: "",
    docs: "",
    tags: [],
  };
}

export function parseMcp(text: string): McpForm {
  let raw: Record<string, unknown>;
  try {
    raw = JSON.parse(text) as Record<string, unknown>;
  } catch {
    throw new Error("JSON 无效");
  }
  const transport = String(raw.transport ?? "stdio") as McpTransport;
  if (!TRANSPORTS.includes(transport)) {
    throw new Error(`未知的传输方式：${String(raw.transport)}`);
  }
  return {
    id: String(raw.id ?? ""),
    name: String(raw.name ?? ""),
    transport,
    command: typeof raw.command === "string" ? raw.command : "",
    args: Array.isArray(raw.args) ? raw.args.filter((arg): arg is string => typeof arg === "string") : [],
    env: recordOf(raw.env),
    url: typeof raw.url === "string" ? raw.url : "",
    headers: recordOf(raw.headers),
    description: typeof raw.description === "string" ? raw.description : "",
    homepage: typeof raw.homepage === "string" ? raw.homepage : "",
    docs: typeof raw.docs === "string" ? raw.docs : "",
    tags: Array.isArray(raw.tags) ? raw.tags.filter((tag): tag is string => typeof tag === "string") : [],
  };
}

/** 只写后端 canonical 认识且非空的字段；空字符串/空对象不写，保持真源干净。 */
export function renderMcp(form: McpForm): string {
  const output: Record<string, unknown> = {
    id: form.id,
    name: form.name,
    transport: form.transport,
  };
  if (form.command.trim()) output.command = form.command.trim();
  output.args = form.args;
  if (Object.keys(form.env).length > 0) output.env = form.env;
  if (form.url.trim()) output.url = form.url.trim();
  if (Object.keys(form.headers).length > 0) output.headers = form.headers;
  if (form.description.trim()) output.description = form.description.trim();
  if (form.homepage.trim()) output.homepage = form.homepage.trim();
  if (form.docs.trim()) output.docs = form.docs.trim();
  if (form.tags.length > 0) output.tags = form.tags;
  return `${JSON.stringify(output, null, 2)}\n`;
}

export function validateMcp(form: McpForm): string | null {
  if (!/^[A-Za-z0-9._-]+$/.test(form.id.trim()) || form.id.trim() === "." || form.id.trim() === "..") {
    return "ID 只能包含字母、数字、点、下划线和短横线";
  }
  if (!form.name.trim()) return "名称不能为空";
  if (form.transport === "stdio" && !form.command.trim()) {
    return "stdio 必须填写命令";
  }
  if (form.transport !== "stdio" && !form.url.trim()) {
    return "HTTP / SSE / WS 必须填写 URL";
  }
  return null;
}

export function linesToRecord(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const index = trimmed.indexOf("=");
    if (index <= 0) continue;
    out[trimmed.slice(0, index).trim()] = trimmed.slice(index + 1).trim();
  }
  return out;
}

export function recordToLines(record: Record<string, string>): string {
  return Object.entries(record)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}

function recordOf(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out: Record<string, string> = {};
  for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
    if (typeof item === "string") out[key] = item;
  }
  return out;
}