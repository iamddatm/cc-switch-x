# 第三方声明

本仓库不打包、不链接、也不复制下列项目的源码文件；只借用其中若干**机制与格式约定**。
每个来源按其原始许可证保留版权声明如下。若本文件与来源仓库的 `LICENSE` 冲突，以来源仓库为准。

## 借用清单

| 来源 | 许可证 | 借用的机制 | 本仓库对应位置 |
|---|---|---|---|
| [teamai-cli](https://github.com/Tencent/teamai-cli) | MIT（Tencent） | ① 规则文件的 `RuleFormat` 三段式（render / bodyEquals / mergeBodyIntoTeam + `previousRenders` 认旧渲染）；② 子代理的中间 `AgentSpec` 与 `tool_extras.<tool>` 反向映射；③ hook 条目用 `description` 前缀作归属标记；④ Codex agent TOML 用 `'''` 多行字面量保持可读；⑤ 各工具 rules/agents 落点是否存在的证据（`LEGACY_RULE_DIRS`、`agentFileExtensionForTool`） | `src-tauri/src/domain/agent.rs` 的能力表与 `adapters/ledger.rs` 的归属台账；①②③④ 落在 Plan 3 Task 2–7 的 `adapters/{rules,subagents,hooks}.rs` |
| [cc-switch](https://github.com/farion1231/cc-switch) | MIT | ① 本地库 `tools` 表与 `enabled_<app>` 列的读取映射（导入器只读，不写它的库）；② 「每工具一个适配器模块」的分层思路；③ 会话 provider 的路径与字段映射（11 类工具的会话文件位置、字段形状与恢复命令）；④ WebDAV MKCOL / PUT / MOVE / HEAD / GET 与 Basic Auth 传输思路 | ① `src-tauri/src/import/{source,mcp,prompts,skills}.rs`；② 各 `adapters/*.rs`；③ `src-tauri/src/sessions/providers/`；④ `src-tauri/src/net/webdav.rs` |
| [vercel-labs/skills](https://github.com/vercel-labs/skills) | MIT（Vercel, Inc.） | `SKILL.md` 的目录布局与 frontmatter 字段约定（skills 作为整目录资产分发） | `src-tauri/src/adapters/skills.rs`、`import/skills.rs` |

上述借用均为**机制与公开格式**，不含其代码文件；本仓库的实现独立编写，命名与代码结构按本项目约定。

---

## teamai-cli

```
Tencent is pleased to support the open source community by making teamai-cli available.

Copyright (C) 2026 Tencent. All rights reserved.

teamai-cli is licensed under MIT. teamai-cli does not impose any additional restrictions beyond those specified in the license.

Terms of the MIT License:
--------------------------------------------------------------------
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice (including the next paragraph) shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## cc-switch

```
MIT License

Copyright (c) 2025 Jason Young

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## vercel-labs/skills

```
MIT License

Copyright (c) 2026 Vercel, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
