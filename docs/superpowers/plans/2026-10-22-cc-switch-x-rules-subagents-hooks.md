# CC Switch X Rules / Subagents / Hooks 与漂移检测实现计划（Plan 3/5）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**本计划的写法约定**:容易出事的断言(归属识别、往返不丢字段、漂移判定、能力级 verified)给完整测试代码;机械断言用 `/* 要断言什么 */` 点明要求,实现时必须补成真实断言,不能留空或 skip。接口签名是契约。

**Goal:** 补齐最后三类资产(Rules / Subagents / Hooks)的 canonical 模型、按 agent 渲染写入与反向扫描导入,把「已生效」从「已写入」里区分出来(Qoder CLI 回读),并建立漂移检测:磁盘实际与期望不一致时只报告,让用户在「采纳磁盘 / 重刷真源」之间二选一,绝不静默覆盖。

**Architecture:** 沿用 Plan 2 的分层。新增两块:
- `adapters/{rules,subagents,hooks}.rs` —— 每类资产的**渲染 / 回推 / 认出自家旧渲染**三段式,加一个共享的**归属标记**约定。
- `services/drift.rs` + `index` 的 `drift` 表 —— 落盘后记期望指纹,启动/手动 `rescan` 时比对「重新渲染 vs 磁盘实际」。

**Tech Stack:** 在 Plan 2 之上新增 `toml`（解析 Codex 的 agent `.toml` 与 `config.toml` 读侧；写侧仍用 `toml_edit`）。

**Spec:** `docs/superpowers/specs/2026-10-22-cc-switch-x-design.md`（覆盖 3.2 的后三个面板、3.3 四态、4.2 的 rules/subagents/hooks、5.1–5.4、6、7.3、10 的 M2）

**许可**：本计划的渲染映射与「归属标记」机制参考 `references/teamai-cli`（MIT, `src/resources/rule-format.ts`、`agent-format.ts`、`hooks.ts`）。只借机制与格式,不复制其代码文件；落地时在 `THIRD_PARTY_NOTICES.md` 保留其 MIT 版权声明（Task 1 建立该文件）。

---

## 事实来源与置信度

Plan 2 之后又做了一轮定向调研。**关键结论:这台机器上没有任何真实的 rules / subagents 文件样本**,所以下表的依据分三档,写代码时按档处理。

| 事实 | 出处 | 置信度 |
|---|---|---|
| Qoder 规则落点 `~/.qoder-cn/rules/<name>.md`、`~/.qoder/rules/<name>.md`；frontmatter 为 `trigger: glob` + `glob: a, b`（逗号裸值、单数 `glob`、`{a,b}` 要展开），无 paths 则 `trigger: always_on` | teamai-cli `qoder-rule.ts` 头部注释（形式取自 `alibaba/tron-one-agent` 的真实规则文件）+ `qoderclicn` 二进制里的解析器源码 | 已验证（两源互证；本机无样本） |
| Qoder 解析器还认 `paths`、`description`、兼容 `alwaysApply`；`model_decision` 缺 `description` 会降级成 `manual`；无 frontmatter 默认 `always_on` | `qoderclicn` 二进制解析器源码 | 已验证（源码级） |
| Codex `~/.codex/rules/default.rules` 是 `prefix_rule(pattern=["git","add"], decision="allow")` 这种执行策略 DSL,**不是** markdown 规则 | 本机文件 + teamai `LEGACY_RULE_DIRS` 明写「Codex does not read .md files in its rules directory」 | 已验证 |
| Pi **不读** `~/.pi/agent/rules`,它的团队规则走 `~/.pi/agent/AGENTS.md` | teamai `LEGACY_RULE_DIRS`（pi 两条：user 与 project） | 已验证 |
| Claude 没有用户级 rules 目录（`~/.claude/rules` 本机不存在,也没有别处证据） | 本机 | 未确证 → 按不支持处理 |
| Subagent 文件扩展名按工具分：`.md`（claude / qoder / qoder-cn / codebuddy…）、`.agent.md`（copilot）、`.toml`（codex）、`.json`（kiro） | teamai `agentFileExtensionForTool` | 已验证（代码级） |
| Codex 的 agent TOML 用 `name/description/developer_instructions/model?`,**故意不写 `tools`**；多行值要用 `'''` 字面量,否则 16 行提示词会塌成一条 700 字符的转义行 | teamai `renderTomlAgent` + `canUseTomlLiteral` 注释 | 已验证（代码级） |
| Claude/Qoder 的 agent frontmatter 是 `name/description/model?/tools?` + 工具私有字段平铺；反向解析时非公共字段收进 `tool_extras.<tool>` | teamai `renderMarkdownAgent` / `reverseFromClaude` | 已验证（代码级） |
| Qoder CLI 的 agent 解析额外认 `model`、`allowed-tools`（也接受 `allowed_tools`）、`when-to-use`/`whenToUse`、`metadata.type` | `qoderclicn` 二进制 | 待复核（无文件样本） |
| Hooks 条目形状：`{matcher?, description?, hooks:[{type:"command", command, timeout?}]}`；`description` 是工具容忍但忽略的字段,可当归属标记 | 本机 `~/.qoder/settings.json`、`~/.claude/settings.json` 实测 + teamai `isTeamClaudeEntry` | 已验证 |
| 归属识别做法：写入时 `description = "[<app>:hook:<id>] …"`,清理时按前缀判定；再用「旧渲染重算」认出自家早期写的条目 | teamai `TEAMAI_CUSTOM_HOOK_PREFIX = '[teamai:hook:'` + `RuleFormat.previousRenders` | 已验证（机制级） |
| Qoder CN 的 hook 事件名共 28 个（`PreToolUse`…`PermissionDenied`）；hook `type` 枚举 `command\|runtime\|http\|prompt\|agent`,我们只写 `command` | `qoderclicn` 二进制 enum | 已验证（源码级） |
| Codex hooks 在独立 `~/.codex/hooks.json`,顶层 `hooks`,事件 `Interrupt/SessionStart/Stop/UserPromptSubmit`,条目**无 matcher** | 本机文件 | 已验证 |
| `~/.qoder-cn/settings.json` 当前**没有** `hooks` 也没有 `agents` 键 | 本机实测 | 已验证（新增子树属追加,不覆盖） |
| Qoder CN CLI 能力：`mcp add-json -s user\|local\|project <name> <json>`（默认 `local`）、`mcp add/remove/get/list/enable/disable`；`skills list/enable/disable/install/link/uninstall`；`agents` **只有 list**；`hooks` **只有 migrate --from-claude**；`config set/get/unset` | 本机 `qoderclicn --help` 实测 | 已验证 |
| Qoder 国际版**没有独立 CLI**,`qoder` 只是 IDE 启动器 | 本机实测（`~/.qoder/bin` 不存在） | 已验证 |
| `hooks migrate --from-claude` 是单向迁移、会改写文件 | CLI help | 已验证（因此**不能**当 CRUD 用） |

### 需要在本计划里纠正的既有代码

`src-tauri/src/domain/agent.rs` 现在给每个 agent 一个**全局** `verified: bool`,并把 qoder-cn 标成 `true`。这是过宽的:qoder-cn 只对 MCP 有实测依据,rules/subagents/hooks 三类的落点在本机连目录都不存在、也没有任何样本文件。**Task 1 就是把这一个 bool 换成按资产类型的 `{supported, verified}`**,否则 Plan 3 的面板会对着没验证过的落点开写权限。

## Global Constraints（继承 Plan 1/2,另加）

- 只写自己创建的东西。判定归属**只允许**两种依据:① 条目/文件里自带的归属标记；② 用已知渲染函数重算后逐字节相等。都不成立就保守跳过并在报告里点名。
- 归属标记只能放在目标工具**本就容忍**的字段里(hooks 用 `description`),不得往 `settings.json` 里发明私有键。
- 文件型资产(rules / subagents)的归属记在 `state/applied/<agent>.json` 台账里,与 Plan 2 的 prompts 指纹同一处,不往资产文件正文塞标记。
- 渲染必须可往返:`reverse(render(canonical)) == canonical`(允许工具私有字段落进 `tool_extras`)。往返不相等的字段一律不许引入。
- 未实测的落点一律 `verified=false`,UI 不给开关,只给「未实测」说明。新增 `verified` 只能由真机验证换来。
- 不擅自打开 Codex 的 `[features].hooks`:那是用户级开关,写它等于改用户可感知行为。检测不到就报「需要用户手动启用」,不代劳。
- 调外部 CLI 一律 `Command::new` 传参数组,不拼 shell 字符串；CLI 不存在或退出码非零 → 回落文件写入并在报告里写明走了哪条路。
- 真机验证一律用临时配置根(`local/agents/<id>.json` 的 `baseOverride`),禁止拿用户真实 `~/.claude`、`~/.qoder-cn` 当试验田。
- 不把 `~/.claude/settings.json` 原文写进仓库、日志或 UI(里面有明文 `ANTHROPIC_AUTH_TOKEN`)。

## Review Focus

1. **归属误删**:用户自己写的 hook 条目 / 规则文件必须活过我们的 disable 与 uninstall。
2. **旧渲染认账**:改了渲染格式之后,老版本我们写的文件仍要能被认出并安全替换(`previousRenders`)。
3. **往返不丢字段**:Codex 的 `developer_instructions`、Claude 的 `tools`、Qoder 的 `allowed-tools` 各自映射清楚,扫描导入再写回不能把工具私有字段丢掉。
4. **TOML 可读性**:多行提示词必须走 `'''` 字面量,不能塌成一行转义。
5. **事件名拼错**:`PreToolUse` 写成 `PreTool` 之类的未知事件必须被拒,而不是静默写进去。
6. **CLI 默认 scope 陷阱**:`mcp add-json` 不写 `-s` 默认落 `local`(项目级),我们必须显式 `--scope user`,否则用户级资产写进项目目录。
7. **漂移误报**:把「用户手改」和「我们上次写的」区分开,只有指纹不匹配才算漂移；文件不存在不算。
8. **能力级 verified 不回退**:任何一处把 `verified` 重新并回全局 bool 都算回归。

---

## File Structure

```
THIRD_PARTY_NOTICES.md
src-tauri/src/
  domain/{agent.rs,rule.rs,subagent.rs,hook.rs}      # 能力级 verified + 三类 canonical 扩展
  adapters/{mod,rules.rs,subagents.rs,hooks.rs,ledger.rs}
  services/{scan.rs,drift.rs}
  commands/{scan.rs,drift.rs}
  index/{schema.rs}                                   # 追加 drift 表
src/
  components/{DriftBanner,AssetForm}.tsx
  panels/{RulesPanel,SubagentsPanel,HooksPanel}.tsx
  hooks/{useDrift,useAssetEditor}.ts
  __tests__/{RulesPanel,SubagentsPanel,HooksPanel,DriftBanner}.test.tsx
```

---

### Task 1: 能力级 verified 与归属台账地基

**Files:** Modify `domain/agent.rs`、`services/agents.rs`、`commands/agents.rs`、`src/types.ts`、`src/components/AgentMatrix.tsx`；Create `adapters/ledger.rs`、`THIRD_PARTY_NOTICES.md`

**Interfaces:**
- 用 `AssetSupport { supported: bool, verified: bool }` 取代 `AgentCapabilities` 的 7 个 bool；`AgentDescriptor.support: BTreeMap<AssetKind, AssetSupport>`,`AssetKind::{Mcp,Skill,Prompt,Rule,Subagent,Hook,Session}`（与 `AssetType` 一一对应再加 Session）。
  - 保留 `AssetType`（真源目录用），`AssetKind` 只用于能力表 —— 两套并存会让「能力」和「目录」互相猜,所以 `AssetKind::from_asset_type()` 与 `dir_name()` 必须互逆,由测试钉住。
  - 注册表初始标注:**claude** mcp/skill/prompt `verified=true`、subagent `true`（teamai 代码级证据 + Claude 官方格式）、rule `supported=false`、hook `true`；**codex** rule `false`（执行策略 DSL）、hook `true`；**gemini** 只 mcp/skill/prompt；**qoder** 全部 `verified=false`（A2b 未确证）；**qoder-cn** mcp `true`、skill `true`、prompt `true`、rule/subagent/hook **`false`**（无样本,Task 2/4/6 真机验证后才升 `true`）；**pi** mcp/skill/prompt `true`、rule/subagent/hook `false`。
- `ledger.rs`：`AppliedLedger { entries: BTreeMap<String, AppliedEntry> }`,`AppliedEntry { asset_type, asset_id, target_path, hash, rendered_by: u32 }`,`rendered_by` 是渲染器版本号,给 `previousRenders` 用。读写 `state/applied/<agent>.json`。
- `THIRD_PARTY_NOTICES.md`：记 teamai-cli / cc-switch / vercel-labs-skills 的 MIT 声明与本仓库借用的机制清单。

- [x] **Step 1: 写失败测试**

```rust
// domain/agent.rs 末尾
#[cfg(test)]
mod support_tests {
    use super::*;

    fn agent(id: &str) -> AgentDescriptor {
        builtin_agents().into_iter().find(|a| a.id == id).unwrap()
    }

    #[test]
    fn verification_is_per_capability_not_one_global_flag() {
        let qoder_cn = agent("qoder-cn");
        assert!(qoder_cn.support[&AssetKind::Mcp].verified, "MCP 有实测依据");
        // 规则/子代理/hook 三个落点本机连样本都没有,不能跟着 MCP 一起亮灯
        assert!(!qoder_cn.support[&AssetKind::Rule].verified);
        assert!(!qoder_cn.support[&AssetKind::Subagent].verified);
        assert!(!qoder_cn.support[&AssetKind::Hook].verified);
        assert!(
            qoder_cn.support[&AssetKind::Rule].supported,
            "落点存在只是没验证,不该被当成不支持"
        );
    }

    #[test]
    fn agents_without_a_capability_say_so_not_unverified() {
        let codex = agent("codex");
        assert!(
            !codex.support[&AssetKind::Rule].supported,
            "Codex 的 rules 目录是执行策略 DSL,不是 markdown 规则"
        );
        assert!(!agent("pi").support[&AssetKind::Rule].supported, "Pi 不读 ~/.pi/agent/rules");
    }

    #[test]
    fn asset_kind_and_dir_name_are_inverse() {
        for asset_type in AssetType::ALL {
            let kind = AssetKind::from_asset_type(asset_type);
            assert_eq!(kind.dir_name(), asset_type.dir_name());
            assert_eq!(AssetType::from_kind(kind), Ok(asset_type));
        }
        // Session 不是真源资产,没有目录
        assert_eq!(AssetType::from_kind(AssetKind::Session), Err(AssetKind::Session));
    }
}
```

```rust
// adapters/ledger.rs 末尾
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AssetKind, AssetType};
    use tempfile::tempdir;

    #[test]
    fn ledger_roundtrips_and_replaces_only_its_own_key() {
        let dir = tempdir().unwrap();
        let store = crate::store::Store::init(dir.path().join("store")).unwrap();
        record(&store, "qoder-cn", AssetKind::Rule, "style", &dir.path().join("r.md"), "h1", 1).unwrap();
        record(&store, "qoder-cn", AssetKind::Hook, "fmt", &dir.path().join("s.json"), "h2", 1).unwrap();
        let text = std::fs::read_to_string(store.root().join("state/applied/qoder-cn.json")).unwrap();
        assert_eq!(text.matches("\"assetId\"").count(), 2, "{text}");

        // 同一资产重新落盘只更新自己那条，别的不该被抹掉
        record(&store, "qoder-cn", AssetKind::Rule, "style", &dir.path().join("r.md"), "h3", 2).unwrap();
        let ledger = load(&store, "qoder-cn").unwrap();
        assert_eq!(ledger.get(AssetKind::Rule, "style").unwrap().hash, "h3");
        assert_eq!(ledger.get(AssetKind::Hook, "fmt").unwrap().hash, "h2");
    }
}
```

- [x] **Step 2: 运行确认失败** — `cargo test … support_tests ledger` → FAIL。
- [x] **Step 3: 实现**（`AgentCapabilities` 保留为 `AssetKind` 的便捷构造,避免 Plan 1 的构造点全改；`AgentStatus` 增加 `capabilities: Vec<CapabilityStatus>` 给前端）
- [x] **Step 4: 运行** — `cargo test` + `pnpm test` + `pnpm build` 全绿（`AgentMatrix` 改读能力表：`supported && !verified` → 「未实测,暂不提供开关」；`!supported` → 整行不出现）
- [x] **Step 5: 提交** — `git commit -m "refactor: 能力级 verified 与落盘台账地基"`

#### Task 1 执行记录（2026-10-22）

落地的接口与本节写法的差异，后续任务按**实际接口**引用：

- `AgentDescriptor.support: SupportMap`（`BTreeMap<AssetKind, AssetSupport>`），配套
  `support_of()`（私有）、`supported(kind)`、`switchable(kind)`、`capabilities()`。
  Step 3 说的「`AgentCapabilities` 保留为便捷构造」没有成立：7 个 bool 的构造点全在
  `builtin_agents()` 里，一起删掉更干净，`supports(&[(AssetKind, AssetSupport)])` 取代它，
  未列出的格子补 `AssetSupport::OFF`（漏写一格不等于「验证过它不支持」）。
- `AssetKind` 与 `AssetType` 的互逆用 `AssetKind::{from_asset_type, try_as_asset_type}` +
  `AssetType::from_kind`（后者把 `Session` 变成 `Err`），`dir_name()` 返回 `Option`。
  另有 `snake_name()`：台账与前端用的单数名（`mcp/skill/prompt/rule/subagent/hook/session`），
  **刻意不同于**真源目录名（复数），别把它当目录名用。
- `adapters/ledger.rs` 的 `AppliedLedger` 除 `record/load` 外还有 `get`、`forget`、
  `find_by_path`。归属判定用 **路径 + 字节指纹** 双条件，不按资产 id —— 提示词是「整份文件
  一个落点、多个资产轮流占」，按 id 找会把「我们上次写的 A」误判成外来内容。
  `prompt.rs` 已从原来的单条 `AppliedPrompt` 迁到台账（`RENDERER_VERSION = 1`）；
  旧格式文件会解析成空台账，后果是下一次 activate 多问一次确认（保守方向，不会误删）。
- 能力标注相对本节表格的**两处修正**（都按 Global Constraints「未实测一律 false」，不以表格为准）：
  - **pi 的 MCP 降为未实测**：依据只有 pi 自带 docs 与 `dist/extensions/mcp/config.js` 的读侧源码，
    本机从未真机写入过（Plan 2 已记「MCP 尚未真机写入验证」）。标 `verified=true` 就是
    Plan 3 要修的那种过宽宣称。
  - **qoder-cn 的 prompt 升为已实测**：`~/.qoder-cn/AGENTS.md` 被本机 Qoder CN CLI 实际读成全局约定
    （B 档：实物 + 被消费的证据），不再挂在 MCP 那一格的光上。
- 新增的一致性闸：`capability_table_and_declared_targets_never_disagree`（paths.rs）钉住
  「supported ⇔ 落点存在」，rules_dir / agents_dir / hooks 三者一起查；以后加 agent 漏配一格会当场失败。
- 门禁：`cargo test` 118 通过、`pnpm test` 23 通过、`pnpm build`（tsc + vite）通过、
  `cargo fmt --check` 干净；`cargo clippy --all-targets` 只剩 `import/mod.rs:105 list_column`
  一条 Plan 2 遗留的 `if let` 风格告警（`.flatten()` 的确切语义未查证，不在本任务里瞎改）。

---

### Task 2: Rules 的渲染与回推（Qoder 形态）

**Files:** Modify `domain/rule.rs`；Create `adapters/rules.rs`

**Interfaces:**
- canonical 扩展：`RuleFrontmatter { name, description, paths, trigger: Option<RuleTrigger> }`,`RuleTrigger::{AlwaysOn, Glob, Manual, ModelDecision}`。`trigger` 缺省时按 `paths` 派生(有 → Glob,无 → AlwaysOn),派生规则与 teamai 一致。
- `adapters::rules::RuleFormat { extension, render, body_equals, merge_body_into_team, previous_renders }`（对齐 teamai 的 `RuleFormat`,字段名用 Rust 惯例）。
- `RULE_FORMATS`:qoder / qoder-cn → `QODER_RULE_FORMAT`；claude / codex / gemini / pi → 无(不写规则文件)。
- 渲染产物：`---\ntrigger: glob\nglob: a, b\n---\n\n<正文>\n` 或 `---\ntrigger: always_on\n---\n\n<正文>\n`。`glob` 行**不加引号**(Qoder Desktop 就这么写),`{a,b}` 先展开成独立 glob 再去重。
- 回推:`merge_body_into_team(tool_file, existing_team_md)` 只把正文换回去,frontmatter 由真源自己保留 —— 这是往返不产生假 diff 的关键。

- [x] **Step 1: 写失败测试**

```rust
#[test]
fn paths_become_a_single_unquoted_glob_line() {
    let rendered = render_rule(&qoder(), "src/**", &rule_with_paths(&["src/**".into(), "docs/**".into()]));
    assert_eq!(
        rendered,
        "---\ntrigger: glob\nglob: src/**, docs/**\n---\n\n正文\n"
    );
}

#[test]
fn brace_alternation_is_expanded_and_deduplicated() {
    let rendered = render_rule(&qoder(), "x", &rule_with_paths(&["src/{a,b}/*.ts".into(), "src/a/*.ts".into()]));
    assert!(rendered.contains("glob: src/a/*.ts, src/b/*.ts"), "{rendered}");
    assert_eq!(rendered.matches("src/a/*.ts").count(), 1, "展开后不能重复");
}

#[test]
fn a_rule_without_paths_is_always_on() {
    let rendered = render_rule(&qoder(), "x", &rule_without_paths());
    assert_eq!(rendered, "---\ntrigger: always_on\n---\n\n正文\n");
}

#[test]
fn body_survives_the_round_trip_and_frontmatter_stays_owned_by_the_source() {
    let team = rule_with_paths(&["src/**".into()]);
    let tool_file = render_rule(&qoder(), "style", &team);
    // 用户在 Qoder 那边只改了正文
    let edited = tool_file.replace("正文", "改过的正文");
    assert!(!body_equals(&qoder(), &edited, &team));
    let merged = merge_body_into_team(&qoder(), &edited, Some(&team));
    assert_eq!(parse_rule("style", &merged).unwrap().body, "改过的正文");
    // 真源自己的 paths 不能被工具侧派生的 trigger/glob 反向覆盖
    assert_eq!(parse_rule("style", &merged).unwrap().meta.paths, vec!["src/**".to_string()]);
}

#[test]
fn an_older_render_is_still_recognised_as_ours() {
    // 早期版本直接抄了真源正文（没有 trigger 行），仍要能被认出来并替换
    let legacy = verbatim_render(&team);
    assert!(delivered_renders(&qoder()).iter().any(|render| render(&team) == legacy));
}
```

- [x] **Step 2: 运行确认失败** — FAIL。
- [x] **Step 3: 实现**
- [x] **Step 4: `cargo test … rules` → PASS**
- [x] **Step 5: 提交** — `git commit -m "feat: 添加 Rules 的 Qoder 渲染与回推"`

#### Task 2 执行记录（2026-10-22）

实际落地的签名，Task 3 按这个引用（与上面示例代码的差异都写在后面）：

- `adapters::rules::render_rule(agent: &AgentDescriptor, rule: &Rule) -> Result<String, DomainError>`
  —— 示例里那个多余的 `"src/**"` 参数去掉了：渲染的输入只有 agent 与 canonical 规则，
  glob 从 `rule.meta.paths` 来，不需要第二处真相。
- `body_equals(tool_text, rule)` / `merge_body_into_team(tool_text, Option<&Rule>)` /
  `rule_from_tool_file(id, tool_text)` **不带 agent**：三者只做「剥 frontmatter、比正文」，
  这一步与工具形态无关（`.md` 与将来的 `.mdc` 都一样）；按 agent 分派的只有
  `render_rule`、`format_for_agent`、`delivered_renders`。少一个参数就少一处「传了但没用」的假签名。
- `RuleFormat { extension, render, scope_fields, previous_renders }`，`render`/`previous_renders`
  是 `fn(&Rule) -> Result<String, DomainError>`。今天只有 `QODER_RULE_FORMAT`（qoder 与 qoder-cn 共用），
  其余 agent `format_for_agent → None`，渲染退成 canonical 逐字抄（`verbatim_render`），
  同时它也是 qoder 的 `previous_renders[0]`。
- `domain::rule::render_rule` 改名 **`render_canonical`** 并改成返回 `Result`：
  原来是 `serde_yaml::to_string(&meta).unwrap_or_default()`，序列化一旦失败会写出
  「没有 frontmatter 的空文件」，而 rule 的 frontmatter 正是 `name` 的来源。
- `RuleFrontmatter` 增加 `trigger: Option<RuleTrigger>`（`always_on|glob|manual|model_decision`），
  `Rule::effective_trigger()` 负责派生。示例没写的两种情形按下面处理，都是为了不产出无效文件：
  - `trigger: manual` / `model_decision` **透传**，不被 `paths` 反过来改成 glob；
    `model_decision` 带上 `description`（缺 description 时 Qoder 自己降级成 manual，我们不改写、不假装）。
  - 显式 `trigger: glob` 但 `paths` 为空 → 渲染成 `always_on`。空 `glob:` 行在 Qoder 侧没有意义。
  - 认不出的 `trigger` 值在 `parse_rule` 阶段就是 `DomainError::Yaml`，不会静默变成默认值。
- canonical 正文统一为「trim + 单个结尾换行」（`merge_body_into_team` 与 `rule_from_tool_file` 都这么产出），
  否则 `render_canonical` 写出的文件有没有尾换行取决于来源，push 时会造出假 diff。
- `rule_from_tool_file` 从 `glob:` 行反推 `paths`（逗号切分后仍过一遍 brace 展开与去重），
  这是 Task 3 扫描导入把工具规则变成 canonical 的入口；认不出的 `trigger` 字符串留 `None`
  交给派生，不硬编成 `always_on`。
- 门禁：`cargo test` 135 通过（本任务 +17）、`cargo fmt --check` 干净、
  `cargo clippy --all-targets` 只剩 Task 1 记下的那条 Plan 2 遗留告警。
- [ ] **Step 5: 提交** — `git commit -m "feat: 添加 Rules 的 Qoder 渲染与回推"`

---

### Task 3: Rules 落盘、卸载与扫描导入

**Files:** Create `adapters/rules_deploy.rs`（或并入 `rules.rs`）、`services/scan.rs`；Modify `commands/`

**Interfaces:**
- `rules::deploy(store, agent, rule_id, home, base) -> Result<Deployed, AppError>`:目标 `<rules_dir>/<id>.md`,写前 `Store::backup`,落台账(`rendered_by`)。
- `rules::remove(store, agent, rule_id, home, base) -> Result<Removed, AppError>`:只有「台账里有记录」或「文件内容等于某个已知渲染」才删,否则 `Foreign`。
- `scan::rules(store, index, agent, home, base) -> Result<ScanReport, AppError>`:读 `<rules_dir>/*.md`,用 `merge_body_into_team` 反推 canonical,新文件建条目、已存在的只比正文；`ScanReport { imported, updated, skipped_foreign }`。
- 命令:`set_rule_enabled`、`scan_rules`。

- [x] **Step 1: 写失败测试**

#### Task 3 执行记录（2026-10-22）

- `rules::deploy(store, index, agent, rule_id, home, base) -> Deployed`，
  `Deployed::{Written(ApplyReport), Foreign}`。**去掉了示例里的 `Already`**：
  「什么都没写」已经由 `ApplyReport.skipped` 表达，再加一个变体就是两处真相各说各话。
- `rules::remove(...) -> Removed`，`Removed::{Ours, Foreign, Absent}`（无 payload；
  skills 那个 `Removed::Ours(LinkKind)` 带方式是因为删除方式各不相同）。
- **落盘与取消都走 `ApplyService`**，为此给 `Action` 加了 `Remove`（目标文件应当不存在）：
  删前一样备份、一样进 `runs`、失败一样回滚，`revert_last` 能把删掉的规则文件找回来
  （`reverting_a_removal_brings_the_file_back` 钉住这条）。绕开流水线自己写删除路径就会丢掉
  备份与逐项报告，spec 6 的账对不上。
- **归属判定**（`is_ours`）两条依据，都不成立就是 `Foreign`：
  ① 台账里该条目的 `targetPath + hash` 与盘上字节相等；② 盘上字节等于当前 canonical 的
  任一已知渲染（含 `previous_renders`）。用户手改过的自家文件算 `Foreign` —— 那是漂移
  （Task 8）要报告的东西，不该在 disable 里被静默删掉。
- `needs_restart` 对规则恒为 `true`（重新加载语义没有实测依据）→ 状态记 `Written` 而非 `Effective`。
- `services::scan::rules(...) -> ScanReport { imported, updated, unchanged, skipped: Vec<SkippedFile> }`。
  示例里的 `skipped_foreign` 换成带 `reason` 的列表：解析失败、扩展名不是 `.md`、
  文件名推不出合法 ID（纯中文被 `slug` 判空）、两个文件撞同一个 ID —— 四种都得点名，
  「静默丢文件」是导入器最坏的失败方式。另加 `unchanged` 计数，因为**幂等要能被断言**：
  deploy 完立刻扫一遍必须 `(0,0,1)`。
- 命令：`set_rule_enabled` 返回实测标签（`written` / `removed` / `absent`），`Foreign` 报
  `InvalidInput`（与 skills 一致，UI 能看到到底动没动）；`scan_rules` 返回 `ScanReport`。
  `commands::agents::{descriptor_for, override_base}` 导出给两个新命令复用。
- **能力闸的现状**：没有任何 agent 的 Rule 是 `verified`（Task 1 把 qoder-cn 降下来了），
  所以 `set_rule_enabled` / `scan_rules` 对真实用户一律 `Unsupported`。这是有意的：
  升 `verified` 只能由真机写入换来，而真机写入走 tests 里的临时配置根（Task 11），
  不靠放宽面板闸门来「先验证了再说」。
- 门禁：`cargo test` 152 通过（本任务 +17）、`cargo fmt --check` 干净、
  `cargo clippy --all-targets` 仍只剩 Plan 2 那条遗留告警。

```rust
#[test]
fn deploy_creates_the_dir_and_only_removes_what_it_delivered() {
    // 目标 rules 目录不存在 → 应创建；用户自己的 another.md 必须活着
    std::fs::write(rules_dir.join("another.md"), "用户自己的规则").unwrap();
    deploy(&store, &qoder_cn(), "style", &home, None).unwrap();
    assert!(rules_dir.join("style.md").is_file());
    assert!(matches!(remove(&store, &qoder_cn(), "style", &home, None).unwrap(), Removed::Ours));
    assert_eq!(std::fs::read_to_string(rules_dir.join("another.md")).unwrap(), "用户自己的规则");
}

#[test]
fn a_foreign_file_with_the_same_name_is_never_overwritten() {
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(rules_dir.join("style.md"), "---\ntrigger: manual\n---\n\n别人的").unwrap();
    assert_eq!(deploy(&store, &qoder_cn(), "style", &home, None).unwrap(), Deployed::Foreign);
    assert!(rules_dir.join("style.md").is_file());
    assert_eq!(std::fs::read_to_string(rules_dir.join("style.md")).unwrap().contains("别人的"), true);
}

#[test]
fn scanning_a_tool_rule_becomes_a_canonical_rule_with_the_same_body() {
    // Qoder 侧 trigger: glob + glob: a, b → canonical paths 应当拿回两个值
    let report = scan_rules(&store, &mut index, &qoder_cn(), &home, None).unwrap();
    assert_eq!(report.imported, 1);
    let text = String::from_utf8(store.read(AssetType::Rule, "style").unwrap()).unwrap();
    assert_eq!(parse_rule("style", &text).unwrap().meta.paths, vec!["src/**".to_string()]);
}
```

- [x] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Rules 落盘与原生扫描导入"`。

---

### Task 4: Subagents 渲染(claude / qoder 的 `.md` 与 Codex 的 `.toml`)

**Files:** Modify `domain/subagent.rs`；Create `adapters/subagents.rs`

**Interfaces:**
- 渲染映射(逐条对齐 teamai `agent-format.ts`):
  - claude / qoder / qoder-cn → `<id>.md`,frontmatter `name/description/model?/tools?` + `tool_extras.<agent>` 平铺,正文 = `instructions`。
  - codex → `<id>.toml`,键 `name/description/developer_instructions/model?`,**不写 `tools`**(Codex 用 `mcp_servers`),多行值用 `'''` 字面量。
- 反向:`reverse_subagent(agent, filename, content) -> Result<Subagent, DomainError>`,非公共字段收进 `tool_extras.<agent>`。
- Qoder 的 `allowed-tools` / `allowed_tools` 读进来要归一到 `tools`,写出时用 `tools`(Plan 3 不发明新键)。

- [x] **Step 1: 写失败测试**

```rust
#[test]
fn claude_and_qoder_render_markdown_frontmatter() {
    let rendered = render_subagent(&claude(), &spec()).content;
    assert!(rendered.starts_with("---\nname: reviewer\ndescription: 代码审查\n"), "{rendered}");
    assert!(rendered.contains("tools:\n  - Read\n  - Grep"), "{rendered}");
    assert!(rendered.trim_end().ends_with("正文最后一行"), "正文必须在末尾");
}

#[test]
fn codex_renames_instructions_and_drops_tools() {
    let rendered = render_subagent(&codex(), &spec()).content;
    assert!(rendered.contains("developer_instructions = '''\n"), "多行提示词要走字面量:\n{rendered}");
    assert!(!rendered.contains("tools"), "Codex 不认 tools,写进去只会误导");
    assert_eq!(reverse_subagent(&codex(), "reviewer.toml", &rendered).unwrap().instructions, spec().instructions);
}

#[test]
fn a_multiline_prompt_survives_the_toml_round_trip_byte_for_byte() {
    let mut spec = spec();
    spec.instructions = "行1\n行2 含 ' 单引号\n\t缩进行\n".to_string();
    let rendered = render_subagent(&codex(), &spec).content;
    assert_eq!(reverse_subagent(&codex(), "a.toml", &rendered).unwrap().instructions, spec.instructions);
}

#[test]
fn tool_private_fields_land_in_tool_extras_and_come_back() {
    let file = "---\nname: r\ndescription: d\nwhen-to-use: 需要审查时\n---\n\n正文\n";
    let spec = reverse_subagent(&qoder_cn(), "r.md", file).unwrap();
    assert_eq!(spec.tool_extras["qoder-cn"]["when-to-use"], "需要审查时");
    // 再渲染出去时这个私有字段不能丢
    assert!(render_subagent(&qoder_cn(), &spec).content.contains("when-to-use"));
}
```

- [x] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Subagents 的多形态渲染与解析"`。

#### Task 4 执行记录（2026-10-22）

- `render_subagent(agent, &Subagent) -> Result<Rendered, DomainError>`（`Rendered { extension, content }`），
  `reverse_subagent(agent, filename, content) -> Result<Subagent, DomainError>`。
  示例里 `render_subagent(&claude(), &spec()).content` 的用法照旧，只是返回 `Result`——
  写不出合法 TOML（例如 extras 里有 `null`）必须失败，不能悄悄丢键。
- **`DomainError` 加了 `EmptyDescription` / `EmptyInstructions`**：`name` 之外的两个必填项
  原先共用 `EmptyName`，报出来的话没法告诉用户到底缺了什么。（`domain/hook.rs` 里也有一处
  同样借用 `EmptyName` 的写法，留到 Task 6 一起收拾。）
- `domain::subagent::render_canonical(&Subagent) -> Result<String, DomainError>` 新增：
  canonical 是 YAML，序列化失败不能写成空文件覆盖真源（与 `rule::render_canonical` 同因）。
- TOML 写入仍只用已有的 `toml_edit`（**没有新增 `toml` 依赖**）：先由 `DocumentMut` 出全篇，
  再把「合格的多行值」从基本字符串换成 `'''` 字面量。合格条件与 teamai 一致（含 `'''`、
  以 `'` 结尾、带 `\r`/NUL/DEL 之类控制字符的都不行，C1 允许）。
  Rust 这边 `str::replace` 本来就是字面替换，不存在 JS 那个 `$&` / `$$` 被展开、
  把提示词里的美元符号吃掉的问题。
- 实测到的 `toml_edit` 行为（测试里钉住）：**不合格的多行值它会自己用 `"""` 多行基本字符串**，
  而我们只断言「字节级往返」，不断言具体形式——形式变了不影响 Codex 读，也不影响漂移比对。
  `\r` 这类控制字符会被转义写出去，读回来仍然是原字节。
- markdown 的 frontmatter 由 `serde_yaml` 出，块序列是 libyaml 的**零缩进**风格
  （`tools:\n- Read`），与示例里写的 `tools:\n  - Read` 不同。选它的原因：只有一处 YAML 输出器，
  产物保证合法，两家都用 YAML 解析器读 frontmatter；字节风格只在「认自家渲染」时与**我们自己**
  的输出比对（规则那边同理，靠 `body_equals` 只比正文）。
- 单行提示词落 Codex 时是基本字符串（`can_use_toml_literal` 要求必须含换行），
  字面量只为多行内容服务——这条与 teamai 一致，示例里那句断言是我照抄错了。
- Qoder 的 `allowed-tools` / `allowed_tools` 两种写法都读进 `tools`，写出只用 `tools`；
  Codex 侧 `tools` 不是公共字段，原样进 `tool_extras.codex`，反向时不会凭空造出列表。
- 门禁：`cargo test` 163 通过（本任务 +11）、`cargo fmt --check` 干净、
  `cargo clippy --all-targets` 只剩 Plan 2 那条遗留告警。

---

### Task 5: Subagents 落盘与扫描导入

复用 Task 3 的 deploy/remove/scan 形状,差异只在扩展名与目录(`agents_dir`)。**必须新增的断言**:同一 agent 目录里别人的 `.md`(不是我们的渲染)不被扫描器抢注成资产 —— 扫描时若文件带 `tool_extras` 之外的未知键或 frontmatter 不合 canonical,记入 `skipped_foreign` 并给出原因。

- [ ] Step 1 失败测试:`deploy_uses_the_tool_extension`(codex 出 `.toml`、qoder 出 `.md`)、`scan_skips_files_that_are_not_ours`、`removing_a_subagent_keeps_the_builtin_list`(内置 agent 不在我们台账里,一律不动)。
- [x] Step 2–5:红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Subagents 落盘与扫描导入"`。

#### Task 5 执行记录（2026-10-22）

- **`services/scan.rs` 抽出共用骨架**：私有 `trait FileAsset`（目录、扩展名、
  `canonical_text`、`matches_existing`）+ 一份 `scan_assets`，rules 与 subagents 各交一个实现。
  两类资产的扫描流程逐行相同（找文件 → 推 ID → 撞名 → 反推 canonical → 与既有真源比 →
  写真源 + 记索引），差异只在四个映射点；复制 50 行的话，「不静默丢文件」这类约束
  以后改一处必漏一处。
- `Deployed` / `Removed` 上移到 `adapters/mod.rs`（两类整文件资产共用同一套结果语义），
  归属判定上移到 `ledger::bytes_are_ours`（两条依据写在 Global Constraints 里，就该只有一处实现）。
- **`targets` 是真源侧独有字段**：`canonical_text(existing = Some(_))` 反推后把 `targets` 带回来，
  `matches_existing` 比较前先补上同一份 `targets`——否则每次扫都把「什么都没变」报成 `updated`，
  并且会把用户设的部署目标抹掉（`a_source_only_field_is_not_losing_its_values_on_rescan` 钉住这两点）。
- **示例里 `scan_skips_files_that_are_not_ours` 的落地口径**：不是「凡不是我们落的一律不导入」
  （那会让扫描等于只能导入自己写的文件，失去 spec 7.3「采纳磁盘」的意义），而是三条一起成立：
  ① 反推不出合法 canonical 的文件（缺 description / 正文为空 / TOML 语法坏 / 扩展名不对 /
  文件名推不出 ID / 撞 ID）→ 点名跳过并给原因，绝不半个字段地导入；
  ② 真源已存在但读不出来 → 跳过并点名，**不**当成「没有真源」直接覆盖；
  ③ `remove` 只删得出自家落盘的文件，内置的 `helper.md` 一律不动
  （`removing_a_subagent_keeps_the_builtin_list`）。rules 那边口径相同。
- 命令 `set_subagent_enabled` / `scan_subagents` 与 rules 同形：同一能力闸
  （`descriptor_for(agent_id, AssetKind::Subagent)`），返回实测标签，`Foreign` 报错。
- Codex 侧扫描验证到一处真实差异：`.toml` 里没有 `tools`，反推出来 `tools` 必须是空列表而不是
  猜一个；私有字段（如 `model_reasoning_effort`）落在 `tool_extras.codex`。
- 门禁：`cargo test` 176 通过（本任务 +13）、`cargo fmt --check` 干净、
  `cargo clippy --all-targets` 只剩那条 Plan 2 遗留告警。前端本任务无改动（面板在 Task 10）。

---

### Task 6: Hooks 写入 settings.json(带归属标记)

**Files:** Modify `domain/hook.rs`；Create `adapters/hooks.rs`

**Interfaces:**
- canonical 已有 `Hook { name, event, matcher, command, command_windows, timeout }`。新增校验:`event` 必须在 28 个已确证事件名白名单里；`timeout` 单位统一为**秒**并在渲染时按目标工具换算(Claude/Qoder 的 `timeout` 是秒,写死前用真机样本核对)。
- 归属标记:`description = "[ccswitchx:hook:<id>]"`(放在 matcher 组上,工具容忍该字段)。`id` 用 canonical 资产 id。
- `hooks::render_entry(hook, agent) -> Value`:qoder/claude → `{"matcher": "...", "description": "[ccswitchx:hook:x]", "hooks":[{"type":"command","command":"...","timeout":N}]}`；`command_windows` 存在且当前是 Windows 时用它。
- `hooks::plan_enable / plan_disable`:读目标 `settings.json` → 只动 `hooks` 子树 → 按 `description` 前缀增删**自己的**条目 → 其余条目与键原样(键序保留,靠 `preserve_order`)。
- `hooks::foreign_count`:报告该文件里非我们的条目数,UI 明示「这些是你自己的,我们不动」。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn enabling_appends_to_the_event_and_keeps_every_foreign_entry() {
    // fixture settings.json：SessionStart 已有一条用户自己的 + 别的顶层键
    let plan = plan_enable(&qoder(), &hook("fmt", "SessionStart", "cargo fmt")).unwrap();
    let after: Value = serde_json::from_slice(&plan.contents).unwrap();
    let entries = after["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "不能把用户自己的 hook 挤掉");
    assert_eq!(entries[0]["description"], "user's own");
    assert_eq!(entries[1]["description"], "[ccswitchx:hook:fmt]");
    assert_eq!(after["statusLine"]["command"], "echo hi", "非管辖键必须不变");
    assert_eq!(after["permissions"]["defaultMode"], "acceptEdits");
}

#[test]
fn disabling_removes_only_the_marked_entry() {
    // 先 enable 再 disable：用户自己那条必须还在，且顺序不变
    let after: Value = /* … */;
    let entries = after["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["description"], "user's own");
}

#[test]
fn an_unknown_event_is_rejected_instead_of_silently_written() {
    assert!(matches!(hook("h", "PreTool", "x"), Err(_)), "拼错的事件名不能写进用户配置");
}

#[test]
fn a_file_without_the_marker_is_left_alone_even_if_the_command_matches() {
    // 用户自己复制了同一条命令：没有归属标记就不许删
    let removed = plan_disable(&qoder(), "fmt", …).unwrap();
    assert!(String::from_utf8(removed.contents).unwrap().contains("cargo fmt"));
}

#[test]
fn windows_command_is_used_only_on_windows() {
    let entry = render_entry(&hook_with_windows_command("fmt", "pwsh -File a.ps1"), &qoder());
    // 本机就是 Windows，断言取的是 command_windows
    assert_eq!(entry["hooks"][0]["command"], "pwsh -File a.ps1");
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加 Hooks 写入 settings.json 与归属标记"`。

---

### Task 7: Codex 的 hooks 变体与「不代劳」边界

**Files:** Modify `adapters/hooks.rs`

- Codex 写 `~/.codex/hooks.json`(顶层 `hooks`,条目无 matcher),并在检测到 `[features] hooks` 不为 true 时**只报告不代写**:返回 `HooksNeedUserAction { file, why }`,UI 显示「需要你在 `config.toml` 里打开 `[features].hooks = true`」并给出该文件的绝对路径。
- 断言:`enabling_a_codex_hook_writes_hooks_json_not_config_toml`、`a_missing_features_flag_is_reported_not_written`、`codex_entries_carry_no_matcher`。
- 提交:`git commit -m "feat: 支持 Codex 的 hooks 落点并显式不代开功能开关"`。

---

### Task 8: 漂移检测(期望指纹 vs 磁盘实际)

**Files:** Create `services/drift.rs`；Modify `index/schema.rs`、`index/mod.rs`、`commands/`

**Interfaces:**
- `index` 追加 `drift(id, asset_type, asset_id, agent_id, detected_at, kind, resolution)`；`kind ∈ {modified, missing}`(文件被改 / 文件被删)。
- `drift::rescan(store, index, home) -> Result<Vec<DriftRecord>, AppError>`:遍历台账每一项 → 重新渲染期望内容 → 与磁盘实际比 → 不一致就 upsert 一条 `drift` 并返回；一致则清掉该键的旧 drift 记录。
- `drift::resolve(store, index, drift_id, action: AdoptDisk | RepushSource) -> Result<ResolveOutcome, AppError>`
  - `AdoptDisk`:把磁盘内容反向解析进真源(复用 Task 2/4 的 `merge_body_into_team` / `reverse_subagent`),更新台账指纹,drift 记 `resolution="adopted"`。
  - `RepushSource`:重渲染并走 Plan 2 的 apply 流水线,drift 记 `"repushed"`。
- **绝不静默覆盖**:`apply` 之前若该目标存在未解决的 drift,必须返回 `Err(AppError::DriftUnresolved)`,让 UI 先问用户。
- 触发时机:启动后一次 + 手动「重新检查」按钮。文件监听(自动 rescan)不在本计划,列进 Plan 5。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn a_hand_edit_becomes_drift_and_blocks_silent_reapply() {
    deploy_rule(&store, &qoder_cn(), "style", &home, None).unwrap();
    std::fs::write(&target, "---\ntrigger: always_on\n---\n\n用户手改\n").unwrap();
    let drifts = rescan(&store, &mut index, &home).unwrap();
    assert_eq!(drifts.len(), 1);
    assert_eq!(drifts[0].kind, DriftKind::Modified);
    assert!(matches!(
        set_rule_enabled(&store, …, true).err(),
        Some(AppError::DriftUnresolved(_)),
        "有未解决漂移时不能直接覆盖"
    ));
}

#[test]
fn adopting_the_disk_pulls_the_edit_back_into_the_source_instead_of_losing_it() {
    resolve(&store, &mut index, drift.id, AdoptDisk).unwrap();
    let text = String::from_utf8(store.read(AssetType::Rule, "style").unwrap()).unwrap();
    assert!(text.contains("用户手改"), "采纳磁盘要真的把改动收进真源");
    assert!(rescan(&store, &mut index, &home).unwrap().is_empty(), "解决后不该还挂着漂移");
}

#[test]
fn repushing_the_source_restores_the_render() {
    resolve(&store, &mut index, drift.id, RepushSource).unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), render_rule(&qoder(), "style", &team()));
}

#[test]
fn a_deleted_file_is_reported_as_missing_not_silently_recreated() {
    std::fs::remove_file(&target).unwrap();
    let drifts = rescan(&store, &mut index, &home).unwrap();
    assert_eq!(drifts[0].kind, DriftKind::Missing);
    assert!(!target.exists(), "rescan 只报告，不动盘");
}

#[test]
fn an_untracked_file_is_never_drift() {
    // 用户自己新建的规则文件不在台账里 → 不该报漂移（那是我们的漏管，不是漂移）
    assert!(rescan(&store, &mut index, &home).unwrap().is_empty());
}
```

- [ ] **Step 2–5:** 红 → 实现 → 绿 → 提交 `git commit -m "feat: 添加落盘后的漂移检测与两种处置"`。

---

### Task 9: Qoder CLI 优先路径与回读

**Files:** Create `adapters/cli.rs`；Modify `adapters/mcp.rs`、`services/apply.rs`

**Interfaces:**
- `cli::QoderCli::discover() -> Option<PathBuf>`(找 `qoderclicn`,不假设路径)。
- `add_mcp(&self, name, json) -> Result<(), CliError>`:必须 `["mcp","add-json","--scope","user",name,json]` —— **默认 scope 是 `local`,漏了 `--scope user` 会把用户级资产写进项目目录**。
- `remove_mcp`、`mcp_list`、`skills_link`、`skills_unlink`。
- `read_back(agent, asset_kind, id) -> Option<bool>`:`mcp get <name>` 成功退出即「已生效」；CLI 不可用时回落 Plan 2 的「解析文件即已生效」,并在 `AgentStatus` 里标 `readBack = "file"` 而非 `"cli"`,不把降级说成成功验证。
- 落点:`verified` 的 qoder-cn MCP 走 CLI 优先 → 失败回落文件；CLI 不存在时**不报错**,走文件路径并在报告里写「走了文件写入」。

- [ ] **Step 1: 写失败测试**
  - `mcp_add_json_always_passes_scope_user`:参数数组构造的纯函数断言,含 `--scope` 与 `user` 相邻两项。
  - `a_nonzero_exit_falls_back_to_file_writing_and_says_so`:注入一个必然失败的 CLI 命令,断言最终文件写成功且 `ApplyReport.method_used == File`。
  - `read_back_reports_effective_only_when_the_cli_confirms_it`:CLI 回读失败时状态停在 `Written`,不升 `Effective`。
- [ ] Step 2–5:红 → 实现 → 绿 → 提交 `git commit -m "feat: Qoder MCP 与 Skills 走 CLI 优先并回读确认"`。
- [ ] **Step 6(真机,需用户点头)**:在临时配置根 + 临时 store 下跑一次 `qoderclicn mcp add-json --scope user`,diff `~/.qoder-cn/settings.json` 确认字段位置(A1),再把这条升为 `verified`。不点头就不做。

---

### Task 10: 三个面板 + 漂移 UI

**Files:** Create `panels/{RulesPanel,SubagentsPanel,HooksPanel}.tsx`(占位→可用)、`components/{DriftBanner,HooksSafety}.tsx`、`hooks/useDrift.ts`；Modify `App.tsx`、`lib/api.ts`、`types.ts`

**要点:**
- 三类都用「列表 + 表单编辑器」而非裸 JSON:rules 是 markdown + paths 输入；subagents 是 YAML 表单(name/description/instructions/model/tools)；hooks 是事件下拉(28 个白名单)+ matcher + **完整命令展示**(spec 4.2:hooks 是可执行内容,UI 必须展示完整命令,不得省略号截断)。
- `DriftBanner`:某资产有未解决漂移时,在面板顶部显示「磁盘与真源不一致」+ 两个按钮(采纳磁盘 / 重刷真源),并给出目标文件绝对路径。
- `HooksSafety`:显示「该文件里还有 N 条不是本工具写的 hook,我们不会动它们」。
- 能力表驱动:`supported=false` 的 agent 整行不出现；`supported && !verified` 显示「未实测」而非可点开关。
- 测试:`hooks_panel_shows_the_full_command_without_truncation`、`drift_banner_offers_both_resolutions_and_no_silent_write`、`unverified_capability_shows_no_switch`、`event_dropdown_only_offers_known_events`。
- 提交:`git commit -m "feat: 添加 Rules/Subagents/Hooks 面板与漂移处置"`。

---

### Task 11: 端到端验证(沙箱)

- [ ] 全量门禁:`pnpm test`、`pnpm build`、`cargo test`、`cargo build`。
- [ ] 沙箱真机:临时 store + `baseOverride` 指向临时配置根,跑「导入 → 勾规则 → 勾子代理 → 勾 hook → 手改磁盘 → rescan → 采纳磁盘」全链路,逐张看截图。
- [ ] 复核并升档:`qoder-cn` 的 rule / subagent / hook 三项在真机 diff 通过后从 `verified=false` 升 `true`,并回写本文件「事实来源」表；升不了就保持 `false` 并在计划里注明原因。
- [ ] 复核 Qoder IDE(A2b):写一条 MCP 到 `~/.qoder/mcp.json`,确认 IDE 是否读到；读不到就把 qoder 保持全 `false`。
- [ ] 清理临时目录与 `settings.json`,确认 `~/.claude.json`、`~/.qoder-cn/settings.json` 等真实文件哈希未变。
- [ ] 提交:`git commit -m "test: 补充三类资产与漂移检测的端到端验证记录"`。

---

## 自检

- **Spec 覆盖**:3.2 的 Rules/Subagents/Hooks 三面板(Task 10)、3.3 四态与漂移二选一(Task 8/10)、4.2 三类 canonical(Task 2/4/6)、5.1 契约的 `render/read/apply/remove/read_back`(Task 3/5/6/9)、5.3 的 CLI 优先与不擅改用户键(Task 9 + Global Constraints)、5.4 漂移纪律(Task 8)、7.3 原生扫描导入(Task 3/5)、M2 验收逐条(三类可被识别、hook 命令完整可见、关闭后干净撤销、settings.json 其他键不变)。
- **未覆盖并显式推迟**:文件监听自动 rescan(Plan 5)、`mcp.enableAllProjectMcpServers` 等用户键永不触碰、Rules 对 claude/codex/gemini/pi 的支持(无证据,标不支持)。
- **类型一致性**:`AssetKind` 与 `AssetType` 互逆由 Task 1 测试钉住；`AgentStatus.capabilities` 与前端 `types.ts` 的 `CapabilityStatus` 字段名由 Task 1 的 JSON 形状断言钉住(沿用 Plan 2 的契约测试做法)。
- **风险**:Task 2/4 的格式来自 teamai-cli 与二进制解析器,**本机没有任何真实样本文件**;所以 Task 11 的真机复核是硬门槛 —— 没跑通之前 qoder-cn 的 rule/subagent/hook 必须停在 `verified=false`,不能因为测试全绿就当已生效。
