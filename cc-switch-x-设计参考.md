# cc-switch-x 设计参考:以 cc-switch 为底座的「多 Agent 配置统一客户端」

> 定位:**参考资料 / 立项依据**。目标只有一个 —— 做一个客户端,把散在各 AI 编码 Agent 里的
> **提示词、MCP、Skills 和各自的配置**收敛到一处定义、多处下发,替代 cc-switch 的这部分能力。
> cc-switch 的其他功能(provider 切换、本地路由、故障转移、用量、会话、账号、云同步)**全部不做**。
>
> 本文档事实来源:cc-switch / vercel-labs-skills / teamai-cli 的 GitHub 仓库树与 README(2026-10-22 用 `gh api`
> 实查)、本机 `C:/Users/Administrator/.qoder-cn` 与 `.qoder` 目录实测、`qoderclicn {mcp,skills,agents,hooks} --help`
> 实测输出、[Qoder CLI 官方文档](http://docs.qoder.com/zh/cli/settings-reference)(仅拿到搜索摘要,见 §10)。
> 每条结论后面标了验证状态。

---

## 0. 验证状态图例(全文适用)

| 标记 | 含义 |
|---|---|
| `已验证` | 看过仓库树 / README 原文 / 本机文件系统,可直接引用 |
| `文档所述` | 上游 README 这么写,实现细节没读源码,**别当事实用** |
| `未验证` | 推断,需要读代码或实测才能落地 |
| `待确认` | 缺信息或需拍板,本文档故意留空,不要猜 |

---

## 1. 立项前提:为什么要在 cc-switch 上做减法

| 事实 | 状态 |
|---|---|
| cc-switch 是 MIT 授权,Rust + Tauri 2,Win/mac/Linux 桌面 App,147k★,最近推送 2026-10-21,默认分支 `main` | 已验证(`gh api repos/farion1231/cc-switch`) |
| 它的 Prompts / MCP / Skills 三块面板确实是「一处定义 → 逐工具开关下发」的形态 | 文档所述(README「MCP, Prompts & Skills」小节) |
| cc-switch **不支持 Qoder / Qoder CN**:全仓库树 grep `qoder` = **0 处** | 已验证 |
| 官方态度:Qoder CLI 在 tracker issue #1855 里状态是 📋 Requested(16 赞,#5112 open);早期 #742 回复「暂时不在计划内」,#3072 / #4612 被关成 duplicate | 已验证(issue 正文与 `state_reason`) |
| 结论:**给 cc-switch 提 Qoder 适配 PR 大概率不被收**,只能 fork 自维护 → fork 的动机成立 | — |

另一个动机:cc-switch 的功能重心在「provider / 模型接入 + 用量 + 会话 + 云同步」,配置管理只是其中三块面板。
我们只要那三块。减法本身能砍掉绝大部分代码量(见 §3.2 量化)。

---

## 2. 目标与非目标

### 2.1 目标(M1–M4 全部要满足)

1. **三类资产、一处定义**:MCP server、Skill(含 skill 目录/仓库来源)、Prompt(全局提示词文件)。
2. **多 Agent 矩阵**:每个资产 × 每个已接入工具 = 一个同步开关;打开即写入该工具的原生落点,关闭即撤销。
3. **可提交的单一真源**:真源在本机一个目录 + 一个 SQLite 库,能整体导出、换机导入、git 化(可选)。
4. **不静默改文件**:任何写入前先备份,任何漂移(工具自己改了配置)先报告再动手。
5. **接入 Qoder CN CLI(一等目标)与 Qoder / Qoder IDE** —— 这是 fork 存在的核心理由,属于目标而非附加项。
   CLI 与 IDE 是否在矩阵里合成一行,取决于 A2b 的结论(两者的配置面读法可能不同)。
6. Windows 优先(与本仓库 `apps/desktop` 同平台),mac/linux 顺带。

### 2.2 明确不做

| 不做什么 | 理由 |
|---|---|
| Provider / API Key / 模型切换、本地路由网关、配额故障转移 | 是 cc-switch 的主战场,与我们要解决的问题正交;需要时用 magpie / 原 cc-switch |
| 用量与成本统计、Token 看板 | 同上 |
| Sessions 阅读视图、会话恢复 | 与配置无关 |
| 账号登录(Copilot / ChatGPT / xAI OAuth)、订阅余额 | 与配置无关 |
| WebDAV / S3 云同步、托盘切换、开机自启、自动更新 | 首版用「导出文件」代替;更新另议 |
| Projects 整套快照切换 | 属于 provider 维度的组合,首版不做 |
| Deep Link(`ccswitch://`)全量移植 | 只保留 MCP/Skill/Prompt 三类导入链路,provider 链路删 |

> 非目标要写进 README 开头,否则 fork 会被人反复问「为什么不加 provider」。

---

## 3. 底座能力盘点:哪些留、哪些拆

以下路径来自 `gh api repos/farion1231/cc-switch/git/trees/main?recursive=1`(共 1838 条),状态 = 已验证存在;
「作用」列除注明外均属 **未验证**(只看了文件名与 README,没读实现),落地前先读源码确认。

### 3.1 必须保留 / 改造的内核

| 路径 | 文件数 | 作用(待读码确认) |
|---|---|---|
| `src-tauri/src/app_config.rs` | 1 | `enum AppType` 工具注册中心,**加一个 agent 从这张表开始**(gh code search 命中) |
| `src-tauri/src/mcp/` | 11 | 逐工具 MCP 写入适配:`mod.rs` `claude.rs` `codex.rs` `gemini.rs` `agy.rs` `grokbuild.rs` `hermes.rs` `mcode.rs` `opencode.rs` `pi.rs` `validation.rs`。**Qoder 适配主要落在这里** |
| `src-tauri/src/claude_mcp.rs`、`gemini_mcp.rs` | 2 | 早期单文件形态的 MCP 读写(与 `mcp/` 的关系未验证) |
| `src-tauri/src/prompt.rs`、`prompt_files.rs`、`services/pi_prompt_files.rs` | 3 | Prompt 库与「写到工具提示词文件」的落地逻辑 |
| `src-tauri/src/commands/{mcp,prompt,skill}.rs` | 3 | 三个面板的 Tauri command 入口 |
| `src-tauri/src/database/` | 20 | SQLite:`mod.rs` `schema.rs` `migration.rs` `backup.rs` `dao/{mcp,prompts,skills}.rs`。**真源的持久层** |
| `src-tauri/src/services/backup_storage.rs` | 1 | 改写配置文件前的备份(README 称「首次改写前自动备份」,属文档所述) |
| `src-tauri/src/commands/{settings,import_export,misc,env}.rs` | 4 | 设置、导入导出 |
| `src/components/{mcp,skills,prompts}/` | 6 / 8 / 10 | 三个面板的 UI |
| `src/components/{JsonEditor,MarkdownEditor,ConfirmDialog}.tsx`、`src/components/ui/`、`src/components/common/`、`src/components/shell/` | — | 编辑器与外壳 |
| `src/hooks/useBackupManager.ts`、`useDirectorySettings.ts`、`useDragSort.ts`、`useDebouncedValue.ts`、`useDarkMode.ts`、`useGlobalProxy.ts` | — | 前四个直接相关;`useGlobalProxy` 属网络层,可能连带 provider → 待确认能否摘干净 |

### 3.2 建议整块拆掉的

| 路径 | 文件数 | 处理 |
|---|---|---|
| `src-tauri/src/proxy/` | **75** | 本地路由网关,全删 |
| `src/components/providers/` | **91** | Provider 面板,全删 |
| `src/components/sessions/`、`usage/`、`quota/`、`profiles/`、`workspace/`、`proxy/` | 30 / 24 / 4 / 3 / 3 / 1 | 全删 |
| `src-tauri/src/commands/` 中的 `provider.rs` `failover.rs` `subscription.rs` `balance.rs` `coding_plan.rs` `stream_check.rs` `model_fetch.rs` `session_manager.rs` `usage.rs` `omo.rs` `copilot.rs` `codex_oauth.rs` `xai_oauth.rs` `agy_account.rs` `plugin.rs` `tool_uninstall.rs` `tool_node_requirement.rs` `s3_sync.rs` `webdav_sync.rs` `custom_icon.rs` `deeplink.rs` `workspace.rs` `profile.rs` `lightweight.rs` | 约 22 / 40 | 删除,`commands/mod.rs` 同步摘注册 |
| 各工具 provider 读写:`{agy,app,claude_desktop,codex,gemini,grok,hermes,mcode,openclaw,opencode,pi}_config.rs` | 11 | **不能整删** —— 里面同时有 provider 字段和 MCP/prompt 字段,要拆成「只保留 MCP/prompt 相关读写」的薄适配器。这是删代码时最容易翻车的一处,必须先读码分诊 |
| `src-tauri/src/{codex_state_db,codex_history_migration,codex_rollout_file,agy_smoke_tests,app_store,auto_launch,claude_plugin}.rs` | 7 | 会话/历史/插件商店相关,基本可删(`claude_plugin.rs` 与 skill 安装可能重叠 → 待确认) |
| `src-tauri/src/deeplink/` | 11 | 保留 `mcp.rs` `skill.rs` `prompt.rs` `parser.rs` `mod.rs` `utils.rs`;删 `provider.rs` `codex_link.rs` `grok_link.rs` `icon_fetch.rs` |

**量化结论(已验证计数)**:Rust command 层可删约 55%(22/40),前端 provider 面板 + 会话 + 用量 + 配额 + 档案 + 工作区
共约 156 个文件可删,网关层 75 个文件可删。剩下的核心就是 `app_config` + `mcp/` + prompt + skills + database + backup +
三个面板,外加必须拆诊的 11 个 `*_config.rs`。

> ⚠ 拆 provider 的连锁风险:数据库 `schema.rs` / `migration.rs` 里 provider 表和 MCP/skill/prompt 表大概率同库同 migration
> 链。**已发布过的 migration 只追加不修改**(与本仓库 `apps/desktop/db` 同一纪律),fork 首版建议:保留空表不写数据,
> 而不是删表 —— 删表会让从 cc-switch 导入的老用户数据文件报错。

---

## 4. 加一个 Agent 到底要写什么(fork 的主循环)

一个 Agent 适配器 = 六个落点问题 + 一个存在性判定。

| # | 要回答的问题 | 例(Claude Code) | **Qoder CN CLI**(实测 + 官方文档,详见 §4.3) |
|---|---|---|---|
| 1 | 全局配置根目录 | `~/.claude/` | `~/.qoder-cn/`(本机 `C:/Users/Administrator/.qoder-cn/`)已验证 |
| 2 | 项目级配置根目录 | `.claude/` + 项目 `.mcp.json` | `<项目>/.qoder/settings.json` + `<项目>/.mcp.json`(文档所述;本仓库根目前无 `.qoder*` 目录,已验证) |
| 3 | 提示词文件 | `CLAUDE.md` / `AGENTS.md` | **`~/.qoder-cn/AGENTS.md`**(6940 字节,已验证;`/init` 生成、`/memory` 编辑) |
| 4 | MCP server 落点与格式 | `~/.claude.json` / `.mcp.json`(JSON `mcpServers`) | **`~/.qoder-cn/settings.json` 顶层 `mcpServers` 字段**(文档所述;本机顶层无该字段 = 用户级 0 个 server,`mcp` 分组下只有 `enableAllProjectMcpServers`,与文档一致) |
| 5 | Skills 目录 | `~/.claude/skills/` | `~/.qoder-cn/skills/`(已验证存在,当前为空)+ 项目级 `.qoder/skills/` |
| 6 | 自定义命令(同样是提示词资产) | `.claude/commands/*.md` | `~/.qoder-cn/commands/*.md`、项目级 `.qoder/commands/`(文档所述;**本机该目录不存在**,即尚未使用) |
| 7 | 存在性判定 | 目录存在即已安装 | vercel-labs/skills 用 `existsSync(join(home, '.qoder-cn'))`;本机另有 `qodercn.cmd` / `qoder-cn.cmd` / `qoder.cmd` 三个入口(已验证,归属差异见 §6 A11) |

### 4.1 Skills 落点参照表(可直接抄)

来自 `vercel-labs/skills` 的 `docs/agents.md`(已验证原文,**含 Qoder 两条**)。这张表是目前最省事的「工具 → skills 目录」
事实来源,比逐家读官方文档快:

| Agent | `--agent` | 项目路径 | 全局路径 |
|---|---|---|---|
| Claude Code | `claude-code` | `.claude/skills/` | `~/.claude/skills/` |
| Cline / Codex / Kimi Code CLI / Pi / Warp / Zed | 同名 | `.agents/skills/` | `~/.agents/skills/` |
| Cursor | `cursor` | `.agents/skills/` | `~/.cursor/skills/` |
| Gemini CLI | `gemini-cli` | `.agents/skills/` | `~/.gemini/skills/` |
| GitHub Copilot | `github-copilot` | `.agents/skills/` | `~/.copilot/skills/` |
| **Qoder** | `qoder` | `.qoder/skills/` | `~/.qoder/skills/` |
| **Qoder CN** | `qoder-cn` | `.qoder/skills/` | `~/.qoder-cn/skills/` |
| Trae / Trae CN | `trae` / `trae-cn` | `.trae/skills/` | `~/.trae/skills/` / `~/.trae-cn/skills/` |
| Windsurf | `windsurf` | `.windsurf/skills/` | `~/.codeium/windsurf/skills/` |
| CodeBuddy / Lingma / Kiro CLI / iFlow CLI / Qwen Code | 各自 | `.codebuddy|.lingma|.kiro|.iflow|.qwen/skills/` | 同前缀加 `~/` |

两条要点:
- **`~/.agents/skills/` 正在成为通用约定**(Amp / Replit / Universal 共用它;cc-switch README 也写了「可选存到 `~/.agents/skills`」,
  属文档所述)。→ fork 的真源目录就定在 `~/.agents/skills/`,各 agent 目录用**链接**指过去,别复制。
- **Windows 建符号链接需要权限或开发者模式**,某些环境直接失败。→ 必须实现三级降级:
  `symlink → junction(目录) → 复制 + 清单校验`,并在 UI 上如实显示用的是哪一种。此约束是 Windows 平台事实,
  降级链的具体实现细节 **未验证**,要在 M2 实测。

### 4.2 与上游的分工

`npx skills` 已经把「skill 的**安装/更新/来源解析**」做成了生态(skills.sh 注册表、`add/find/list/update/remove`,支持 79 个 agent)。
**不要重写这一层**。fork 只做「本机已有 skill → 各 agent 的启用矩阵」,以及「从 skills.sh/GitHub 装一次、多处生效」的编排。
如果上游 CLI 能直接当子进程调用,优先调用而不是自己实现一套来源解析。

### 4.3 Qoder CN CLI 适配专章(本 fork 的一等目标)

> 这一节决定 fork 的核心卖点能不能落地。可执行名 `qoderclicn`(`qodercn.cmd` → `~/.qoder-cn/entry/qodercn.cmd`),
> 命令输出为**本机实测**(`qodercn <sub> --help`,2026-10-22);配置分层与字段语义来自
> [docs.qoder.com/zh/cli/settings-reference](http://docs.qoder.com/zh/cli/settings-reference)、
> [mcp-reference](http://docs.qoder.com/zh/cli/mcp-reference)、[settings](http://docs.qoder.com/zh/cli/settings)
> 与 [docs.qoder.cn/cli/commands](https://docs.qoder.cn/cli/commands)(**文档所述**,WebFetch 被限,只拿到搜索摘要,落地前逐页复核)。

#### (1) Qoder CN CLI 的可管理面比「三类资产」更宽

cc-switch 的面板是 MCP / Skills / Prompts 三块。Qoder CN CLI 的可管理面更宽,fork 要么显式支持、要么显式声明不做:

| 资产 | 落点 | 官方管理入口(实测) |
|---|---|---|
| MCP server | `settings.json` 顶层 `mcpServers`(用户/项目/本地三档) | `qoderclicn mcp add \| add-json \| remove \| get \| list \| enable \| disable \| reset-project-choices` |
| Skills | `~/.qoder-cn/skills/`、项目 `.qoder/skills/` | `qoderclicn skills list \| enable \| disable \| install <source>` |
| Subagents | `agents`(受 `--setting-sources user,project,local` 过滤) | `qoderclicn agents list [--setting-sources ...]`、TUI `/agents` |
| Hooks | `settings.json` 顶层 `hooks` | `qoderclicn hooks migrate`(从 Claude Code 迁移;**其余子命令我截断没看全,待确认 A10**)、TUI `/hooks` |
| 提示词(全局) | `~/.qoder-cn/AGENTS.md` | TUI `/init`、`/memory` |
| 提示词(命令) | `~/.qoder-cn/commands/*.md`(Prompt 类型命令可在 headless 跑:`qodercn -p '/git-commit'`) | TUI `/commands` |
| 插件(打包以上全部) | 插件目录 `.qoder-plugin/plugin.json` + `commands/` `agents/` `skills/` `hooks/` `bin/` `.mcp.json`;启用状态在 `settings.json` 顶层 `enabledPlugins` | `qoderclicn plugins`(别名 `plugin`)、`--plugin-dir`、TUI `/plugins` `/marketplace`;本机已有 `~/.qoder-cn/plugins/{cache,data,marketplaces,known_marketplaces.json}` |

#### (2) 关键设计决定:**能走 CLI 就不改文件**

cc-switch 的写入模型是「直接读写各工具的 JSON/TOML」。对 Qoder CN CLI,更稳的顺序是:

1. **首选官方子命令**(`mcp add-json --scope user`、`skills install`、`mcp remove`):语义由官方保证,`enable/disable` 之类的
   状态位即使存在私有存储里也会被正确处理;
2. **退路才是直接改 `settings.json`**,且必须 **按键级合并** —— 见下条;
3. **读侧用 `mcp list` / `mcp get` / `skills list` / `agents list` 做「生效态」校验**:UI 显示的是 CLI 认账的结果,
   不是「我以为写进去了」。这一条同时就是 §5.3 漂移检测的现成探针,成本极低。

#### (3) `settings.json` 是共享文件 —— 按键级合并,禁止整份覆盖

本机实测 `settings.json` 顶层键:`security`、`permissions`、`mcp`、`model`、`statusLine`(同目录另有 `AGENTS.md`、
`argv.json`、`state.json`、`skill-usage.json`、`umid-cache.json`、`installation_id` 等文件,与本次改动无关)。
也就是说同一份文件里同时装着**权限规则、安全策略、模型选择、状态栏** —— 这些是用户的既有配置,不是我们的资产。

> ⚠ 红线:适配器**只允许增删自己管辖的键**(MCP 只碰 `mcpServers.<name>`;若把 Hooks 纳入范围,才额外碰顶层 `hooks`
> 子树),其余键一律不动。写入前反序列化 → 改子树 → 序列化,未知键原样保留。整份写穿会毁掉用户的 `permissions`,
> 属于不可接受的破坏。(对照本仓库教训:`sync_local_state` 是整份写穿,已知陷阱 §3 专门警告过。)

#### (4) 生效前提与作用域语义(必须显示给用户)

- **`mcpServers` 改动需重启**(文档标注「需重启」);会话内可用 `/mcp reload`(别名 `/mcp refresh`)重新发现。
  → UI 落盘后要提示「需重启或执行 /mcp reload」,不能报「已生效」。
- **scope 三档且默认是 `local`**:`mcp add --scope <user|local|project>`,默认值为 **local**(实测 help 原文)。
  我们做「用户级统一分发」必须**显式传 `--scope user`**,否则配置只会落在当前项目的本地级。
- **项目级 server 默认逐个批准**:覆盖顺序 用户级 → 项目级 `settings.json` → 项目级 `.mcp.json` → 本地级 → CLI 参数;
  `mcp.enableAllProjectMcpServers: true` 自动全批准,`mcp.enabledProjectMcpServers: ["name", ...]` 按名白名单。
  本机 `mcp` 分组里正是 `enableAllProjectMcpServers` —— **这是用户自己的安全开关,fork 不得擅自改写**(不擅自改用户可感知行为)。
  `mcp reset-project-choices` 可以清批准记录,属于高风险操作,要做二次确认。
- 其他相关开关(文档所述):`mcp.allowed` / `mcp.excluded`、`mcp.lazyLoad` 或 `QODER_MCP_LAZY=1`(懒加载只暴露
  `mcp_list`/`mcp_get`/`mcp_call` 三个 meta 工具)、启动参数 `--mcp-config`、`--strict-mcp-config`、`--allowed-mcp-server-names`。
  → **懒加载会影响我们的「生效态校验」**:开了 lazyLoad 时 `mcp list` 的语义可能不同,待确认。
- transport 支持 `stdio | sse | http | ws`(实测 help),凭据形式 `-e KEY=value`、`-H "X-Api-Key: v"`、
  `--headers-helper <script>`。→ 对应 §5.3 第 4 条:值不进真源,`-e`/`-H` 用引用形式在 apply 时解析。

#### (5) 与「插件」机制的关系(建议 M4 之后再看)

Qoder 的 Plugin 机制本身就是「commands + agents + skills + hooks + `.mcp.json` 打成一个目录,装完自动发现」。
如果目标是「一次定义、多处生效」,那么**给 Qoder 侧生成一个插件包**可能比逐条写 settings 更符合官方语义
(卸载干净、启用状态由 CLI 管)。代价是粒度粗(整包启用/禁用),与 cc-switch 那种「逐资产 × 逐工具开关」的矩阵不同型。

> 取舍(推荐):**M2 先走 `mcp add-json --scope user` + skills 目录软链/复制**,把矩阵跑通;
> 「生成插件包」作为 M5 的可选升级路径,等矩阵语义稳定再评估。不要在 M2 同时做两套写入模型。

---

## 5. 数据模型与核心机制

### 5.1 真源目录(建议)

```
~/.ccswitchx/
  assets.db            # SQLite:资产与同步矩阵(唯一真源)
  mcp/servers.json      # 规范化 MCP 定义,不含工具专属格式
  prompts/<id>.md       # 提示词正文,工具无关
  skills/               # → 实际真身放 ~/.agents/skills/,这里只做引用
  backups/<ts>/...      # 每次 apply 前的原文快照
  drift.log             # 漂移检测结论
```

**渲染,不 fork**(思路来自 tackroom,非 cc-switch 原有设计):`mcp/servers.json` 是唯一一份,按目标工具渲染成
Codex 的 TOML、Claude 的 JSON、opencode 的片段……**工具专属格式一律生成,不手改**。这样新增 agent 不改动已有资产。

### 5.2 SQLite 表(草案,字段从简)

| 表 | 关键字段 | 说明 |
|---|---|---|
| `tools` | `id, display_name, config_root, prompt_file, mcp_target, skills_dir, commands_dir, agents_target, write_kind(cli/file/plugin_bundle), needs_restart, detect_kind, enabled` | 即 `AppType` 的持久化视图;`write_kind` / `needs_restart` 是 §5.3 第 6 条的依据 |
| `mcp_servers` | `id, name, transport, command, args_json, env_keys_json, url, enabled` | `env_keys_json` 只存**变量名**,不存值 |
| `mcp_sync` | `server_id, tool_id, state(on/off/paused)` | 矩阵 |
| `prompts` | `id, title, body_md, tool_id(nullable=通用), updated_at` | 工具无关的一份 |
| `prompt_active` | `tool_id, prompt_id` | 每工具最多一个激活项;写入前先把磁盘现内容抓回库,别丢用户手写内容 |
| `skills` | `id, source_kind(repo/local/npm), source_ref, canonical_path, link_kind(symlink/junction/copy)` | `link_kind` 是实测结果,不是配置 |
| `skill_sync` | `skill_id, tool_id, state` | 矩阵 |
| `apply_runs` | `id, started_at, dry_run, plan_json, result, backup_dir` | 每次落盘的账 |
| `drift` | `asset_id, tool_id, detected_at, kind, resolution` | 漂移记录 |

### 5.3 七条硬机制(设计约束,不是可选项)

1. **apply 必须幂等 + 先 dry-run**。`apply(plan)` 返回 diff,UI 确认后落盘;重跑同一计划结果不变。
2. **落盘前备份原文**到 `backups/<ts>/`,且备份失败则中止整批(不能「备好了才写」)。
3. **漂移检测**:定时或手动 `rescan()`,比对「渲染期望」与「磁盘实际」。发现不一致 → **报告并让用户选**:采纳磁盘 / 重刷真源。
   禁止静默覆盖(agentsync 的 chezmoi 式双向 merge 是这类工具最正确的形态,值得抄)。
4. **密钥不进真源**:MCP 的 `env` 值只存变量名或 `${secret:x}` 引用,渲染时才解析;导出的 zip / git 仓库里不得出现明文值。
   (与本项目红线一致:`apps/desktop` 客户端不持有密钥的逻辑同理。)
5. **逐项结果如实返回**:一次 apply 涉及 N 个工具 × M 个资产,任一失败不能报整体成功 —— 本仓库 `core-publish`
   刚踩过「按整体结果掩盖逐项失败」的坑,同一纪律适用于这里。
6. **写入通道分级 + 生效态以工具自己认账为准**:每个工具的适配器声明 `write_kind = cli | file | plugin_bundle`;
   `apply` 之后必须用官方只读命令回读校验(`qoderclicn mcp list` / `skills list` / `agents list`、`claude mcp list` ……),
   UI 上区分「已写入」与「已生效」,并显示是否需要重启或 `/mcp reload`。
7. **共享文件按键级合并**:配置根文件(尤其 `settings.json` 这类同时装权限/模型/状态栏的文件)只做
   反序列化 → 改自己管辖的子树 → 序列化,**未知键原样保留**,禁止整份写穿;写失败即回滚到本次备份。

### 5.4 命令面(Tauri command 草案)

```
list_tools / detect_tools
upsert_mcp / set_mcp_sync / preview_apply / apply / revert_last
upsert_prompt / activate_prompt / import_prompt_from_tool
discover_skills / install_skill / set_skill_sync / refresh_skill_matrix
scan_drift / resolve_drift
export_bundle / import_bundle
```

前端只有 4 个页面 + 1 个设置:**Agents(工具矩阵)/ MCP / Skills / Prompts / Settings**。首屏即「工具 × 资产」矩阵视图,
而不是 cc-switch 的「每工具一页 + 侧栏三面板」——矩阵才是一眼看清同步状态的正确交互(取舍:矩阵在资产多时密度高,
需要搜索与按工具过滤,见本仓库红线 5「信息密度高是布局问题」的同一思路)。

---

## 6. 待确认清单(动代码前必须逐条钉死,禁止拍板)

| # | 问题 | 已知的实测线索 | 怎么确认 |
|---|---|---|---|
| ~~A1~~ | ~~Qoder CN 的 user 级 MCP server 落在哪个文件~~ → **已解决(2026-10-22)**:`settings.json` **顶层 `mcpServers`** | 官方 settings-reference / mcp-reference 均写 `mcpServers` 为顶层项,`mcp` 分组只放开关;本机 `settings.json` 顶层键为 `security/permissions/mcp/model/statusLine`(无 `mcpServers` = 用户级 0 条,与文档一致),`mcp` 下只有 `enableAllProjectMcpServers`;`qoderclicn mcp add --scope user|local|project` 实测存在 | **仍需在真机跑一次写操作复核**:`qodercn mcp add-json --scope user` 后 diff `settings.json`,确认落在顶层且不动其他键 —— 这一步没跑之前,§4.3 的字段位置算「文档所述」 |
| A2 | 项目级配置的读取行为:`<项目>/.qoder/settings.json` 与项目根 `.mcp.json` 是否真被 CLI 读取;同名覆盖顺序是否确为「用户级 → 项目 settings → 项目 .mcp.json → 本地级 → CLI 参数」 | 文档所述;mcp 子命令有 `reset-project-choices`(针对 project-scoped `.mcp.json`),侧面印证项目级机制存在,已验证命令面 | 在临时目录造一个 `.mcp.json`,`qodercn mcp list` 与 `mcp get <name>` 对照 |
| A2b | **Qoder IDE(桌面版)与 CLI 的配置面是否同一份文件** —— 若 IDE 不读 CLI 的项目级路径,矩阵里就必须把「Qoder IDE」与「Qoder CN CLI」当两个独立工具,不能合并 | 官方 connectors 文档称 IDE 自定义 MCP 存 `~/.qoder/settings.json` 的 `mcpServers`;另有一篇第三方问答称桌面版 MCP 管理界面不区分用户/项目级、不读项目根 `.mcp.json`(**未验证,可信度中**) | 真机在 IDE 里加一个 server,看写进哪个文件;再在 IDE 里看项目级条目是否显示 |
| A3 | Qoder 是否读 `~/.qoder-cn/AGENTS.md` 作为全局提示词,与项目 `AGENTS.md` 的优先级 | 文件存在已验证,语义未验证 | 真机双向验证(写一句只有全局能看到的指令) |
| A4 | skill 在 Qoder CN 的 frontmatter 要求(与 `SKILL.md` 标准差异) | `~/.qoder-cn/skills/` 为空,无样本可对照 | 放一个最小 skill 实测加载 |
| A5 | `*_config.rs` 里 provider 与 MCP/prompt 代码的耦合度 | 文件清单已验证,内部结构未读 | 读 11 个文件,先画依赖图再删 |
| A6 | 从 cc-switch 老用户配置导入的可行性(SQLite schema 兼容性) | schema 文件位置已知,表结构未读 | 读 `database/schema.rs` |
| A7 | Windows 上 symlink / junction / copy 三降级的真实表现 | 平台约束是常识,具体到各 agent 目录未测 | M2 建实验用例 |
| A8 | magpie 是否也管 MCP/Skills(它已支持 Qoder,但只管模型) | README 主线只讲模型;`internal/qoder/endpoints.go` 存在已验证 | 读 magpie 文档,避免重复造 |
| **A9** | `qoderclicn skills enable/disable` 的**启用状态存在哪** —— 若在 `settings.json` 之外的私有存储,我们直接复制文件到 `skills/` 的做法可能与 CLI 的启用态不一致(矩阵显示「已同步」而 CLI 显示 disabled) | 命令面已验证(`skills list/enable/disable/install`);本机 `~/.qoder-cn/skill-usage.json`(231 字节)与 `settings.json` 里未见 enable 字段 | 装一个 skill → disable → diff `settings.json`/`skill-usage.json`/目录,找状态位 |
| A10 | `qoderclicn hooks` 的完整子命令(实测 help 只截到 `migrate`)与 hooks 写入口 | 部分已验证(`hooks migrate` 从 Claude Code 迁移) | `qoderclicn hooks --help` 全量输出;`/hooks` TUI 走一遍 |
| A11 | **三个入口的归属**:`qodercn.cmd`(→ `~/.qoder-cn/entry/`)、`qoder-cn.cmd`(→ `D:/Program Files/QoderCN/bin/`)、`qoder.cmd`(→ `AppData/Local/Programs/Qoder/bin/`)分别对应 CN CLI / CN 旧装 / 国际版还是 IDE 附带?各自的配置根与版本可能不同,决定矩阵里到底要放几个「Qoder」条目 | 三条路径均已验证(`Get-Command`);归属未验证 | 逐个跑 `--version` 与 `mcp list`,比对读写的配置根 |
| A12 | `mcp.lazyLoad`(或 `QODER_MCP_LAZY=1`)开启后,`mcp list` / `mcp get` 的输出是否仍反映完整生效态 —— 直接影响 §5.3 第 6 条的「回读校验」可靠性 | 文档所述(懒加载只暴露 `mcp_list`/`mcp_get`/`mcp_call`) | 本机开懒加载实测 |

> A1 的阻塞已解除,但**「在真机跑一次写操作并 diff」仍是 M2 的第一个动作**;在那之前 §4.3 的字段位置按「文档所述」用。
> A9 / A11 是新的潜在阻塞:A9 决定 skills 用「文件分发」还是「CLI 安装」,A11 决定矩阵里 Qoder 是一行还是三行。

---

## 7. 里程碑

| 阶段 | 交付 | 验收(可重跑的检查点) |
|---|---|---|
| **M0 分诊** | fork + 编译通过;`*_config.rs` 依赖图;删 provider/proxy 后能起 | `cargo build` + `npm run build` 通过;启动后三个面板可见;**此阶段不删代码,只画图和标记** |
| **M1 减法** | 按 §3.2 删干净;`schema.rs` 保留 provider 空表不写数据 | 全局 grep `provider|proxy|quota|session|usage` 无残留引用;三面板功能与删前一致 |
| **M2 Qoder CN CLI 适配** | **前置动作(必须先做)**:真机 `qoderclicn mcp add-json --scope user` 写一条并 diff `settings.json`,复核 A1 的字段位置、确认其他顶层键逐字节不变;解 A9(skills 启用态存哪)与 A11(几个 Qoder 入口)决定矩阵行数。交付:`mcp/qoder.rs` + `mcp/qoder_cn.rs`(写通道 `write_kind=cli` 优先,文件写为退路)、prompt 落点(`AGENTS.md` + `commands/*.md`)、skills 目录接入 | 打开矩阵开关 → `qoderclicn mcp list` 真能看到该 server **且** `settings.json` 的 `security/permissions/model/statusLine` 逐字节未变(diff 断言);关掉 → 干净撤销;备份目录有原文;界面区分「已写入 / 已生效」并提示需重启或 `/mcp reload` |
| **M3 机制补强** | dry-run / diff UI / `rescan()` 漂移检测 / `revert_last` / 密钥引用 | 手改工具配置 → 扫出漂移并给出两选一;误覆盖场景可回滚 |
| **M4 可迁移** | 导出/导入 bundle;从 cc-switch 导入(若 A6 可行);README 与「不做什么」声明 | 换机导入后矩阵状态与源机一致;bundle 内 grep 不到明文密钥 |
| **M5(观望)** | 仅当真实需要:projects 快照、后台常驻扫描、可选 git 化真源 | 不进首版承诺 |

---

## 8. 同类项目对照(取什么、避什么)

| 项目 | ★(实测) | 取 | 避 |
|---|---|---|---|
| [cc-switch](https://github.com/farion1231/cc-switch)(Rust/Tauri) | 147k | 底座本体:每工具一模块的适配器分层、SQLite + 备份、三面板交互 | 不支持 Qoder;90% 代码与我们无关 |
| [vercel-labs/skills](https://github.com/vercel-labs/skills) | 36k | **`src/agents.ts` 的 agent 注册表写法**(qoder / qoder-cn 两条已在表内,`skillsDir` + `globalSkillsDir` + `detectInstalled` 三字段即可) —— 我们 `tools` 表照此建模;`~/.agents/skills` 约定 | 只管 skills,没有 MCP/prompt 概念 |
| [Tencent/teamai-cli](https://github.com/Tencent/teamai-cli) | 5.6k | **唯一把 Qoder / Qoder CN 覆盖到 skills+rules+env+agents+hooks+MCP 六项的方案**(README 矩阵;并有 `qoder-cn-hooks-scope.test.ts` 等专项测试) —— 若只想今天就用,选它;它的 dashboard(:3721)是「观测+分发开关」而非编辑器,这个定位很克制,值得学 | 强绑团队 Git 仓库 + `teamai.yaml`,单人用偏重 |
| [spxrogers/agentsync](https://github.com/spxrogers/agentsync) | 37 | **机制最干净**:单一真源 → 32 agent 渲染、chezmoi 式双向 merge、`${secret:}` age 库、apply 写入目标目录的 local-only git 历史可 revert | v0.1.0 beta,布局会变;无 UI |
| [tackroom](https://github.com/staz6/tackroom) | 2 | **设计思路**:canonical 内容不含工具名/机器路径(lint 强制)、「render 不 fork」、`doctor` 在 tempdir 重推导再 diff、公开引擎 + 私有 overlay 两仓库 | 星数极低,别当依赖 |
| [magpie](https://github.com/yetone/magpie) | 18.8k | 事实:它**已支持 Qoder**(有 `internal/qoder/endpoints.go`,issue 在讨论 Qoder IDE 被重复识别成 VS Code),可作为 provider 侧的外挂 | 模型网关不是我们要造的轮子 |
| [DaniAkash/agent-toolkit](https://github.com/DaniAkash/agent-toolkit)(agent-mcp-manager) | 2 | 库形态:函数式写 MCP 条目到 23 个 agent,带 dry-run / `rescan()` 漂移 / `isInstalled()` —— **正好覆盖 M3 三件事**,若许可允许可直接用而不是自己写 | 项目极小,成熟度未知 |
| [mcp-dockmaster](https://github.com/dcSpark/mcp-dockmaster) | 89 | 桌面 + CLI + 库三形态一套内核的产品形状 | 只管 MCP |
| **Qoder CN CLI 自身**(`qoderclicn`) | — | **别造轮子的地方**:`mcp add-json/add/remove/get/list/enable/disable/reset-project-choices`、`skills install/list/enable/disable`、`agents list`、`plugins`、`hooks migrate` 全部官方自带(本机实测),既当写入通道,又当「生效态」回读探针(§5.3 第 6 条)。官方还给了 `hooks migrate` 从 Claude Code 迁移 —— 跨工具迁移的语义它比我们先想到 | 它**没有**跨工具矩阵、没有单一真源、没有 dry-run/回滚 —— 这正是 fork 的空白处。注意 CLI 是 per-tool 视角,不能拿来当多 agent 编排层 |

---

## 9. 一句话决策

- **想立刻有「Qoder + 其他 agent 共用一套 skills/MCP」** → 先用 `teamai-cli`(六项已 ✓),别等这个 fork。
- **想要 cc-switch 那种桌面 GUI** → 不要给 cc-switch 补 Rust 适配器;直接 fork 它做 §3 的减法,新增 agent 的成本从「一个模块 + 若干面板」降到「`tools` 表一行 + 一个 `mcp/<tool>.rs` 的若干落点函数(§4 那七问的答案)」。
- 这个 fork 的价值主张必须窄到:**多 agent 的 提示词 + MCP + Skills 矩阵,一处定义、可回滚、支持 Qoder**。超出一行的需求都推给上游或别的工具。

---

## 10. 本文档未做与后续

- 未读 cc-switch 任何 `.rs` / `.tsx` 实现细节 —— §3 的「作用」列在 M0 前都只是猜测,M0 的产出就是把它们换成事实。
- A1(Qoder CN 的 MCP 落点)已从「未知」降为「文档所述 + 本机一致」,**但仍缺一次真机写操作 + diff 的复核**(M2 前置动作);
  A2b / A9 / A10 / A11 / A12 未解,其中 A9 决定 skills 走「文件分发」还是「CLI 安装」,A11 决定矩阵里 Qoder 是一行还是三行。
- Qoder CLI 文档只拿到搜索摘要(WebFetch 被服务端限额,未能逐页通读)。§4.3 里的字段名、scope 语义、懒加载行为
  在通读原文前按 **文档所述** 对待;CLI 子命令面与本机路径是实测的,可信。
- 首版只承诺 MCP + Skills + Prompts(`AGENTS.md` 与 `commands/*.md`)三类。**Subagents / Hooks / Plugins / Memories 不做**,
  但它们在 Qoder CN CLI 里是可管理的同类资产 —— 要不要纳入属于产品取舍,需拍板后才能进 M2 之后的范围(见 A10)。
- 未做许可合规复核:MIT 允许 fork 与再分发,但需保留原版权与 LICENSE 副本(具体条款待读 `LICENSE` 原文确认)。
- 本文档不含代码,不含工时估算;里程碑的验收点是「可重跑的检查」而不是排期。
