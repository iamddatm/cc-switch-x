# CC Switch X 导入与 agent 适配实现计划（Plan 2/5）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**本计划的写法约定**:涉及密钥外泄、非管辖键、幂等、链接降级这些容易出事的断言,测试代码给全;纯机械的断言用 `/* 要断言什么 */` 注释点明要求,实现时必须补成真实断言,不能留空、不能 skip。接口签名是契约,跨任务引用时以本文件为准。

**Goal:** 让 CC Switch X 从 cc-switch 一键接管 MCP / Skills / Prompts 及其逐 agent 启用状态，并把这三类资产真正写入各 agent 的原生配置（写入前备份、键级合并、写后回读、可回滚），同时补齐 Skills 与 Prompts 两个面板。

**Architecture:** 在 Plan 1 的四层之上新增三块：`import`（把 cc-switch 的 SQLite 行转成 canonical 资产 + 密钥外移 + agent 矩阵）、`adapters`（每个 agent 的落点与渲染/回读，共享 `apply` 流水线）、`secrets`（本机明文密钥的唯一存放处，canonical 里只留 `${secret:…}` 引用）。前端加导入向导与「资产 × agent」矩阵开关。

**Tech Stack:** 在 Plan 1 基础上新增 `toml_edit`（保格式改 TOML）、`serde_json` 的 `preserve_order` feature（保键序改 JSON）、`dirs`/`std::os::windows` 的符号链接与 junction 调用。

**Spec:** `docs/superpowers/specs/2026-10-22-cc-switch-x-design.md`（本计划覆盖 3.1、3.2 的 MCP/Skills/Prompts、4.5、5、6、7.1、7.3、10 的 M1）

**范围裁剪（相对 Plan 1 的拆分说明）**：云快照导入只做**格式层**（`db.sql` + 已解包的 `skills/`），WebDAV/S3 的下载与解包属传输层，留在 Plan 5。Rules / Subagents / Hooks 的 canonical 面板与漂移检测在 Plan 3。

---

## 事实来源与置信度

写适配器前先固定依据。下表标出每条事实的出处，**「未实测」的条目不得作为已验证行为来断言**（Plan 1 全局约束：未实测的 agent 能力一律先隐藏开关）。

| 事实 | 出处 | 置信度 |
|---|---|---|
| cc-switch 主库 `~/.cc-switch/cc-switch.db`，版本 4.0.16，无 `PRAGMA user_version` 迁移表 | 本机 SQL 复核 | 已验证 |
| `mcp_servers` 列：`id,name,server_config,description,homepage,docs,tags,enabled_{claude,codex,gemini,opencode,hermes,grokbuild,mcode,pi}` | 本机 `PRAGMA table_info` | 已验证 |
| `server_config` 只有两型：`{type:"stdio",command,args,env}` / `{type:"http",url}`；凭据明文内嵌（`env` 值与 `url` 的 query） | 本机取值 | 已验证 |
| `prompts` 列：`id,app_type,name,content,description,enabled,created_at,updated_at`；本机 `app_type` 出现 `claude/codex/pi` | 本机 `PRAGMA` + 采样 | 已验证 |
| `skills` 列含 `directory,repo_owner,repo_name,repo_branch,readme_url,installed_at,content_hash,updated_at` 与 `enabled_*`；**`skills` 没有 `enabled_pi` 列**（`mcp_servers` 有） | 本机 `PRAGMA` | 已验证 |
| `skill_repos(owner,name,branch,enabled)`；`~/.cc-switch/skills/<dir>/SKILL.md` 共 81 个目录 | 本机 | 已验证 |
| 来源元数据只在 DB 与 `skill-backups/<ts>_<name>/meta.json`（DB 行的 camelCase 快照），skill 目录内无来源文件 | 本机 | 已验证 |
| `~/.cc-switch/settings.json` 有 `webdavSync.password` 明文、`currentProvider*`、`skillSyncMethod` | 本机 | 已验证（**均不导入**，见 2.2 非目标） |
| Claude 用户级 MCP 落点是 `~/.claude.json` 顶层 `mcpServers`（不在 `settings.json`） | 子代理实测本机文件 | 待复核（Task 7 用 fixture 断言，不依赖真机） |
| Codex MCP 落点 `~/.codex/config.toml` 的 `[mcp_servers.<name>]`；hooks 在独立 `hooks.json` 且需 `[features].hooks = true` | 子代理实测本机文件 | 待复核 |
| Codex `rules/default.rules` 是命令执行策略 DSL，**不是** markdown 规则 | 子代理实测 | 已验证（Plan 3 据此把 Codex 的 Rules 标为不支持） |
| Qoder CN CLI：`~/.qoder-cn/settings.json` 顶层 `mcpServers`，`requiresRestart:true`、`mergeStrategy:"shallow_merge"`；`rules/*.md` 用 `trigger: always_on\|glob\|manual\|model_decision` + 逗号裸值 `glob:`；`agents/*.md`；`skills/`；提示词 `AGENTS.md` | 子代理读二进制内默认档 | 待复核（Task 10 真机 diff） |
| Qoder 国际版 IDE：`~/.qoder/mcp.json` 只有 `{"mcpServers":{}}`；hooks 在 `~/.qoder/settings.json` | 子代理实测本机文件 | 未实测（IDE 究竟读 `mcp.json` 还是 `settings.json.mcpServers` 属 spec A2b，**首版只写 `mcp.json` 并在 UI 标注需实测确认**） |
| Gemini：只有 `GEMINI.md`、`settings.json.mcpServers`、`skills/` | 子代理实测本机文件 | 待复核 |
| teamai-cli 的 `DEFAULT_TOOL_PATHS`（`src/types.ts:400`）与 `AgentSpec`（`src/resources/agent-format.ts:76`）可作落点/字段参考 | 仓库内代码 | 已验证 |

## Global Constraints（继承 Plan 1，另加）

- 只读打开 cc-switch 的数据：`file:...?mode=ro`，任何代码路径都不得写 `~/.cc-switch/**`。
- 导入是**复制不移动**；结束后 cc-switch 保持原样，UI 提示「可自行清理」。
- canonical 文件、索引、日志、导出物里**都不得出现明文密钥**；密钥只进 `secrets.local.json`，canonical 里是 `${secret:<scope>.<key>}` 引用。
- 引用解析失败 → 该条资产写入失败并如实报告，**不写空值、不静默跳过**。
- 适配器只允许改自己管辖的键/文件；未管辖的键、注释、格式必须保持不变（TOML 逐字节、JSON 保键序）。
- 写入 agent 配置前必须备份到 `backups/<ts>/`，任一备份失败则整批中止。
- 回读确认前不得把状态报成「已生效」；需要重启/reload 的 agent 单独提示。
- 真机验证一律用**临时配置根**（`local/agents/<id>.json` 的路径覆盖指到临时目录），禁止拿用户真实的 `~/.claude`、`~/.codex` 当试验田。
- 参考 teamai-cli / cc-switch 的实现思路时保留 MIT 版权声明；本项目只借格式与思路，不复制代码文件。

## Review Focus

最容易踩坑、但单体测试容易漏掉的输入/状态，每条都钉在对应任务的测试里：

1. **密钥外泄**：导入后 canonical 与 `index.db` 里 grep 不到原明文 token；`url` 的 query 参数里的 token 也要抽走。
2. **列名漂移**：`skills` 没有 `enabled_pi`；出现未知 `enabled_*` 列时忽略并记入预览报告，不 panic 也不误建 agent。
3. **非管辖键被覆盖**：给适配器一份带注释、带未知键、键序被打乱的 fixture，断言未管辖部分不变。
4. **幂等**：同一份 cc-switch 数据导入两次条目数不变；同一份 apply 计划重跑不产生重复写入。
5. **skill 文件缺失**：DB 有行但目录不存在 → 报告「记录存在但文件缺失」，不得建空目录冒充导入成功。
6. **Windows 链接降级**：symlink 需要开发者模式；降级链 symlink → junction → copy 必须把**实际用到的方式**记进 `local/agents/<id>.json`，不能把 copy 报成 symlink。
7. **删除归属**：只删自己创建的东西——`~/.claude/skills/` 里非自家创建的实体目录必须原样保留。
8. **Prompts 整文件接管**：目标文件已有内容且不是我们写的 → 必须先让用户选「另存为一条 prompt」或「确认覆盖」，不得静默覆盖。

---

## File Structure

```
src-tauri/src/
  domain/secret.rs                      # ${secret:…} 引用解析与渲染
  store/secrets.rs                      # secrets.local.json 读写
  adapters/{mod,paths,mcp,skills,prompt}.rs
  services/apply.rs                     # 计划 → 备份 → 写 → 回读 → 报告 → 回滚
  import/{mod,source,ccswitch,mcp,prompts,skills}.rs
  index/schema.rs                       # 追加 agent_state / runs 两张表
  commands/{import,agents}.rs
src/
  panels/SkillsPanel.tsx, panels/PromptsPanel.tsx   # 由占位改为可用
  components/{AgentMatrix,ImportDialog}.tsx
  hooks/{useAgents,useImport}.ts
  __tests__/{ImportDialog,SkillsPanel,PromptsPanel,McpPanel.matrix}.test.tsx
```

---

### Task 1: 密钥引用与本机保管

**Files:** Create `src-tauri/src/domain/secret.rs`、`src-tauri/src/store/secrets.rs`；Modify `domain/mod.rs`、`store/mod.rs`、`Cargo.toml`

**Interfaces:**
- `SecretRef { scope: String, key: String }`；`secret::reference(scope, key) -> String`（`${secret:mcp.playwright.API_TOKEN}`）；`secret::parse_reference(&str) -> Option<SecretRef>`；`secret::find_references(&str) -> Vec<SecretRef>`（供写入前解析）。
- `Secrets::load(path) / save(path)`、`get(&SecretRef) -> Option<String>`、`insert(&SecretRef, &str)`、`resolve_text(&str) -> Result<String, SecretsError>`（把文本里所有引用换成明文，缺一个就整体失败）。
- `SecretsError::Missing(SecretRef)`、`BadReference(String)`。

- [ ] **Step 1: 写失败测试**（`domain/secret.rs` 末尾）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_roundtrip() {
        let text = reference("mcp.playwright", "API_TOKEN");
        assert_eq!(text, "${secret:mcp.playwright.API_TOKEN}");
        let parsed = parse_reference(&text).unwrap();
        assert_eq!(parsed.scope, "mcp.playwright");
        assert_eq!(parsed.key, "API_TOKEN");
    }

    #[test]
    fn finds_every_reference_in_a_url() {
        let url = "https://api.tavily.com/mcp?tavilyApiKey=${secret:mcp.tavily.tavilyApiKey}&v=2";
        let refs = find_references(url);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], SecretRef { scope: "mcp.tavily".into(), key: "tavilyApiKey".into() });
    }

    #[test]
    fn rejects_incomplete_reference() {
        assert!(parse_reference("${secret:nocolon}").is_none());
        assert!(parse_reference("${secret:a.b}").is_none()); // scope 不足 kind.id 两段
        assert!(parse_reference("${secret:mcp.pw.}").is_none()); // key 为空
        assert!(parse_reference("mcp.pw.TOKEN").is_none()); // 缺 ${} 包裹
    }

    #[test]
    fn splits_on_the_last_dot_so_dotted_asset_ids_work() {
        // 资产 ID 本身允许 '.'（store 的 is_safe_id），所以按**最后一个点**切：
        // mcp.a.b.TOKEN -> scope=mcp.a.b, key=TOKEN；密钥名是环境变量名，不含点
        assert_eq!(
            parse_reference("${secret:mcp.a.b.TOKEN}").unwrap(),
            SecretRef { scope: "mcp.a.b".into(), key: "TOKEN".into() }
        );
    }
}
```

> 切分规则必须在写实现前定死：`scope` 至少 `kind.id` 两段、按最后一个点分、body 含空白即非法。`resolve_text` 遇到**解析不了的 `${secret:…}` 候选要报 `BadReference`**，不能把它当普通文本原样写进 agent 配置。

`store/secrets.rs` 的测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_reference_is_an_error_not_an_empty_value() {
        let dir = tempdir().unwrap();
        let mut secrets = Secrets::load(&dir.path().join("secrets.local.json")).unwrap();
        secrets.insert(&SecretRef { scope: "mcp.pw".into(), key: "TOKEN".into() }, "abc");
        secrets.save(&dir.path().join("secrets.local.json")).unwrap();

        let loaded = Secrets::load(&dir.path().join("secrets.local.json")).unwrap();
        assert_eq!(loaded.resolve_text("${secret:mcp.pw.TOKEN}").unwrap(), "abc");
        assert!(matches!(
            loaded.resolve_text("${secret:mcp.pw.NOPE}"),
            Err(SecretsError::Missing(_))
        ));
    }
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test --manifest-path src-tauri/Cargo.toml secret` → FAIL（类型未定义）。
- [ ] **Step 3: 实现**（`Secrets` 用 `BTreeMap<String, BTreeMap<String, String>>`，`load` 缺文件返回空集而非报错，`save` 走 `atomic::write_atomic`；`resolve_text` 用一次线性扫描替换 `${secret:…}`，任一引用缺失即 `Err`）
- [ ] **Step 4: `cargo test … secret` → PASS**
- [ ] **Step 5: 提交** — `git add src-tauri/src/domain src-tauri/src/store src-tauri/Cargo.toml && git commit -m "feat: 添加密钥引用与本机保管"`

---

### Task 2: agent 注册表、落点与本机检测

**Files:** Create `src-tauri/src/adapters/paths.rs`；Modify `domain/agent.rs`；Create `src-tauri/src/services/agents.rs`

**Interfaces:**
- 落点路径一律以 `~/` 开头声明（沿用 Plan 1 `config_root: "~/.qoder-cn"` 的写法），由 `paths::expand_root(declared: &str, override_root: Option<&Path>, home: &Path) -> PathBuf` 统一展开；有 `local/agents/<id>.json` 的 `config_root` 覆盖时用覆盖值，否则用真实 home —— 真机验证就是靠这个把写入指到临时目录。
- **Claude 的用户级 MCP 在 `~/.claude.json`，它是 `~/.claude/` 的同级文件而不是配置根内的文件**，所以 `file` 一律写完整 `~/...` 路径，不做「配置根 + 相对名」拼接。
- `McpTarget::{JsonRoot { file: &'static str, key: &'static str }, TomlTable { file: &'static str, table: &'static str } }`
- `AgentPaths { mcp: McpTarget, skills_dir: &'static str, prompt_file: &'static str, rules_dir: Option<&'static str>, agents_dir: Option<&'static str>, hooks: HooksTarget }`（`HooksTarget::{SettingsJson, File(&'static str), Unsupported}`）
- `agent::builtin_agents()` 扩到 5 个：`claude`、`codex`、`gemini`、`qoder`、`qoder-cn`；每个带 `paths`、`mcp_needs_restart: Option<bool>`、`verified: bool`（`qoder` 为 `false`，UI 据此隐藏开关）。
  - `mcp_needs_restart` 用三态：只有 Qoder CN 有 `requiresRestart` 的实测依据（`Some(true)`），其余一律 `None` = 未实测，UI 保守提示「可能需要重启」，**不得因为没有依据就写成 `Some(false)`**。
- `LocalAgentState { base_override: Option<PathBuf> }`，读写 `local/agents/<agent-id>.json`（`serde(default)`，Task 8 再追加 `link_kind` 字段，只加不改）。
- `AgentStatus { id, display_name, detected, verified, mcp_needs_restart, mcp_path }`（serde camelCase，给前端矩阵表头用）。
- `agents::detect_all(store: &Store, home: &Path) -> Result<Vec<AgentStatus>, AppError>`：只看配置根在不在；**只读** `local/agents/<id>.json` 的覆盖，检测本身不回写状态。
- `agents::save_override(store, agent_id, base: Option<&Path>)`：写 `local/agents/<id>.json` 的 `baseOverride`（`~` 锚点覆盖）。

- [ ] **Step 1: 写失败测试**（`adapters/paths.rs` 末尾）

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_mcp_lives_in_claude_json_not_settings() {
        let claude = crate::domain::agent::builtin_agents()
            .into_iter()
            .find(|a| a.id == "claude")
            .unwrap();
        match claude.paths.mcp {
            McpTarget::JsonRoot { file, key } => {
                assert_eq!(file, "~/.claude.json");
                assert_eq!(key, "mcpServers");
            }
            other => panic!("Claude 的 MCP 落点被改成了 {other:?}"),
        }
        assert_eq!(claude.mcp_needs_restart, None, "Claude 的重启语义没有实测依据");
    }

    #[test]
    fn override_root_wins_over_the_real_home() {
        let dir = tempdir().unwrap();
        let got = expand_root("~/.claude.json", None, &dir.path().join("fake-home"));
        assert_eq!(got, dir.path().join("fake-home/.claude.json"));

        let got = expand_root("~/.claude.json", Some(&dir.path().join("sandbox")), &dir.path().join("fake-home"));
        assert_eq!(got, dir.path().join("sandbox/.claude.json"), "真机验证靠覆盖根把写入引到临时目录");
    }

    #[test]
    fn qoder_ide_target_is_marked_unverified() {
        let qoder = crate::domain::agent::builtin_agents()
            .into_iter()
            .find(|a| a.id == "qoder")
            .unwrap();
        assert!(!qoder.verified, "A2b 未实测，不得当作已确认能力暴露给用户");
    }
}
```

`services/agents.rs` 的测试：`detect_all` 对存在的配置根报 `detected=true`；`local/agents/claude.json` 写入 `baseOverride` 后，检测与后续落点都用覆盖目录；清除覆盖后回到真实 home。

- [ ] **Step 2: 运行确认失败** — `cargo test … agents` → FAIL。
- [ ] **Step 3: 实现**（落点常量按「事实来源」表填；`verified=false` 的 agent 在 `capabilities` 之外再带 `hidden_panes`，前端读取后不渲染开关）
- [ ] **Step 4: `cargo test … agents` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 补全 agent 注册表与配置落点"`

---

### Task 3: 写入流水线（计划 → 备份 → 写 → 回读 → 报告 → 回滚）

**Files:** Create `src-tauri/src/services/apply.rs`；Modify `index/schema.rs`、`index/mod.rs`、`state.rs`

**Interfaces:**
- `AssetState::{Off, Written, Effective, Drifted}` 定义在 `domain/asset.rs`（四态是领域概念，索引层也要用，放 apply 里会让 `index` 反向依赖 `services`）。serde `snake_case`，落库存 `as_str()`，`from_db()` 遇到未知值直接报错。
- `PlanItem { agent_id, asset_type, asset_id, action: Action::{Enable, Disable, Refresh}, target_path, #[serde(skip)] contents, diff, needs_restart }`
  - **`contents` 是适配器已经渲染好的完整字节**（键级合并发生在渲染侧）。流水线只负责备份/写入/回读/回滚，不掺渲染逻辑，因此不需要一个会随适配器膨胀的 op 枚举。
- `ApplyService::apply(&Store, &mut Index, &[PlanItem]) -> Result<ApplyReport, AppError>`：先备份全部已存在目标（这步失败时还没写过任何东西）→ 逐项写入并回读比对 → 任一项失败按倒序还原已写项并返回 `Err` → 全部成功才写 `agent_state` 与 `runs`。
- `ApplyService::revert_last(&mut Index) -> Result<ApplyReport, AppError>`：取最近一次 `reverted=0` 的 run，按 `result` 里的 `{target, backup}` 还原（`backup` 为 null 表示当时是新建，回滚即删除），并把相关项标成 `Drifted`、把该 run 置 `reverted=1`（不能被回滚两遍）。
- `ApplyReport { written, skipped, ok, backup_dir, error }`
- 索引新增：`agent_state(agent_id, asset_type, asset_id, state, last_error, applied_at)`、`runs(id, started_at, dry_run, plan_json, result, backup_dir, reverted)`；方法 `set_agent_state / agent_states / record_run / last_unreverted_run / mark_run_reverted`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn apply_backs_up_and_records_state_per_item() { /* 目标文件先有内容 → apply 后 backups/<ts>/ 里有原件，agent_state 记 written */ }

#[test]
fn a_failing_item_aborts_the_batch_and_restores_earlier_ones() {
    // 两个目标，第二个的父目录被做成只读文件制造写失败
    // 断言：第一个目标的磁盘内容与写入前一致（已回滚），report.ok == false，runs 里记 failed
}

#[test]
fn rerunning_the_same_plan_is_idempotent() {
    let first = ApplyService::apply(&store, &mut index, &plan).unwrap();
    let state_after_first = read_target(&target);
    let second = ApplyService::apply(&store, &mut index, &plan).unwrap();
    assert_eq!(read_target(&target), state_after_first);
    assert_eq!(second.written, 0, "已生效项不得重复写");
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test … apply` → FAIL。
- [ ] **Step 3: 实现**（顺序固定为：备份全部目标 → 逐项写 → 逐项回读 → 汇总；`written==0` 时不写 runs 记录以外任何东西；任何一步失败立即用备份回滚已写项）
- [ ] **Step 4: `cargo test … apply` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加资产写入流水线与回滚"`

---

### Task 4: 导入源（本机库 / 已解包快照）与预览

**Files:** Create `src-tauri/src/import/{mod,source}.rs`

**Interfaces:**
- `ImportSource::{InstalledDb(PathBuf), SnapshotDir(PathBuf)}`；`source::connect(&ImportSource) -> Result<Connection, ImportError>`
  - `InstalledDb` → `Connection::open_with_flags(uri, SQLITE_OPEN_READ_ONLY)`，URI 形如 `file:<path>?mode=ro`。
  - `SnapshotDir` → 目录内必须有 `manifest.json`（校验 `format == "cc-switch-webdav-sync"`）与 `db.sql`；把 `db.sql` 用 `execute_batch` 灌进 `tempfile` 里的临时库再打开。**下载与解包不在本计划**（Plan 5）。
- `ImportPreview { mcp_count, prompts_count, skills_count, conflicts: Vec<String>, ignored_columns: Vec<String>, missing_skill_dirs: Vec<String> }`
- `import::preview(&Path, ccswitch_home: &Path) -> Result<ImportPreview, ImportError>`（只读，不写任何文件）

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn opens_installed_db_read_only() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("cc-switch.db");
    seed_fake_ccswitch(&db); // 用与真实库相同的列名建表插数据
    let before = std::fs::read(&db).unwrap();
    let conn = source::connect(&ImportSource::InstalledDb(db.clone())).unwrap();
    let _ = conn.query_row("SELECT COUNT(*) FROM mcp_servers", [], |r| r.get::<_, i64>(0)).unwrap();
    drop(conn);
    assert_eq!(std::fs::read(&db).unwrap(), before, "导入绝不能写 cc-switch 的库");
}

#[test]
fn snapshot_dir_requires_manifest_format_and_db_sql() {
    let dir = tempdir().unwrap();
    assert!(source::connect(&ImportSource::SnapshotDir(dir.path().to_path_buf())).is_err());
}

#[test]
fn unknown_enabled_columns_are_reported_not_fatal() {
    // 建表时多一列 enabled_quantum，预览里 ignored_columns 含 "quantum"，其余照常计数
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test … import` → FAIL。
- [ ] **Step 3: 实现**（`seed_fake_ccswitch` 用 `mcp_servers/prompts/skills/skill_repos` 的真实列名，见「事实来源」表）
- [ ] **Step 4: `cargo test … import` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加 cc-switch 导入源与预览"`

---

### Task 5: MCP 导入器（含密钥外移）

**Files:** Create `src-tauri/src/import/mcp.rs`

**Interfaces:**
- `import::mcp::convert(server_config: &str, row: &McpRow) -> Result<(McpServer, Vec<(SecretRef, String)>), ImportError>`
- `import::mcp::run(store: &Store, secrets: &mut Secrets, index: &mut Index, conn: &Connection, policy: ConflictPolicy) -> Result<ImportCount, ImportError>`
- `ConflictPolicy::{Skip, Overwrite, KeepBoth}`（同 ID 时；`KeepBoth` 用 `<id>-2` 递增后缀）
- `enabled_*` 列 → `agents/<agent>.json` 的 `enabled.mcp` 列表；agent id 不在注册表里的列忽略并计入 `ignored_columns`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn stdio_env_secrets_are_moved_out_of_the_canonical_file() {
    let cfg = r#"{"type":"stdio","command":"npx","args":["-y","x"],"env":{"API_TOKEN":"sk-live-123"}}"#;
    let (server, secrets) = convert(cfg, &row("pw", "Playwright")).unwrap();
    let text = serde_json::to_string(&server).unwrap();
    assert!(!text.contains("sk-live-123"), "canonical 里不得留明文");
    assert!(text.contains("${secret:mcp.pw.API_TOKEN}"));
    assert_eq!(secrets[0].1, "sk-live-123");
}

#[test]
fn url_query_token_is_extracted_too() {
    let cfg = r#"{"type":"http","url":"https://api.tavily.com/mcp?tavilyApiKey=tvly-abc"}"#;
    let (server, secrets) = convert(cfg, &row("tavily", "Tavily")).unwrap();
    assert_eq!(server.url.unwrap(), "https://api.tavily.com/mcp?tavilyApiKey=${secret:mcp.tavily.tavilyApiKey}");
    assert_eq!(secrets[0].1, "tvly-abc");
}

#[test]
fn enabled_columns_become_the_agent_matrix() { /* enabled_claude=1、enabled_codex=0 → claude 的 enabled.mcp 含 pw，codex 不含 */ }

#[test]
fn importing_twice_does_not_duplicate_entries() { /* 同一份源跑 run 两次，list_ids 与矩阵计数不变 */ }
```

- [ ] **Step 2: 运行确认失败** — `cargo test … import::mcp` → FAIL。
- [ ] **Step 3: 实现**（`type` → `Transport` 映射：`stdio/http/sse/ws`，未知值报 `ImportError::UnsupportedTransport` 并跳过该条、计入报告，不得整批失败）
- [ ] **Step 4: `cargo test … import::mcp` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 导入 cc-switch 的 MCP 资产并外移密钥"`

---

### Task 6: Prompts 与 Skills 导入器 + 导入记录

**Files:** Create `src-tauri/src/import/{prompts,skills}.rs`；Modify `import/mod.rs`

**Interfaces:**
- `import::prompts::run(...)`：`prompts` 行 → `assets/prompts/<app_type>-<slug(name)>.md`（frontmatter `title/description`，正文 `content`）；`enabled=1` 的行写进 `agents/<app_type>.json` 的 `activePrompt`（每 agent 只留一条，多条时取 `updated_at` 最新并在预览里报告「多个候选激活」）。
- `import::skills::run(...)`：复制 `~/.cc-switch/skills/<directory>/` → `assets/skills/<id>/`，来源信息写 `assets/skills/<id>/.meta.json`（`{repoOwner,repoName,repoBranch,readmeUrl,installedAt,contentHash}`，camelCase）；目录不存在 → 计入 `missing_skill_dirs`，不建空目录。
- `import::finish(store, preview, counts)` → 写 `state/import.json`（来源、时间、cc-switch 版本、各类计数、被忽略的列）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn prompt_rows_become_markdown_and_the_active_pointer() { /* claude 两行 enabled → 取 updated_at 最新者作 activePrompt */ }

#[test]
fn skill_copy_does_not_move_the_source() {
    let src = ccswitch_home.join("skills").join("demo");
    import::skills::run(/* … */).unwrap();
    assert!(src.join("SKILL.md").exists(), "复制不移动");
    let dst = store.asset_dir(AssetType::Skill).join("demo");
    assert!(dst.join("SKILL.md").exists());
    let meta_text = std::fs::read_to_string(dst.join(".meta.json")).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta_text).unwrap();
    assert_eq!(meta["repoOwner"], "acme");
}

#[test]
fn missing_skill_directory_is_reported_not_created() {
    // DB 有行但目录不存在 → missing_skill_dirs 含该 directory，且 assets/skills/<id> 不存在
}
```

- [ ] **Step 2: 运行确认失败** — `cargo test … import::` → FAIL。
- [ ] **Step 3: 实现**
- [ ] **Step 4: `cargo test … import` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 导入 cc-switch 的 Prompts 与 Skills"`

---

### Task 7: MCP 适配器写入（键级合并 + 非管辖键不变）

**Files:** Create `src-tauri/src/adapters/mcp.rs`

**Interfaces:**
- `adapters::mcp::render(agent: &AgentDescriptor, server: &McpServer, resolved: &str) -> Result<Rendered, AppError>`（`Rendered::{JsonObject(Value), TomlDocument(String)}`）
- `adapters::mcp::enable(store, secrets, target_root: &Path, agent, server) -> Result<(), AppError>` / `disable(...)`
- 工具专属字段（`disabled`、`timeout`）**不写进 canonical**，只在渲染时按 agent 追加（spec 4.2）。

- [ ] **Step 1: 写失败测试**（用 fixture 目录当配置根）

```rust
#[test]
fn json_merge_keeps_foreign_keys_and_their_order() {
    // fixture ~/.claude.json（键序故意不是字母序）：
    // {"permissions":{"defaultMode":"acceptEdits"},"mcpServers":{"theirs":{"command":"x"}},
    //  "statusLine":{"type":"command","command":"echo hi"},"env":{"FOO":"bar"}}
    enable(&root, "claude", &server("newone")).unwrap();
    let text = std::fs::read_to_string(root.join(".claude.json")).unwrap();
    let after: serde_json::Value = serde_json::from_str(&text).unwrap();

    let keys: Vec<&String> = after.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        vec!["permissions", "mcpServers", "statusLine", "env"],
        "未管辖键必须原样保留且顺序不变（依赖 serde_json 的 preserve_order）"
    );
    assert_eq!(after["permissions"]["defaultMode"], "acceptEdits");
    assert_eq!(after["statusLine"]["command"], "echo hi");
    assert_eq!(after["env"]["FOO"], "bar");
    assert!(after["mcpServers"].get("theirs").is_some(), "别人的 MCP 不能被删");
    assert!(after["mcpServers"].get("newone").is_some());
}

#[test]
fn toml_merge_touches_only_its_own_table() {
    // fixture config.toml 含注释、空行、[features] 与别人的 [mcp_servers.theirs]
    let before = std::fs::read_to_string(&cfg).unwrap();
    enable(&root, "codex", &server("codegraph")).unwrap();
    let after = std::fs::read_to_string(&cfg).unwrap();
    let unrelated = |s: &str| -> Vec<String> {
        s.lines()
            .filter(|l| !l.starts_with("#! our marker") && !in_our_table(s, l))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(unrelated(&before), unrelated(&after), "注释与空行也必须逐字节不变");
}

#[test]
fn disable_removes_only_our_key_and_keeps_the_rest() { /* 先 enable 再 disable，第三方键仍在 */ }

#[test]
fn unresolved_secret_fails_the_item() { /* canonical 引用指向不存在的密钥 → Err，不写出空值 */ }
```

> `toml_merge_touches_only_its_own_table` 里的 `in_our_table` 是测试辅助函数：按 `\[mcp_servers\.codegraph\]` 段落边界把属于本次写入的行剔掉，其余行整串比对。实现时写清楚段边界，不要用「包含子串」近似判断。

- [ ] **Step 2: 运行确认失败** — `cargo test … adapters::mcp` → FAIL。
- [ ] **Step 3: 实现**（JSON 走 `serde_json` + `preserve_order`；TOML 走 `toml_edit`，只 `item(key)` 增删；写盘统一用 `store::atomic::write_atomic`）
- [ ] **Step 4: `cargo test … adapters` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加 MCP 适配器与键级合并写入"`

---

### Task 8: Skills 分发与 Prompts 整文件接管

**Files:** Create `src-tauri/src/adapters/{skills,prompt}.rs`

**Interfaces:**
- `LinkKind::{Symlink, Junction, Copy}`；`skills::deploy(store, target_root, agent, id) -> Result<LinkKind, AppError>`（降级链 symlink → junction → copy，结果写回 `local/agents/<id>.json` 的 `link_kind`）
- `skills::remove(...)`：只删「指向我们真源目录」的链接或我们复制的目录；实体目录非自家创建则保留并报告 `Adopted`。
- `prompt::activate(store, target_root, agent, prompt_id, takeover: Takeover) -> Result<ActivateOutcome, AppError>`，`Takeover::{Confirm, SaveExistingAs(String)}`；目标文件已存在且指纹不是我们上次写入 → 返回 `ActivateOutcome::NeedsConfirmation`（不写盘）。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn deploy_records_the_link_kind_it_actually_used() { /* 断言返回值与 local/agents 里存的一致 */ }

#[test]
fn remove_keeps_foreign_skill_directories() {
    // 目标目录里放一个非自家实体目录 → remove 后仍在
}

#[test]
fn activating_over_someone_elses_file_requires_confirmation() {
    std::fs::write(&target, "用户自己写的提示词").unwrap();
    assert!(matches!(activate(/* Takeover::Confirm */), Ok(ActivateOutcome::NeedsConfirmation)));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "用户自己写的提示词", "确认前不得覆盖");
}

#[test]
fn save_existing_as_stores_the_old_content_then_activates() { /* 旧内容成为一条新 prompt，随后目标文件等于新激活项 */ }
```

- [ ] **Step 2: 运行确认失败** — `cargo test … adapters::` → FAIL。
- [ ] **Step 3: 实现**（Windows 用 `std::os::windows::fs::{symlink_dir, symlink_file}`；junction 无 std 支持，用 `Command::new("cmd").args(["/C","mklink","/J",…])` 并在报告里注明来源；非 Windows 走 `std::os::unix::fs::symlink`）
- [ ] **Step 4: `cargo test … adapters` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加 Skills 分发与 Prompts 整文件接管"`

---

### Task 9: 前端——导入向导、矩阵开关、Skills 与 Prompts 面板

**Files:** Create `commands/import.rs`、`commands/agents.rs`、`src/components/{ImportDialog,AgentMatrix}.tsx`、`src/hooks/{useAgents,useImport}.ts`；Modify `panels/{McpPanel,SkillsPanel,PromptsPanel}.tsx`、`lib/api.ts`、`types.ts`、`App.tsx`

**Interfaces:**
- 后端命令：`import_preview(source)`、`import_apply(source, policy)`、`list_agents()`、`set_asset_enabled(asset_type, id, agent_id, enabled)`、`activate_prompt(prompt_id, agent_id, takeover)`、`get_asset_state(asset_type, id)`。
- 前端 `api.importPreview / importApply / listAgents / setAssetEnabled / activatePrompt`。
- `AgentMatrix`：一行一个 agent，四态文案 `未同步 / 已写入 / 已生效 / 有漂移`；`verified=false` 的 agent 显示「未实测，暂不提供开关」而不是可点开关。

- [ ] **Step 1: 写失败测试**

```tsx
// src/__tests__/ImportDialog.test.tsx
test("previews counts and applies with the chosen policy", async () => {
  render(<ImportDialog onDone={vi.fn()} />);
  expect(await screen.findByText(/MCP 5/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "开始导入" }));
  await waitFor(() => expect(api.importApply).toHaveBeenCalledWith(expect.any(Object), "skip"));
});

test("tells the user cc-switch data was not modified", async () => {
  render(<ImportDialog onDone={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "开始导入" }));
  expect(await screen.findByText(/cc-switch 数据保持原样/)).toBeInTheDocument();
});
```

```tsx
// src/__tests__/McpPanel.test.tsx 追加
test("toggling an agent switch calls setAssetEnabled", async () => { /* 点 claude 行的开关 → 断言参数 */ });
test("unverified agents show no switch", async () => { /* qoder 行显示「未实测」 */ });
```

```tsx
// src/__tests__/PromptsPanel.test.tsx
test("activating a prompt over foreign content asks for confirmation", async () => {
  vi.mocked(api.activatePrompt).mockResolvedValue({ kind: "needsConfirmation", path: "C:/x/CLAUDE.md" });
  // 点「激活」→ 出现「另存为一条 prompt / 确认覆盖」两个按钮，且未再次调用写接口
});
```

- [ ] **Step 2: 运行确认失败** — `pnpm test` → FAIL。
- [ ] **Step 3: 实现**（SkillsPanel：列表 + 链接方式徽标 + 逐 agent 开关 + 「安装方式：symlink/junction/copy」实测值；PromptsPanel：列表 + 每 agent 当前激活项 + 接管确认弹窗）
- [ ] **Step 4: `pnpm test` + `pnpm build` → PASS**
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加导入向导与资产矩阵开关面板"`

---

### Task 10: 端到端验证（真机，全部指向临时配置根）

**Files:** 无新增（验证任务）

- [ ] **Step 1: 全量门禁**
  - `pnpm test`、`pnpm build`、`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo build --manifest-path src-tauri/Cargo.toml` → 全部 PASS。
- [ ] **Step 2: 用真实 cc-switch 数据跑一次导入**
  - 存储根与 agent 配置根全部指到 `%TEMP%/ccswitchx-e2e/`（用 `local/agents/*.json` 覆盖 + 临时 `settings.json`）。
  - 断言：导入计数与 `SELECT COUNT(*) FROM mcp_servers/prompts/skills` 一致；`grep -r "sk-\|tvly-\|api_key" <临时存储根>/assets` 无命中；`~/.cc-switch/**` 的 mtime 与大小不变。
- [ ] **Step 3: 复核「待复核」事实**
  - 在临时根里对 claude / codex / gemini 各写一条 MCP，diff 目标文件确认只动自家键。
  - Qoder CN：真机 `qoderclicn mcp get <name>` 回读，确认 A1（`settings.json.mcpServers`）成立；不成立则把该 agent 退回 `verified=false` 并更新本表。
- [ ] **Step 4: 真机 UI 验证**（CDP 驱动，同 Plan 1 的做法）
  - 导入向导预览 → 执行 → MCP 面板出现导入条目 → 逐 agent 开关写入临时配置根 → Skills 面板显示链接方式 → Prompts 激活触发接管确认。截图逐张查看后再清理临时目录。
- [ ] **Step 5: 提交** — `git commit -m "test: 补充导入与适配的端到端验证记录"`（把 Step 2/3 的实测结论回写进本文件「事实来源」表）

---

## 自检

- **Spec 覆盖**：M1 全部（本地一键导入 MCP/Skills/Prompts + 矩阵、既有 agent 三类落盘与回读）；3.1 的 7 个步骤逐条对应 Task 4–6；5.1 适配器契约的 `render/apply/remove/read_back` 落在 Task 7–8 与 Task 3 的回读；5.3 的「CLI 优先」写入顺序**本计划未实现**（依赖外部可执行文件，标为 Plan 3 的 Qoder 适配任务）。
- **类型一致性**：canonical `McpServer` 沿用 Plan 1 的字段；`Transport` 与 cc-switch `server_config.type` 的取值域一致（`stdio/http/sse/ws`），未知值按条跳过并报告。
- **Review Focus 对应**：密钥外泄 Task 5；列名漂移 Task 4；非管辖键 Task 7；幂等 Task 3/5；skill 缺失 Task 6；链接降级 Task 8；删除归属 Task 8；Prompts 接管 Task 8/9。
- **已知缺口**：① `qoderclicn` CLI 写入路径未实现；② 云快照的下载/解包未实现（只做了 `SnapshotDir` 格式层）；③ Codex 的 Rules 无法映射（其 `rules/*.rules` 是执行策略 DSL），留到 Plan 3 的能力矩阵里显式标不支持；④ 漂移检测与 `rescan()` 在 Plan 3。
- **实现期定下的取舍**：不设 `ApplyService::plan()` —— 适配器渲染出 `PlanItem`（含 `diff`）本身就是 dry-run，UI 先展示 diff 再调 `apply`，避免两处规划各说各话。
