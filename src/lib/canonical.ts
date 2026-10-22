/**
 * 三类真源文件的最小编解码器。真源是文本文件（唯一真相），面板要显示成表单就必须能
 * 读它、也要能写回它，所以这里只支持**我们自己产出的那一小撮 YAML**：
 * 顶层扁平键、`paths:` / `tools:` 是 `- item` 块序列、字符串值可带引号。
 *
 * 见到读不懂的行就抛错（面板把它显示出来），不猜、也不静默丢字段 ——
 * 静默丢字段等于把用户真源里的内容抹掉。后端在写盘前还会再校验一遍。
 */

export interface RuleForm {
  name: string;
  description: string;
  paths: string[];
  /** 留空表示不写这一行，由后端按 paths 派生 */
  trigger: string;
  body: string;
}

export interface SubagentForm {
  name: string;
  description: string;
  instructions: string;
  model: string;
  tools: string[];
  /** 部署目标；空数组表示不限定 */
  targets: string[];
  /** 工具私有字段原样带着，编辑别的 agent 时不能把它丢掉 */
  toolExtrasRaw: string;
}

export interface HookForm {
  name: string;
  event: string;
  matcher: string;
  command: string;
  commandWindows: string;
  timeout: string;
}

type Entry = string | string[];

/**
 * 扁平 YAML 的子集：顶层 `key: value`、`key:` 后面跟一块内容 ——
 * 全是 `- item` 就当序列（serde_yaml 把块序列写在第 0 列，也接受缩进写法），
 * 否则当块标量（去掉缩进后原样返回文本）。
 * `instructions` 与 `tool_extras` 都靠这条规则带回来，不需要各自的解析器。
 */
function readYaml(text: string, kind: string): Map<string, Entry> {
  const out = new Map<string, Entry>();
  const lines = text.replace(/^\uFEFF/, "").split(/\r?\n/);
  const isBlockLine = (line: string) => /^\s+\S/.test(line) || /^-\s/.test(line);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (!line.trim()) continue;
    const pair = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
    if (!pair) throw new Error(`${kind} 里读不懂的一行：${line}`);
    const key = pair[1];
    const inline = pair[2].trim();
    // `|-` 这类块标量指示符不是值本身，跳过它去读下面的缩进块
    if (inline && !/^[|>][-+]?$/.test(inline)) {
      out.set(key, unquote(inline));
      continue;
    }
    const block: string[] = [];
    while (index + 1 < lines.length && (isBlockLine(lines[index + 1]) || !lines[index + 1].trim())) {
      block.push(lines[index + 1]);
      index += 1;
    }
    const dedented = dedent(block);
    if (!dedented) {
      out.set(key, "");
    } else if (dedented.split("\n").every((entry) => entry.startsWith("- "))) {
      out.set(key, dedented.split("\n").map((entry) => unquote(entry.slice(2).trim())));
    } else {
      out.set(key, dedented);
    }
  }
  return out;
}

function scalar(entry: Map<string, Entry>, key: string): string {
  const value = entry.get(key);
  return typeof value === "string" ? value : "";
}

function sequence(entry: Map<string, Entry>, key: string): string[] {
  const value = entry.get(key);
  return Array.isArray(value) ? value : [];
}

function unquote(value: string): string {
  const trimmed = value.trim();
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

/** 只有会被 YAML 误读的写法才加引号；中文这类普通字符保持原样，diff 才读得下去。 */
function quoteIfNeeded(value: string): string {
  if (value === "") return '""';
  const needsQuotes =
    /^[\s>|@`"'%#\[\]{}&*!?:,\-]/.test(value) ||
    /\s$/.test(value)
    || /: /.test(value)
    || / #/.test(value)
    || value.includes("\n");
  return needsQuotes ? JSON.stringify(value) : value;
}

/** 去掉块内容的共同缩进（不是全部缩进）：嵌套层的相对缩进要保住。 */
function dedent(block: string[]): string {
  const indented = block.filter((line) => line.trim());
  if (indented.length === 0) return "";
  const width = Math.min(
    ...indented.map((line) => line.length - line.replace(/^\s+/, "").length),
  );
  return block
    .map((line) => line.slice(Math.min(width, line.length - line.replace(/^\s+/, "").length)))
    .join("\n")
    .trimEnd();
}

export function parseRule(text: string): RuleForm {
  const [frontmatter, body] = splitMarkdown(text);
  const entry = readYaml(frontmatter, "规则");
  return {
    name: scalar(entry, "name"),
    description: scalar(entry, "description"),
    paths: sequence(entry, "paths"),
    trigger: scalar(entry, "trigger"),
    body,
  };
}

export function renderRule(form: RuleForm): string {
  const lines = [`name: ${quoteIfNeeded(form.name)}`];
  if (form.description) lines.push(`description: ${quoteIfNeeded(form.description)}`);
  if (form.paths.length > 0) {
    lines.push("paths:");
    for (const path of form.paths) lines.push(`- ${quoteIfNeeded(path)}`);
  }
  if (form.trigger) lines.push(`trigger: ${form.trigger}`);
  return `---\n${lines.join("\n")}\n---\n\n${form.body.replace(/\n+$/, "")}\n`;
}

export function parseSubagent(text: string): SubagentForm {
  const entry = readYaml(text, "子代理");
  const extras = scalar(entry, "tool_extras");
  return {
    name: scalar(entry, "name"),
    description: scalar(entry, "description"),
    instructions: scalar(entry, "instructions"),
    model: scalar(entry, "model"),
    tools: sequence(entry, "tools"),
    targets: sequence(entry, "targets"),
    // `tool_extras: {}` 这种空写法回来时是字面量 {}，等于没有私有字段
    toolExtrasRaw: extras === "{}" ? "" : extras,
  };
}

/**
 * 子代理真源是 YAML，正文里多行文本走 `|-` 块标量 —— 这样改一行提示词在 diff 里
 * 也只动那一行，而不是整块变成一个转义长行。
 */
export function renderSubagent(form: SubagentForm): string {
  const lines = [`name: ${quoteIfNeeded(form.name)}`, `description: ${quoteIfNeeded(form.description)}`];
  lines.push("instructions: |-");
  for (const line of form.instructions.replace(/\n+$/, "").split("\n")) {
    lines.push(`  ${line}`);
  }
  if (form.model) lines.push(`model: ${quoteIfNeeded(form.model)}`);
  if (form.tools.length > 0) {
    lines.push("tools:");
    for (const tool of form.tools) lines.push(`- ${quoteIfNeeded(tool)}`);
  } else {
    lines.push("tools: []");
  }
  const extras = form.toolExtrasRaw.trim();
  if (extras.startsWith("{") || extras.startsWith("[")) {
    lines.push(`tool_extras: ${extras}`);
  } else if (extras) {
    lines.push("tool_extras:");
    for (const line of extras.split("\n")) lines.push(`  ${line}`);
  } else {
    lines.push("tool_extras: {}");
  }
  if (form.targets.length > 0) {
    lines.push("targets:");
    for (const target of form.targets) lines.push(`- ${quoteIfNeeded(target)}`);
  } else {
    lines.push("targets: []");
  }
  return `${lines.join("\n")}\n`;
}

export function parseHook(text: string): HookForm {
  const entry = readYaml(text, "hook");
  return {
    name: scalar(entry, "name"),
    event: scalar(entry, "event"),
    matcher: scalar(entry, "matcher"),
    command: scalar(entry, "command"),
    commandWindows: scalar(entry, "command_windows"),
    timeout: scalar(entry, "timeout"),
  };
}

export function renderHook(form: HookForm): string {
  const lines = [
    `name: ${quoteIfNeeded(form.name)}`,
    `event: ${form.event}`,
    `matcher: ${quoteIfNeeded(form.matcher)}`,
    `command: ${quoteIfNeeded(form.command)}`,
  ];
  if (form.commandWindows) lines.push(`command_windows: ${quoteIfNeeded(form.commandWindows)}`);
  else lines.push("command_windows: null");
  if (form.timeout) lines.push(`timeout: ${Number(form.timeout)}`);
  return `${lines.join("\n")}\n`;
}

/** markdown 文件切成 frontmatter 与正文；没有 frontmatter 时整份当正文。 */
function splitMarkdown(text: string): [string, string] {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text.replace(/^\uFEFF/, ""));
  if (!match) {
    throw new Error("规则缺少 frontmatter（--- 包起来的那段）");
  }
  return [match[1], match[2].replace(/^\n+/, "")];
}

/** 逗号分隔的输入变成列表（rules 的 paths、subagents 的 tools 都这么录入）。 */
export function toList(value: string): string[] {
  return value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

export function fromList(values: string[]): string {
  return values.join(", ");
}
