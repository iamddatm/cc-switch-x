# 项目级 Skills 多 Agent 同步实现计划（P1+P2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 CC Switch X 按项目扫描 Skills 目录、展示多 agent 矩阵，并在用户选定目标后用整目录复制/删除在多 agent 之间同步，冲突留给手动处理。

**Architecture:** 项目注册表存本机 `local/projects.json`，项目同步基线存项目内 `.ccswitchx/skills.json`；各 agent 项目目录互为副本，不引入中央库。扫描与哈希是纯读；同步计划与落盘分离，落盘前备份、失败整批回滚。前端新增侧栏范围切换和一个独立的项目 Skills 面板，不改动现有全局 Skills 面板。

**Tech Stack:** Rust（Tauri 2、serde、sha2、chrono、tempfile）+ React 18 + TypeScript + Vite + Vitest + Tailwind。

**Spec:** `docs/superpowers/specs/2026-10-26-project-level-skills-design.md`。本计划覆盖设计文档 5.1–5.4、6、7.1–7.4、7.6、8、9、10 中与“扫描 + 手动/自动同步”有关的部分。设计 7.5 的安装/更新/卸载与 11 的 P3 另立计划；13 的 MCP 项目级另立设计。

## Global Constraints

- Windows 优先；所有路径用 `PathBuf`，项目根必须 `canonicalize` 后再校验与保存。
- 项目级与用户级隔离：只读写用户选定的项目根下的 agent 目录；绝不碰 `~/.claude`、`~/.qoder`、`~/.codex` 等用户级目录。
- 不引入中央库、不做项目内主副本、不在项目里创建 symlink / junction；同步只复制或删除整目录。
- 只有 `targets` 里的目录参与同步判定；未勾选目录只展示，不驱动作何写入。
- 多目标自上次基线后发生变化时一律视为冲突：不写盘，等用户手动选择；单目标变化才自动同步。
- 每次替换/删除前备份到 `backups/projects/<projectId>/<timestamp>/<dir>/<skillId>/`；批次失败按备份回滚。
- `.ccswitchx/skills.json` 不存绝对路径；所有 JSON 原子写、键序稳定。
- JSON 损坏必须显式报错，禁止当空表并覆盖。
- 共享物理目录按目录 key 判能力：只有映射到它且已在 Settings 启用的全部 agent 都实测通过，才允许勾选。
- 项目范围只处理 Skills；MCP / 提示词 / Rules / Subagents / Hooks 在项目范围置灰不可点。
- 所有用户可见文案用中文；代码标识符用英文。
- 所有跨 IPC 的 DTO 使用 camelCase；Rust 结构体加 `#[serde(rename_all = "camelCase")]`。

## Review Focus

1. 把 home、storeRoot、或用户级 agent 配置根（`~/.claude` 等）内的路径添加为项目时，必须拒绝，不能扫描到用户级 skills。
2. skill 目录是符号链接，或目录内包含逃逸项目的符号链接时，必须拒绝/跳过并提示，绝不能跟随删除。
3. 两个目标自基线后都发生变化（改+改、删+改）时，手动和自动同步都必须不写盘，返回冲突。
4. 目标目录对应 agent 在 Settings 被停用或目录不可用时，同步必须跳过并标记，不能写进 UI 看不见的目录。
5. `.ccswitchx/skills.json` 损坏时，列表/同步必须报错并提供显式重建，不能静默覆盖或丢失归属。

---

## File Structure

**新增后端文件**

- `src-tauri/src/domain/project.rs`：项目注册表/账本类型、目录 key、路径校验。
- `src-tauri/src/store/projects.rs`：`local/projects.json` 读写。
- `src-tauri/src/services/project_scan.rs`：目录扫描、树指纹、账本读写、基线初始化。
- `src-tauri/src/services/project_registry.rs`：项目注册、移除、最近打开、自动同步设置、只读视图。
- `src-tauri/src/services/project_sync.rs`：差异计算、同步计划、落盘、备份/回滚、目标选择。
- `src-tauri/src/commands/projects.rs`：薄 IPC 层。

**修改的后端文件**

- `src-tauri/src/domain/mod.rs`、`src-tauri/src/store/mod.rs`、`src-tauri/src/services/mod.rs`、`src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/state.rs`、`src-tauri/src/error.rs`、`src-tauri/src/commands/backup.rs`（复用 `open_directory`）。

**新增前端文件**

- `src/components/shell/ScopeSwitcher.tsx`：侧栏范围切换 + 项目弹层。
- `src/hooks/useProjects.ts`：项目注册表、当前项目、自动同步。
- `src/hooks/useProjectSkills.ts`：项目 Skills 视图与刷新。
- `src/panels/ProjectSkillsPanel.tsx`：项目 Skills 矩阵。
- `src/components/projects/ProjectConflictDialog.tsx`：冲突处理。
- `src/__tests__/ProjectSkillsPanel.test.tsx`、`src/__tests__/ScopeSwitcher.test.tsx`。

**修改的前端文件**

- `src/types.ts`、`src/lib/api.ts`、`src/components/shell/Sidebar.tsx`、`src/components/shell/AppShell.tsx`、`src/__tests__/App.test.tsx`。

---

### Task 1: 项目领域模型与路径校验

**Files:**
- Create: `src-tauri/src/domain/project.rs`
- Modify: `src-tauri/src/domain/mod.rs`
- Test: `src-tauri/src/domain/project.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `ProjectEntry { id: String, path: PathBuf, name: String, added_at: String, last_opened_at: String, auto_sync: bool }`
  - `ProjectRegistry { version: u32, projects: Vec<ProjectEntry> }`（Default：version=1，空列表）
  - `ProjectSkillDirKey::{Claude, Qoder, Agents}`，`as_str() -> &'static str`，`const ALL: [Self; 3]`，Serialize 为 `"claude" | "qoder" | "agents"`
  - `ProjectSkillDirSpec { key: ProjectSkillDirKey, relative_path: &'static str, agent_ids: &'static [&'static str], verified: bool }`
  - `pub const PROJECT_SKILL_DIRS: [ProjectSkillDirSpec; 3]`
  - `pub fn dir_spec(key: ProjectSkillDirKey) -> &'static ProjectSkillDirSpec`
  - `pub fn validate_project_root(path: &Path, home: &Path, store_root: &Path, user_level_roots: &[PathBuf]) -> Result<PathBuf, AppError>`
  - `ProjectDirState { present: bool, tree_hash: Option<String>, files: BTreeMap<String, String> }`（derive `Default`）
  - `ProjectSkillSource { kind: String, repo: Option<String>, branch: Option<String>, subpath: Option<String>, tree_hash: Option<String> }`
  - `ProjectSkillOp { dir: String, action: String, at: String }`
  - `ProjectSkillEntry { targets: Vec<String>, source: Option<ProjectSkillSource>, dirs: BTreeMap<String, ProjectDirState>, last_op: Option<ProjectSkillOp> }`
  - `ProjectLedger { version: u32, updated_at: String, skills: BTreeMap<String, ProjectSkillEntry> }`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn project_skill_dirs_are_stable() {
    let specs = &PROJECT_SKILL_DIRS;
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[0].key.as_str(), "claude");
    assert_eq!(specs[0].relative_path, ".claude/skills");
    assert_eq!(specs[1].key.as_str(), "qoder");
    assert_eq!(specs[1].relative_path, ".qoder/skills");
    assert_eq!(specs[2].key.as_str(), "agents");
    assert_eq!(specs[2].relative_path, ".agents/skills");
}

#[test]
fn validate_rejects_home_store_root_and_user_level_roots() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let store = dir.path().join("store");
    let claude = home.join(".claude");
    let project = home.join("work/app");
    for path in [&home, &store, &claude] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::create_dir_all(&project).unwrap();

    assert!(validate_project_root(&home, &home, &store, &[claude.clone()]).is_err());
    assert!(validate_project_root(&store, &home, &store, &[claude.clone()]).is_err());
    assert!(validate_project_root(&claude, &home, &store, &[claude.clone()]).is_err());
    let ok = validate_project_root(&project, &home, &store, &[claude]).unwrap();
    assert_eq!(ok, project.canonicalize().unwrap(), "home 下的普通项目必须允许");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml domain::project`
Expected: FAIL，`project` 模块不存在。

- [ ] **Step 3: 实现类型与校验**

在 `domain/project.rs` 中定义上述类型；`domain/mod.rs` 增加 `pub mod project;` 和需要的 `pub use`。
`validate_project_root` 的行为顺序：
1. `path.is_dir()` 否则 `AppError::InvalidInput("项目目录不存在")`。
2. `canonicalize`。
3. 等于 `home` → 拒绝。
4. 等于或位于 `store_root` 下 → 拒绝。
5. 位于任一 `user_level_roots` 下 → 拒绝。
6. 返回规范化路径。
不要拒绝 `home` 的子目录，也不要拒绝普通嵌套项目。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml domain::project`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/domain/project.rs src-tauri/src/domain/mod.rs
git commit -m "feat(project-skills): 添加项目级领域模型与路径校验"
```

### Task 2: 项目注册表存储

**Files:**
- Create: `src-tauri/src/store/projects.rs`
- Modify: `src-tauri/src/store/mod.rs`
- Test: `src-tauri/src/store/projects.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `pub fn registry_path(store: &Store) -> PathBuf` → `<storeRoot>/local/projects.json`
  - `pub fn load(store: &Store) -> Result<ProjectRegistry, AppError>`
  - `pub fn save(store: &Store, registry: &ProjectRegistry) -> Result<(), AppError>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn missing_registry_is_empty_but_corrupt_registry_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    assert!(load(&store).unwrap().projects.is_empty());

    let path = registry_path(&store);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ 这不是 JSON").unwrap();
    assert!(matches!(load(&store).unwrap_err(), AppError::Json(_)));
}

#[test]
fn registry_roundtrips_with_camel_case_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let registry = ProjectRegistry {
        version: 1,
        projects: vec![ProjectEntry {
            id: "app-1a2b3c4d5e6f".into(),
            path: dir.path().join("app"),
            name: "app".into(),
            added_at: "2026-10-26T00:00:00Z".into(),
            last_opened_at: "2026-10-26T00:00:00Z".into(),
            auto_sync: true,
        }],
    };
    save(&store, &registry).unwrap();
    assert_eq!(load(&store).unwrap(), registry);
    let text = std::fs::read_to_string(registry_path(&store)).unwrap();
    assert!(text.contains("\"lastOpenedAt\""));
    assert!(text.contains("\"autoSync\""));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::projects`
Expected: FAIL，模块不存在。

- [ ] **Step 3: 实现**

用 `store::atomic::write_atomic` + `serde_json::to_vec_pretty`。`ProjectRegistry` 的 `projects` 保持调用方顺序；不排序，列表顺序由服务层维护。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store::projects`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/store/projects.rs src-tauri/src/store/mod.rs
git commit -m "feat(project-skills): 持久化项目注册表"
```

### Task 3: 扫描、树指纹与项目账本

**Files:**
- Create: `src-tauri/src/services/project_scan.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/project_scan.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `ProjectSkillSnapshot { id: String, dirs: BTreeMap<String, ProjectDirState> }`
  - `ProjectScan { skills: BTreeMap<String, ProjectSkillSnapshot>, dirs: Vec<ProjectSkillDirKey>, warnings: Vec<String> }`
  - `pub fn enabled_dir_keys(enabled_agent_ids: &[String]) -> Vec<ProjectSkillDirKey>`
  - `pub fn scan(project_root: &Path, keys: &[ProjectSkillDirKey]) -> Result<ProjectScan, AppError>`
  - `pub fn tree_state(dir: &Path) -> Result<ProjectDirState, AppError>`
  - `pub fn ledger_path(project_root: &Path) -> PathBuf`
  - `pub fn load_ledger(project_root: &Path) -> Result<ProjectLedger, AppError>`
  - `pub fn save_ledger(project_root: &Path, ledger: &ProjectLedger) -> Result<(), AppError>`
  - `pub fn ensure_baseline(project_root: &Path, scan: &ProjectScan) -> Result<ProjectLedger, AppError>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn scan_unions_skills_across_agent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_skill(&root.join(".claude/skills"), "a");
    seed_skill(&root.join(".qoder/skills"), "a");
    seed_skill(&root.join(".agents/skills"), "b");

    let scan = scan(root, &ProjectSkillDirKey::ALL).unwrap();
    assert_eq!(scan.skills.keys().cloned().collect::<Vec<_>>(), vec!["a", "b"]);
    assert_eq!(scan.skills["a"].dirs["claude"].present, true);
    assert_eq!(scan.skills["a"].dirs["qoder"].present, true);
    assert_eq!(scan.skills["a"].dirs["agents"].present, false);
}

#[test]
fn baseline_records_targets_empty_and_never_copies_content() {
    let dir = tempfile::tempdir().unwrap();
    seed_skill(&dir.path().join(".claude/skills"), "a");
    let scan = scan(dir.path(), &ProjectSkillDirKey::ALL).unwrap();
    let ledger = ensure_baseline(dir.path(), &scan).unwrap();

    assert!(ledger.skills["a"].targets.is_empty());
    assert!(ledger.skills["a"].dirs["claude"].present);
    assert!(!dir.path().join(".qoder/skills/a").exists());
    assert!(ledger_path(dir.path()).is_file());
}

#[test]
fn tree_state_is_order_independent_and_tracks_file_changes() {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("a");
    std::fs::create_dir_all(skill.join("references")).unwrap();
    std::fs::write(skill.join("SKILL.md"), "v1").unwrap();
    std::fs::write(skill.join("references/x.md"), "x").unwrap();

    let first = tree_state(&skill).unwrap();
    std::fs::write(skill.join("references/y.md"), "y").unwrap();
    let added = tree_state(&skill).unwrap();
    assert_ne!(first.tree_hash, added.tree_hash);
    std::fs::write(skill.join("SKILL.md"), "v2").unwrap();
    let updated = tree_state(&skill).unwrap();
    assert_ne!(added.tree_hash, updated.tree_hash);
    std::fs::remove_file(skill.join("references/x.md")).unwrap();
    let removed = tree_state(&skill).unwrap();
    assert_ne!(updated.tree_hash, removed.tree_hash);
}

#[cfg(windows)]
#[test]
fn symlink_skill_dirs_are_skipped_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let skills = dir.path().join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    let target = dir.path().join("outside");
    seed_skill(dir.path(), "outside");
    if std::os::windows::fs::symlink_dir(&target, skills.join("evil")).is_err() {
        return; // 没有开发者模式/权限时跳过，避免测试环境差异
    }

    let scan = scan(dir.path(), &[ProjectSkillDirKey::Claude]).unwrap();
    assert!(!scan.skills.contains_key("evil"));
    assert!(scan.warnings.iter().any(|w| w.contains("符号链接")));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_scan`
Expected: FAIL，模块不存在。

- [ ] **Step 3: 实现**

- `enabled_dir_keys`：遍历 `PROJECT_SKILL_DIRS`，只要 `agent_ids` 中任意一个出现在启用列表就返回该 key，按固定顺序去重。
- `tree_state`：递归遍历；`symlink_metadata` 遇到符号链接直接返回 `AppError::InvalidInput`；Windows junction 用 `read_link` 能读到目标的一并拒绝；文件哈希用 `sha2::Sha256`；相对路径统一 `/` 分隔；树哈希 = 对排序后的 `path\0hash\n` 再 SHA-256，前缀 `sha256:`。
- `scan`：每个 key 的目录不存在就当全部 absent；子目录含 `SKILL.md` 才算 skill；任何符号链接目录/文件写入 `warnings` 并跳过，不中断整个扫描。
- `ensure_baseline`：账本已存在就 load 返回，不覆盖；否则创建 version=1、`updated_at` 用 `chrono::Utc::now().to_rfc3339()`、每个 skill `targets: []`、`dirs` 取扫描快照；原子写。
- `load_ledger`：缺失返回默认空账本；解析失败必须返回 `AppError::Json`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_scan`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/project_scan.rs src-tauri/src/services/mod.rs
git commit -m "feat(project-skills): 扫描项目 skills 并生成同步基线"
```

### Task 4: 项目注册与只读视图

**Files:**
- Create: `src-tauri/src/services/project_registry.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/project_registry.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `ProjectView { id: String, path: PathBuf, name: String, added_at: String, last_opened_at: String, auto_sync: bool, exists: bool }`
  - `ProjectDirMeta { key: ProjectSkillDirKey, label: String, relative_path: String, agents: Vec<String>, verified: bool }`
  - `ProjectDirView { key: ProjectSkillDirKey, label: String, relative_path: String, agents: Vec<String>, verified: bool, state: String }`（`present | absent | different | untracked | unavailable`）
  - `ProjectSkillView { id: String, name: String, description: Option<String>, targets: Vec<String>, dirs: BTreeMap<String, ProjectDirView>, source: Option<ProjectSkillSource>, last_op: Option<ProjectSkillOp> }`
  - `ProjectSkillsView { project: ProjectView, dirs: Vec<ProjectDirMeta>, skills: Vec<ProjectSkillView>, conflict_count: usize }`
  - `pub fn project_id(path: &Path) -> String`
  - `pub fn list_projects(store: &Store) -> Result<Vec<ProjectView>, AppError>`
  - `pub fn add_project(store: &Store, home: &Path, path: &Path, enabled_agent_ids: &[String]) -> Result<ProjectView, AppError>`
  - `pub fn remove_project(store: &Store, id: &str) -> Result<(), AppError>`
  - `pub fn touch_project(store: &Store, id: &str) -> Result<ProjectView, AppError>`
  - `pub fn set_auto_sync(store: &Store, id: &str, enabled: bool) -> Result<ProjectView, AppError>`
  - `pub fn list_project_skills(store: &Store, project_id: &str, agents: &[AgentStatus]) -> Result<ProjectSkillsView, AppError>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn add_project_writes_baseline_and_deduplicates_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let home = dir.path().join("home");
    let project = home.join("work/app");
    std::fs::create_dir_all(&project).unwrap();
    seed_skill(&project.join(".claude/skills"), "a");

    let first = add_project(&store, &home, &project, &["claude".into()]).unwrap();
    assert!(first.exists);
    assert!(project.join(".ccswitchx/skills.json").is_file());
    let ids_before = list_projects(&store).unwrap().len();

    let second = add_project(&store, &home, &project, &["claude".into()]).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(list_projects(&store).unwrap().len(), ids_before);
}

#[test]
fn project_id_is_stable_and_safe_for_file_names() {
    let dir = tempfile::tempdir().unwrap();
    let id = project_id(dir.path());
    assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')));
    assert_eq!(id, project_id(dir.path()));
}

#[test]
fn list_project_skills_marks_presence_and_untracked() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let home = dir.path().join("home");
    let project = home.join("work/app");
    std::fs::create_dir_all(&project).unwrap();
    seed_skill(&project.join(".claude/skills"), "a");
    let view = add_project(&store, &home, &project, &["claude".into()]).unwrap();
    let agents = vec![agent_status("claude", true), agent_status("qoder", true)];
    let skills = list_project_skills(&store, &view.id, &agents).unwrap();
    assert_eq!(skills.skills[0].id, "a");
    assert!(skills.skills[0].targets.is_empty());
    assert_eq!(skills.skills[0].dirs["claude"].state, "untracked");
    assert_eq!(skills.skills[0].dirs["qoder"].state, "absent");
}
```

测试辅助函数 `agent_status(id, enabled)` 在当前模块内构造最小 `AgentStatus`。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_registry`
Expected: FAIL，模块不存在。

- [ ] **Step 3: 实现**

- `project_id`：`slug(文件名)` + `-` + `sha256(canonical path)` 前 12 位；复用 `crate::import::slug` 与 `crate::index::sha256_hex`。
- `add_project`：
  - 用 `builtin_agents()` 的 `config_root` + `adapters::paths::expand_root` 构造 `user_level_roots`。
  - 调 `validate_project_root`。
  - 注册表里已有同 canonical path → 返回已有项目并只更新 `last_opened_at`。
  - 扫 `enabled_dir_keys(enabled_agent_ids)`，`ensure_baseline` 写 JSON。
  - 压入注册表并 `save`。
- `remove_project`：只删注册表条目，不碰项目文件。
- `touch_project` / `set_auto_sync`：改字段后保存并返回视图。
- `list_project_skills`：
  - 找注册项目；路径不存在仍返回 `project.exists=false`，但 skill 列表为空并带错误提示字段由命令层处理。
  - 扫描 + 读账本；skill 行集合 = 扫描并集 ∪ 账本条目。
  - 有账本条目：`targets` 照抄；每个目录按 `targets` 判定 `present/absent`，非 target 但有文件 → `untracked`，未启用 agent 的目录 → `unavailable`；有基线且哈希不同 → `different`。
  - 无账本条目：所有存在目录都是 `untracked`，不存在的为 `absent`。
  - `conflict_count` 本任务先为 0；Task 8 接入 diff 后改为真实值。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_registry`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/project_registry.rs src-tauri/src/services/mod.rs
git commit -m "feat(project-skills): 注册项目并生成只读技能矩阵"
```

### Task 5: IPC 命令与项目同步锁

**Files:**
- Create: `src-tauri/src/commands/projects.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/commands/backup.rs`（`open_directory` 改为 `pub(crate)`）
- Modify: `src-tauri/src/state.rs`（新增 `project_ops`）
- Modify: `src-tauri/src/error.rs`（新增 `Project(String)`）
- Modify: `src-tauri/src/lib.rs`（注册命令）
- Test: `src-tauri/src/services/project_registry.rs` 已覆盖核心逻辑；命令层只做参数转换

**Interfaces:**
- Produces Tauri commands:
  - `list_projects(state) -> Vec<ProjectView>`
  - `add_project(state, path: String) -> ProjectView`
  - `remove_project(state, id: String) -> ()`
  - `touch_project(state, id: String) -> ProjectView`
  - `set_project_auto_sync(state, id: String, enabled: bool) -> ProjectView`
  - `list_project_skills(state, project_id: String) -> ProjectSkillsView`
  - `reveal_project_folder(state, project_id: String) -> ()`
- Produces `AppState.project_ops: Arc<Mutex<()>>`，供后续同步命令串行使用。

- [ ] **Step 1: 写失败测试**

命令层不单独测；先在 `services/project_registry.rs` 增加一个测试，确认 `list_project_skills` 在路径失效时不 panic：

```rust
#[test]
fn missing_project_path_returns_empty_skills_instead_of_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let home = dir.path().join("home");
    let project = home.join("gone");
    std::fs::create_dir_all(&project).unwrap();
    let view = add_project(&store, &home, &project, &["claude".into()]).unwrap();
    std::fs::remove_dir_all(&project).unwrap();

    let skills = list_project_skills(&store, &view.id, &[agent_status("claude", true)]).unwrap();
    assert!(!skills.project.exists);
    assert!(skills.skills.is_empty());
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_registry::missing_project_path`
Expected: FAIL，当前实现会在扫描时直接报 IO 错误。

- [ ] **Step 3: 实现**

- `state.rs`：新增 `pub project_ops: Arc<Mutex<()>>`，`AppState::init` 初始化。
- `error.rs`：新增 `#[error("项目操作失败：{0}")] Project(String)`。
- `commands/projects.rs`：
  - 每个命令先从 settings 读启用 agent；`list_agents` 的启用逻辑复用 `detect_all` + `is_agent_enabled`。
  - 无网络/重 IO 的列表命令同步执行；同步/写入命令用 `spawn_blocking` 并先拿 `project_ops`。
  - `add_project` / `remove_project` 等只操作注册表，用 `run_blocking` 包装 IO。
- `commands/backup.rs`：把 `fn open_directory` 改为 `pub(crate) fn open_directory`；项目命令复用。
- `lib.rs`：在 `invoke_handler` 注册七个命令。

- [ ] **Step 4: 跑测试与编译确认通过**

Run:
```bash
cargo test --manifest-path src-tauri/Cargo.toml services::project_registry
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected: PASS / 无错误。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/commands/projects.rs src-tauri/src/commands/mod.rs src-tauri/src/commands/backup.rs src-tauri/src/state.rs src-tauri/src/error.rs src-tauri/src/lib.rs
git commit -m "feat(project-skills): 暴露项目注册与只读矩阵命令"
```

### Task 6: 侧栏范围切换与项目弹层

**Files:**
- Create: `src/components/shell/ScopeSwitcher.tsx`
- Create: `src/hooks/useProjects.ts`
- Modify: `src/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/components/shell/Sidebar.tsx`
- Modify: `src/components/shell/AppShell.tsx`
- Test: `src/__tests__/App.test.tsx`、`src/__tests__/ScopeSwitcher.test.tsx`

**Interfaces:**
- Produces in `types.ts`:
  - `ProjectScope = "global" | "project"`
  - `ProjectView { id, path, name, addedAt, lastOpenedAt, autoSync, exists }`
  - `ProjectSkillDirKey = "claude" | "qoder" | "agents"`
  - `ProjectDirMeta { key, label, relativePath, agents, verified }`
  - `ProjectDirView extends ProjectDirMeta { state: "present" | "absent" | "different" | "untracked" | "unavailable" }`
  - `ProjectSkillView { id, name, description, targets, dirs, source, lastOp }`
  - `ProjectSkillsView { project, dirs, skills, conflictCount }`
- Produces in `api.ts`: `listProjects`, `addProject(path)`, `removeProject(id)`, `touchProject(id)`, `setProjectAutoSync(id, enabled)`, `listProjectSkills(projectId)`, `revealProjectFolder(projectId)`。
- Produces `useProjects()`：`{ projects, active, activeId, loading, error, refresh, add, remove, select, setAutoSync }`；`add` 内部调用 `open({ directory: true, multiple: false })`，取消时返回 `null`。
- Produces `ScopeSwitcher` props：`{ scope, projects, activeProjectId, onScopeChange, onSelectProject, onAddProject, onRemoveProject, onRelocateProject }`。
- `Sidebar` 新增 props：`scope`、`projects`、`activeProjectId`、`onScopeChange`、`onSelectProject`、`onAddProject`、`onRemoveProject`、`onRelocateProject`。
- `AppShell` 在 `scope === "project"` 且当前 pane 属于 `mcp | prompt | rule | subagent | hook` 时自动 `setPane("skill")`；`sessions | agents | network | backup | settings` 不受影响。

- [ ] **Step 1: 写失败测试**

`ScopeSwitcher.test.tsx`：

```tsx
test("项目范围只允许 Skills，其他资产不可点", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /全局配置/ }));
  fireEvent.click(screen.getByRole("button", { name: "项目" }));
  const mcp = screen.getByRole("button", { name: "MCP" });
  expect(mcp).toBeDisabled();
  expect(mcp).toHaveAttribute("title", "仅支持全局配置");
  expect(screen.getByRole("button", { name: "Skills" })).toBeEnabled();
});

test("切到项目范围时从 MCP 自动跳到 Skills", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /全局配置/ }));
  fireEvent.click(screen.getByRole("button", { name: "项目" }));
  expect(screen.getByRole("heading", { name: "Skills" })).toBeInTheDocument();
});
```

`App.test.tsx` 的 api mock 增加 `listProjects: vi.fn().mockResolvedValue([])`，否则现有测试会因未 mock 的调用报错。

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm test -- src/__tests__/ScopeSwitcher.test.tsx src/__tests__/App.test.tsx`
Expected: FAIL，`ScopeSwitcher` 不存在 / 按钮找不到。

- [ ] **Step 3: 实现**

- `ScopeSwitcher`：一行按钮，全局显示“全局配置”，项目显示文件夹名 + “项目”徽标；点击展开弹层。弹层包含：
  - “切换范围”里的“全局配置”；
  - “项目”列表（按 `lastOpenedAt` 降序），当前项打勾；行尾 `Menu` 提供“在文件夹中显示”“移除项目”；`exists === false` 时显示“路径不存在”，菜单提供“重新定位”“移除”；
  - 底部“添加项目…”。
  - 点击外部/Esc 关闭，焦点回触发器；沿用 `CheckPopover` 的监听方式。
- `useProjects`：启动时 `listProjects`；`activeId` 从 `localStorage.getItem("ccswitchx.projectId")` 读取，失效则回退第一个；选择项目时 `touchProject` 并写 localStorage。
- `Sidebar`：在资产组上方渲染 `ScopeSwitcher`；`SidebarGroup` 增加 `disabledPanes?: Set<Pane>`，项目范围下 `ASSET_ITEMS` 里除 `skill` 外全部 disabled；按钮加 `disabled`、`aria-disabled`、`title="仅支持全局配置"`、`opacity-45`，不触发 `onSelect`。
- `AppShell`：持有 `scope`（localStorage `"ccswitchx.scope"`）和 `useProjects`；项目范围且当前 pane 是非 Skill 资产时跳到 Skills；`skill` pane 在项目范围且无当前项目时显示 `ProjectEmptyState`（含“添加项目…”按钮，调用 `useProjects.add`），有项目时先渲染一个本地占位 `<div>项目 Skills 面板（Task 7 接入）</div>`，Task 7 再替换为真实 `ProjectSkillsPanel`。

- [ ] **Step 4: 跑测试与构建确认通过**

Run:
```bash
pnpm test -- src/__tests__/ScopeSwitcher.test.tsx src/__tests__/App.test.tsx
pnpm build
```
Expected: PASS / 构建成功。

- [ ] **Step 5: 提交**

```bash
git add src/components/shell/ScopeSwitcher.tsx src/hooks/useProjects.ts src/types.ts src/lib/api.ts src/components/shell/Sidebar.tsx src/components/shell/AppShell.tsx src/__tests__/App.test.tsx src/__tests__/ScopeSwitcher.test.tsx
git commit -m "feat(project-skills): 侧栏支持全局与项目范围切换"
```

### Task 7: 项目 Skills 只读矩阵

**Files:**
- Create: `src/hooks/useProjectSkills.ts`
- Create: `src/panels/ProjectSkillsPanel.tsx`
- Modify: `src/components/shell/AppShell.tsx`（接入真实面板）
- Test: `src/__tests__/ProjectSkillsPanel.test.tsx`

**Interfaces:**
- `useProjectSkills(projectId: string)` → `{ data: ProjectSkillsView | null, loading, error, refresh }`。
- `ProjectSkillsPanel({ project }: { project: ProjectView })`。
- 面板展示：
  - `PageHeader` 标题 Skills，描述 `project.name · project.path`；操作：“打开文件夹”“刷新”。
  - 表头：名称 + 每个 `ProjectDirMeta` 列（label + relativePath 的 title）+ 操作列。
  - 行：skill 名称、描述、每个目录的状态徽标（有 / 无 / 不同 / 未跟踪 / 待实测 / 不可用）。
  - 空状态：项目没有 skill 时显示“项目里还没有 Skills”，描述列出三个目录路径。
  - 错误：`Notice` 显示后端错误；`exists === false` 时显示“项目路径不存在”。

- [ ] **Step 1: 写失败测试**

```tsx
vi.mock("../lib/api", () => ({
  api: {
    listProjectSkills: vi.fn(),
    revealProjectFolder: vi.fn(),
  },
}));

test("renders project skill matrix with per-directory state", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project: projectView(),
    dirs: [
      { key: "claude", label: "Claude Code", relativePath: ".claude/skills", agents: ["claude"], verified: true },
      { key: "qoder", label: "Qoder", relativePath: ".qoder/skills", agents: ["qoder"], verified: true },
    ],
    skills: [{
      id: "code-review",
      name: "code-review",
      targets: [],
      dirs: {
        claude: { key: "claude", label: "Claude Code", relativePath: ".claude/skills", agents: ["claude"], verified: true, state: "present" },
        qoder: { key: "qoder", label: "Qoder", relativePath: ".qoder/skills", agents: ["qoder"], verified: true, state: "absent" },
      },
      source: null,
      lastOp: null,
    }],
    conflictCount: 0,
  });

  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("code-review")).toBeInTheDocument();
  expect(screen.getByText("有")).toBeInTheDocument();
  expect(screen.getByText("无")).toBeInTheDocument();
});

test("shows empty state when project has no skills", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue({
    project: projectView(),
    dirs: [],
    skills: [],
    conflictCount: 0,
  });
  render(<ProjectSkillsPanel project={projectView()} />);
  expect(await screen.findByText("项目里还没有 Skills")).toBeInTheDocument();
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm test -- src/__tests__/ProjectSkillsPanel.test.tsx`
Expected: FAIL，组件不存在。

- [ ] **Step 3: 实现**

- `useProjectSkills`：`useEffect` 在 `projectId` 变化时加载；`refresh` 重新调用；错误转字符串。
- `ProjectSkillsPanel`：用 Tailwind 复刻现有矩阵的间距；状态徽标颜色用现有 `Badge` 的 tone 或普通 span。
- `AppShell`：把占位替换为 `<ProjectSkillsPanel project={activeProject} />`。
- “打开文件夹”调用 `api.revealProjectFolder(project.id)`；“刷新”调用 `refresh`。

- [ ] **Step 4: 跑测试与构建确认通过**

Run:
```bash
pnpm test -- src/__tests__/ProjectSkillsPanel.test.tsx
pnpm build
```
Expected: PASS / 构建成功。

- [ ] **Step 5: 提交**

```bash
git add src/hooks/useProjectSkills.ts src/panels/ProjectSkillsPanel.tsx src/components/shell/AppShell.tsx src/__tests__/ProjectSkillsPanel.test.tsx
git commit -m "feat(project-skills): 添加项目 Skills 只读矩阵"
```

### Task 8: 差异计算与同步计划

**Files:**
- Create: `src-tauri/src/services/project_sync.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/project_sync.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `ChangeKind::{Add, Update, Delete}`
  - `DirChange { dir: String, kind: ChangeKind, state: ProjectDirState, mtime: i64 }`
  - `SyncAction { skill_id: String, action: String, dir: String, source_dir: Option<String> }`（action：`copy | delete`）
  - `SyncConflict { skill_id: String, dirs: Vec<String>, recommended: Option<String> }`
  - `SyncPlan { actions: Vec<SyncAction>, conflicts: Vec<SyncConflict>, warnings: Vec<String> }`
  - `pub fn diff_entry(entry: &ProjectSkillEntry, current: &BTreeMap<String, ProjectDirState>) -> Vec<DirChange>`
  - `pub fn plan_sync(ledger: &ProjectLedger, scan: &ProjectScan, unavailable: &[String]) -> SyncPlan`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn one_changed_target_propagates_to_other_targets() {
    let entry = entry_with_targets(&["claude", "qoder"]);
    let current = states(&[("claude", "h2"), ("qoder", "h1")]);
    let changes = diff_entry(&entry, &current);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].dir, "claude");
}

#[test]
fn two_changed_targets_are_a_conflict_and_produce_no_actions() {
    let entry = entry_with_targets(&["claude", "qoder"]);
    let current = states(&[("claude", "h2"), ("qoder", "h3")]);
    let ledger = ledger_with("a", entry);
    let scan = scan_with("a", current);
    let plan = plan_sync(&ledger, &scan, &[]);
    assert!(plan.actions.is_empty());
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].skill_id, "a");
}

#[test]
fn delete_plus_edit_is_always_a_conflict() {
    let entry = entry_with_targets(&["claude", "qoder"]);
    let mut current = states(&[("claude", "h2")]);
    current.insert("qoder".into(), ProjectDirState { present: false, ..Default::default() });
    let plan = plan_sync(&ledger_with("a", entry), &scan_with("a", current), &[]);
    assert!(plan.actions.is_empty());
    assert_eq!(plan.conflicts.len(), 1);
}

#[test]
fn unavailable_targets_are_skipped_not_deleted() {
    let entry = entry_with_targets(&["claude", "qoder"]);
    let current = states(&[("claude", "h2")]);
    let plan = plan_sync(&ledger_with("a", entry), &scan_with("a", current), &["qoder".into()]);
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].action, "copy");
    assert_eq!(plan.actions[0].dir, "qoder");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_sync`
Expected: FAIL，模块不存在。

- [ ] **Step 3: 实现**

- `diff_entry`：只比较 `entry.targets` 与 `current`；缺失 baseline/current 按 `add` / `delete` 处理；都有且哈希不同为 `update`。
- `plan_sync`：
  - 跳过 `unavailable` 里的目录，不清除 target、不生成 delete。
  - 单变化：`Add/Update` 生成其他 target 的 `copy`，源是该目录；`Delete` 生成其他 target 的 `delete`，但执行前要确认其他 target 当前哈希等于 baseline（在 Task 9 落盘时再校验）。
  - 多变化：如果所有变化目录当前状态完全相同且都是 `Add/Update`，生成刷新基线的 no-op（无 action）；否则生成 conflict，无 action。
  - `recommended`：在非删除变化目录中取最大 `mtime`；删除参与时不推荐。
- `mtime`：目录内文件最大 mtime；目录不存在为 0。仅用于冲突推荐，不参与自动决策。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_sync`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/project_sync.rs src-tauri/src/services/mod.rs
git commit -m "feat(project-skills): 计算目录差异与同步计划"
```

### Task 9: 同步落盘、备份与回滚

**Files:**
- Modify: `src-tauri/src/services/project_sync.rs`
- Modify: `src-tauri/src/commands/projects.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/services/project_sync.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `SyncOutcome { skill_id: String, dir: String, action: String, status: String, backup_path: Option<PathBuf>, error: Option<String> }`
  - `SyncReport { outcomes: Vec<SyncOutcome>, conflicts: Vec<SyncConflict>, warnings: Vec<String> }`
  - `pub fn sync_project(store: &Store, project_id: &str, project_root: &Path, resolutions: &[Resolution]) -> Result<SyncReport, AppError>`
  - `Resolution { skill_id: String, dir: String, action: String }`（`use | keepBoth`）
  - Tauri command `sync_project_skills(project_id, resolutions) -> SyncReport`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn sync_copies_missing_target_and_backs_up_before_delete() {
    let fixture = project_fixture();
    seed_skill(&fixture.root.join(".claude/skills"), "a");
    let view = fixture.add_project(&["claude".into(), "qoder".into()]);
    fixture.set_targets(&view.id, "a", &["claude", "qoder"]);

    let report = fixture.sync(&view.id, &[]).unwrap();
    assert!(report.conflicts.is_empty());
    assert!(fixture.root.join(".qoder/skills/a/SKILL.md").is_file());

    std::fs::remove_dir_all(fixture.root.join(".qoder/skills/a")).unwrap();
    let report = fixture.sync(&view.id, &[]).unwrap();
    assert!(report.conflicts.is_empty());
    assert!(!fixture.root.join(".claude/skills/a").exists());
    assert!(backup_root(&fixture.store).join("projects").is_dir());
}

#[test]
fn sync_conflict_writes_nothing() {
    let fixture = project_fixture();
    seed_skill(&fixture.root.join(".claude/skills"), "a");
    seed_skill(&fixture.root.join(".qoder/skills"), "a");
    let view = fixture.add_project(&["claude".into(), "qoder".into()]);
    fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
    std::fs::write(fixture.root.join(".claude/skills/a/SKILL.md"), "claude v2").unwrap();
    std::fs::write(fixture.root.join(".qoder/skills/a/SKILL.md"), "qoder v2").unwrap();

    let before_claude = std::fs::read(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap();
    let report = fixture.sync(&view.id, &[]).unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(std::fs::read(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap(), before_claude);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_sync::sync_`
Expected: FAIL，`sync_project` 不存在。

- [ ] **Step 3: 实现**

- `sync_project`：
  1. 拿 `project_ops` 锁（命令层）；命令层先按 `project_id` 从注册表取项目，路径不存在直接 `NotFound`。
  2. `load_ledger` + `scan`；`unavailable` = 账本 `targets` 中不在本次扫描目录里的 key（通常是对应 agent 被停用）；再把 `unavailable` 传给 `plan_sync`。
  3. 先应用 `resolutions`：只接受当前冲突里出现的 skill/dir，未知项返回 `AppError::Project`；`use` 指定目录作为源并按单变化传播；`keepBoth` 先把落败目录另存为 `<skill>-copy`，再按 `use` 处理。没有 resolution 的冲突保持不写盘。
  4. 对每个 action：
     - `copy`：源目录必须存在；目标父目录下建临时目录，`copy_dir` 后 rename 替换；目标存在先备份。
     - `delete`：目标存在且当前树哈希等于 baseline 才删；先备份；不相等返回 conflict。
     - `keepBoth`：把落败目录复制成 `<skill>-copy`（重名递增），不写 target。
  5. 所有 action 成功后再更新账本基线（逐目录 `tree_state`）与 `last_op`；任何失败按备份回滚已处理目录，账本不写。
  6. 所有 target 都删除后移除该 skill 条目。
  7. 计划里没有 action 但存在“多个目录当前状态完全相同”的变化时，也要刷新这些目录的基线与 `updated_at`，否则下次会重复判定。
- 备份路径：`<storeRoot>/backups/projects/<projectId>/<timestamp>/<dir>/<skillId>/`。
- `copy` 目标在项目根内校验：`target.starts_with(project_root)`；符号链接拒绝。
- 命令层：`sync_project_skills` 用 `run_blocking`，先拿 `project_ops` 和 `enter_write`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_sync`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/project_sync.rs src-tauri/src/commands/projects.rs src-tauri/src/lib.rs
git commit -m "feat(project-skills): 落盘同步并支持备份回滚"
```

### Task 10: 目标选择、移除副本与卸载

**Files:**
- Modify: `src-tauri/src/services/project_sync.rs`
- Modify: `src-tauri/src/commands/projects.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/services/project_sync.rs`（`#[cfg(test)]`）

**Interfaces:**
- Produces:
  - `pub fn set_targets(store: &Store, project_id: &str, project_root: &Path, skill_id: &str, targets: &[String]) -> Result<ProjectSkillView, AppError>`
  - `pub fn remove_copy(store: &Store, project_id: &str, project_root: &Path, skill_id: &str, dir: &str) -> Result<SyncReport, AppError>`
  - `pub fn uninstall(store: &Store, project_id: &str, project_root: &Path, skill_id: &str) -> Result<SyncReport, AppError>`
  - Tauri commands `set_project_skill_targets(project_id, skill_id, targets) -> ProjectSkillView`、`remove_project_skill_copy(project_id, skill_id, dir) -> SyncReport`、`uninstall_project_skill(project_id, skill_id) -> SyncReport`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn adding_a_second_target_copies_from_existing_target() {
    let fixture = project_fixture();
    seed_skill(&fixture.root.join(".claude/skills"), "a");
    let view = fixture.add_project(&["claude".into(), "qoder".into()]);
    fixture.set_targets(&view.id, "a", &["claude"]);
    fixture.sync(&view.id, &[]).unwrap();

    fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
    assert!(fixture.root.join(".qoder/skills/a/SKILL.md").is_file());
}

#[test]
fn untracking_keeps_files_but_removing_copy_deletes_them() {
    let fixture = project_fixture();
    seed_skill(&fixture.root.join(".claude/skills"), "a");
    let view = fixture.add_project(&["claude".into(), "qoder".into()]);
    fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
    fixture.sync(&view.id, &[]).unwrap();

    fixture.set_targets(&view.id, "a", &["claude"]);
    assert!(fixture.root.join(".qoder/skills/a").is_dir(), "取消勾选保留文件");

    fixture.remove_copy(&view.id, "a", "qoder");
    assert!(!fixture.root.join(".qoder/skills/a").exists());
    assert!(fixture.root.join(".claude/skills/a").is_dir());
}

#[test]
fn uninstall_removes_all_copies_and_ledger_entry() {
    let fixture = project_fixture();
    seed_skill(&fixture.root.join(".claude/skills"), "a");
    let view = fixture.add_project(&["claude".into(), "qoder".into()]);
    fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
    fixture.sync(&view.id, &[]).unwrap();
    fixture.uninstall(&view.id, "a");

    assert!(!fixture.root.join(".claude/skills/a").exists());
    assert!(!fixture.root.join(".qoder/skills/a").exists());
    let ledger = load_ledger(&fixture.root).unwrap();
    assert!(!ledger.skills.contains_key("a"));
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_sync::targets`
Expected: FAIL，函数不存在。

- [ ] **Step 3: 实现**

- `set_targets`：
  - `targets` 只能包含已验证且当前启用的目录 key；非法 key 直接 `AppError::Project`，不写盘。
  - 已存在的 target 保留；
  - 新增 target：找现有 target 的源内容；新目录已有不同内容 → 返回 `AppError::Project("目标目录已有不同内容，请先处理冲突")`，不写；
  - 新目录缺失 → 从源复制并写基线；
  - 取消 target → 从 `targets` 移除，保留文件和基线。
- `remove_copy`：先备份再删指定目录；从 targets 移除，基线置 `present:false`；若 targets 为空则移除条目。
- `uninstall`：对所有 targets 逐个备份并删除；移除条目。
- 命令层用 `run_blocking` + `project_ops` + `enter_write`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::project_sync`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/project_sync.rs src-tauri/src/commands/projects.rs src-tauri/src/lib.rs
git commit -m "feat(project-skills): 支持目标选择、移除副本与卸载"
```

### Task 11: 前端同步操作与冲突处理

**Files:**
- Modify: `src/types.ts`（`ProjectSyncReport`、`ProjectConflict`、`ProjectConflictResolution`）
- Modify: `src/lib/api.ts`
- Modify: `src/panels/ProjectSkillsPanel.tsx`
- Create: `src/components/projects/ProjectConflictDialog.tsx`
- Test: `src/__tests__/ProjectSkillsPanel.test.tsx`

**Interfaces:**
- `api.setProjectSkillTargets(projectId, skillId, targets) -> ProjectSkillView`
- `api.syncProjectSkills(projectId, resolutions: ProjectConflictResolution[] = []) -> ProjectSyncReport`
- `api.removeProjectSkillCopy(projectId, skillId, dir) -> ProjectSyncReport`
- `api.uninstallProjectSkill(projectId, skillId) -> ProjectSyncReport`
- `ProjectConflictDialog({ conflicts, onCancel, onResolve })`，`onResolve(resolutions: ProjectConflictResolution[])`。

- [ ] **Step 1: 写失败测试**

```tsx
test("checking a target calls setProjectSkillTargets and sync sends resolutions", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithOneSkill());
  vi.mocked(api.setProjectSkillTargets).mockResolvedValue(viewWithOneSkill().skills[0]);
  vi.mocked(api.syncProjectSkills).mockResolvedValue({ outcomes: [], conflicts: [], warnings: [] });

  render(<ProjectSkillsPanel project={projectView()} />);
  const qoder = await screen.findByRole("checkbox", { name: /Qoder/ });
  fireEvent.click(qoder);
  await waitFor(() => expect(api.setProjectSkillTargets).toHaveBeenCalledWith("p1", "a", ["qoder"]));

  fireEvent.click(screen.getByRole("button", { name: "立即同步" }));
  await waitFor(() => expect(api.syncProjectSkills).toHaveBeenCalledWith("p1", []));
});

test("sync conflict opens dialog and resolves with chosen directory", async () => {
  vi.mocked(api.syncProjectSkills)
    .mockResolvedValueOnce({
      outcomes: [],
      conflicts: [{ skillId: "a", dirs: ["claude", "qoder"], recommended: "claude" }],
      warnings: [],
    })
    .mockResolvedValueOnce({ outcomes: [], conflicts: [], warnings: [] });

  render(<ProjectSkillsPanel project={projectView()} />);
  fireEvent.click(await screen.findByRole("button", { name: "立即同步" }));
  expect(await screen.findByText("需要处理冲突")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "保留 Claude Code" }));
  await waitFor(() =>
    expect(api.syncProjectSkills).toHaveBeenLastCalledWith("p1", [
      { skillId: "a", dir: "claude", action: "use" },
    ]),
  );
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm test -- src/__tests__/ProjectSkillsPanel.test.tsx`
Expected: FAIL，按钮/对话框不存在或 API 未 mock。

- [ ] **Step 3: 实现**

- 矩阵每列的 checkbox：`checked = targets.includes(key)`；`disabled = !dir.verified || dir.state === "unavailable"`；`aria-label = "同步到 " + dir.label`；变更调 `api.setProjectSkillTargets` 后 `refresh`。
- “立即同步”按钮：`api.syncProjectSkills(project.id, [])`；`conflicts.length > 0` 打开对话框；否则显示结果条。
- `ProjectConflictDialog`：每个冲突展示目录和推荐项；按钮“保留 X”生成 `{skillId, dir, action:"use"}`；“两边都保留”生成 `{..., action:"keepBoth"}`；确认后调用 `api.syncProjectSkills(project.id, resolutions)` 并刷新。
- 行菜单加“移除副本”（按目录）和“卸载”（全部目标）；调用对应 API 后刷新。
- 结果条显示新增/更新/删除数量和失败项；错误用 `Notice`。

- [ ] **Step 4: 跑测试与构建确认通过**

Run:
```bash
pnpm test -- src/__tests__/ProjectSkillsPanel.test.tsx
pnpm build
```
Expected: PASS / 构建成功。

- [ ] **Step 5: 提交**

```bash
git add src/types.ts src/lib/api.ts src/panels/ProjectSkillsPanel.tsx src/components/projects/ProjectConflictDialog.tsx src/__tests__/ProjectSkillsPanel.test.tsx
git commit -m "feat(project-skills): 支持手动同步与冲突处理"
```

### Task 12: 自动同步与最终门禁

**Files:**
- Modify: `src/hooks/useProjects.ts`
- Modify: `src/panels/ProjectSkillsPanel.tsx`
- Modify: `src/__tests__/ProjectSkillsPanel.test.tsx`
- Modify: `src/__tests__/ScopeSwitcher.test.tsx`

**Interfaces:**
- `useProjects().setAutoSync(id, enabled)` 调 `api.setProjectAutoSync`。
- `ProjectSkillsPanel` 顶部加“自动同步”checkbox；开启状态来自 `project.autoSync`。
- 面板在首次加载项目且 `project.autoSync` 为 true、且该项目已有 target 时执行一次自动同步；同一次挂载不重复执行。手动“刷新”在自动开启时也触发同步。

- [ ] **Step 1: 写失败测试**

```tsx
// 测试辅助：viewWithOneSkill(skillOverrides, projectOverrides)
test("auto sync runs once on mount when enabled and targets exist", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(
    viewWithOneSkill({ targets: ["claude"] }, { autoSync: true }),
  );
  vi.mocked(api.syncProjectSkills).mockResolvedValue({ outcomes: [], conflicts: [], warnings: [] });

  render(<ProjectSkillsPanel project={projectView({ autoSync: true })} />);
  await screen.findByText("code-review");
  await waitFor(() => expect(api.syncProjectSkills).toHaveBeenCalledTimes(1));
});

test("toggling auto sync persists through setProjectAutoSync", async () => {
  vi.mocked(api.listProjectSkills).mockResolvedValue(viewWithOneSkill());
  vi.mocked(api.setProjectAutoSync).mockResolvedValue(projectView({ autoSync: true }));

  render(<ProjectSkillsPanel project={projectView({ autoSync: false })} />);
  fireEvent.click(await screen.findByRole("checkbox", { name: "自动同步" }));
  await waitFor(() => expect(api.setProjectAutoSync).toHaveBeenCalledWith("p1", true));
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm test -- src/__tests__/ProjectSkillsPanel.test.tsx`
Expected: FAIL，自动同步控件不存在。

- [ ] **Step 3: 实现**

- `useProjects.setAutoSync` 更新注册表并刷新当前项目。
- `ProjectSkillsPanel` 用 `useRef` 记录已自动同步的 `project.id`，避免 StrictMode/重渲染重复触发。
- 自动同步结果用同一结果条展示；有冲突时只显示“有 N 个冲突待处理”入口，不自动弹对话框；手动点“立即同步”才弹对话框。
- 自动同步不改变“首次添加项目不复制”的规则：`targets` 为空时直接跳过。

- [ ] **Step 4: 跑完整门禁**

Run:
```bash
pnpm test
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected: 全部 PASS；`cargo check` 无错误。

- [ ] **Step 5: 提交**

```bash
git add src/hooks/useProjects.ts src/panels/ProjectSkillsPanel.tsx src/__tests__/ProjectSkillsPanel.test.tsx src/__tests__/ScopeSwitcher.test.tsx
git commit -m "feat(project-skills): 支持自动同步与刷新"
```

---

## Follow-up Plans

本计划只交付 P1+P2。以下仍按设计文档推进，但不在本计划内实现：

- **Plan 2：项目级 Skills 来源安装/更新/卸载**（设计 7.5、11 的 P3）。复用 `skill_source` 的预览/下载，直接写入勾选的项目目录；来源写 `repo/branch/subpath/treeHash`；本地/ZIP 不提供在线更新。
- **Plan 3：项目级 MCP**（设计 13）。单独立项，先解决项目配置随仓库提交时的密钥处理。
- **后续：** 其他资产项目级支持、项目预设、文件监听、跨机器同步均不在当前范围。

## Self-Review

- 设计 5.1–5.4、6、7.1–7.4、7.6 均有对应任务；7.5 与 13 明确放到后续计划。
- 所有任务都有失败测试、实现、通过验证、提交四段；命令名、类型名、字段名前后一致。
- Review Focus 的五类输入分别落在 Task 1/4（路径拒绝）、Task 3（符号链接）、Task 8/9（冲突不写盘）、Task 5/10（不可用目标跳过）、Task 3（损坏账本报错）。
- 计划长度控制在可执行范围；未把完整函数体写进计划，只固定签名、类型和关键断言。
