# CC Switch X 设计文档

> 状态：待实现
> 日期：2026-10-22
> 本文取代 `cc-switch-x-设计参考.md` 中与本文冲突的部分（尤其是「fork cc-switch 做减法」与「不做云同步」两条）。
> 事实来源：cc-switch 源码（`D:/repos/1github/cc-switch`，MIT）、teamai-cli 源码（`references/teamai-cli`，MIT）、vercel-labs/skills（`references/vercel-skills`，MIT）、本机 cc-switch 数据库实测。

## 1. 背景与定位

cc-switch 已提供 MCP / Skills / Prompts 的集中管理，但不支持 Qoder 系工具；给它补适配需要长期维护自己的 fork，不划算。

CC Switch X 是一个**独立桌面客户端**，不复用 cc-switch 的代码和进程，只在首次导入时读取它的数据。它接管六类 AI 资产的管理与分发：

1. MCP servers
2. Skills
3. Prompts（全局提示词文件）
4. Rules（按路径/始终生效的规则）
5. Subagents（子代理定义）
6. Hooks（生命周期钩子）

cc-switch 继续负责 provider、API Key、路由、故障转移、用量、会话、账号等；本项目不碰这些。用户导入后不再在 cc-switch 里管理上述六类资产。

## 2. 目标与非目标

### 2.1 目标

- **一处定义、多处下发**：每类资产在 CC Switch X 里维护一份，按 agent 勾选同步。
- **一键接管**：首次启动从 cc-switch 导入 MCP / Skills / Prompts 及其逐 agent 启用状态；此后 CC Switch X 是唯一真源。
- **覆盖 Qoder 系**：Qoder CN（IDE + CLI）、Qoder 国际版 IDE 作为一等公民；配置面共用则矩阵合并，不共用则分列。
- **首版只做用户级**：只写各 agent 的用户级目录（`~/.qoder-cn`、`~/.qoder`、`~/.claude` 等）；项目级目录仅允许只读扫描，不做分发。
- **可独立维护**：文本优先的真源目录 + 自己的 schema；cc-switch 停更或卸载后，客户端仍可完整工作。
- **不静默改文件**：写入前备份、按键级合并、写后回读、漂移检测、可回滚。
- **同步**：Git（逐资产 diff / 回滚）+ WebDAV / S3（快照），手动与自动。
- **界面与习惯贴近 cc-switch**：侧栏资产面板、逐工具开关、编辑器、设置页结构保持一致。
- **历史对话（本地）**：浏览、搜索、结构化阅读各 agent 的会话，导出 Markdown、复制恢复命令；只在本机读取原生会话文件，不进入 Git / 云同步。

### 2.2 非目标

- Provider / API Key / 模型切换 / 本地路由 / 故障转移 / 用量统计 / 账号登录 / 订阅余额。
- 历史对话的云同步、云端备份与跨设备合并：会话只在本机读取与索引，不进 Git / WebDAV / S3。
- 会话批量删除 / 清理：首版只做浏览、搜索、阅读、导出与恢复；删除能力后续再评估。
- 不 fork cc-switch，不调用 cc-switch 的可执行文件，不依赖其 schema 或 migration。
- 不做团队协作、权限、审计（teamai-cli 的团队仓库模式不在范围内）。
- 不做 Env 注入（teamai-cli 的 `env exec`）与 Models。
- 项目级分发：首版不向 `<repo>/.qoder/`、`<repo>/.claude/` 等目录写入；项目级配置只允许只读扫描与展示。
- 自定义命令（`commands/*.md`）：首版不做，作为后续的第七类资产。
- 首版不做插件打包、自动更新、托盘常驻；可作为后续版本。

### 2.3 反目标（即使实现也不能算成功）

- 导入后仍要求用户回 cc-switch 管理六类资产。
- 云端出现明文密钥。
- 写入 agent 配置时覆盖用户的 provider / permissions / model 等非本客户端管辖的键。
- 两个设备同时编辑后静默丢数据。

## 3. 用户可见行为

### 3.1 首次启动与一键导入

1. 检测 `~/.cc-switch/cc-switch.db` 与 `~/.cc-switch/skills/`，只读打开数据库。
2. 展示导入预览：MCP 数量、Skills 数量、Prompts 数量、各 agent 的启用关系、冲突项。
3. 用户确认后执行导入：
   - MCP：读取 `mcp_servers` 行，`server_config` 转成 canonical JSON；`enabled_*` 列映射为 agent 矩阵。
   - Prompts：读取 `prompts` 行（`app_type` + `content`），转成 canonical Markdown 与「每 agent 激活项」。
   - Skills：复制 `~/.cc-switch/skills/<dir>` 到新真源目录；数据库行提供名称、来源、启用矩阵。
   - 密钥值：从 MCP `env` 中抽出，写入本地 secrets 文件；canonical 文件里只留引用。
4. 导入采用**复制不移动**；cc-switch 数据保持原样，导入结束给出「可自行清理」的提示。
5. 导入按稳定 ID 幂等；再次导入只补新增/变更，不重复建条目。
6. 提供「从 cc-switch 云快照导入」：读取其 WebDAV/S3 的 `manifest.json` + `db.sql` + `skills.zip`，走同一套 importer。
7. 导入完成后写入 `state/import.json`（来源、时间、cc-switch 版本/schema 版本、计数）。

### 3.2 六类资产面板

侧栏项目与 cc-switch 保持同构：

- **MCP**：列表 + 新建/编辑（JSON 编辑器）+ 逐 agent 开关 + 同步状态。
- **Skills**：本地/仓库来源、安装/更新、逐 agent 开关；显示链接方式（symlink / junction / copy）。
- **Prompts**：每个 agent 的全局提示词文件；每个 agent 同时只有一个激活项。
- **Rules**：Markdown + frontmatter（`trigger: always_on` 或 `trigger: glob` + `glob`）；按 agent 勾选。
- **Subagents**：canonical YAML 编辑器；按 agent 渲染为 md / toml / json。
- **Hooks**：事件 + matcher + 命令；按 agent 勾选；UI 明示「这是可执行命令」。
- **Sessions（会话）**：按项目/时间分组，搜索标题、目录、消息或会话 ID；阅读页结构化呈现正文、思考、工具调用、工具结果与图片；支持会话内查找、导出 Markdown、复制恢复命令。会话不参与分发，也没有同步开关。

前六个面板是「资产 × agent」矩阵，支持搜索、按 agent 过滤、批量开关。Sessions 是本地只读视图，不进入矩阵。首版所有面板只管理用户级资产；项目级配置仅在扫描导入时只读展示，不出现分发开关。

### 3.3 状态与反馈

每个「资产 × agent」显示四态：

- `未同步`：未勾选。
- `已写入`：文件/配置已写入，尚未回读确认。
- `已生效`：官方只读命令或解析回读确认。
- `有漂移`：磁盘实际与期望不一致，提供「采纳磁盘 / 重刷真源」二选一。

需要重启或 reload 的 agent（如 Qoder MCP）单独提示，不能把「已写入」报成「已生效」。

### 3.4 设置

- 存储位置（默认 `~/.ccswitchx/`）。
- 各 agent 的配置根覆盖（与 cc-switch 的习惯一致）。
- 导入（cc-switch 本地/云端；从 agent 原生配置扫描 Rules/Subagents/Hooks）。
- 同步：Git / WebDAV / S3。
- 备份策略与占用空间。
- 外观：深色 / 浅色 / 跟随系统。

### 3.5 界面基线

以 cc-switch 的现有 UI 为基线：侧栏、列表卡片、开关、编辑弹窗、设置页分组、空状态与错误提示的措辞位置尽量一致。首版中文为主，文案可先中文、结构预留 i18n。视觉细节可复用 cc-switch（MIT）的 Tailwind 组件，保留版权声明。

## 4. 数据设计（文本优先）

### 4.1 目录布局

```
~/.ccswitchx/
  assets/
    mcp/<id>.json
    skills/<id>/...
    prompts/<id>.md
    rules/<id>.md
    subagents/<id>.yaml
    hooks/<id>.yaml
  agents/
    <agent-id>.json          # 跨设备同步：启用矩阵、激活的 prompt、链接方式
  local/
    agents/<agent-id>.json   # 本机：pathOverrides、检测结果；不参与任何同步
  secrets.local.json          # 只在本机；永不进入 Git / WebDAV / S3 / 导出包
  index.db                    # SQLite 派生索引；可随时重建；不参与同步
  state/
    import.json
    applied/<agent-id>.json   # 最近一次写入的指纹与结果
    drift.json
  backups/<ts>/...
  logs/
  .gitignore
```

- 文本文件是真源；`index.db` 只是缓存（列表查询、搜索、指纹）。
- 会话不复制进真源目录：只读各 agent 的原生会话文件，解析结果写 `index.db` 的会话索引；原始文件与索引都不参与同步。
- 外部修改（用户手改、`git pull`、其他设备同步）后，文件监听触发重新索引。
- ID 使用稳定 slug；显示名可改，ID 不改。资产重命名不改变 ID。

### 4.2 Canonical 格式

**MCP** `assets/mcp/<id>.json`：

```json
{
  "id": "playwright",
  "name": "playwright",
  "transport": "stdio",
  "command": "npx",
  "args": ["-y", "@playwright/mcp@latest"],
  "env": { "SOME_TOKEN": "${secret:mcp.playwright.SOME_TOKEN}" },
  "url": null,
  "headers": {},
  "description": "",
  "homepage": "",
  "tags": []
}
```

- 不写任何工具专属字段（如 Qoder 的 `disabled`、Codex 的 TOML 形态）。
- 密钥只存 `${secret:<scope>.<key>}` 引用；真实值在 `secrets.local.json`。

**Prompts** `assets/prompts/<id>.md`：纯 Markdown；元数据（标题、描述、来源）放 frontmatter。激活关系在 `agents/<agent>.json` 的 `activePrompt`。

- **所有权模型（整文件接管）**：激活一条 prompt = 目标提示词文件（`CLAUDE.md` / `AGENTS.md` 等）的全文变成该 prompt 的内容。首次激活时，如果目标文件已有内容且不是我们上次写入的，先让用户选择「把现有内容另存为一条 prompt」或「确认覆盖」。激活后，目标文件的任何外部修改（用户手改、`/init`、`/memory`）都算漂移：只报告并让用户选择「采纳磁盘」或「重刷真源」，绝不静默覆盖。
- 纯文本文件无法像 JSON/TOML 那样按键合并；M1 的「非管辖键逐字节不变」验收不适用于 Prompts，对应断言改为「文件全文等于激活的 prompt，否则报漂移」。

**Rules** `assets/rules/<id>.md`：Markdown + frontmatter：

```yaml
---
name: frontend-style
description: 前端代码风格
paths: ["src/web/**"]   # 空 = 始终生效
---
正文...
```

渲染时按 agent 转成 Qoder 的 `trigger/glob`、Cursor 的 `.mdc`、Copilot 的 `.instructions.md`、Kiro steering、Trae/CodeBuddy 的格式等；无规则目录的 agent 不出现开关。

**Subagents** `assets/subagents/<id>.yaml`：采用 teamai-cli 的 AgentSpec 思路：

```yaml
name: reviewer
description: 代码审查
instructions: |
  ...
model: inherit
tools: [Read, Grep]
tool_extras: {}
targets: []
```

按 agent 渲染为 `.md` / `.agent.md` / `.toml` / `.json`。

**Hooks** `assets/hooks/<id>.yaml`：

```yaml
name: format-on-stop
event: Stop
matcher: ""
command: "..."
commandWindows: "..."
timeout: 60
```

按 agent 渲染进对应 settings/hooks 文件。Hooks 是可执行内容，UI 必须展示完整命令；客户端自身不执行它。

**Skills** `assets/skills/<id>/`：标准 skill 目录（含 `SKILL.md`）。来源信息（repo、ref、安装时间）写 `agents` 之外的 `.meta.json` 或索引。

### 4.3 Agent 注册表与矩阵

Agent 注册表是代码内的静态数据（不是用户可编辑的真源），字段：

- `id`, `displayName`
- `configRoot`（用户级）、`projectRoot`（仅只读扫描用，不参与分发）
- `supports`: `{ mcp, skills, prompts, rules, subagents, hooks, sessions }`
- 每类的落点与写入方式：`file | cli`、`needsRestart`、`readBack` 命令
- `detect`: 目录/可执行文件存在性

`agents/<agent-id>.json` 存用户态：

```json
{
  "enabled": {
    "mcp": ["playwright"],
    "skills": ["lark-wiki"],
    "rules": ["frontend-style"],
    "subagents": ["reviewer"],
    "hooks": []
  },
  "activePrompt": "team-default",
  "skillLinkMode": "auto"
}
```

本机专属信息（`pathOverrides`、检测到的实际配置根、上一台设备的链接方式）存 `local/agents/<agent-id>.json`，不进 Git、不进云快照；换机后按本机检测重新生成。

### 4.4 索引

`index.db` 表从简：

- `assets(type, id, name, updated_at, hash)`
- `agent_state(agent_id, asset_type, asset_id, state, last_error, applied_at)`
- `runs(id, started_at, dry_run, plan_json, result, backup_dir)`
- `drift(id, asset_type, asset_id, agent_id, detected_at, kind, resolution)`
- `sessions(agent_id, session_id, project_path, title, source_path, updated_at, message_count)`
- `session_messages(session_id, seq, role, kind, content, tool_name)`，另建 FTS 表用于全文搜索

索引可全量重建；不存唯一数据。启动时若 `index.db` 缺失或版本不符，直接重建。会话索引只从原生文件派生，删掉会话文件后索引项随之清理。

### 4.5 密钥

- `secrets.local.json` 永不参与同步、导出、备份包；文件权限尽量收紧。
- 导入 cc-switch 时把明文 env 值抽到这里，canonical 文件里替换为引用。
- 渲染到 agent 配置时解析引用；解析失败则该 item 失败并如实报告，不写空值。
- 若用户选择「导出含密钥的备份」，必须显式二次确认，并单独加密（首版可不提供）。

## 5. Agent 适配层

### 5.1 适配器契约

每个 agent 实现：

- `detect() -> Installed`
- `render(asset_type, asset) -> ExpectedFile[]`（canonical → 原生）
- `read(asset_type, id) -> NativeAsset`（用于导入/漂移）
- `apply(plan_item) -> Result`（按键级合并、原子写、备份）
- `remove(asset_type, id) -> Result`（只删自己管辖的键/文件）
- `read_back(asset_type, id) -> EffectiveState`（CLI 只读命令或解析文件）
- `capabilities`

写入规则：

- 共享配置文件（如 Qoder `settings.json`）只做「反序列化 → 改自己管辖的子树 → 序列化」，未知键原样保留。
- 数组型配置（如 hooks 列表）按「我们写入的条目有标记/指纹」识别，删除只删自己的条目；无标记时保守处理。
- 任何写入前备份原文；备份失败则中止该批。

### 5.2 首版 agent 与能力

以 teamai-cli 的能力矩阵与 cc-switch 的适配器为参考，首版目标是：

- **既有 agent（通过导入自动出现）**：Claude Code、Codex、Gemini CLI、Grok Build、OpenCode、Hermes、Pi、MiniMax Code（mcode）、Agy（以本机 cc-switch 数据实际出现的为准）。六类能力按各工具实际支持情况裁剪；未验证的能力先隐藏开关，不硬写。
- **新增 agent**：
  - **Qoder CN**（一行）：IDE 与 CLI 共用 `~/.qoder-cn`。IDE 启动器为 `qoder-cn.cmd`（1.1.2），CLI 为 `qoderclicn`（1.1.76），`qodercn.cmd` 只是转发到 CLI。
  - **Qoder 国际版**（一行）：本机只发现 IDE 启动器 `qoder.cmd`（1.24.2），未发现独立 CLI；配置根 `~/.qoder`。若后续出现 CLI，默认与 IDE 共用配置根，仍为一行。
  - Qoder / Qoder CN 支持 skills、rules、mcp、subagents、hooks；prompts 落 `AGENTS.md`。
  - 用户级落点：`~/.qoder/{skills,rules,agents}` + `~/.qoder/settings.json`；`~/.qoder-cn/{skills,rules,agents}` + `~/.qoder-cn/settings.json`。
  - **本机实测待解**：国际版 `~/.qoder` 下同时存在 `settings.json`（含 `hooks`、插件开关）与独立的 `mcp.json`（顶层 `mcpServers`）；需真机写一次确认 IDE 实际读取哪一份（A2b）。
  - Qoder CN CLI 能力实测：MCP 完整（`add/add-json/remove/get/list/enable/disable`）；Skills 有 `list/enable/disable/install/link/uninstall`；Subagents 只有 `list`；Hooks 只有 `migrate`。因此 Rules / Subagents / Hooks 只能文件写入，Skills 优先 CLI，MCP 优先 CLI + 回读。
- **会话覆盖**：Claude Code、Codex、Gemini CLI、Grok Build、Hermes、MiniMax Code、OpenClaw、OpenCode、Pi、Agy（解析思路参考 cc-switch 的 `session_manager/providers`）；Qoder CN 的 JSONL（`~/.qoder-cn/projects/<项目>/<uuid>.jsonl`）。Qoder 国际版 IDE 的会话位置待测（可能在 IDE workspaceStorage，而非 `~/.qoder`），未确认前不承诺。

### 5.3 Qoder 写入顺序

1. 首选官方 CLI：`qoderclicn mcp add-json --scope user`、`mcp remove`、`skills install` 等。
2. Skills 优先 `qoderclicn skills link <path>` 指向本机真源目录（更新即时生效）；CLI 不可用时再软链/复制。
3. 退路才是直接改 `settings.json`（或国际版实测确认的 `mcp.json`），且严格按键级合并。
4. Rules / Subagents / Hooks 没有官方写命令，按文件写入；Hooks 只写 `settings.json` 的 `hooks` 子树。
5. 写后回读：`qoderclicn mcp list/get`、`skills list`、`agents list`；Rules/Hooks 靠解析文件。
6. `mcpServers` 改动提示「需重启或 `/mcp reload`」。
7. 不擅自改写 `mcp.enableAllProjectMcpServers`、`permissions`、`security`、`model`、`statusLine` 等用户键。

### 5.4 与 cc-switch 并存

cc-switch 切换 provider 时可能整份改写 agent 配置文件。CC Switch X 必须：

- 每次落盘后记录期望指纹；
- 启动、定时、或检测到文件变化时扫描漂移；
- 发现被覆盖 → 报告并让用户选择「重刷真源 / 采纳磁盘」；
- 绝不静默覆盖用户的新内容。

## 6. 应用、校验与回滚

一次 apply 的固定流程：

1. 生成计划（资产 × agent 的变更集）。
2. dry-run 返回 diff（新增/修改/删除键）。
3. 用户确认。
4. 逐项备份原文到 `backups/<ts>/`；任一备份失败则整批中止。
5. 逐项写入（原子写 + 按键级合并）。
6. 逐项回读校验；记录 `已写入` / `已生效` / `失败`。
7. 汇总逐项结果，任一失败不得报整体成功。
8. 支持 `revert_last`：按备份回滚，且回滚同样逐项报告。

幂等：同一计划重复执行结果不变；已生效项不重复写。

## 7. 导入

### 7.1 从 cc-switch 本地导入

- 只读连接（`mode=ro` + `busy_timeout`），不修改 cc-switch 数据。
- 读取 `mcp_servers`、`prompts`、`skills`、`skill_repos`。
- `enabled_*` 列 → agent 矩阵；未知列（未来新增 agent）忽略并记录日志。
- Skills 文件从 `~/.cc-switch/skills/` 复制；缺失文件报告为「记录存在但文件缺失」。
- 导入前展示预览与冲突策略：同 ID 已存在时 `跳过 / 覆盖 / 保留双方`。

### 7.2 从 cc-switch 云快照导入

- WebDAV/S3 读取其 `manifest.json`（`cc-switch-webdav-sync`）→ 下载 `db.sql` + `skills.zip` → 临时目录还原 → 复用 7.1 的 importer。
- 只在用户显式点击时执行；导入完成后不改动远端。

### 7.3 从 agent 原生配置扫描

- Rules / Subagents / Hooks 在 cc-switch 中没有对应数据，首次使用时可从各 agent 的原生目录扫描：
  - 识别「我们可管理的格式」（Qoder rules/subagents、Claude agents、hooks 配置等）；
  - 导入为 canonical 资产，标记来源为 `native-scan`；
  - 已存在同内容资产则复用，不重复创建；无法无损转换的跳过并报告。
- 扫描范围可包含项目级目录，但只读；首版不向项目级写入。
- 扫描只读，不移动、不删除原生文件。

## 8. 同步

### 8.1 Git

- 真源目录本身是一个 Git 工作区；启用 Git 同步时 `git init`（若尚未初始化）。
- 远端与分支在设置里配置；凭据交给系统 Git（credential helper / SSH agent），客户端不保存 token。
- 变更后防抖自动 `add` + `commit` + `push`；启动时与定时 `pull --rebase`。
- `.gitignore`：`index.db`、`secrets.local.json`、`local/`、`backups/`、`logs/`、`state/`、临时文件。
- 冲突处理：`pull --rebase` 失败即暂停自动同步，列出冲突文件；提供「保留本地 / 保留远端 / 手动编辑」。文本优先使手动合并可行。
- 提交信息：`sync: <device> <timestamp>`，或按变更类型生成；不含密钥。
- 系统未安装 Git 时禁用该传输并给出指引，不影响其他功能。

### 8.2 WebDAV / S3

- 打包真源目录（排除 8.1 的本地文件）为确定性 zip，附 `manifest.json`（格式标识 `ccswitchx-bundle`、版本、设备名、时间、每个 artifact 的 SHA256）。
- 上传顺序：artifact 先、manifest 后；下载校验 SHA256。
- 自动同步：变更防抖后上传；启动/定时检查远端 manifest 是否比本地新，提示拉取。
- 冲突：远端与本地都变更时，不自动覆盖；让用户选择「用远端替换本地（先备份）」或「用本地覆盖远端」。
- 与 cc-switch 的远端互相隔离：使用独立目录/文件前缀，不写 `db.sql` / `skills.zip` 的既有布局。

### 8.3 多设备语义

- 推荐同一时间只在一台设备编辑；Git 可做文件级合并，WebDAV/S3 是快照级选择。
- 任何拉取/覆盖前先本地备份。
- 密钥与设备级设置（路径覆盖、同步凭据）永不同步。

## 9. 架构与技术栈

- **桌面壳**：Tauri 2（Windows 优先，mac/Linux 顺带）。
- **前端**：React + TypeScript + Vite + Tailwind；组件风格对齐 cc-switch，可复用其 MIT 代码并保留声明。
- **后端**：Rust。
  - `domain`：六类资产的领域模型与校验。
  - `store`：文件读写、原子写、备份、目录监听（notify）。
  - `index`：SQLite（rusqlite）派生索引与查询。
  - `adapters/<agent>`：渲染、写入、回读、漂移。
  - `importer/ccswitch`、`importer/native`。
  - `sync/{git,webdav,s3}`：Git 调用系统 `git` 子进程；WebDAV/S3 用 reqwest + S3 兼容客户端。
  - `sessions`：各 agent 会话文件解析、统一消息模型、FTS 索引、Markdown 导出、恢复命令生成。
  - `commands`：Tauri command 面。
- **测试**：适配器用临时目录 + 真实文件做契约测试；同步用本地 WebDAV/S3 mock（或 MinIO）与临时 Git 仓库；导入用 cc-switch 样本数据库。

## 10. 里程碑与验收

### M0 骨架

- Tauri 工程可启动；六面板骨架与 cc-switch 风格侧栏可见。
- 文本真源目录与索引可初始化/重建；目录布局明确区分跨设备同步部分与 `local/` 本机部分。
- 验收：`cargo build`、前端构建通过；删掉 `index.db` 能自动重建；`local/` 与 `secrets.local.json` 不出现在同步清单。

### M1 导入与既有资产

- cc-switch 本地一键导入（MCP/Skills/Prompts + 矩阵）。
- 既有 agent 的 MCP / Skills / Prompts 落盘与回读。
- 验收：导入计数与 cc-switch 数据库一致；开关能写入并撤回；非管辖键逐字节不变；备份存在。

### M2 六类资产与 Qoder

- Rules / Subagents / Hooks 的 canonical 模型、面板、渲染与扫描导入。
- Qoder CN CLI / Qoder 国际版 / Qoder IDE 适配；实测配置根与入口归属。
- 验收：三类新资产在 Qoder 侧可被识别；hooks 命令在 UI 完整可见；关闭后干净撤销；settings.json 其他键不变。

### M3 历史对话

- 会话 provider：Claude、Codex、Gemini、Grok Build、Hermes、MiniMax Code、OpenClaw、OpenCode、Pi、Agy、Qoder CN。
- 列表（按项目/时间分组）、全文搜索、结构化阅读页、导出 Markdown、复制恢复命令。
- 会话索引只从原生文件派生；不复制会话内容，不进同步。
- 验收：选一个真实会话，列表、搜索、阅读、导出、恢复命令均可用；删除/新增会话文件后索引能增量更新；同步包里不含会话内容。

### M4 同步

- Git 同步（本地仓库、远端、冲突暂停）。
- WebDAV / S3 快照（manifest 校验、冲突选择、自动同步）。
- 验收：两台设备模拟目录可往返；冲突不静默覆盖；同步包中 grep 不到密钥、`local/` 与会话内容。

### M5 补强与发布

- 漂移检测、revert_last、从 cc-switch 云快照导入、设置完善、打包。
- 验收：手改原生配置能扫出漂移并二选一；回滚可用；安装包在干净 Windows 机器可启动。

## 11. 风险与待验证

| # | 事项 | 影响 | 验证方式 |
|---|---|---|---|
| A2b | Qoder 国际版 IDE 实际读 `~/.qoder/mcp.json` 还是 `settings.json` 的 `mcpServers`；Qoder CN IDE 是否与 CLI 完全共用配置 | 国际版 MCP 适配器落点 | 真机写一条 MCP，diff 两份文件 |
| A9 | Qoder `skills enable/disable` 状态存哪（本机 `skill-usage.json` 只有使用次数，未发现启用位） | 文件分发还是 CLI 安装 | 装一个 skill → disable → diff 文件 |
| A11 | 三个 Qoder 入口的归属（已实测：`qoder`/`qoder-cn` 是 IDE 启动器，`qoderclicn` 是 CN CLI，`qodercn.cmd` 转发到它；国际版未发现独立 CLI） | 矩阵条目数量 | 剩余：确认 IDE 是否会调用 CLI 写入 |
| A12 | `lazyLoad` 下 `mcp list` 是否完整 | 回读校验可靠性 | 开启懒加载实测 |
| B1 | cc-switch 切 provider 是否覆盖 MCP 段 | 并存稳定性 | 切换前后 diff agent 配置 |
| B2 | Rules / Subagents / Hooks 各 agent 格式差异 | 适配器工作量 | 每个 agent 放最小样本实测 |
| B3 | Windows symlink / junction 权限 | Skills 分发方式 | 三种方式逐一实测 |
| B4 | Git 冲突的用户可理解性 | 同步体验 | 双设备冲突演练 |
| B5 | 大量 skills 的压缩/传输 | 同步耗时 | 用本机 81 skills 压测 |
| C1 | 各 agent 会话格式差异与版本漂移 | 解析器覆盖率 | 每类 agent 取真实样本做契约测试 |
| C2 | Qoder 国际版 IDE 会话位置/格式未知 | 是否能在矩阵里承诺其历史对话 | 检查 IDE workspaceStorage 与真实会话 |
| C3 | 会话含敏感内容 | 导出与索引的本地保护 | 导出前提示；索引仅本地；不进入同步包 |

## 12. 留给实现者的决定

### 已定的取舍（不再留给实现者）

- Skills 来源安装：Rust 原生实现 Git / ZIP / 本地路径的安装与更新，不依赖 Node/npx；skills.sh 在线搜索后续再做。
- Prompts：整文件接管 + 首次激活捕获 + 漂移报告（见 4.2）。
- 设备级设置（路径覆盖、检测结果）：放 `local/`，永不参与同步。
- 项目级：只读扫描，不分发。
- Commands：首版非目标。

### 仍留给实现者

- canonical JSON/YAML 的具体字段名与版本号。
- 索引表结构与重建策略细节。
- 各 agent 适配器的渲染细节（以实测为准）。
- Git 用系统 CLI 还是 libgit2（本文倾向系统 CLI）。
- 自动同步的防抖/轮询间隔。
- 冲突 UI 的具体交互。
- 会话的统一消息模型、各 provider 的解析细节、FTS 增量策略。
- 各 agent 的恢复命令模板与参数。
- 是否以及何时引入插件打包（把多类资产打成一个 Qoder plugin）。

## 13. 许可与合规

- cc-switch、teamai-cli、vercel-labs/skills 均为 MIT；复用代码或格式实现时保留原版权与 LICENSE 副本。
- 本项目自己的代码建议同样使用 MIT，便于长期维护与再分发。