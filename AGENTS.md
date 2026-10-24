# AGENTS.md

## 项目概览

CC Switch X 是 Tauri 2 + React 的桌面应用，用纯文本文件作为唯一真源，统一管理 MCP / Skills / Prompts / Rules / Subagents / Hooks 六类 AI 编程资产，并把它们分发到 Claude Code、Codex、Qoder CN 等 agent 的用户级配置；另提供只读的本地会话浏览、搜索与导出。

技术栈：Rust（`domain` / `store` / `index` / `adapters` / `services` / `commands`）+ React 18 + TypeScript + Vite + Tailwind + Vitest。Windows 优先。
设计文档：`docs/superpowers/specs/2026-10-22-cc-switch-x-design.md`；实现计划：`docs/superpowers/plans/`。代码与测试是最终事实。

## 常用命令

```bash
pnpm install
pnpm dev                    # 只启动前端
pnpm tauri dev              # 启动桌面应用
pnpm build                  # tsc -b && vite build
pnpm test                   # Vitest 全量
pnpm test -- src/__tests__/RulesPanel.test.tsx   # 单个测试文件
pnpm test -- -t "drift_banner"                  # 按名称过滤
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml some_test_name
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
node scripts/check-release-version.mjs
```

- `cargo check` / `cargo test` 前先跑 `pnpm build`：`tauri::generate_context!` 在编译期读取 `dist/`。
- 没有独立 lint/format 脚本；TypeScript 检查由 `pnpm build` 的 `tsc -b` 覆盖。
- Vitest 只收集 `src/**/*.test.{ts,tsx}`；`references/` 是 gitignore 的第三方参考仓库，不要放测试或作为构建输入。
- `--ignored` 的导入测试依赖本机已安装 cc-switch，默认不在 CI 运行。

## 后端分层（src-tauri/src）

- `domain/`：canonical 资产模型、校验、frontmatter；`domain/agent.rs` 是代码内的静态 agent 注册表（落点、能力、CLI 名）。
- `store/`：文本真源目录布局、原子写、备份、staging 与 `secrets.local.json`。
- `index/`：SQLite 派生索引（assets / agent_state / runs / drift / sessions / FTS）；`AppState::init` 启动时全量重建 assets，`index.db` 可随时删除。
- `adapters/`：各 agent 的渲染、键级合并、CLI 通道与归属台账；`ledger.rs` 记录「我们往哪个路径写了哪份字节」。
- `services/`：`apply`（备份→写→回读→记录→失败整批回滚）、`drift`、`scan`、`import`、`assets`、`agents`、`skills_*`、`sessions`。
- `commands/`：薄 Tauri IPC 层；`state.rs` 持有 Store、Index、设置、secrets 与真实 home。
- `sessions/`：各工具只读会话 provider 注册表；大文件解析走 `spawn_blocking`。
- `net/`：GitHub 归档下载、ZIP 安全解压与代理配置；同步网络层由调用方放进 `spawn_blocking`，批量 Skill 任务用 `AppState::begin_skill_job` 互斥。

## 前端（src）

- `components/shell/AppShell.tsx` 按 `Pane` 切换面板；面板在 `panels/`，通用矩阵与表单组件在 `components/`。
- 所有后端调用集中在 `lib/api.ts`（`invoke`），参数用 camelCase；`types.ts` 是前后端类型的单一来源。数据获取在 `hooks/`，mutation 后手动 `refresh()`，没有全局状态库。
- 面板测试 mock `../lib/api`；能力表用 `test/agents.ts` 的 `capabilities()` 构造。用户可见文案用中文。

## 真源与数据目录

默认根为 `~/.ccswitchx/`（可在设置中改）：
- `assets/{mcp,skills,prompts,rules,subagents,hooks}/`：文本真源；Skill 是包含 `SKILL.md` 的目录。
- `agents/<agent-id>.json`：跨设备同步的启用矩阵与 `activePrompt`。
- `local/agents/<agent-id>.json`：本机专属 `baseOverride`、实测链接方式，不参与同步。
- `state/applied/<agent-id>.json`：落盘归属台账；`state/import.json`：导入记录。
- `index.db`：派生索引；`secrets.local.json`：密钥；`backups/`、`logs/`、`state/` 不同步。
- 本机设置：`%APPDATA%/cc-switch-x/settings.json`；修改 store root 只写设置文件，重启应用后才生效。

canonical 格式：MCP 为 JSON；Prompt / Rule 为 Markdown + YAML frontmatter；Subagent / Hook 为 YAML；Skill 为目录 + `SKILL.md`（来源信息在 `.meta.json`）。

## 必须保持的约束

- **文本优先**：真源是可读文本；`index.db` 只是缓存，任何只存在于索引里的数据都是设计错误。
- **能力闸**：`AssetSupport` 的 `supported` 与 `verified` 必须分开；只有两者都为真（`switchable()`）才允许 UI 开关和写命令。`verified` 必须有真机写入或本机实物证据，测试全绿不能升档；新增 agent/能力改 `domain/agent.rs`，并保持能力表与 `paths` 一致。
- **不碰外来内容**：共享 JSON/TOML 只改自己管辖的子树；TOML 用 `toml_edit` 保留注释与格式，JSON 用 `preserve_order` 保留键序（但无法保留原始排版）。整文件资产按台账记录的路径 + 哈希判定归属（Rules/Subagents 还认当前/历史渲染的等价字节）；Hooks 用 `description` 标记，Skills 用链接或 `.ccswitchx` 标记。无法确认归属时返回 `Foreign`，宁可不动。
- **Prompts 整文件接管**：目标文件已有非我方内容时，必须先让用户选择「另存为 prompt」或「确认覆盖」；激活后任何外部修改都算漂移，不得静默覆盖。
- **漂移纪律**：启动与手动 rescan 只报告；对受台账管理的整文件资产（Prompt / Rule / Subagent），启用或换激活遇到未解决漂移时返回 `DriftUnresolved`。处置只有「采纳磁盘」（改真源、不动盘）或「重刷真源」（走写入流水线）两种。
- **写入流水线**：任何落盘先备份；原子写后回读校验；任一项失败整批回滚。CLI 路径必须在调用 CLI 前 snapshot，读回确认后才算 `effective`，否则只能报 `written` 并在 `note` 说明原因。
- **CLI 安全**：参数一律用数组传给 `Command`，不拼 shell 字符串；`qoderclicn` 的 MCP 命令必须显式 `--scope user`，回读也钉在 user scope。`[features].hooks` 这类用户级开关只提示、不代写。
- **检测门禁**：写文件前经 `ensure_agent_detected` 检查配置根；不存在就拒绝，不替用户创建工具目录。
- **密钥**：canonical 只存 `${secret:scope.key}` 引用；真实值只在 `secrets.local.json`。解析失败必须让该项失败，禁止写空值或残缺引用。
- **会话只读**：不把会话内容复制进 `assets/`、`local/`、`backups/` 或同步包；只在 `index.db` 建派生索引，导出必须由用户显式选择路径。
- **同时涉及 index 与 secrets 时，锁顺序固定为 index → secrets**；网络/批量任务不阻塞 UI 线程。
- **ID 安全**：资产 ID 直接参与文件名，只允许 `[A-Za-z0-9._-]`，长度不超过 128；外部 ID 先经 `import::slug`。
- **路径**：Windows 优先，一律 `PathBuf`；声明式落点以 `~/` 开头，经 `adapters::paths::expand_root` 展开。

## 测试与真机验证

- Rust 单测使用 `tempfile`；`src-tauri/tests/asset_chain_e2e.rs` 通过 `baseOverride` 把 agent 配置根指到临时沙箱，并核对真实 `~/.claude` 等文件的哈希前后一致。
- 真机/UI 验证同样必须使用临时 store 与 `baseOverride`；禁止拿用户真实的 `~/.claude`、`~/.codex`、`~/.qoder-cn` 当试验田。`scripts/seed-ui-e2e.ps1` 铺沙箱数据，`scripts/drive-ui-e2e.cjs` 连接 CDP 操作面板并截图。

## 发布

- 同步修改 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 的版本号，运行 `cargo check` 更新 `src-tauri/Cargo.lock`，并新增 `docs/release-notes/v<版本>.md`。
- 本地门禁：`node scripts/check-release-version.mjs`。推送 `vX.Y.Z` tag 后，`.github/workflows/release.yml` 构建 Windows x64 NSIS、签名并发布 latest。
- 签名私钥与密码只放本机和 GitHub Secrets，绝不入库；复用 cc-switch / teamai-cli / vercel-labs/skills 的代码或格式时保留 MIT 声明，见 `THIRD_PARTY_NOTICES.md`。