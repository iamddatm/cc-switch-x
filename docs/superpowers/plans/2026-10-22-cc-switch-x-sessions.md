# CC Switch X 历史对话（本地浏览 + 恢复）实现计划（Plan 4/5）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把各 agent 的原生会话文件解析成统一模型并写入本地派生索引，提供按项目/时间分组的列表、跨会话全文搜索、结构化阅读页、会话内查找、Markdown 导出与恢复命令；全程只读，不复制会话内容，不进入同步。

**Architecture:** 沿用 Plan 1–3 的分层，新增：
- `domain/session.rs`：统一的会话/消息模型与 SQLite 字符串映射。
- `sessions/`：provider 注册表 + 每个工具一个纯解析器（`fn(&Path) -> Result<ParsedSession>`），发现规则与恢复命令模板都记在注册表里。
- `index/sessions.rs` + `schema.rs` 的会话表与 FTS5：索引只从原生文件派生，删掉 `index.db` 可重建。
- `services/sessions.rs`：增量扫描（mtime + size 指纹）、失效清理、搜索编排；`commands/sessions.rs` 是对外命令面。

**Tech Stack:** 在 Plan 3 之上新增 `tauri-plugin-dialog`（导出时选保存位置）与前端 `@tauri-apps/plugin-dialog`、`zstd`（读取 Codex `.jsonl.zst`）；全文搜索用 `rusqlite` bundled 自带的 FTS5（`tokenize='trigram'`，CJK 子串可搜）。

**Spec:** `docs/superpowers/specs/2026-10-22-cc-switch-x-design.md`（覆盖 3.2 的 Sessions 面板、4.4 的会话索引表、5.2 的会话覆盖、9 的 `sessions` 模块、M3 验收、C1/C2/C3）

**许可**：各 provider 的**格式事实**参考 `references/cc-switch`（MIT, `src-tauri/src/session_manager/providers/*.rs`）。只借格式与解析规则，不复制其代码文件；本计划最后更新 `THIRD_PARTY_NOTICES.md` 中 cc-switch 的借用清单。

---

## 事实来源与置信度

下表来自对 `D:/repos/1github/cc-switch`（MIT）的只读检查。引用路径省略前缀
`src-tauri/src/session_manager/`。**它是格式事实的来源，不是可直接复制的代码**：
本计划只借字段映射与路径规则，实现按本项目模型重写。

| 事实 | 出处 | 置信度 |
|---|---|---|
| Claude 会话在 `<config>/projects/**/*.jsonl`；跳过 `agent-*` 与 `journal.jsonl`；`isSidechain`/`isMeta` 行丢弃 | `providers/claude.rs:38-44,221-226,351-385` | 代码级 |
| Claude 行类型 `system/user/assistant`；块 `text/thinking/redacted_thinking/tool_use/server_tool_use/tool_result/image`；工具结果按 `tool_use_id` 配对 | `providers/claude.rs:423-655,728-844` | 代码级 |
| Claude 标题在重复出现的 `type=ai-title` 的 `aiTitle`（取最后一次）；旧版 `custom-title` 作回退；再退首条真实用户文本 / 项目目录 basename。`sessionId` 取记录或文件名 stem；`cwd` 为项目目录；坏行跳过 | 本机 291 个主会话样本；`providers/claude.rs:95-232,280-288` | 高 |
| Claude 恢复命令 `claude --resume {id}`；删除时连同 `<stem>/` 附属目录（`subagents/`、`tool-results/`）一起处理 | `providers/claude.rs:46-79,217` | 代码级 |
| Codex 会话在 `<config>/sessions` 与 `archived_sessions`，递归 `*.jsonl`（另有 `.jsonl.zst` 压缩形态）；同会话两种形态只留普通 `.jsonl` | `providers/codex.rs:62-83,2176-2194`；`codex_rollout_file.rs:27-61,123-133` | 代码级 |
| Codex 行类型 `response_item/event_msg/turn_context/token_usage_record/compacted/session_meta`；`session_meta.payload` 提供 `id/cwd`；`message.role` 的 `developer/system` 视为注入的 system | `providers/codex.rs:615-776,904-935` | 代码级 |
| Codex 标题优先级 `event_msg.thread_name_updated.thread_name` > 首条用户消息 > 目录 basename（v1 不读 state.db/session_index）；子代理会话（`source.subagent`）跳过；长会话中途重复 `session_meta`，只第一份有效 | 本机 27 个样本；`providers/codex.rs:99-229,1989-2095` | 高 |
| Codex 恢复命令 `codex resume {id}` | `providers/codex.rs:2086` | 代码级 |
| Gemini 会话在 `<dir>/tmp/<project_hash>/chats/session-*.json` 或 `.jsonl`；`.jsonl` 优先，同名 `.json` 隐藏 | `providers/gemini.rs:19-73` | 代码级 |
| Gemini `.jsonl` 需按 CLI 的 `loadConversationRecord` 回放：`sessionId` 行是元数据，字符串 `id` 行原位覆盖，`$set` 合并，`$set.messages` 整体替换，`$rewindTo` 截断；消息 `type` 为 `user/gemini/info/error` | `providers/gemini.rs:75-239` | 代码级 |
| Gemini 标题取首条非注入用户消息；`startTime/lastUpdated` 为时间；项目目录来自 `tmp/<project>/.project_root` 文件内容 | `providers/gemini.rs:45-57,514-547` | 代码级 |
| Gemini 恢复命令 `gemini --resume {id}` | `providers/gemini.rs:545` | 代码级 |
| Pi 会话在 `~/.pi/agent/sessions/<project>/<file>.jsonl`（默认深度 2）；首行必须是 `type=session` 头（`id/cwd/timestamp`）；文件上限 128MB | `providers/pi.rs:18-20,515-539,641-720` | 代码级 |
| Pi 后续行 `type=message/model_change/thinking_level_change/compaction/branch_summary/custom_message`；消息角色 `user/assistant/toolResult/bashExecution/branchSummary/compactionSummary`；树形分支只输出活动祖先链 | `providers/pi.rs:368-513`；`providers/pi_blocks.rs:37-116` | 代码级 |
| Pi 恢复命令是 `pi --session <绝对文件路径>`（不是会话 id）；标题 `session_info.name` > 首条用户消息 > `cwd` basename | `providers/pi.rs:328-366` | 代码级 |
| Qoder CN 会话位置按 spec 为 `~/.qoder-cn/projects/<项目>/<uuid>.jsonl`；本机已确认与 Claude 同构，另有 `state.json`/SQLite/CLI task 等存储（v1 不读） | spec 5.2 第 294 行；本机 44 个主会话样本 | 高 |
| OpenCode 数据根 `XDG_DATA_HOME/opencode` 或 `~/.local/share/opencode`；优先 `opencode.db`（V1 `session/message/part`，V2 `session_v2/session_message`），文件存储 `storage/{session,message,part}` 回退 | `providers/opencode.rs:19-79,109-126,196-256` | 代码级 |
| OpenCode 恢复命令 `opencode -s {id}`；标题空时用 `directory` basename | `providers/opencode.rs:164-194,743` | 代码级 |
| Agy 会话在 `~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl`（IDE 根 `~/.gemini/antigravity/brain/...`）；`USER_INPUT` → user，`PLANNER_RESPONSE`/`source=MODEL` → assistant；摘要 DB `conversation_summaries.db` | `providers/agy.rs:16-61,219-257` | 代码级 |
| Agy 恢复命令 `agy --conversation={id}`；无 transcript 时仍可从摘要库列出 | `providers/agy.rs:147-159`；`agy_config.rs:309-331` | 代码级 |
| Grok Build 会话目录含 `summary.json`（`info.id/cwd`、`generated_title/session_summary`、`created_at/last_active_at`）与 `chat_history.jsonl`（`system/user/assistant/tool`，OpenAI 形状 `tool_calls`） | `providers/grokbuild.rs:36-118,189-226` | 代码级 |
| Grok Build 恢复命令 `grok --resume {id}` | `providers/grokbuild.rs:224` | 代码级 |
| MiniMax Code（mcode）数据在 `~/.minimax/v2/sqlite/runtime-state.sqlite`；表 `local_runtime_sessions`/`local_runtime_message_rows`（或 legacy `local_runtime_messages.display_messages_json`）；只暴露 user/assistant 文本 | `providers/mcode.rs:9-116` | 代码级 |
| mcode 恢复命令需带数据目录环境变量：`$env:MINIMAX_DATA_DIR='<dir>'; mcode --session <id>`（Windows），id 只允许字母数字/`_`/`-` | `providers/mcode.rs:36-61` | 代码级 |
| OpenClaw 会话在 `~/.openclaw/agents/<agent>/sessions/*.jsonl`，与 Pi 同构；`sessions.json` 的 `displayName` 优先作标题；无恢复命令 | `providers/openclaw.rs:30-75,119-144,261` | 代码级 |
| Hermes 数据在 `%LOCALAPPDATA%\hermes`（回退 `~\AppData\Local\hermes`）：`state.db`（`sessions/messages`）与 `sessions/*.jsonl`；SQLite 优先；Windows 上还支持 `\\wsl$\...` 经 `wsl.exe` 读取 | `providers/hermes.rs:20-76,520-679`；`hermes_config.rs:51-80` | 代码级 |
| Hermes 无恢复命令；消息按 `id ASC` 而非时间排序 | `providers/hermes.rs:297,448-469` | 代码级 |
| cc-switch **没有**持久化会话索引/FTS；列表每次全扫目录，搜索只在前端内存里做（只索引元数据，不索引正文） | `session_manager/mod.rs:58-99`；`src/hooks/useSessionSearch.ts:27-69` | 代码级 |
| cc-switch 的正文读取是懒加载 + 分块（≤256KB/包、≤150 条/包），工具全文按 `ContentRef` 分页（默认/最大 512K 字符）；图片上限 20MB，本地图片只允许 png/jpg/jpeg/gif/webp/bmp | `session_manager/cache.rs:284-309`；`content.rs:25-40,541-634` | 代码级 |
| cc-switch 的图像索引存的是 `ContentRef`（jsonl offset/len/pointer），不是把 base64 写进索引；本计划改为「≤512 KiB 内联图转 data URL，超出/文件引用只存占位」，索引体积可控且实现简单 | `model.rs:329-378`；本计划取舍 | 设计决定 |
| cc-switch 的导出是前端生成 Markdown + 后端保存对话框；恢复命令由 provider 提供，复制 `cd` 命令时按 POSIX 单引号转义。本计划用双引号并**拒绝**含引号/换行的项目路径（更保守） | `src/components/sessions/reader/exportMarkdown.ts:32-150`；`SessionReader.tsx:425-436` | 代码级 |

### 本机真实样本（2026-10-23 只读盘点）

| Provider | 本机样本 | 数量/规模 | 关键字段事实 | 置信度 |
|---|---|---|---|---|
| Claude Code | 有 | 291 个主 `.jsonl`（约 710 MB）+ 742 个子代理文件（约 688 MB） | 首行是控制行（`mode`/`ai-title`/`last-prompt` 等）；标题在重复出现的 `type=ai-title` 的 `aiTitle`；`sessionId` 与 `session_id` 并存；`cwd` 在 `user`/`assistant` 信封上；内容块 `text/thinking/tool_use/tool_result/image`；子代理文件在 `<会话>/subagents/agent-*.jsonl` | 高 |
| Codex | 有 | 27 个 `sessions/YYYY/MM/DD/rollout-*.jsonl`（约 10 MB） | 首行 `session_meta`（`payload.id/cwd`）；长会话中途重复 `session_meta`；标题在 `event_msg.payload.type=thread_name_updated` 的 `thread_name`，`session_index.jsonl` 也有；`response_item` 与 `event_msg` 内容重复，以 `response_item` 为准；图片块 `input_image.image_url` | 高 |
| Pi | 有 | 51 个 `~/.pi/agent/sessions/<bucket>/<ts>_<uuid>.jsonl`（约 27 MB） | 全部 `version=3`；首行 `type=session` 含 `id/cwd/timestamp`；工具结果是独立 `role=toolResult` 消息；`bashExecution` 是独立角色；图片块 `{type:"image",data,mimeType}`；`custom` 扩展事件不在标准消息里 | 高 |
| Qoder CN | 有 | 44 个主 `.jsonl` + 41 个子代理文件 | 首行 `workspace-directories`（`directories[]`），第 2 行 `runtime-config`；标题在重复出现的 `type=ai-title` 的 `aiTitle`；`cwd` 在信封上；子代理在 `<会话>/subagents/*.jsonl`；另有 `state.json`、SQLite `local.db`、AppData 的 `task-*.session.execution.jsonl`、旧式缓存四套存储 | 高 |
| Gemini CLI | **无** | 只有 `settings.json`/`GEMINI.md`，无 `tmp/chats` | 只能按 cc-switch 代码级事实实现，夹具手写 | 低（无样本） |
| Grok/Hermes/Mcode/OpenClaw/OpenCode/Agy | **无** | 目录不存在 | 只能按 cc-switch 代码级事实实现，夹具手写 | 低（无样本） |
| Qoder 国际版 | **无可用会话** | `local.db` 的 chat 表 0 行；`state.vscdb` 只有视图状态 | 不在承诺范围（C2），不实现 | 中高 |

**结论**：Task 15 只能把 Claude / Codex / Pi / Qoder CN 升为 `verified=true`；
Gemini 与其余六类保持 `verified=false`，UI 显示「解析器未实测」。Qoder CN 的 v1 只读
`projects/**/*.jsonl` 主会话，不读 `state.json`、SQLite、CLI task execution 与旧式缓存
（它们要么是重复来源，要么是 IDE 私有 schema；写进计划只会扩大未验证面）。

## Global Constraints（继承 Plan 1–3，另加）

- **只读**：会话文件只读不写，绝不移动、改名或删除原生文件；索引写失败不能反过来改源文件。
- **内容不落地**：会话正文只进 `index.db`；不得写进 `assets/`、`agents/`、`local/`、`backups/`、`logs/` 或任何同步文件。导出只在用户显式操作时写到用户选定的路径。
- **索引可重建**：删掉 `index.db` 后重新扫描必须得到同样的列表与搜索结果；任何只有索引里才有的数据都算设计错误。
- **逐文件容错**：单个文件解析失败只报告该文件，不能中断整次扫描；一条坏行不能吞掉整个会话的其余内容。
- **时间统一**：所有时间戳归一为 epoch 毫秒；解析不出时间时回落文件 mtime（不额外编造来源标记）。
- **格式未实测要说出来**：本机无真实样本的 provider 解析器标 `verified=false`，UI 显示「解析器未实测」；不得把「按源码推测」说成「已验证」。
- **测试不碰真实家目录**：契约测试与端到端测试一律用临时目录 + `baseOverride`；真实样本只用于人工核对，不复制进仓库。
- **路径与命令**：路径一律 `PathBuf`；恢复命令只做展示/复制，不由后端执行。
- **搜索可执行**：查询串必须经过 FTS5 引号转义；1–2 个字符的查询走 `LIKE`，不得拼出会让 SQLite 报错的语句。
- **大文件不阻塞**：JSONL 逐行读取；单行超过 8 MiB 记一次失败并跳过该行，不把整个文件读进内存。
- **索引体积有上限**：每条消息进索引的 `content` 最多 8 KiB（UTF-8 字节）；超出部分截断并追加
  `\n…[已截断，原文 N 字符]`。FTS 只索引这份截断后的内容；`Image` 行不进 FTS。
  内联图片转 data URL 上限 512 KiB，超出只存占位。本机 Claude 主会话约 710 MB，这条是索引不被撑爆的硬约束。

## Review Focus

1. **坏行不毁整场**：一条非法 JSON 行只丢这一行，前后的消息仍要进索引，并报告文件与行号。
2. **同 ID 不串消息**：两个文件给出同一个 `(agent, session_id)` 时，只能有一套消息；较旧/重复的那份要点名，不能把两边的消息混在一起。
3. **删文件不留下幽灵**：原生文件被删除/改名后，下一次扫描必须清掉对应会话、消息与 FTS 行。
4. **中文与短词搜索**：中文短语、1–2 个字的查询、带引号/星号/百分号的查询都要能搜到或安全返回空，不能报 SQL 错误。
5. **隐私边界**：扫描后 `assets/`、`local/`、`backups/`、`logs/` 里 grep 不到会话正文；导出必须由用户点按钮并选择路径。
6. **重复扫描幂等**：文件没变时不重复解析、不重写消息、不重复 FTS 行；`unchanged` 计数能被断言。
7. **超长会话不卡死**：阅读页分页加载；单条消息内容过大时截断展示但保留「已截断」提示。
8. **图片降级**：内联图片能显示；文件引用或超限图片显示占位与原因，不能让整页渲染失败。
9. **覆盖目录隔离**：`local/agents/<id>.json` 设了 `baseOverride` 时，扫描只看覆盖目录；真实 `~/.claude` 等不读。
10. **未知记录不丢整场**：provider 新增的未知 `type` 只计数跳过，已有消息仍要显示。

---

## File Structure

```
src-tauri/src/
  domain/session.rs                  # 统一模型 + DB 字符串映射
  sessions/mod.rs                    # 注册表、发现、共享工具
  sessions/providers/{claude,codex,gemini,pi,qoder_cn,opencode,agy,grokbuild,mcode,openclaw,hermes}.rs
  sessions/export.rs                 # Markdown 渲染 + 恢复命令
  index/schema.rs                    # 追加 sessions/session_messages/session_files/FTS
  index/sessions.rs                  # 会话查询（列表/详情/搜索）
  services/sessions.rs               # 增量扫描、清理、搜索编排
  commands/sessions.rs               # Tauri 命令
src/
  panels/SessionsPanel.tsx           # 列表 + 搜索 + 分组
  components/sessions/{SessionReader,SessionTimeline,SessionMessage,SessionSearch}.tsx
  hooks/useSessions.ts
  lib/sessionGroups.ts               # 纯分组/时间桶
  __tests__/{SessionsPanel,SessionReader,sessionGroups}.test.tsx|ts
src-tauri/tests/fixtures/sessions/   # 合成样本（不含真实用户内容）
src-tauri/tests/sessions_e2e.rs
```

---

### Task 1: 会话模型、provider 注册表与文件发现

**Files:** Create `domain/session.rs`、`sessions/mod.rs`；Modify `domain/mod.rs`、`lib.rs`

**Interfaces:**
- `SessionRole::{User, Assistant, Tool, System}`、`SessionMessageKind::{Text, Thinking, ToolCall, ToolResult, Image, Event}`，二者 `serde(rename_all="snake_case")` 并提供 `ALL`、`as_db()/from_db()`（未知值返回 `Err`，不编默认值）。
- `ParsedSession { session_id: String, title: String, project_path: Option<PathBuf>, created_at: Option<i64>, updated_at: i64, messages: Vec<ParsedMessage>, warnings: Vec<SessionWarning> }`
- `ParsedMessage { role: SessionRole, kind: SessionMessageKind, content: String, tool_name: Option<String> }`
- `SessionWarning { line: usize, message: String }`：坏行/未知记录只记警告，不打断整场解析；`line` 从 1 开始。
- `SessionProviderStatus { agent_id: String, display_name: String, verified: bool, parse_ready: bool, roots: Vec<String>, root_exists: bool, found_files: usize, session_count: usize }`；`root_exists` = 任一展开后的根存在。
- `SessionSummary { agent_id: String, session_id: String, title: String, project_path: Option<String>, source_path: String, created_at: Option<i64>, updated_at: i64, message_count: u32, parse_error: Option<String> }`
- `SessionMessage { seq: u32, role: SessionRole, kind: SessionMessageKind, content: String, tool_name: Option<String> }`、`SessionDetail { session: SessionSummary, messages: Vec<SessionMessage>, total: u32 }`
- `SessionProvider { agent_id: &'static str, display_name: &'static str, roots: &'static [&'static str], verified: bool, discover: fn(&Path, bool) -> Vec<PathBuf>, parse: Option<fn(&Path) -> Result<Vec<ParsedSession>, SessionError>>, resume: Option<fn(&SessionSummary) -> Option<String>> }`。
  `discover(root, override_active)` 的第二个参数为 `true` 时表示 `baseOverride` 生效：只许看传进来的 `root`，不许再读环境变量或真实家目录。
  `resume` 是函数而不是字符串模板：Pi 要绝对文件路径、mcode 要平台相关的环境变量前缀，字符串模板表达不了这些差异。
  **统一签名是 `Vec<ParsedSession>`**：单文件解析器返回只含一个元素的 Vec；SQLite/多会话源（OpenCode、Mcode、Hermes、Agy 摘要库）一次返回全部会话。这样扫描器只有一条路径，不用为「文件 provider / 数据库 provider」写两套循环。
- `providers() -> &'static [SessionProvider]`；`provider(agent_id) -> Option<&'static SessionProvider>`。
- `discover_provider(provider, home, base_override) -> Vec<PathBuf>`：`base_override` 存在时替换该 provider 所有根里的 `~` 锚点（复用 `adapters::paths::expand_root`）；结果按路径排序、去重。
  provider 自己的 `discover(root, override_active)` 负责特殊根（`HERMES_HOME`、`MINIMAX_DATA_DIR`/`MAVIS_DATA_DIR`、`XDG_DATA_HOME`、Windows `%LOCALAPPDATA%`）；**`base_override` 优先级最高**，`override_active=true` 时不再读环境变量与真实家目录。
- `SessionError::{Io, Json, MissingId, Unsupported(String)}`；坏行不返回 Err，而是进 `ParsedSession.warnings`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn provider_registry_covers_every_promised_agent() {
    let ids: Vec<_> = providers().iter().map(|p| p.agent_id).collect();
    for expected in ["claude","codex","gemini","grokbuild","hermes","mcode","openclaw","opencode","pi","agy","qoder-cn"] {
        assert!(ids.contains(&expected), "缺少 {expected}：{ids:?}");
    }
    assert!(!ids.contains(&"qoder"), "国际版会话位置未确认（C2），不在承诺范围");
}

#[test]
fn base_override_replaces_the_home_anchor_for_every_root() {
    let home = Path::new("C:/Users/u");
    let sandbox = Path::new("D:/sandbox");
    // 先在 sandbox/.claude/projects/p1/s1.jsonl 放一个空文件，否则断言对空集合是空转
    std::fs::create_dir_all(sandbox.join(".claude/projects/p1")).unwrap();
    std::fs::write(sandbox.join(".claude/projects/p1/s1.jsonl"), "").unwrap();
    let claude = provider("claude").unwrap();
    let files = discover_provider(claude, home, Some(sandbox));
    assert!(files.iter().all(|p| p.starts_with(sandbox)), "{files:?}");
    assert!(files.iter().all(|p| !p.starts_with(home)), "覆盖后不能再去真实家目录");
}

#[test]
fn roles_and_kinds_roundtrip_through_db_strings() {
    for kind in SessionMessageKind::ALL {
        assert_eq!(SessionMessageKind::from_db(kind.as_db()), Ok(kind));
    }
    assert!(SessionMessageKind::from_db("nope").is_err());
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test … session` → FAIL。
- [ ] **Step 3: 实现** — 注册表先录入全部 provider 的 `roots`，`parse` 与 `resume` 一律先为 `None`（对应 Task 2–9 逐个填上）；`discover` 先用「根目录下按扩展名递归」的通用实现，各 provider 的特殊层级在对应解析任务里覆盖。
- [ ] **Step 4: 运行** — `cargo test … session` → PASS。
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加会话模型与 provider 注册表"`

### Task 2: Claude 解析器（Qoder CN 复用基座）

**Files:** Create `sessions/providers/claude.rs`、`src-tauri/tests/fixtures/sessions/claude.jsonl`；Modify `sessions/mod.rs`

**Interfaces:**
- `pub fn parse_claude(path: &Path) -> Result<ParsedSession, SessionError>`（单文件）；
  `pub fn parse_claude_all(path: &Path) -> Result<Vec<ParsedSession>, SessionError>`（注册表用，包一层 `vec![…]`）；
  `pub fn discover_claude(root: &Path, _override_active: bool) -> Vec<PathBuf>`；
  `pub fn resume_claude(summary: &SessionSummary) -> Option<String>`（返回 `claude --resume <id>`；后续各 provider 的 `resume_*` 同签名）。
- 注册表 `claude` 项：`roots=["~/.claude/projects"]`、`parse=Some(parse_claude_all)`、`discover=discover_claude`、`resume=Some(resume_claude)`（返回 `claude --resume <id>`）。
- 后续单文件 provider（Codex / Gemini / Pi / Qoder CN / Grok / OpenClaw / Hermes JSONL）都按同样方式提供 `_all` 包装；Task 7/9 的 SQLite 解析器直接返回 `Vec<ParsedSession>`。
- 把逐行枚举与块映射写成可配置的 `ClaudeFamily { title_fields: &[&str], project_fallback: ProjectFallback }`，
  Task 6 的 Qoder CN 复用同一份实现（本机样本已确认两者同构）。

**映射规则：**
- 逐行 `serde_json::from_str::<Value>`；坏行 → `warnings.push({line, message})`，继续下一行。
- 丢弃 `isSidechain == true` 或 `isMeta == true` 的行（不记警告，那是正常记录）。
- 控制行（`mode`、`attachment`、`last-prompt`、`permission-mode`、`agent-name`、`file-history-*`、`queue-operation`、`worktree-state`、`relocated`、`cost-state`、`atis-latch`、`continued-in`、`active-leaf`、`workspace-directories`、`runtime-config` 等）忽略，不进消息也不记警告；**未知 `type` 同样静默忽略**（样本里控制行类型比消息类型多，报警会把正常文件刷成失败）。
- 标题：`type == "ai-title"` 的 `aiTitle`（会重复出现，**取最后一次**）；旧版 `type == "custom-title"` 的 `customTitle` 作为回退（同样取最后一次）。标题值 trim、去空、截 80 字符。
- `type` 为 `user`/`assistant` → 处理 `message.content`（字符串或数组）。
- `type == "system"` 且 `content` 是非空字符串 → Event（content 原文）；其余 system 控制行忽略。
- 块映射：`text` → Text；`thinking`/`redacted_thinking` → Thinking；`tool_use`/`server_tool_use` → ToolCall（`tool_name = name`，content 为 `input` 的 JSON）；`tool_result` → ToolResult（`tool_name` 用 `tool_use_id` 从本文件已见的调用表反查，查不到为 `None`）；`image` → Image（`source.type=="base64"` 且 data 长度 ≤ 512 KiB → `data:<media_type>;base64,<data>`；否则 content 为 `[图片：<media_type>，<原因>]`）。
- 角色按块定：Text/Thinking/Image 继承信封角色（User/Assistant），ToolCall 固定 `Assistant`，ToolResult 固定 `Tool`，Event 固定 `System`；同一信封里的多个块按出现顺序各自成行。
- 未知块类型 → Text（content 为整块 JSON 的紧凑序列化）并记一条 warning；不能静默丢内容。
- 工具调用与结果的内部 `ContentRef` / diff / 预览字段**不引入**：本计划只需要可读正文与全文搜索；顶层 `toolUseResult` 不读（`message.content` 里的 `tool_result` 已足够）。
- `session_id`：任一记录的 `sessionId` 或 `session_id`，否则文件名 stem；都没有 → `SessionError::MissingId`。
- `project_path`：任一信封的 `cwd`；Qoder 复用时再回退到 `workspace-directories.directories[0]`。
- `created_at`：首个可解析 `timestamp`；`updated_at`：最后一个可解析 `timestamp`，都没有则用文件 mtime。
- 跳过文本：以 `<local-command-caveat>`、`<local-command-stdout>`、`<local-command-stderr>`、`<task-notification>` 开头的用户文本不参与标题，但**仍作为消息保留**（阅读页要能看到）。
- **发现规则**：递归 `projects/`；跳过文件名以 `agent-` 开头的文件与 `journal.jsonl`，并跳过路径里带 `/subagents/` 的文件（本机 742 个子代理文件不进入列表）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn claude_extracts_text_thinking_calls_results_and_images() {
    let parsed = parse_claude(&fixture("claude.jsonl")).unwrap();
    assert_eq!(parsed.session_id, "s1");
    assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
    assert_eq!(parsed.title, "修复登录");
    let kinds: Vec<_> = parsed.messages.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, vec![Text, Thinking, Text, ToolCall, ToolResult, Image]);
    assert_eq!(parsed.messages[3].tool_name.as_deref(), Some("Read"));
    assert_eq!(parsed.messages[4].tool_name.as_deref(), Some("Read"));
    assert!(parsed.messages[5].content.starts_with("data:image/png;base64,"));
}

#[test]
fn claude_takes_the_last_ai_title_and_ignores_control_lines() {
    // 真实样本里 ai-title 会重复出现；首行的 mode/last-prompt 等控制行不是错误
    let parsed = parse_claude(&fixture("claude-with-control-lines.jsonl")).unwrap();
    assert_eq!(parsed.title, "最后的标题");
    assert!(parsed.warnings.is_empty(), "控制行不能被当成坏行");
}

#[test]
fn a_bad_claude_line_is_reported_and_the_rest_survives() {
    let parsed = parse_claude(&fixture("claude-with-broken-line.jsonl")).unwrap();
    assert_eq!(parsed.warnings.len(), 1);
    assert_eq!(parsed.warnings[0].line, 4);
    assert_eq!(parsed.messages.len(), 6, "坏行前后的消息都要在");
}

#[test]
fn sidechains_are_dropped_without_warnings() {
    let parsed = parse_claude(&fixture("claude-with-sidechain.jsonl")).unwrap();
    assert!(parsed.warnings.is_empty());
    assert!(!parsed.messages.iter().any(|m| m.content.contains("子代理内部")));
}

#[test]
fn claude_discovery_skips_subagent_and_journal_files() {
    let files = discover_claude(&fixture_dir("claude-projects"), false);
    assert!(files.iter().any(|p| p.ends_with("s1.jsonl")));
    assert!(!files.iter().any(|p| p.to_string_lossy().contains("/subagents/")));
    assert!(!files.iter().any(|p| p.ends_with("journal.jsonl")));
    assert!(!files.iter().any(|p| p.file_name().unwrap().to_string_lossy().starts_with("agent-")));
}

#[test]
fn an_oversized_inline_image_becomes_a_placeholder() {
    let parsed = parse_claude(&fixture("claude-with-huge-image.jsonl")).unwrap();
    let image = parsed.messages.iter().find(|m| m.kind == Image).unwrap();
    assert!(image.content.starts_with("[图片："), "{}", image.content);
    assert!(!image.content.contains("base64"));
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test … claude` → FAIL。
- [ ] **Step 3: 实现** — 逐行读取（`BufReader::lines`），单行超过 8 MiB 记警告并跳过；时间解析用 `chrono::DateTime::parse_from_rfc3339`，解析失败按「没有时间」处理。
- [ ] **Step 4: 运行** — `cargo test … claude` → PASS；`cargo fmt --check` 干净。
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加 Claude 会话解析器"`

#### Task 2 执行记录（留给执行者）

- Qoder CN 复用同一 `ClaudeFamily` 实现；若实现中发现 Qoder 私有块类型，在 Task 6 记录并只改映射层。

---

### Task 3: Codex 解析器

**Files:** Create `sessions/providers/codex.rs`、`src-tauri/tests/fixtures/sessions/codex.jsonl`（及 `codex.jsonl.zst` 由测试代码现场压缩生成）；Modify `sessions/mod.rs`、`Cargo.toml`

**Interfaces:**
- `pub fn parse_codex(path: &Path) -> Result<ParsedSession, SessionError>`；`parse_codex_all`（`vec![…]` 包装）；`discover_codex(root: &Path, override_active: bool) -> Vec<PathBuf>`；注册表 `codex`：`roots=["~/.codex/sessions","~/.codex/archived_sessions"]`，`parse=Some(parse_codex_all)`，`resume=Some(resume_codex)`（返回 `codex resume <id>`）。
- 新增依赖 `zstd = "0.13"`：`.jsonl.zst` 用 `zstd::stream::read::Decoder` 流式解压，不整文件读入内存。

**映射规则：**
- 发现：递归找 `*.jsonl` 与 `*.jsonl.zst`；同一逻辑路径同时存在时只留 `.jsonl`（去掉 `.zst` 后比较）。
- 行类型：
  - `session_meta`：**只有第一次出现**用于 `payload.id`、`payload.cwd`、`payload.timestamp`；长会话中途会重复写 `session_meta`，后续只当快照忽略，不能覆盖 id/cwd。`payload.source.subagent` 存在 → 整个文件跳过（返回空会话，扫描器不建条目）。
  - `response_item`：`payload.type == "message"` 的 `role` 为 `user` → User，`assistant` → Assistant，`developer`/`system` → System；内容块 `input_text`/`output_text` → Text，`input_image` → Image（`image_url` 以 `data:` 开头且 ≤512 KiB 才保留，否则 `[图片：<detail>]` 占位）。`payload.type == "reasoning"` → Thinking（`summary[].text` 与 `content[].text` 合并）；`function_call`/`custom_tool_call`/`local_shell_call`/`web_search_call` → ToolCall（`name` 为工具名，content 为 `arguments`/`input` 的 JSON）；`function_call_output`/`custom_tool_call_output` → ToolResult（`tool_name` 用 `call_id` 反查，`output` 为数组时 JSON 序列化）。
  - `event_msg`：**只用于标题与事件**。`payload.type == "thread_name_updated"` 的 `thread_name`（重复出现取最后一次）作为标题；`task_complete`/`turn_aborted` 等 → Event（content 为类型名）；`user_message`/`agent_message`/`exec_command_end` 等正文副本**忽略**（以 `response_item` 为 canonical，避免双计）。
  - 其他顶层 `type`（`turn_context`/`token_count`/`world_state` 等）静默忽略；未知 `response_item.payload.type` 记 `warnings`（`line, "未知 response_item：{type}"`），不中断。
- 注入文本：`# AGENTS.md instructions`、`<environment_context>`、`<user_instructions>` 开头的用户文本保留为消息，但不参与标题。
- 标题优先级：`thread_name_updated` > 首条非注入用户文本（80 字符）> 目录 basename > `session_id`；**不读 Codex `state.db` 与 `session_index.jsonl`**（v1 明确简化，避免依赖内部 schema 与脆弱的根目录回溯）。
- 时间：首个/最后一个可解析 `timestamp`；`.zst` 的 `updated_at` 优先用文件 mtime（尾部无法流式读取时）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn codex_maps_session_meta_messages_reasoning_and_tools() {
    let parsed = parse_codex(&fixture("codex.jsonl")).unwrap();
    assert_eq!(parsed.session_id, "c1");
    assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
    assert_eq!(parsed.title, "修复构建");
    let kinds: Vec<_> = parsed.messages.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, vec![Text, Thinking, ToolCall, ToolResult, Event]);
    assert_eq!(parsed.messages[2].tool_name.as_deref(), Some("exec"));
    assert_eq!(parsed.messages[3].tool_name.as_deref(), Some("exec"));
}

#[test]
fn codex_uses_thread_name_and_ignores_repeated_session_meta() {
    let parsed = parse_codex(&fixture("codex-repeated-meta.jsonl")).unwrap();
    assert_eq!(parsed.session_id, "c1", "中途重复的 session_meta 不能改 id");
    assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
    assert_eq!(parsed.title, "最后的线程名");
}

#[test]
fn event_msg_duplicates_are_not_counted_twice() {
    let parsed = parse_codex(&fixture("codex-event-duplicates.jsonl")).unwrap();
    assert_eq!(parsed.messages.iter().filter(|m| m.role == User).count(), 1);
    assert_eq!(parsed.messages.iter().filter(|m| m.role == Assistant).count(), 1);
}

#[test]
fn a_subagent_session_file_is_skipped_entirely() {
    let parsed = parse_codex(&fixture("codex-subagent.jsonl")).unwrap();
    assert!(parsed.messages.is_empty(), "子代理会话不进入列表");
}

#[test]
fn a_zstd_compressed_session_reads_the_same_as_the_plain_one() {
    let plain = parse_codex(&fixture("codex.jsonl")).unwrap();
    let compressed = parse_codex(&fixture("codex.jsonl.zst")).unwrap();
    assert_eq!(plain, compressed);
}

#[test]
fn unknown_codex_records_are_counted_not_fatal() {
    let parsed = parse_codex(&fixture("codex-with-unknown-type.jsonl")).unwrap();
    assert!(parsed.warnings.iter().any(|w| w.message.contains("未知 response_item")));
    assert!(!parsed.messages.is_empty());
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Codex 会话解析器"`。

### Task 4: Gemini 解析器

**Files:** Create `sessions/providers/gemini.rs`、`tests/fixtures/sessions/{gemini.json,gemini.jsonl}`；Modify `sessions/mod.rs`

**Interfaces:** `parse_gemini(path) -> Result<ParsedSession>`、`parse_gemini_all`、`discover_gemini(root, override_active) -> Vec<PathBuf>`；注册表 `gemini`：`roots=["~/.gemini/tmp"]`、`resume=Some(resume_gemini)`（返回 `gemini --resume <id>`）。

**本机无样本**：本机 `~/.gemini` 只有 `settings.json`/`GEMINI.md`，没有 `tmp/chats`。本任务按
cc-switch 代码级事实实现，`verified=false`，夹具是手写合成样本。

- 发现：`~/.gemini/tmp/<project>/chats/session-*.json` 与 `session-*.jsonl`；只下到 `chats` 一层；同名 `.jsonl` 存在时 `.json` 隐藏。
- 旧 `.json`：整个文件是 `{sessionId, startTime, lastUpdated, messages:[...]}`。
- 新 `.jsonl` 回放（必须按顺序）：
  1. `{"$rewindTo": "<id>"}`：删除该 id 及之后的消息；找不到 id 则清空消息。
  2. 带字符串 `id` 的行：消息；同 id 原位覆盖（位置不变）。
  3. `{"$set": {...}}`：合并元数据；若 `$set.messages` 是数组，整体替换消息。
  4. 带字符串 `sessionId` 的行：元数据；若带 `messages` 数组，逐条 upsert。
  5. 只有元数据一行的文件：消息为空，仍是合法会话。
- 消息 `type`：`user` → User；`gemini` → Assistant；`info`/`error` → Event（`content` 为文本）；其他忽略。
- Assistant：先 `thoughts[]`（`subject`/`description` 合并成 Thinking），再正文，再 `toolCalls[]`（`name`/`args`/`resultDisplay`/`result` → ToolCall + ToolResult）。工具状态文字保留在 ToolResult 的 content 前缀（`[error] ` / `[interrupted] `），不建状态列。
- `project_path`：读 `<root>/tmp/<project>/.project_root` 的内容（trim）并作为路径；文件缺失则 `None`。
- 标题：首条非忽略用户文本；忽略空、以 `/` 或 `?` 开头、以 `<session_context>`/`<hook_context>` 开头的文本；80 字符。
- 恢复：`gemini --resume {id}`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn gemini_jsonl_replay_applies_set_upsert_and_rewind() {
    let parsed = parse_gemini(&fixture("gemini.jsonl")).unwrap();
    let texts: Vec<_> = parsed.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(texts, vec!["第一问", "第一答", "第二问"], "rewind 之后的消息不能回来");
}

#[test]
fn gemini_old_json_and_jsonl_documents_agree() {
    let old = parse_gemini(&fixture("gemini.json")).unwrap();
    let new = parse_gemini(&fixture("gemini.jsonl")).unwrap();
    assert_eq!(old.messages, new.messages);
    assert_eq!(new.session_id, "g1");
}

#[test]
fn gemini_reads_the_project_root_file() {
    let parsed = parse_gemini(&fixture("gemini/demo/chats/session-1.jsonl")).unwrap();
    assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Gemini 会话解析器"`。

---

### Task 5: Pi 解析器

**Files:** Create `sessions/providers/pi.rs`、`tests/fixtures/sessions/pi.jsonl`；Modify `sessions/mod.rs`

**Interfaces:** `parse_pi(path) -> Result<ParsedSession>`、`parse_pi_all`、`discover_pi(root, override_active) -> Vec<PathBuf>`；注册表 `pi`：`roots=["~/.pi/agent/sessions"]`、`resume=Some(resume_pi)`（返回 `pi --session "<源文件绝对路径>"`；路径含 `"`/CR/LF 时返回 `None`）。

**本机样本**：51 个 `version=3` 的 `.jsonl`；首行 `type=session`；工具结果是独立
`role=toolResult` 消息；`bashExecution` 是独立角色；图片块 `{type:"image",data,mimeType}`。

- 发现：`~/.pi/agent/sessions/<project>/<file>.jsonl`（默认深度 2）；文件 >128MB 或条目 >500,000 跳过并报告。
- 首行必须是 `{"type":"session","id":...,"version":...,"cwd":...,"timestamp":...}`；缺头 → `SessionError::Unsupported("Pi 会话缺少 session 头")`。
- 后续每条 entry 有 `id` 与 `parentId`（v<2 用 `legacy-<index>` 链式 parent）。**只输出活动分支**：从最后一个 entry 沿 `parentId` 回溯到根，取这条链上的 entry；重复 id 取最后一次出现；遇到环或缺失 parent 停止回溯。
- entry 映射：`type=="message"` 时按 `message.role`：
  - `user` → User（content 字符串或数组的 `text`/`image` 块；图片 `{type:"image",data,mimeType}` → data URL，>512 KiB 占位）；
  - `assistant` → Thinking（`content[].type=="thinking"`）、Text（`type=="text"`）、ToolCall（`type=="toolCall"`，`name` + `arguments`）；
  - `toolResult` → ToolResult（`toolCallId` 反查工具名；`content` 里的文本/图片分别输出）；
  - `bashExecution` → ToolCall（工具名 `bash`，content 为 `command`）+ ToolResult（content 为 `output`，`exitCode` 非 0 时前缀 `[exit N] `）；
  - `branchSummary`/`compactionSummary` → Event；
  - `system` → 跳过。
- `type=="session_info"` 的 `name` 作为标题候选；否则首条 User 文本；否则 `cwd` basename；80 字符。
- `session_id` 取头 `id`；`project_path` 取头 `cwd`；时间取头 `timestamp` 与消息 `timestamp` 的最大值。
- 恢复：`pi --session <源文件绝对路径>`（不是 id）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn pi_follows_only_the_active_branch() {
    let parsed = parse_pi(&fixture("pi.jsonl")).unwrap();
    assert!(!parsed.messages.iter().any(|m| m.content.contains("被放弃的分支")));
    assert!(parsed.messages.iter().any(|m| m.content.contains("活动分支")));
}

#[test]
fn pi_maps_tool_calls_results_and_bash_execution() {
    let parsed = parse_pi(&fixture("pi.jsonl")).unwrap();
    assert!(parsed.messages.iter().any(|m| m.kind == ToolCall && m.tool_name.as_deref() == Some("read")));
    assert!(parsed.messages.iter().any(|m| m.kind == ToolResult && m.tool_name.as_deref() == Some("read")));
    assert!(parsed.messages.iter().any(|m| m.kind == ToolCall && m.tool_name.as_deref() == Some("bash")));
}

#[test]
fn pi_without_a_session_header_is_a_named_error() {
    assert!(matches!(parse_pi(&fixture("pi-no-header.jsonl")), Err(SessionError::Unsupported(_))));
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Pi 会话解析器"`。

---

### Task 6: Qoder CN 解析器

**Files:** Create `sessions/providers/qoder_cn.rs`（只写发现/标题适配并复用 `parse_claude`）；Modify `sessions/mod.rs`

**Interfaces:** `parse_qoder_cn(path) -> Result<ParsedSession>`、`parse_qoder_cn_all`、`discover_qoder_cn(root, override_active) -> Vec<PathBuf>`；注册表 `qoder-cn`：`roots=["~/.qoder-cn/projects"]`、`parse=Some(parse_qoder_cn_all)`、`resume=Some(resume_qoder_cn)`（返回 `qoderclicn --resume <id>`）。

**本机事实（已核对）：** 主会话 `~/.qoder-cn/projects/<slug>/<uuid>.jsonl` 与 Claude 同构：
`user`/`assistant` 信封 + `message.content` 块（`text/thinking/tool_use/tool_result/image`），
额外有控制行 `workspace-directories`（首行，`directories[]`）、`runtime-config`、`active-leaf`、
`ai-title`（重复，`aiTitle`）、`last-prompt`；子代理在 `<会话>/subagents/*.jsonl`。
**不得把样本内容复制进仓库**；测试夹具用同构字段手写在 `tests/fixtures/sessions/qoder-cn.jsonl`。

- 实现方式：把 Task 2 的 Claude 解析器抽出可配置的 `ClaudeFamily`（标题字段 `aiTitle`/`customTitle`、
  项目回退 `workspace-directories.directories[0]`），Qoder CN 复用同一实现，只改
  `roots=["~/.qoder-cn/projects"]`、跳过 `/subagents/` 路径、`resume=Some("qoderclicn --resume {id}")`。
- v1 只读主 JSONL：`state.json`、SQLite `local.db`、AppData 的 `task-*.session.execution.jsonl`
  与旧式缓存**不读**（重复来源/IDE 私有 schema）；状态里写明「其他 Qoder 存储未接入」。
- 若实现时发现块类型有 Qoder 私有扩展：未知块类型转 Text（保留 JSON 原文）并记警告。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn qoder_cn_uses_ai_title_and_workspace_directories() {
    let parsed = parse_qoder_cn(&fixture("qoder-cn.jsonl")).unwrap();
    assert_eq!(parsed.session_id, "q1");
    assert_eq!(parsed.project_path.as_deref(), Some(Path::new("D:/repo")));
    assert_eq!(parsed.title, "Qoder 会话标题");
    assert!(parsed.messages.iter().any(|m| m.kind == ToolCall));
}

#[test]
fn qoder_cn_discovery_skips_subagent_files() {
    let files = discover_qoder_cn(&fixture_dir("qoder-projects"), false);
    assert!(!files.iter().any(|p| p.to_string_lossy().contains("/subagents/")));
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Qoder CN 会话解析器"`。

---

### Task 7: OpenCode 与 Agy 解析器

**Files:** Create `sessions/providers/opencode.rs`、`sessions/providers/agy.rs`、`tests/fixtures/sessions/opencode/...`、`tests/fixtures/sessions/agy/...`；Modify `sessions/mod.rs`

**Interfaces:** `parse_opencode_all(path) -> Result<Vec<ParsedSession>>`、`discover_opencode(root, override_active) -> Vec<PathBuf>`（有 `opencode.db` 时只返回 DB，否则返回 session JSON 文件）；`parse_agy_all(path) -> Result<Vec<ParsedSession>>`、`discover_agy(root, override_active) -> Vec<PathBuf>`（transcript 文件 + 摘要 DB）。注册表 `opencode.resume=Some(resume_opencode)`（`opencode -s <id>`）、`agy.resume=Some(resume_agy)`（`agy --conversation=<id>`）。

#### OpenCode

- 数据根：`XDG_DATA_HOME/opencode`，否则 `~/.local/share/opencode`。
- 优先 SQLite `opencode.db`：
  - V1 表 `session(id,title,directory,time_created,time_updated)`、`message(id,session_id,time_created,data)`、`part(id,message_id,data)`；按 `time_created,id` 排序。
  - V2 表 `session_v2`、`session_message(id,session_id,seq,type,time_created,data)`；按 `seq ASC, rowid ASC`，只接受 `type ∈ user/assistant/system`。
  - 混合库：按会话实际所在表读取；SQLite 存在时不再读文件存储。
- 文件存储回退：`storage/session/**/*.json` + `storage/message/<sessionID>/*.json` + `storage/part/<messageID>/*.json`。
  解析时从 session JSON 路径向上找到包含 `storage/` 的数据根，再拼 message/part 目录，不依赖当前工作目录。
- part 映射：`text` → Text（跳过 `synthetic`/`ignored`）；`reasoning` → Thinking；`tool` → ToolCall + ToolResult（`callID`、`tool`、`state.status`、`state.input/output`）；`agent`/`patch`/`file` → Event/Text；`step-*`/`snapshot`/`compaction`/`subtask`/`retry` 跳过。
- 标题：session `title`；空时用 `directory` basename。`project_path = directory`。时间 `time_created/time_updated`。
- 恢复：`opencode -s {id}`。

#### Agy

- 根：`~/.gemini/antigravity-cli/brain/<id>/.system_generated/logs/transcript_full.jsonl` 与 `~/.gemini/antigravity/brain/...`；索引 DB `conversation_summaries.db`（表 `conversation_summaries`）。
- 行映射：`type=="USER_INPUT"` → User（只取 `<USER_REQUEST>...</USER_REQUEST>` 内文本，取不到则原文）；`type=="PLANNER_RESPONSE"` 或 `source=="MODEL"` → Assistant；其余 → System Event。按 `step_index` 去重（后写覆盖）。
- 未识别 step 有 `content` 就按 Text 保留；没有就把整条 JSON pretty-print 成 Text（不丢未来类型）。
- 标题/项目：优先摘要库 `title`、`workspace_uris`（第一个 `file://` 转路径）、`last_modified_time`；无摘要库时用首个 USER_INPUT 文本、`created_at`/`last_active_at`。
- 只有摘要库、没有 transcript 的会话：列出元数据，消息为空；阅读页提示「只有索引，无 transcript」。`discover` 先返回 transcript 文件、后返回摘要 DB，同一 `session_id` 在等时间/等消息数时 transcript 胜出。
- 恢复：`agy --conversation={id}`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn opencode_sqlite_v1_reads_session_message_parts_in_order() {
    // 构造 session/message/part 三表；断言 User 文本 → ToolCall → ToolResult 的顺序与 tool_name
}

#[test]
fn opencode_v2_sorts_by_seq_not_timestamp() {
    // 两条消息 time_created 倒序但 seq 正序；断言输出按 seq 排
}

#[test]
fn opencode_prefers_sqlite_over_file_storage() {
    // 同一 session 在 SQLite 与文件存储各有一份不同标题；断言取 SQLite 标题
}

#[test]
fn agy_maps_user_input_and_planner_response_and_dedupes_steps() {
    // 同一 step_index 写两次；断言只保留后写的内容，且 USER_REQUEST 内文本被抽出
}

#[test]
fn agy_unknown_step_is_preserved_as_text_not_dropped() {
    // 未知 type 且无 content；断言消息仍是 Text 且包含原始 JSON 字段
}

#[test]
fn agy_metadata_only_session_has_no_messages_but_is_listed() {
    // 摘要库有、transcript 无；断言 session 元数据可读、messages 为空
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 OpenCode 与 Agy 会话解析器"`。

---

### Task 8: Grok Build 与 OpenClaw 解析器

**Files:** Create `sessions/providers/grokbuild.rs`、`sessions/providers/openclaw.rs`、夹具；Modify `sessions/mod.rs`

**Interfaces:** `parse_grokbuild(path) -> Result<ParsedSession>`（输入是 `chat_history.jsonl`，同目录 `summary.json` 提供元数据）、`parse_grokbuild_all`、`discover_grokbuild(root, override_active)`；`parse_openclaw`、`parse_openclaw_all`、`discover_openclaw(root, override_active)`。注册表 `grokbuild.resume=Some(resume_grokbuild)`（`grok --resume <id>`），`openclaw.resume=None`。

- **Grok Build**：`~/.grok/{sessions,archived_sessions}` 下递归找 `summary.json`；同目录 `chat_history.jsonl` 是正文。`summary.json` 的 `info.id`/`info.cwd`/`generated_title`/`session_summary`/`created_at`/`last_active_at` 提供元数据；`chat_history.jsonl` 的 `system/user/assistant/tool` 映射为 System/User/Assistant/ToolResult，assistant 的 OpenAI 形状 `tool_calls[]` → ToolCall。恢复 `grok --resume {id}`。
- **OpenClaw**：`~/.openclaw/agents/<agent>/sessions/*.jsonl`，与 Pi 同构；复用 `pi::parse_pi` 的 entry 映射（不要把 Pi 的活动分支逻辑用于 OpenClaw：OpenClaw 按文件顺序全取）。`sessions.json` 的 `displayName` 优先作标题；`cwd` 作项目目录；无恢复命令。
- 两个 provider 都只读；`sessions.json` / `summary.json` 缺失时给出可解释的 `SessionError`，不 panic。

- [ ] **Step 1: 写失败测试** — `grok_reads_summary_and_chat_history`、`grok_title_falls_back_to_first_user_message`、`openclaw_reuses_pi_message_mapping`、`openclaw_display_name_wins_over_first_message`。
- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Grok Build 与 OpenClaw 会话解析器"`。

---

### Task 9: MiniMax Code 与 Hermes 解析器

**Files:** Create `sessions/providers/mcode.rs`、`sessions/providers/hermes.rs`、夹具；Modify `sessions/mod.rs`

**Interfaces:** `parse_mcode_all(db_path) -> Result<Vec<ParsedSession>>`、`discover_mcode(root, override_active)`；`parse_hermes_all(path) -> Result<Vec<ParsedSession>>`（DB 一次返回全部；JSONL 单文件返回一个）、`discover_hermes(root, override_active)`。注册表 `mcode.resume=Some(resume_mcode)`（Windows 返回 `$env:MINIMAX_DATA_DIR='<dir>'; mcode --session <id>`，其他平台返回 `env MINIMAX_DATA_DIR=<dir> mcode --session <id>`），`hermes.resume=None`。

- **MiniMax Code（mcode）**：`~/.minimax/v2/sqlite/runtime-state.sqlite`（或 `MINIMAX_DATA_DIR`/`MAVIS_DATA_DIR`）。
  - 查 `local_runtime_sessions`：过滤 `visibility <> 'hidden'`、`archived = 0`、`parent_session_id IS NULL`、`session_kind NOT IN ('peek','channel','cron')`。
  - 有 `local_runtime_message_row_migrations` 时读 `local_runtime_message_rows`；否则解析 `local_runtime_messages.display_messages_json` 数组。
  - 只保留 `role IN ('user','assistant')` 的 `msg_content` 文本；时间 `created_at_ms`，缺失时解析 JSON 时间。
  - 恢复命令：Windows `$env:MINIMAX_DATA_DIR='<目录>'; mcode --session <id>`，其他平台 `env MINIMAX_DATA_DIR=<目录> mcode --session <id>`；id 只含 `[A-Za-z0-9_-]` 时才生成，目录中的单引号按平台规则转义。
- **Hermes**：原生路径 `%LOCALAPPDATA%\\hermes`（回退 `~\\AppData\\Local\\hermes`），`state.db` + `sessions/*.jsonl`；`discover` 先返回 `state.db`、后返回 JSONL，同一 `session_id` 按扫描器的重复规则处理（等时间/等消息数时先处理的 SQLite 胜出）。
  - SQLite：`sessions(id,title,cwd|directory,started_at|created_at,last_activity_at|ended_at|updated_at)`、`messages(role,content,tool_calls,tool_call_id,timestamp,...)`；只显示 `active=1 OR compacted=1`（列存在时）；`tool` → ToolResult，`assistant` → Thinking + Text + ToolCall，其他 → Text。
  - JSONL：扁平 `{role,content}` 或 `{type:"message",message:{...}}`；`type=session|init` 提供 id/title/cwd。
  - **WSL 路径（`\\\\wsl$\\...`）本计划不支持**：发现时跳过并报告「Hermes WSL 会话暂不支持」；原生 Windows 路径照常解析。恢复命令：无。
- 两个 provider 都按只读连接打开 SQLite（`OpenFlags::SQLITE_OPEN_READ_ONLY`），并设 `busy_timeout(500ms)`。

- [ ] **Step 1: 写失败测试** — `mcode_filters_hidden_archived_and_child_sessions`、`mcode_prefers_migrated_rows_over_legacy_blob`、`mcode_resume_command_quotes_the_data_dir`、`hermes_sqlite_is_preferred_over_jsonl`、`hermes_jsonl_supports_flat_and_nested_shapes`、`hermes_wsl_paths_are_reported_not_parsed`。
- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 MiniMax Code 与 Hermes 会话解析器"`。

---

### Task 10: 会话索引、增量扫描与全文搜索

**Files:** Modify `index/schema.rs`、`index/mod.rs`、`error.rs`；Create `index/sessions.rs`、`services/sessions.rs`；Modify `services/mod.rs`

**Interfaces:**
- schema 追加：

```sql
CREATE TABLE IF NOT EXISTS sessions (
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    title TEXT NOT NULL,
    project_path TEXT,
    source_path TEXT NOT NULL,
    created_at INTEGER,
    updated_at INTEGER NOT NULL,
    message_count INTEGER NOT NULL,
    PRIMARY KEY (agent_id, session_id)
);
CREATE TABLE IF NOT EXISTS session_messages (
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    tool_name TEXT,
    PRIMARY KEY (agent_id, session_id, seq)
);
CREATE TABLE IF NOT EXISTS session_files (
    agent_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    session_id TEXT,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL,
    parse_error TEXT,
    PRIMARY KEY (agent_id, source_path)
);
CREATE VIRTUAL TABLE IF NOT EXISTS session_messages_fts USING fts5(
    content,
    agent_id UNINDEXED,
    session_id UNINDEXED,
    seq UNINDEXED,
    tokenize='trigram'
);
```

- `pub const INDEX_CONTENT_CAP: usize = 8 * 1024;`（`index/sessions.rs`），截断标记固定为 `\n…[已截断，原文 {chars} 字符]`。
- `Index::upsert_session(&mut self, agent_id, &ParsedSession, source_path, mtime, size) -> Result<(), IndexError>`：事务内替换该会话全部消息与 FTS 行，再 upsert `sessions` 与 `session_files`；`message_count = parsed.messages.len()`（扁平块数，一个信封可能拆成多行）；写入前按 `INDEX_CONTENT_CAP` 截断 `content`，`Image` 行跳过 FTS。
- `Index::delete_session(&mut self, agent_id, session_id)`：删 `sessions`、`session_messages`、FTS。
- `Index::delete_sessions_by_source(&mut self, agent_id, source_path, keep: &[String]) -> Result<usize, IndexError>`：删掉该来源下不在 `keep` 里的会话（多会话 SQLite 源重扫后清理消失的会话；`keep` 为空即删光）。
- `Index::list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionSummary>, IndexError>`，`SessionFilter { agent_id: Option<String>, offset: u32, limit: u32 }`，并提供 `SessionFilter::all()`（测试与「全部」查询用）。`SessionSummary.parse_error` 由 `session_files` 按 `source_path` 左连接得到，不额外写进 `sessions` 表。
- `Index::session_detail(&self, agent_id, session_id, offset, limit) -> Result<SessionDetail, IndexError>`；`total` 是该会话消息行数。
- `Index::search_sessions(&self, query: &str, limit: u32) -> Result<Vec<SessionSearchHit>, IndexError>`，`SessionSearchHit { session: SessionSummary, index: u32, seq: u32, snippet: String }`（每个会话最多一条命中，取最早命中）。
- `Index::search_in_session(&self, agent_id, session_id, query, limit) -> Result<Vec<SessionMatch>, IndexError>`，`SessionMatch { index: u32, seq: u32, snippet: String }`。
- `services::sessions::scan_all(store: &Store, index: &mut Index, home: &Path) -> Result<SessionScanReport, AppError>`，`SessionScanReport { added, updated, removed, unchanged: usize, failures: Vec<SessionFileFailure>, warnings: Vec<SessionFileWarning> }`。
- `SessionFileFailure { agent_id: String, file: String, reason: String }`：整来源失败或重复 ID。
- `SessionFileWarning { agent_id: String, file: String, line: usize, message: String }`：解析器跳过坏行时的逐条报告。
- `services::sessions::providers_status(index, store, home) -> Result<Vec<SessionProviderStatus>, AppError>`；`roots` 是展开后的绝对路径（含 `baseOverride`），UI 空状态直接显示。
- `AppError` 增加 `#[error("会话解析失败：{0}")] Session(#[from] sessions::SessionError)`，解析错误不折成字符串。

**扫描规则：**
- 对每个 provider 读取 `local/agents/<id>.json` 的 `baseOverride`，发现文件；`parse=None` 的 provider 只在状态里显示「解析器未接入」，不报失败。
- 指纹 = `(mtime_ms, size)`；`session_files` 里指纹相同且无 `parse_error` → `unchanged`，不重读。
- 解析成功：对返回的每个 `ParsedSession` 调 `upsert_session`；多会话来源（SQLite）随后调 `delete_sessions_by_source(source, keep=本轮返回的 id 集合)` 清掉消失的会话。解析失败：写 `session_files.parse_error`，保留上一次成功的会话数据（不删），报告失败。
- 同一 provider 两个来源给出相同 `session_id`：先保留 `updated_at` 较新的；时间相同则保留消息数多的（Agy 的 transcript 会赢过只有摘要库的空会话），再相同则保留先处理的。多余的来源记入 `failures`（原因「会话 ID 重复」），其 `session_files` 行照记以免每次重解析。
- 该 provider 这次没发现来源路径：从 `session_files` 删除，并 `delete_sessions_by_source(path, keep=[])`。**已知限制**：如果被删的是重复来源中胜出的那份，失败来源的指纹没变，不会自动重解析；下次删 `index.db` 全量重建会恢复。v1 不为此加来源映射表（`session_sources`），代价是一处罕见场景要手动重建索引。
- 解析器返回的 `warnings` 逐条进报告的 `warnings`；`session_files.parse_error` 只记「整来源失败」。

- [ ] **Step 1: 写失败测试**

测试夹具：`Sandbox { store, index, home, session_file }` —— `tempfile::tempdir()` 下
`Store::init(dir/store)`、`Index::open(dir/index.db)`、`home = dir/home`，再用
`local/agents/claude.json` 的 `baseOverride` 把 Claude 根指到 `home/.claude`。

```rust
#[test]
fn scanning_is_idempotent_and_only_reparses_changed_files() {
    let fx = sandbox_with_claude_session();
    let first = scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert_eq!((first.added, first.unchanged), (1, 0));
    let second = scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert_eq!((second.added, second.updated), (0, 0));
    assert_eq!(second.unchanged, 1);
}

#[test]
fn a_deleted_session_file_disappears_from_the_index() {
    let fx = sandbox_with_claude_session();
    scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    std::fs::remove_file(&fx.session_file).unwrap();
    let report = scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert_eq!(report.removed, 1);
    assert!(fx.index.list_sessions(&SessionFilter::all()).unwrap().is_empty());
    assert_eq!(fx.index.session_detail("claude", "s1", 0, 10).unwrap_err().to_string(), "未找到：s1");
}

#[test]
fn a_broken_line_does_not_lose_the_rest_of_the_session() {
    let fx = sandbox_with_claude_session();
    append_broken_line(&fx.session_file);
    let report = scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert!(report.warnings.iter().any(|w| w.line == 2), "{:?}", report.warnings);
    assert_eq!(fx.index.session_detail("claude", "s1", 0, 10).unwrap().messages.len(), 2);
}

#[test]
fn fts5_trigram_is_available() {
    // 直接从 schema 建库并插入「工具调用」；断言 MATCH '"工具调用"' 能命中
}

#[test]
fn chinese_and_two_character_queries_are_safe() {
    let fx = sandbox_with_claude_session(); // 正文含「工具调用」
    scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert_eq!(fx.index.search_sessions("工具调用", 10).unwrap().len(), 1);
    assert_eq!(fx.index.search_sessions("工具", 10).unwrap().len(), 1, "1–2 字走 LIKE");
    assert!(fx.index.search_sessions("\" * %", 10).unwrap().is_empty(), "特殊字符不能报错");
}

#[test]
fn duplicate_session_ids_keep_the_newer_and_name_the_other() {
    let fx = sandbox_with_two_files_same_session_id();
    let report = scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert!(report.failures.iter().any(|f| f.reason.contains("会话 ID 重复")));
    let detail = fx.index.session_detail("claude", "s1", 0, 10).unwrap();
    assert!(detail.session.title.contains("较新"), "保留 updated_at 较新的那份");
    assert!(!detail.messages.iter().any(|m| m.content.contains("较旧")));
}

#[test]
fn oversized_content_is_truncated_before_it_enters_the_index() {
    let fx = sandbox_with_huge_tool_result(20 * 1024);
    scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    let detail = fx.index.session_detail("claude", "s1", 0, 10).unwrap();
    let huge = detail.messages.iter().max_by_key(|m| m.content.len()).unwrap();
    assert!(huge.content.contains("[已截断，原文"), "{}", &huge.content[..80.min(huge.content.len())]);
    assert!(huge.content.len() <= 8 * 1024 + 64);
}

#[test]
fn image_rows_are_not_indexed_in_the_fts() {
    let fx = sandbox_with_inline_image("data:image/png;base64,QUJD");
    scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    assert!(fx.index.search_sessions("QUJD", 10).unwrap().is_empty(), "图片 base64 不参与全文搜索");
}

#[test]
fn scanning_never_copies_session_content_into_the_store() {
    let fx = sandbox_with_claude_session_with_marker("SESSION_SECRET_MARKER");
    scan_all(&fx.store, &mut fx.index, &fx.home).unwrap();
    // 只查 assets/agents/local/backups/logs；index.db 本来就是派生缓存，允许含正文
    assert!(!store_text_files_contain(&fx.store, "SESSION_SECRET_MARKER", &["index.db", "index.db-*"]));
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加会话索引与增量扫描"`。
  `rusqlite` 0.32 的 bundled SQLite 已在 `libsqlite3-sys/build.rs` 里启用 `SQLITE_ENABLE_FTS5`，
  不写运行时降级；另加一条 `fts5_trigram_is_available` 测试（建表 + 中文子串命中），让编译/启动
  阶段就暴露能力缺失，而不是在用户搜索时静默返回空。

---

### Task 11: 会话命令面

**Files:** Create `commands/sessions.rs`；Modify `commands/mod.rs`、`lib.rs`

**Interfaces（Tauri 命令，参数 camelCase 由前端传入）：**
- `scan_sessions(state) -> Result<SessionScanReport, AppError>`
- `list_sessions(state, agent_id: Option<String>, query: Option<String>, offset: Option<u32>, limit: Option<u32>) -> Result<Vec<SessionSummary>, AppError>`（`query` 非空时内部转调 `search_sessions` 并去掉 snippet，只为保留一个「列表入口」）
- `get_session(state, agent_id: String, session_id: String, offset: Option<u32>, limit: Option<u32>) -> Result<SessionDetail, AppError>`
- `search_sessions(state, query: String, limit: Option<u32>) -> Result<Vec<SessionSearchHit>, AppError>`
- `search_in_session(state, agent_id: String, session_id: String, query: String, limit: Option<u32>) -> Result<Vec<SessionMatch>, AppError>`
- `session_resume(state, agent_id: String, session_id: String) -> Result<ResumeInfo, AppError>`（Task 14 实现）
- `export_session_markdown(state, agent_id: String, session_id: String, dest: String) -> Result<String, AppError>`（Task 14 实现）
- `session_providers(state) -> Result<Vec<SessionProviderStatus>, AppError>`

**规则：** 每次读命令先 `scan_all`？不。扫描只由 `scan_sessions` 触发；面板首次使用显式调用。命令层只做锁与参数校验，业务在 `services::sessions`。
**`scan_sessions` 必须 `async` + `tauri::async_runtime::spawn_blocking`**：本机 Claude 主会话 710 MB，
同步命令会把窗口卡住；`AppHandle` 是 `Send`，在阻塞任务里取 `app.state::<AppState>()`，锁的持有范围不出闭包。

- [ ] **Step 1: 写失败测试** — 直接测服务：`list_sessions` 的 `agent_id/query/limit` 过滤；`get_session` 的 offset/limit 与 `total`；未知会话返回 `NotFound`。
- [ ] **Step 2–5:** 红 → 实现 → `cargo test` 绿 → 提交 `git commit -m "feat: 添加会话查询命令"`。

---

### Task 12: Sessions 列表与搜索 UI

**Files:** Create `hooks/useSessions.ts`、`lib/sessionGroups.ts`、`__tests__/SessionsPanel.test.tsx`、`__tests__/sessionGroups.test.ts`；Modify `panels/SessionsPanel.tsx`、`lib/api.ts`、`types.ts`、`App.tsx`（如需要）

**要点：**
- 面板挂载：只 `listSessions()` 读现有索引，**不自动全量扫描**（本机 Claude 主会话约 710 MB，首次扫描可能是分钟级）。索引为空时显示「扫描本机会话」按钮与耗时提示；有数据后顶部是「重新扫描」。扫描期间显示进度态（按钮禁用 + 文案），失败逐文件点名。
- 列表按项目 + 时间分组（`groupSessions(sessions, now)` 纯函数：项目路径为 null 归「未知项目」；组内再按 今天 / 昨天 / 本周 / 更早 分桶，桶内按 `updatedAt` 降序；项目组之间按组内最新时间降序）。
- 搜索框：输入回车或点击搜索 → `searchSessions(query)`；命中消息时显示片段与「在会话内」标记；清空恢复列表。
- agent 过滤器只列出**本次结果里出现的** provider；每个 badge 带 `verified` 状态（未实测 provider 显示「解析器未实测」）。
- 点击会话 → 打开 `SessionReader`（Task 13）；列表与阅读页在同一面板内切换，返回按钮保留搜索词。
- 空状态：列出每个 provider 的扫描根与原因（未发现文件 / 解析器未接入 / 目录不存在）。

- [ ] **Step 1: 写失败测试**

```tsx
test("first_run_offers_a_manual_scan_and_does_not_autoscan", async () => {
  // listSessions 返回空 → 显示「扫描本机会话」，且 scanSessions 一次都没被调用
});

test("sessions_are_grouped_by_project_and_time_then_sorted_by_recent_activity", () => {
  // 两个项目、三个会话；断言项目组标题是绝对路径，组内出现「今天/昨天/更早」桶，桶内最近更新在前
});

test("search_shows_the_matching_snippet_and_session", async () => {
  // api.searchSessions 返回带 snippet 的命中；断言片段与标题都出现
});

test("unverified_provider_sessions_carry_a_warning_badge", async () => {
  // provider status verified=false → 列表项出现「解析器未实测」
});

test("empty_state_names_each_provider_and_why_it_has_no_sessions", async () => {
  // 三个 provider：无文件 / 解析器未接入 / 根目录不存在
});

test("scan_failures_are_listed_not_hidden", async () => {
  // scanSessions 返回 failures → 横幅逐条显示文件与原因
});
```

- [ ] **Step 2–5:** 红 → 实现 → `pnpm test` + `pnpm build` 绿 → 提交 `git commit -m "feat: 添加 Sessions 列表与搜索"`。

---

### Task 13: 阅读页（结构化时间线 + 会话内查找）

**Files:** Create `components/sessions/SessionReader.tsx`、`SessionTimeline.tsx`、`SessionMessage.tsx`、`SessionSearch.tsx`、`__tests__/SessionReader.test.tsx`；Modify `panels/SessionsPanel.tsx`、`lib/api.ts`、`types.ts`

**要点：**
- 打开时 `getSession(agentId, sessionId, 0, 200)`；底部「载入更多」按 200 条递增，显示「已载入 X / 共 N 条」。
- 每条按 `kind` 区分渲染：`text` 用 `whitespace-pre-wrap`；`thinking` 默认折叠；`tool_call` 显示工具名 + 可展开的完整入参；`tool_result` 用 `<pre>` 折叠；`image` 用 `<img>`（data URL），坏图显示占位与原因；`event` 用灰色行。
- 工具调用与结果**全文可见**（折叠只是默认收起，展开后不截断）。内容在索引层已按 8 KiB 截断，阅读页原样显示并保留后端追加的「已截断，原文 N 字符」标记，不再做第二套截断。
- 会话内查找：`searchInSession` 返回 `{index, seq, snippet}`；结果列表点击后 `getSession(offset=index-20, limit=50)`，把目标行滚动到视口并加高亮 class。
- 顶部显示标题、agent、项目绝对路径、源文件绝对路径、更新时间；「复制恢复命令」按钮（Task 14）。
- 搜索词高亮：只对当前载入消息做文本切分渲染，不用 `dangerouslySetInnerHTML`。

- [ ] **Step 1: 写失败测试**

```tsx
test("reader_renders_thinking_tool_call_and_tool_result_as_distinct_blocks", () => {
  // 四种 kind 各一条；断言「思考」「工具调用」「工具结果」标签与工具名出现，工具结果在 <pre>
});
test("reader_paginates_without_losing_previously_loaded_messages", () => {
  // 第一页 200 条、total 300；点「载入更多」后断言首条仍在、第二页 100 条追加
});
test("in_session_search_jumps_to_the_matching_message", () => {
  // searchInSession 返回 {index: 120}；点击后断言 getSession 以 offset=100 调用，目标行有高亮
});
test("search_terms_are_rendered_as_text_not_html", () => {
  // 消息含 <script>alert(1)</script>；断言页面出现该文本、没有 script 元素
});
test("truncated_message_shows_the_backend_marker", () => {
  // content 含「已截断，原文 99999 字符」；断言标记可见
});
test("broken_image_falls_back_to_a_placeholder", () => {
  // img onError 后断言占位文案出现，页面不抛错
});
```

- [ ] **Step 2–5:** 红 → 实现 → `pnpm test` + `pnpm build` 绿 → 提交 `git commit -m "feat: 添加会话阅读页与会话内查找"`。

---

### Task 14: Markdown 导出与恢复命令

**Files:** Create `sessions/export.rs`；Modify `commands/sessions.rs`（加两个命令）、`Cargo.toml`、`capabilities/default.json`、`package.json`、`lib.rs`、`lib/api.ts`、`types.ts`、`SessionReader.tsx`

**Interfaces:**
- `sessions::export::render_markdown(&SessionDetail) -> String`；命令侧先按 `total` 取全（不受阅读页 200 条分页影响），`Image` 行只导出 `[图片：<media_type>]` 占位，不把 base64 写进 `.md`。格式固定：

```markdown
# <title>

- Agent: <display name>
- Session: <session_id>
- Project: <project path or (未知)>
- Updated: <YYYY-MM-DD HH:MM:SS>
- Source: <absolute source path>

## 1. User

正文

## 2. Assistant · Tool Call: Read

...
```

- `sessions::export::resume_info(provider, &SessionSummary) -> Option<ResumeInfo>`，`ResumeInfo { agent_id, session_id, cwd: Option<String>, command: String }`；基础命令由 `provider.resume(summary)` 给出（Pi 用源文件路径、mcode 拼平台环境变量，差异都在各自函数里）；有 `project_path` 时前置 `cd "<path>" && `。项目路径含 `"`、CR、LF 或 NUL 时返回 `None`（拒绝生成可注入的命令），UI 隐藏复制按钮并说明原因。
- 命令 `export_session_markdown(...)` 写用户选定路径，返回绝对路径；写入前不创建父目录，父目录不存在就报错。
- 前端用 `@tauri-apps/plugin-dialog` 的 `save()` 选路径；默认文件名把 `\\ / : * ? " < > |` 替换为 `_`（Windows 文件名限制），取消不调用后端；成功后显示「已导出到 <path>」。
- 插件接线的固定步骤：`Cargo.toml` 加 `tauri-plugin-dialog = "2"`；`lib.rs` 的 Builder 加
  `.plugin(tauri_plugin_dialog::init())`；`capabilities/default.json` 的 permissions 加
  `"dialog:allow-save"`；`package.json` 加 `@tauri-apps/plugin-dialog`；前端
  `save({ defaultPath: "<title>.md", filters: [{ name: "Markdown", extensions: ["md"] }] })`。
- 恢复命令用 `navigator.clipboard.writeText` 复制，并显示「已复制」。
- 导出前 UI 明示「会话可能包含密钥与个人信息，导出文件不受本项目保护」。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn markdown_export_keeps_order_and_roles() {
    // 构造 User → Thinking → ToolCall → ToolResult 四条；断言标题、元信息行、编号与 kind 标签按顺序出现
}

#[test]
fn resume_command_includes_the_project_directory_only_when_known() {
    assert_eq!(resume_info(&claude(), &session_with_project("D:/x")).unwrap().command,
               "cd \"D:/x\" && claude --resume s1");
    assert_eq!(resume_info(&claude(), &session_without_project()).unwrap().command,
               "claude --resume s1");
}

#[test]
fn a_project_path_that_could_break_out_of_the_command_is_refused() {
    assert!(resume_info(&claude(), &session_with_project("D:/x\" && rm -rf /")).is_none());
}
```

```tsx
test("export_asks_for_a_destination_and_reports_the_path", () => {
  // mock save() 返回 "D:/out/a.md"；断言 exportSessionMarkdown 收到该路径并显示「已导出到」
});
test("cancelling_the_save_dialog_does_not_call_the_backend", () => {
  // save() 返回 null；断言 exportSessionMarkdown 没有被调用
});
test("resume_command_is_copied_and_confirmed", () => {
  // mock clipboard.writeText；断言写入完整命令并显示「已复制」
});
```

- [ ] **Step 2–5:** 红 → 实现 → `cargo test` + `pnpm test` + `pnpm build` 绿 → 提交 `git commit -m "feat: 添加会话导出与恢复命令"`。

---

### Task 15: 端到端验证（沙箱 + 真机只读）

- [ ] **Step 1: 全量门禁** — `pnpm test`、`pnpm build`、`cargo test`、`cargo build`。
- [ ] **Step 2: 沙箱 E2E** — `src-tauri/tests/sessions_e2e.rs`：临时 home 下为**全部 11 个 provider** 铺最小合成样本（有真实样本的四个按真实字段形状，其余按 cc-switch 代码级事实）；扫描 → 列表 → 搜索（中文/短词/特殊字符）→ 阅读 → 导出到临时路径 → 恢复命令；手改文件后重扫只更新该会话；删除文件后索引清空；重复扫描幂等；全程真实家目录哈希不变。
- [ ] **Step 3: 真机只读核对（四个有样本的 provider）** — 扫描前记录 `~/.claude`、`~/.codex`、`~/.pi`、`~/.qoder-cn` 的文件数/总大小与关键配置文件哈希（不逐字节哈希 1.4 GB 会话目录）；启动应用，Sessions 面板对真实本机扫描；逐项确认：Claude / Codex / Pi / Qoder CN 的会话条数与扫描报告里的来源文件数一致，不一致的逐条解释（坏文件/重复 ID/子代理跳过）；只记数量，不记内容；搜索一个中文词；打开一个真实会话；导出到临时目录；复制恢复命令。截图留档，不把内容写进仓库或日志。`Gemini`/`Grok`/`Hermes`/`Mcode`/`OpenClaw`/`OpenCode`/`Agy` 本机无样本，只做合成样本测试，不参与真机核对。
- [ ] **Step 4: 性能记录** — 记录首次全量扫描耗时与 `index.db` 大小。若 `index.db` > 500 MB 或首次扫描 > 3 分钟，把 `INDEX_CONTENT_CAP` 降到 4 KiB 后重测，并把数字写进执行记录。
- [ ] **Step 5: 升档结论** — 只把 Claude / Codex / Pi / Qoder CN 标 `verified=true`；Gemini 与其余六类保持 `false`，在计划执行记录里写清原因（本机无样本，只有代码级映射）。
- [ ] **Step 6: 清理与提交** — 删除临时目录与导出文件，确认 `~/.claude`、`~/.codex`、`~/.pi`、`~/.qoder-cn` 的文件数/总大小与关键配置文件哈希未变；更新 `THIRD_PARTY_NOTICES.md` 的 cc-switch 行（补「会话 provider 的路径与字段映射」这一项借用）；`git commit -m "test: 补充会话浏览的端到端验证记录"`。

---

## 自检

- **Spec 覆盖**：
  - 3.2 的 Sessions 面板（列表/分组/搜索/阅读/导出/恢复）→ Task 12–14；
  - 4.4 的 `sessions` / `session_messages` / FTS → Task 10；
  - 5.2 的 11 个会话 provider → Task 2–9；
  - 9 的 `sessions` 模块（解析、统一模型、FTS、导出、恢复命令）→ Task 1–14；
  - M3 验收（真实会话可用、增删文件后索引更新、同步包不含会话）→ Task 15；
  - C1/C2/C3 → 「事实来源与置信度」与 Task 15 的升档限制。
- **显式推迟/缩小**：
  - Qoder 国际版会话（C2 本机无样本）不实现，注册表里没有 `qoder`。
  - Qoder CN 只读 `projects/**/*.jsonl` 主会话；`state.json`、SQLite `local.db`、CLI task execution、旧式缓存不读。
  - Gemini / Grok / Hermes / Mcode / OpenClaw / OpenCode / Agy 本机无样本，解析器按 cc-switch
    代码级事实实现并标 `verified=false`；Hermes 的 WSL（`\\wsl$`）路径跳过并报告。
  - 删除会话、在终端直接恢复、文件监听自动重扫、工具全文按引用懒加载都不在本计划（前两项
    spec 明确非目标，后两项属 Plan 5 / 后续优化）。
  - 索引里的消息内容上限 8 KiB、图片 data URL 上限 512 KiB；全文搜索覆盖的是截断后的内容。
    这是为 710 MB 真实会话规模做的取舍，阅读页会显式标注「已截断」。
- **类型一致性**：`ParsedSession.warnings`、`SessionWarning`、`SessionScanReport.warnings`、
  `SessionFileWarning` 在 Task 1/10 中同名同字段；`SessionProviderStatus.parse_ready` 与
  `SessionProvider.parse: Option<fn>` 一一对应；`SessionFilter::all()`、`INDEX_CONTENT_CAP`
  在 Task 10 定义并被测试引用；前端 `SessionSummary` / `SessionDetail` / `SessionMessage` /
  `SessionSearchHit` / `SessionMatch` 与后端 `serde(rename_all="camelCase")` 对齐。
- **Review Focus 对应测试**：
  1. 坏行 → Task 2/3 的坏行测试 + Task 10 `a_broken_line_does_not_lose_the_rest_of_the_session`；
  2. 重复 ID → Task 10 `duplicate_session_ids_keep_the_newer_and_name_the_other`；
  3. 删文件 → Task 10 `a_deleted_session_file_disappears_from_the_index`；
  4. 中文/短词/特殊字符 → Task 10 `chinese_and_two_character_queries_are_safe`；
  5. 隐私 → Task 10 `scanning_never_copies_session_content_into_the_store` + Task 14 的导出确认；
  6. 幂等 → Task 10 `scanning_is_idempotent_and_only_reparses_changed_files`；
  7. 超长会话/截断 → Task 10 `oversized_content_is_truncated_before_it_enters_the_index` +
     Task 13 `reader_paginates...` / `truncated_message_shows_the_backend_marker`；
  8. 图片降级 → Task 2 大图占位 + Task 13 `broken_image_falls_back_to_a_placeholder`；
  9. 覆盖目录 → Task 1 `base_override_replaces_the_home_anchor_for_every_root`；
  10. 未知记录 → Task 3 `unknown_codex_records_are_counted_not_fatal` + Task 7 的未知 step 测试。
- **风险**：本机 291 个 Claude 主会话约 710 MB，首次扫描与 FTS 建库是唯一可能拖慢体验的环节；
  Task 15 用实测耗时与 `index.db` 大小决定是否把上限从 8 KiB 降到 4 KiB。无真实样本的七个
  provider 只能证明「按代码级事实映射正确」，不能证明「该工具真的这样写文件」；UI 必须如实
  显示「解析器未实测」。