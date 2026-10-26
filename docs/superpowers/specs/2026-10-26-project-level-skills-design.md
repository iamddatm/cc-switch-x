# 项目级 Skills 多 Agent 同步设计

- 日期：2026-10-26
- 状态：草稿（待评审）
- 范围：项目级 Skills 的扫描、矩阵、安装/更新/卸载、多 agent 同步、自动/手动触发
- 后续：项目级 MCP 单独立项，见文末边界

## 1. 背景与目标

项目里的 skill 由各自仓库的 git 管理，不需要跨机器同步。实际痛点是：同一个项目里 Claude Code 读 `.claude/skills/`、Qoder 读 `.qoder/skills/`、Codex/Gemini/Pi 读 `.agents/skills/`；用户常常只写了一份，忘记同步到其他 agent。

目标：

- 在客户端按项目查看全部项目级 skill，以及每个 agent 目录里“有 / 无 / 内容不同”。
- 用户选择要同步到的 agent；客户端把整个 skill 目录复制过去。
- 首次只列出，不自动补齐；之后支持手动和自动同步。
- 以最后操作为准：一边删了、改了、加了文件，同步到其他已勾选目录；删除也传播。
- 支持和全局 Skills 一样的安装、更新检查、更新、卸载。
- 项目范围只处理 Skills；其他资产在项目范围置灰不可点。
- 不引入中央库、不做跨机器同步、不在项目里放第二份 skill 内容。

## 2. 非目标与反目标

非目标：

- 项目级 MCP / Prompts / Rules / Subagents / Hooks；这些仍只支持全局，后续 MCP 单独立项。
- 跨机器同步项目状态；`.ccswitchx/skills.json` 跟随仓库 git，客户端不做额外同步。
- 项目内主副本（`.ccswitchx/skills/<id>/`）和符号链接部署。
- 文件系统实时 watcher；自动同步只在项目激活、手动刷新、手动同步时执行。
- 在应用内编辑 skill 正文；本期只做查看、对比、同步、安装、更新、卸载。
- 团队权限、审计、冲突合并算法（只做整目录覆盖或人工选择）。

反目标：

- 未经用户选择就把一个 agent 的 skill 自动复制到其他 agent。
- 删除未被本项目 JSON 跟踪的目录。
- 两边都改过时静默覆盖。
- 把绝对路径、密钥写进随仓库提交的文件。
- 改动或删除项目根之外的任何文件。

## 3. 约束

- 文本优先：项目状态是可读 JSON；任何只存在于索引里的数据都是设计错误。
- 写入前备份；目录替换原子化；失败整批回滚。
- Windows 优先，全部用 `PathBuf`；项目根必须规范化，拒绝路径穿越和逃逸的符号链接。
- 能力闸：只有实测确认过项目级落点的 agent 才允许写；未实测的列显示“待实测”且不可操作。
- 项目级与用户级完全隔离：不读写 `~/.claude/skills` 等用户级目录。
- 项目 JSON 不存绝对路径；agent 用稳定的目录 key 标识。
- 项目级写入只复制，不创建 symlink / junction。
- 不改项目 `.gitignore`，不替用户提交 git。

## 4. 方案选择

考虑过三种：

- **A. 对等副本 + 项目 JSON 台账（选定）**：各 agent 项目目录互为副本，JSON 记录每个目录的文件清单、哈希和最近操作；安装直接写勾选目录，来源信息记在 JSON。仓库里只有 agent 目录，没有第二份 skill 内容。
- **B. 项目内主副本**：`.ccswitchx/skills/<id>/` 作为真源，再复制到 agent 目录。安装/更新逻辑最简单，但仓库里会同时出现 `.ccswitchx/skills` 和 `.claude/skills` 两份内容，和“项目已有 git 管理的 skills”冲突。
- **C. 复用全局中央库**：`~/.ccswitchx/assets/skills` 做真源再下发到项目。会引入跨项目共享状态，和“项目仓库即真源、不需要跨机器同步”冲突。

选 A。代价是安装/更新需要把来源信息记进项目 JSON，并处理“哪些目录算已跟踪”的边界。

## 5. 用户体验

### 5.1 侧栏范围切换（方案 A）

侧栏顶部、标题栏下方固定一行上下文切换器：

- 全局：显示“全局配置”+ 地球/层叠图标 + 展开箭头。
- 项目：显示文件夹图标 + 项目名 + “项目”徽标 + 展开箭头。
- 点击展开弹层：
  - “切换范围”：全局配置。
  - “项目”：最近项目列表，当前项打勾；支持搜索；行悬停出现“…”菜单（移除项目、在文件夹中显示）。
  - 底部固定“添加项目…”，调用原生目录选择器。
- 添加重复项目时直接定位到已有项，不新增。
- 移除项目只从客户端注册表移除，不删除项目文件，也不删除 `.ccswitchx/skills.json`；确认文案写明。
- 项目路径失效时，行内显示“路径不存在”，提供“重新定位”和“移除”。

### 5.2 项目范围只处理 Skills

- 切到项目范围后，资产组里的 MCP、提示词、Rules、Subagents、Hooks 置灰且不可点击，悬停提示“仅支持全局配置”。
- 如果切换范围时当前面板不是 Skills，自动跳到 Skills。
- “会话”“管理”组不受影响：会话仍是只读本地视图，Agents/网络/备份/Settings 仍是全局设置。
- 未添加任何项目时切到项目范围：自动打开项目弹层或显示“添加项目”空状态。

### 5.3 项目 Skills 面板

- 顶部显示项目名、路径、“打开文件夹”、“立即同步”、自动同步开关。
- 主区沿用现有 Skills 矩阵：
  - 行：项目内所有 agent 目录的 skill 并集。
  - 列：按**物理目录**合并，而不是按 agent 重复显示：
    - Claude Code → `.claude/skills`
    - Qoder / Qoder CN → `.qoder/skills`
    - Codex / Gemini CLI / Pi → `.agents/skills`
  - 列头显示目录 key 和实际使用该目录的 agent 名称；只显示 Settings 里启用的 agent 对应的列。
  - 单元格状态：未部署、已同步、内容不一致、待实测、不适用。
  - 勾选状态表示该目录是否属于这个 skill 的同步目标；取消勾选只写回 `targets` 并保留文件（副本从此不受跟踪）。
  - 目录菜单提供“移除副本”（删除该目录内容）和“卸载”（删除所有目标副本并移除条目），两者都会先备份。
- 首次打开项目：
  - 添加项目时已扫描并写入基线（见 7.6）；矩阵先显示“未跟踪”。
  - 不自动复制；用户逐个或批量勾选目标目录。
  - 勾选只更新 `targets`；点击“同步”才复制内容并刷新基线。
- 安装技能：复用全局安装/来源预览/技能市场入口，目标改为项目目录；安装成功后写入来源信息和基线。
- 更新检查 / 更新：只对带 GitHub 来源信息的 skill 可用；本地目录/ZIP 安装只提示“需重新导入”。
- 卸载：从所有已跟踪目录删除副本，并移除 JSON 条目；删除前备份，内容漂移时先确认。
- 技能详情：展示每个目录的文件清单、树哈希、最近操作、来源；提供“在文件夹中显示”，本期不做应用内编辑。
- 冲突：多于一个目标自上次基线后发生变化（改、删、加任意组合）时一律视为冲突，不自动写盘；弹对比让用户选择保留哪边或“两边都保留”。只有恰好一个目标变化时才自动同步。
- “两边都保留”：把落败版本另存为该目录下的未跟踪 `<id>-copy`（重名时 `-copy-2`），不写入 `targets`。
- 同步结果：顶部结果条或 toast 汇总新增/更新/删除数量与失败项，可展开查看具体目录。

### 5.4 自动与手动同步

- 自动同步默认开启，按项目可关闭，开关存项目注册表。
- 自动触发点：切换到该项目、项目内手动刷新、应用启动后恢复上次项目时。不驻留文件监听。
- 手动同步始终可用，不受自动开关影响。
- 自动同步执行前必须已有跟踪目标；首次添加项目不触发写入。
- 自动同步只处理“恰好一个目标变化”的无歧义情况；出现冲突只记录、不写盘，界面标出待处理数量。
- 自动同步失败不重试轰炸；结果条保留错误，用户可手动重试。

## 6. 数据模型

### 6.1 项目注册表（本机专属）

路径：`<storeRoot>/local/projects.json`

```json
{
  "version": 1,
  "projects": [
    {
      "id": "cc-switch-x-1a2b3c4d5e6f",
      "path": "D:/repos/cc-switch-x",
      "name": "cc-switch-x",
      "addedAt": "2026-10-26T10:00:00Z",
      "lastOpenedAt": "2026-10-26T10:00:00Z",
      "autoSync": true
    }
  ]
}
```

- `id = slug(目录名) + "-" + sha256(规范化绝对路径) 前 12 位`。
- 路径移动视为新项目；旧条目保留为失效路径，可移除或重新定位。
- 注册表不进 assets、不参与 Git 同步、不进备份快照。
- `add_project` 校验：目录必须存在且是目录；规范化后拒绝 home、storeRoot、以及任何位于用户级 agent 配置根（`~/.claude`、`~/.qoder`、`~/.qoder-cn`、`~/.codex` 等）之内的路径；拒绝重复路径；允许嵌套项目和没有 git 的目录，但 UI 提示父子项目可能重复扫描。

### 6.2 项目状态（随项目仓库提交）

路径：`<project>/.ccswitchx/skills.json`

```json
{
  "version": 1,
  "updatedAt": "2026-10-26T10:00:00Z",
  "skills": {
    "code-review": {
      "targets": ["claude", "qoder"],
      "source": {
        "kind": "github",
        "repo": "owner/repo",
        "branch": "main",
        "subpath": "skills/code-review",
        "treeHash": "sha256:..."
      },
      "dirs": {
        "claude": {
          "present": true,
          "treeHash": "sha256:...",
          "files": {
            "SKILL.md": "sha256:...",
            "references/a.md": "sha256:..."
          }
        },
        "qoder": {
          "present": true,
          "treeHash": "sha256:...",
          "files": {}
        }
      },
      "lastOp": {
        "dir": "qoder",
        "action": "delete",
        "at": "2026-10-26T10:05:00Z"
      }
    }
  }
}
```

- `targets` 是物理目录 key，不是逻辑 agent；共享目录只出现一次。
- `dirs` 记录添加项目时扫描到的全部目录快照，便于界面展示；只有 `targets` 参与同步判定。取消勾选后该目录快照保留但不再参与同步，重新勾选时按 7.2 的规则处理。
- `dirs` 记录上次成功同步后的基线；`present: false` 表示该目录当时没有这个 skill。
- `lastOp.action`：`add | update | delete | untrack`。
- `source` 可选；GitHub 来源记录 `repo/branch/subpath/treeHash`，与全局安装的 `SkillSourceInput` 保持一致；不钉 commit，检查更新时取分支 HEAD 的内容哈希比较。
- 本地/ZIP 来源只记录 `kind` 和展示标签，不提供在线更新。
- 不存绝对路径、不用 mtime 作为唯一依据；mtime 只在冲突时辅助判断。
- 所有键按稳定顺序序列化，写文件用原子写，减少 git diff 噪音。
- JSON 损坏时显式报错，不当作空表；提供“重建基线（会丢失删除记录）”的显式操作。

### 6.3 项目级 agent 落点

| 目录 key | 路径（相对项目根） | 读取该目录的 agent | 实测状态 |
| --- | --- | --- | --- |
| `claude` | `.claude/skills` | Claude Code | 待 P0 真机确认 |
| `qoder` | `.qoder/skills` | Qoder、Qoder CN | 待 P0 真机确认 |
| `agents` | `.agents/skills` | Codex、Gemini CLI、Pi | 待 P0 真机确认 |

说明：Codex 的用户级旧路径仍是 `~/.codex/skills`，本期不改；项目级按生态通用落点 `.agents/skills`，必须在临时项目中实测后再把对应列标为可用。能力闸按物理目录判定：只有映射到该目录且已在 Settings 启用的全部 agent 都实测通过，这个目录才可作为同步目标；未实测的列显示“待实测”，不可勾选。

### 6.4 目录指纹

- 指纹 = 按相对路径排序后，逐文件计算 SHA-256，再对整个清单计算一次 SHA-256。
- 只统计普通文件与目录；符号链接一律拒绝或标记为外来，不跟随。
- 不往 skill 目录里写归属标记文件；归属由项目 JSON 的基线和哈希决定，避免污染仓库。

## 7. 同步算法

### 7.1 扫描

1. 读取项目注册表，规范化项目根；不存在则报错。
2. 取 Settings 中启用的 agent，映射到对应项目目录 key；同一物理目录只扫一次。
3. 每个目录下逐个子目录扫描：包含 `SKILL.md` 的目录视为 skill；计算树指纹。
4. 取所有目录的 skill ID 并集；读取 `.ccswitchx/skills.json` 作为基线。

### 7.2 差异分类

对每个 skill、每个目录：

- 基线不存在 + 当前不存在：无变化。
- 基线不存在 + 当前存在：`add`。
- 基线存在 + 当前不存在：`delete`。
- 两者都存在且树哈希相同：无变化。
- 两者都存在且树哈希不同：`update`。

未被 JSON 跟踪的 skill 只展示，不参与同步；用户勾选目标后为该 skill 建立条目并写基线。

- 只有 `targets` 里的目录参与差异判定和“最后操作”来源；未勾选目录即使有副本也只展示为“未跟踪”，其修改不会推向目标目录。
- 新勾选一个已有副本的目录时：若该 skill 已有其他 target，以现有 target 内容为准；新目录内容不同则进入冲突；若这是第一个 target，则以该目录当前内容作为源。
- 取消勾选只移除 target，不删除文件；该副本之后按未跟踪处理。

### 7.3 决策规则

- 变化的目录只有一个：它就是最后操作，把它的当前状态应用到其他 `targets`（改则复制，删则删除）。
- 多个目录变化：
  - 内容与存在状态完全相同：视为同一次操作，刷新基线即可。
  - 其余情况（改+改、删+改、删+删但内容/时间不同）：一律视为冲突，不自动写盘；弹对比，mtime 最新的一侧只作为推荐项。
- 删除传播前必须确认其他目标目录当前内容仍等于基线；不相等说明也被改过，转为冲突。
- 删除传播成功后，若所有 target 都已不存在，移除该 skill 的 JSON 条目；有失败或不可用目标时保留条目并标记“删除待完成”。
- 首次选择目标目录时：若目标缺失则复制；若目标已有同名 skill 且未被跟踪，按安装冲突策略处理（覆盖 / 跳过 / 改名），不静默覆盖。

### 7.4 写入与回滚

- 复制：在目标父目录内创建临时目录，完整复制后 rename 替换；不跨盘。
- 删除：只删除基线确认属于本项目的目录；符号链接、越界路径一律拒绝。
- 每个被替换/删除的目录先备份到 `<storeRoot>/backups/projects/<projectId>/<timestamp>/<dirKey>/<skillId>/`。
- 一次同步的所有动作作为一个批次：任一动作失败，按备份恢复已处理目录，JSON 不更新。
- JSON 最后原子写入；JSON 写入失败同样回滚目录。
- 同一项目同一时间只允许一个同步任务；自动与手动共用互斥锁。

### 7.5 安装 / 更新 / 卸载

- 安装：复用 `skill_source` 的预览/下载/解压，先落到临时目录并校验 `SKILL.md`；再复制到用户勾选的目录；最后写 JSON 条目（targets、source、基线）。
- 目标目录已有同名且未被跟踪：走冲突策略；用户选择覆盖时才替换。
- 更新：仅 GitHub 来源。按 `source.repo/subpath` 拉取新版本；每个目标目录内容仍等于基线才替换；有漂移的目标列为冲突，不静默覆盖。
- 卸载：从所有 `targets` 删除（先备份），移除 JSON 条目；有漂移时先确认。
- 本地目录/ZIP 来源：安装后保留来源标签但不提供在线更新；UI 提示重新导入。
- 取消跟踪：从 `targets` 移除指定目录，保留文件，不更新该目录基线；下次扫描按未跟踪处理。
- 移除单个副本：删除指定目录副本（先备份），从 `targets` 移除，基线置 `present:false`；其他 target 继续跟踪。
- 卸载：删除所有 target 副本（先备份），移除 JSON 条目。
- 被 Settings 停用的 agent 对应 target：保留在 `targets` 中但标记 `unavailable`，同步跳过并在界面提示；重新启用后先显示基线差异，由用户确认再同步。

### 7.6 首次初始化

- 添加项目时立即创建 `.ccswitchx/skills.json`：扫描启用 agent 的项目目录，为并集里的每个 skill 写入当前 `dirs` 基线与 `present` 状态，`targets` 为空。这是用户要求的“首次初始化时在项目里加 JSON 记录”。
- 只写这个 JSON，不复制、不删除任何 skill 内容；此时矩阵显示“未跟踪”。
- 用户勾选目标目录并执行首次同步后，才复制副本、更新 `targets` 与基线。
- 若同一 skill 在多个目录已存在且内容不同，首次同步按 7.3 的冲突规则处理。
- JSON 已存在时不覆盖；按现有基线继续。

## 8. 后端改动

- `domain/project.rs`（新）：项目 ID、目录 key、项目落点表、目录状态与差异类型。
- `domain/agent.rs`：补充项目落点映射；不改变用户级 `AgentPaths` 语义。
- `store/projects.rs`（新）：`local/projects.json` 的读写、规范化路径、原子写。
- `services/project_skills.rs`（新）：
  - 注册/移除/最近项目。
  - 扫描、基线读写、差异计算、同步编排。
  - 安装/更新/卸载的项目级适配。
  - 目录指纹、备份、回滚。
- `services/skill_source.rs` / `skill_update.rs`：抽出可复用的下载、校验、冲突策略；项目级调用时不写中央 assets。
- `commands/projects.rs`（新）：
  - `list_projects` / `add_project` / `remove_project` / `touch_project`
  - `get_project_sync_settings` / `set_project_auto_sync`
  - `list_project_skills` / `sync_project_skills`
  - `preview_project_skill_source` / `install_project_skill`
  - `check_project_skill_updates` / `update_project_skills` / `uninstall_project_skills`
  - `untrack_project_skill_target` / `remove_project_skill_copy` / `resolve_project_skill_conflict`
  - `get_project_skill_diff`
- `state.rs`：新增项目同步互斥锁；不把项目状态塞进 `AppSettings`。
- `error.rs`：新增 `Project(String)`、`SyncConflict` 等错误类型。
- `index/`：本期不加表；项目状态从 JSON + 文件系统即时计算。项目或 skill 数量变大后再考虑派生索引，且必须可从 JSON 重建。
- `backups/`：新增项目级备份目录；不进 Git 同步。

## 9. 前端改动

- `src/components/shell/Sidebar.tsx`：顶部范围切换器、项目弹层、添加/移除项目、项目范围置灰逻辑。
- `src/panels/SkillsPanel.tsx`：增加 scope 属性；项目模式下复用矩阵、安装/来源/市场、详情组件；增加同步/自动同步/冲突 UI。
- `src/components/assets/`：矩阵列头支持“共享目录 + 多 agent 名称”；单元格增加“待实测 / 内容不一致”。
- `src/hooks/useProjects.ts`（新）：项目列表、当前项目、添加/移除、自动同步设置。
- `src/types.ts`：Project、ProjectSkill、ProjectSkillState、ProjectSyncReport、冲突类型。
- `src/lib/api.ts`：项目相关 invoke；参数 camelCase。
- 当前范围/项目选择持久化：本机 localStorage 记录 scope 与 projectId；启动时校验项目仍存在，失效则回退全局。
- 用户可见文案全部中文。

## 10. 测试与真机验证

Rust 单测（全部使用 `tempfile` 临时项目 + 临时 store）：

- 扫描并集、共享目录只扫一次。
- 首次添加写 JSON 基线（targets 为空）且不复制内容；勾选目标后复制。
- 只有 target 参与同步；取消勾选保留文件；非 target 的修改不传播。
- 单边修改、单边删除、目录内文件增删改。
- 删除传播；多目标变化一律进入冲突、不写盘；mtime 只作为推荐项。
- 自动同步遇到冲突不写盘，结果留在待处理列表。
- 外来同名目录不被覆盖/删除。
- 被停用 agent 的 target 跳过并标记不可用；重新启用后先显示差异。
- 所有 target 删光后移除 JSON 条目；有失败目标时保留并标记“删除待完成”。
- JSON 损坏报错；重建基线是显式操作。
- 路径穿越、逃逸符号链接、home/storeRoot/用户级根被拒绝。
- 替换/删除前生成备份；批次失败回滚。
- 安装、更新、卸载；漂移目标不被静默覆盖。
- 取消跟踪、移除单个副本、卸载三种动作各自更新 `targets` 与基线。

前端测试（Vitest，mock `../lib/api`）：

- 范围切换；项目范围下非 Skills 置灰不可点。
- 切换范围时自动跳到 Skills。
- 项目弹层添加/移除/失效路径。
- 项目 Skills 矩阵状态、勾选目标、同步结果、冲突对话框。
- 自动同步开关持久化。

真机验证：

- 临时项目目录 + `baseOverride`/临时 store；禁止使用用户真实仓库或 `~/.claude`、`~/.qoder`。
- 对 Claude、Qoder、Codex/Gemini/Pi 分别确认项目目录能被对应 agent 读取；未取得证据的列保持“待实测”。
- `scripts/seed-ui-e2e.ps1` 扩展项目沙箱；`scripts/drive-ui-e2e.cjs` 操作范围切换与同步并截图。

门禁：

- `pnpm build`、`pnpm test`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo check --manifest-path src-tauri/Cargo.toml`（先 `pnpm build`）

## 11. 分期

- P0：设计评审 + 项目级落点真机确认（Claude / Qoder / `.agents/skills`）。
- P1：项目注册表、侧栏范围切换、只读扫描与矩阵、项目 JSON 基线；不写 skill 内容。
- P2：手动同步（整目录复制/删除、冲突、备份、回滚）。
- P3：安装/更新/卸载 + 技能市场复用 + 自动同步 + 结果反馈。
- P4：项目级 MCP（独立设计，含密钥处理）。
- 其他资产的项目级支持不在本期。

## 12. 已定决策与实现者裁量

已定：

- 侧栏方案 A；项目范围只有 Skills；其他资产置灰不可点。
- 对等副本模型，不做项目主副本、不做中央库、不做符号链接。
- 首次添加项目即写 `.ccswitchx/skills.json` 基线，`targets` 为空，不复制内容；用户勾选目标后才同步。
- 只有 `targets` 参与差异判定和最后操作；取消勾选保留文件，移除副本/卸载才删文件。
- 单目标变化即按最后操作同步（含删除传播）；多目标变化一律视为冲突，手动处理，自动同步不写盘。
- 项目状态 `.ccswitchx/skills.json` 随仓库提交，不存绝对路径；来源记录 `repo/branch/subpath/treeHash`。
- 共享物理目录按“已在 Settings 启用的 reader agent 全部实测通过”才可作为目标。
- 所有 target 删光后移除 JSON 条目；有失败/不可用目标时保留并标记“删除待完成”。
- 项目根拒绝 home、storeRoot 和用户级 agent 配置根内的路径。
- Skills 先行，MCP 后做。

实现者可自行决定（写进实现计划即可）：

- 共享目录的列头展示方式（多 agent 名称堆叠或合并文案）。
- 自动同步的具体结果展示形式（结果条或 toast）。
- 同步批次内动作顺序、备份保留数量。

## 13. 项目级 MCP 边界（后续）

- MCP 项目级需要单独立项：它是共享 JSON/TOML 里的键级合并，不适合照搬 skill 的整目录复制模型。
- 首要未决问题：项目级 MCP 配置通常会随仓库提交，真实密钥不能写进去；需要决定是用环境变量引用、项目本地未跟踪文件，还是仅允许无密钥服务器。
- 本期只保证项目范围下 MCP 置灰不可点，不改 MCP 现有全局行为。
