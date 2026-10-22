import { describe, expect, it } from "vitest";
import {
  fromList,
  parseHook,
  parseRule,
  parseSubagent,
  renderHook,
  renderRule,
  renderSubagent,
  toList,
} from "../lib/canonical";

describe("rules 编解码", () => {
  it("读写一份带 paths 的规则", () => {
    const text =
      "---\nname: style\ndescription: 写法\npaths:\n- src/**\n- docs/**\n---\n\n正文第一行\n正文第二行\n";
    const form = parseRule(text);
    expect(form.name).toBe("style");
    expect(form.paths).toEqual(["src/**", "docs/**"]);
    expect(form.trigger).toBe("");
    expect(form.body).toBe("正文第一行\n正文第二行\n");
    expect(renderRule(form)).toBe(text);
  });

  it("显式 trigger 原样带着，不写 paths 时不产生空列表", () => {
    const rendered = renderRule({
      name: "a",
      description: "",
      paths: [],
      trigger: "manual",
      body: "正文",
    });
    expect(rendered).toBe("---\nname: a\ntrigger: manual\n---\n\n正文\n");
    expect(rendered).not.toContain("paths:");
  });

  it("含冒号的值加引号，读回来还是同一个值", () => {
    const form = {
      name: "a",
      description: "范围: 全部",
      paths: ["src/**"],
      trigger: "",
      body: "正文",
    };
    const rendered = renderRule(form);
    expect(rendered).toContain('description: "范围: 全部"');
    expect(parseRule(rendered).description).toBe("范围: 全部");
  });

  it("没有 frontmatter 时报错而不是当成空规则", () => {
    expect(() => parseRule("只有正文")).toThrow(/frontmatter/);
  });
});

describe("subagents 编解码", () => {
  const text = [
    "name: reviewer",
    "description: 代码审查",
    "instructions: |-",
    "  先看 diff",
    "  再看测试",
    "model: sonnet",
    "tools:",
    "- Read",
    "- Grep",
    "tool_extras:",
    "  claude:",
    "    effort: high",
    "targets: []",
    "",
  ].join("\n");

  it("多行指令走块标量，diff 里只动改过的那一行", () => {
    const form = parseSubagent(text);
    expect(form.instructions).toBe("先看 diff\n再看测试");
    expect(form.tools).toEqual(["Read", "Grep"]);
    expect(form.model).toBe("sonnet");
    // 私有字段整块原样保留，编辑别的东西不能把它丢掉
    expect(form.toolExtrasRaw).toBe("claude:\n  effort: high");
    expect(renderSubagent(form)).toBe(text);
  });

  it("空的 tool_extras 写成 {}，读回来还是空", () => {
    const form = parseSubagent("name: a\ndescription: b\ninstructions: c\ntools: []\ntargets: []\n");
    expect(form.toolExtrasRaw).toBe("");
    expect(renderSubagent(form)).toContain("tool_extras: {}");
  });

  it("逗号输入变成列表", () => {
    expect(toList("Read, Grep ,")).toEqual(["Read", "Grep"]);
    expect(fromList(["Read", "Grep"])).toBe("Read, Grep");
  });
});

describe("hooks 编解码", () => {
  it("Windows 命令留空时写 null，而不是省掉这个键", () => {
    const rendered = renderHook({
      name: "fmt",
      event: "PreToolUse",
      matcher: "Bash",
      command: "cargo fmt",
      commandWindows: "",
      timeout: "30",
    });
    expect(rendered).toContain("command_windows: null");
    expect(rendered).toContain("timeout: 30");
    const back = parseHook(rendered);
    expect(back.command).toBe("cargo fmt");
    expect(back.event).toBe("PreToolUse");
    expect(back.timeout).toBe("30");
  });

  it("读回后端产出的 YAML", () => {
    const form = parseHook(
      'name: fmt\nevent: "SessionStart"\nmatcher: ""\ncommand: "pwsh -NoProfile -c \'a b\'"\ntimeout: 5\n',
    );
    expect(form.event).toBe("SessionStart");
    expect(form.commandWindows).toBe("");
    expect(form.timeout).toBe("5");
  });

  // 真机跑出来的缺陷：`command_windows: null` 被读成字符串 "null"，
  // 于是面板把 "[Windows] null" 当成一条真实命令显示出来。
  it("YAML 的 null 读成空值，不是字符串 null", () => {
    expect(parseHook("command_windows: null\n").commandWindows).toBe("");
    expect(parseSubagent("model: null\n").model).toBe("");
    const form = parseHook("name: a\nevent: Stop\ncommand: x\ncommand_windows: null\n");
    expect(renderHook(form)).toContain("command_windows: null");
    expect(renderHook(form)).not.toContain('"null"');
  });
});
