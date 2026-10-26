# 项目级 Skills 多 Agent 同步设计

- 日期：2026-10-26
- 状态：已修订（2026-10-26 由产品负责人推翻 §4 的方案选择，改为 B「项目内主副本」；本文各节已按 B 重写，原反对 B 的理由与缓解措施保留在 §4 附录）
- 范围：项目级 Skills 的扫描、真源、渲染部署、安装/更新/卸载、迁移、自动/手动同步
- 后续：项目级 MCP 单独立项，见文末边界

## 1. 背景与目标

项目里的 skill 由各自仓库的 git 管理，不需要跨机器同步。实际痛点是：同一个项目里 Claude Code 读 `.claude/skills/`、Qoder 读 `.qoder/skills/`、Codex/Gemini/Pi 读 `.agents/skills/`；用户常常只写了一份，忘记同步到其他 agent。

目标：

- 项目内保留**唯一可编辑真源** `<project>/.ccswitchx/skills/<id>/`，各 agent 目录是它的**渲染产物**，可随时删除重生成。
- 在客户端按项目查看全部 skill、每个渲染目标的部署状态（待部署 / 已同步 / 真源已更新 / 外部已改 / 未跟踪 / 不可用）。
- 用户选择渲染目标；客户端把整个 skill 目录复制过去。首次只列出不自动补齐；之后支持手动与自动同步。
- 支持和全局 Skills **一样的能力面**：搜索、筛选、新建、从文件夹导入、从来源安装（GitHub/ZIP）、技能市场发现、检查更新、更新、编辑正文、批量部署、卸载、详情。
- 项目范围只处理 Skills；其他资产在项目范围置灰不可点。

不再追求的目标（原 §1 的「不在项目里放第二份 skill 内容」）：见 §4 附录。

## 2. 非目标与反目标

非目标：

- 项目级 MCP / Prompts / Rules / Subagents / Hooks；这些仍只支持全局，后续 MCP 单独立项。
- 跨机器同步项目状态；`.ccswitchx/skills.json` 与 `.ccswitchx/skills/` 跟随仓库 git，客户端不做额外同步。
- 符号链接 / junction 部署：项目级一律整目录复制。链接目录会被扫描判为不可解析，从而永远无法同步或删除。
- 团队权限、审计、冲突合并算法（只做整目录覆盖或人工选择）。
- 项目版「从 cc-switch 导入」：`~/.cc-switch` 是用户级数据，项目范围没有对应概念，**不提供**。

反目标：

- 未经用户确认就在用户仓库里新建内容。
- 静默覆盖部署目标里被外部改过的内容（`edited`）。
- 删除或覆盖未被账本跟踪的副本（`untracked`）。
- 把绝对路径、密钥、易变的检查状态写进随仓库提交的文件。
- 改动或删除项目根之外的任何文件（备份与本机检查缓存除外，它们写在 store 里）。

## 3. 约束

- 文本优先：真源与台账都是可读文本；`skills.json` 与 `.ccswitchx/skills/` 随仓库提交。
- 写入前备份；目录替换原子化（同父目录临时名 + rename）；**写后回读校验**；失败整批回滚。
- Windows 优先，全部用 `PathBuf`；项目根必须规范化（去掉 `\\?\` 逐字前缀），拒绝路径穿越和逃逸的符号链接。
- 能力闸：只有实测确认过项目级落点的 agent 才允许写；未实测的列显示「待实测」且不可操作。**勾选侧与写入侧都要校验**（`targets` 可能来自旧版本或手改的 JSON）。
- 项目级与用户级完全隔离：不读写 `~/.claude/skills` 等用户级目录。
- 项目 JSON 不存绝对路径；agent 用稳定的目录 key 标识。
- 真源与渲染基线的目录指纹**不计入点开头条目**（`.DS_Store`、编辑器临时文件不该触发重渲染；真源里也不写 `.meta.json`，见 §6.5）。
- 易变的检查状态（`lastCheckedAt` / `updateAvailable` / `lastCheckError`）不进随仓库提交的文件，只存本机。
- 不改项目 `.gitignore`，不替用户提交 git。agent 目录是否入库由用户仓库自己决定。

## 4. 方案选择（已修订）

**现选 B：项目内主副本。** `.ccswitchx/skills/<id>/` 是项目真源，三个 agent 目录是渲染产物；安装/更新只写真源，渲染唯一入口是同步引擎。

考虑过的三种：

- **A. 对等副本 + 项目 JSON 台账**（原选定，已实现为 v1 账本）：各 agent 目录互为副本，同步在项目内目录之间复制。仓库里只有一份内容，但代价是：真源里没有的 skill 落不进项目；「哪份是最新的」要靠 mtime 与基线推断；安装必须为每个目录单独落盘；与全局面板的「真源 → 落点」模型不一致，UI 无法对齐。
- **B. 项目内主副本**（选定）：安装/更新/编辑逻辑与全局同构，UI 可直接复用；缺点见附录。
- **C. 复用全局中央库**：`~/.ccswitchx/assets/skills` 做真源再下发到项目。会引入跨项目共享状态，和「项目仓库即真源、不需要跨机器同步」冲突。仍不选。

### §4 附录：原反对 B 的理由与缓解

原理由：仓库里会同时出现 `.ccswitchx/skills` 和 `.claude/skills` 两份内容，和「项目已有 git 管理的 skills」冲突。

接受该代价的理由：A 模型下「项目里安装新 skill」在架构上走不通（同步源只能是项目内另一个目录），且与全局面板的语义差异让 UI 无法对齐。

缓解措施：

1. agent 目录在文档与 UI 上一律称「渲染产物」，明确可随时删除重生成；真源是唯一可编辑副本。
2. 目录指纹跳过点开头条目、空 map 不序列化、键序稳定，压 git diff 噪音。
3. 迁移**只新建文件**，不改动任何已有 agent 目录的字节；apply 前展示「会在你仓库里新建哪些文件」的完整清单。
4. 不提供任何 git 介入（不改 `.gitignore`、不代为提交）。

## 5. 用户体验

### 5.1 侧栏范围切换

侧栏顶部、标题栏下方固定一个项目选择器：

- 全局范围显示「全局配置」，项目范围显示文件夹图标 + 项目名 + 「项目」徽标。
- 点击展开弹层：首行「全局配置」（当前范围打勾）；「项目」分组列出最近项目，按最后打开时间倒序；行悬停出现「…」菜单（在文件夹中显示 / 重新定位 / 移除项目）。
- **选中项目即切到项目范围**，不再单独提供范围分段按钮；切回全局点首行「全局配置」。
- 勾只在「项目范围且该项目为当前项」时显示，保证同时只有一项处于选中态。
- 添加重复项目时直接定位到已有项，不新增。
- 移除项目只从客户端注册表移除，不删除项目文件，也不删除 `.ccswitchx/`；确认文案写明。
- 项目路径失效时，行内显示「路径不存在」，提供「重新定位」和「移除」。
- 菜单弹层是 portal 挂到 `document.body` 的，判定「点到外面」时必须把弹层内的 pointerdown 排除，否则菜单项的 click 收不到。

### 5.2 项目范围只处理 Skills

- 切到项目范围后，资产组里的 MCP、提示词、Rules、Subagents、Hooks 置灰且不可点击，悬停提示「仅支持全局配置」。
- 如果切换范围时当前面板不是 Skills，自动跳到 Skills。
- 「会话」「管理」组不受影响。
- 未添加任何项目时切到项目范围：显示「添加项目」空状态。

### 5.3 项目 Skills 面板（与全局同构）

- 顶部：项目名、路径、「打开文件夹」、「立即同步」、自动同步开关、刷新。
- 视图 tab：已安装（带数量）/ 发现（懒挂载后保持隐藏，不重复拉取）。
- 主区矩阵：
  - 行：项目真源里的 skill ∪ 各 agent 目录里出现的 skill（后者标 `untracked`）。
  - 列：按**物理目录**合并 —— Claude Code → `.claude/skills`；Qoder / Qoder CN → `.qoder/skills`；Codex / Gemini CLI / Pi → `.agents/skills`。列头显示目录 key 与实际使用该目录的 agent 名称；只显示 Settings 里启用的 agent 对应的列。
  - 单元格状态六值：`absent` 待部署 / `present` 已同步 / `stale` 真源已更新 / `edited` 外部已改 / `untracked` 未跟踪 / `unavailable` 不可用（含未实测）。
  - 真源自身单独一个徽标：账本记着有、磁盘上没有 → `canonicalMissing`，只能由显式 purge 处置。
  - 勾选表示该目录是否为渲染目标；取消勾选只写回 `targets` 并保留文件（该副本从此按未跟踪处理）。
  - 行菜单：查看详情、编辑正文、收进真源、移除该目录副本、卸载（删真源 + 全部副本）。
- 安装：复用全局的来源预览/下载/解压与市场入口，**目的改为项目真源**；安装时多选渲染目标目录；安装成功后写 `source` 与基线，再由同步引擎渲染。目标目录不可写（未实测）时在任何写入之前就拒绝。
- 更新检查 / 更新：只对带 GitHub 来源的 skill 可用；更新只写**真源**，随后由同步渲染，因此目标会短暂显示 `stale`（一步一个引擎，避免绕过冲突闸）。本地目录/ZIP 来源只提示「需重新导入」。
- 冲突：多于一个目标被外部改动、或目标与真源都不等于基线时一律视为冲突，不自动写盘；弹对比让用户选择「以某侧为准」（胜出方先 `adopt` 回真源，再渲染其余）或「两边都保留」（落败方另存未跟踪 `<id>-copy`）。只有无歧义情况才自动同步。
- 搜索与筛选：与全局共用同一套纯函数与组件 —— 关键词（名称/ID/描述/来源/仓库）、按部署目录、按部署状态、按来源、仓库多选（存排除集 + 「没有来源仓库」桶）；计数 `{visible} / {total} 个 Skill`。
- 空状态三档：正在读取 / 还没有 Skill（给新建、导入、安装入口）/ 没有匹配的 Skill。
- 迁移横幅：v1 项目首次打开时显示「会在你仓库里新建 N 个文件」的清单与确认入口，文案明写「不改动任何 agent 目录的字节」。

### 5.4 自动与手动同步

- 自动同步默认开启，按项目可关闭，开关存项目注册表。
- 自动触发点：切换到该项目、项目内手动刷新、应用启动后恢复上次项目时。不驻留文件监听。
- 手动同步始终可用，不受自动开关影响。
- 自动同步执行前必须已有渲染目标；首次添加项目不触发写入。
- 自动同步只处理无歧义情况（真源变更 → 渲染到干净的 target）；`edited`、`canonicalMissing` 只记录不写盘，界面标出待处理数量。**自动同步永不产 purge。**
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

- `id = slug(目录名) + "-" + sha256(去掉逐字前缀的规范化绝对路径) 前 12 位`。
- 路径一律存可读形式：`canonicalize()` 的 `\\?\` 前缀必须剥掉（explorer.exe 也不认逐字形式）。读入时对旧数据一并还原，否则同一项目会被当成两条。
- 路径移动视为新项目；旧条目保留为失效路径，可移除或重新定位。
- 注册表不进 assets、不参与 Git 同步、不进备份快照。
- `add_project` 校验：目录必须存在；拒绝 home 本身、storeRoot 内部、任何用户级 agent 配置根内部；拒绝重复路径；允许嵌套项目与没有 git 的目录，UI 提示父子项目可能重复扫描。

### 6.2 项目状态（随项目仓库提交）

路径：`<project>/.ccswitchx/skills.json`，`version: 2`

```json
{
  "version": 2,
  "updatedAt": "2026-10-26T10:00:00Z",
  "skills": {
    "code-review": {
      "targets": ["claude", "qoder"],
      "source": {
        "kind": "github",
        "repo": "owner/repo",
        "branch": "main",
        "subpath": "skills/code-review",
        "treeHash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
      },
      "canonical": {
        "present": true,
        "treeHash": "sha256:…",
        "files": { "SKILL.md": "sha256:…" }
      },
      "dirs": {
        "claude": { "present": true, "treeHash": "sha256:…", "files": { "SKILL.md": "sha256:…" } },
        "qoder": { "present": false }
      },
      "lastOp": { "dir": "qoder", "action": "render", "at": "2026-10-26T10:05:00Z" }
    }
  }
}
```

- `targets` 是物理目录 key，不是逻辑 agent；共享目录只出现一次。
- `canonical` 是真源的上次已知状态；`present: false` 表示真源尚未落地（未迁移 / 未采纳）。
- `dirs` 是**上次成功渲染进各 target 的字节基线**；`present: false` 表示当时该 target 没有这个 skill。空 `files` 不序列化。
- `lastOp.action`：`render | delete | adopt | targets | purge | migrate`。
- `source` 可选。`treeHash` 语义钉死为「安装/更新那一刻**上游候选目录**的 `compute_dir_hash`」，与全局 `SkillMeta.contentHash/latestHash` 同算法，**不得**与 `canonical.treeHash` / `dirs[*].treeHash` 互比（两套哈希函数格式不同：前者裸 hex、跳过点文件；后者 `sha256:` 前缀、清单哈希）。
- 不存绝对路径；mtime 不持久化，只在冲突时用于推荐。
- 键按稳定顺序序列化，原子写，减少 git diff 噪音。
- JSON 损坏显式报错，不当作空表；`version` 高于本程序支持的版本 → 明确报错要求升级，不按旧模型解释。
- 提供「重建基线（会丢失删除记录）」的显式操作。

### 6.3 项目级 agent 落点

| 目录 key | 路径（相对项目根） | 读取该目录的 agent | 实测状态 |
| --- | --- | --- | --- |
| `claude` | `.claude/skills` | Claude Code | 已实测（2026-10-26 真机项目正/负对照） |
| `qoder` | `.qoder/skills` | Qoder、Qoder CN | 已实测（`qoderclicn skills list` 报出项目内 SKILL.md 路径） |
| `agents` | `.agents/skills` | Codex、Gemini CLI、Pi | **未实测**，不可作为渲染目标 |

能力闸按物理目录判定：只有映射到该目录且已在 Settings 启用的全部 agent 都实测通过，该目录才可勾选；未实测的列显示「待实测」，不可勾选，且写入侧同样拒绝。真源**不是**落点，不进能力闸的 key 空间。

### 6.4 目录指纹

- 指纹 = 按相对路径排序后逐文件 SHA-256，再对整个清单 SHA-256，带 `sha256:` 前缀。
- 只统计普通文件与目录；**跳过点开头条目**；符号链接一律拒绝或标记为不可解析，不跟随。
- 上游内容比较用另一个函数（`compute_dir_hash`：裸 hex、流式 `path\0<bytes>\0`、跳过点文件与链接、限深 32）。两个哈希各有用途，不互比。
- 不往真源或渲染目录里写归属标记文件；归属完全由 `skills.json` 的基线与哈希决定。

### 6.5 本机缓存与备份

- 检查状态：`<storeRoot>/local/project-skill-checks/<projectId>.json`（`updateAvailable` / `latestHash` / `lastCheckedAt` / `lastCheckError`）。`local/` 已在 store 的 `.gitignore` 内，天然不同步、不进备份快照。缺失按空表处理。
- 备份：`<storeRoot>/backups/projects/<projectId>/<timestampMillis>/{dirKey|canonical|ledger}/<skillId>/`，其中 `canonical` 是真源备份、`ledger` 是整个 `skills.json` 的备份（迁移前必做）。

## 7. 同步算法

### 7.1 扫描

1. 读注册表，规范化项目根；不存在则报错。
2. 取 Settings 启用的 agent，映射到目录 key；同一物理目录只扫一次。
3. 扫每个目录下的子目录：含 `SKILL.md` 才视为 skill；计算指纹；链接目录记为不可解析并告警。
4. 另扫 `<project>/.ccswitchx/skills/` 得真源状态（不进落点 key 空间）。
5. 取真源 ∪ 各目录的 skill ID 并集，读 `skills.json` 作基线。

### 7.2 差异分类（两轴）

对每个 skill、每个渲染目标：target 相对**基线**是否被外部改动（`edited`），以及相对**真源当前指纹**是否落后（`stale`）。

| 条件（`targets ∋ dir`） | 状态 | 自动同步 |
| --- | --- | --- |
| 目录未启用或未实测 | `unavailable` | 跳过并告警 |
| target 不存在 | `absent` | 渲染 |
| target == 基线 == 真源 | `present` | 无事 |
| target == 基线 ≠ 真源 | `stale` | 渲染（安全覆盖） |
| target ≠ 基线（或无基线） | `edited` | 拒绝，必须人工裁决 |
| `dir ∉ targets` 且 target 存在 | `untracked` | 永不触碰 |

真源侧：`canonical.present == true` 但磁盘没有 → `canonicalMissing`，只产冲突。

未被账本跟踪的 skill 只展示；用户「收进真源」后才参与同步。

### 7.3 决策规则

- 真源变更：干净的 target（== 基线或 absent）一律渲染；`edited` 的转冲突，真源**不**自动赢。
- 真源未变、只有 target 被改：一个 `edited` → 冲突，`recommended` 指向它（语义是「要不要收进真源」）；多个 `edited` 无论内容是否相同都视为冲突，不自动写盘。
- 新勾选目标：磁盘没有 → 直接渲染；磁盘有同名且与真源不同 → 冲突，不静默覆盖。
- 删除传播前必须确认其他目标当前内容仍等于基线；不等则转冲突。
- 删除真源与全部副本（卸载）后，或所有 target 与真源都不存在时，才移除 JSON 条目。**真源还在就必须保留条目**，否则真源成删不掉的孤儿。有失败或不可用目标时保留条目并标记「删除待完成」。
- 人工裁决 `use` = 胜出方先 `adopt` 写回真源，再渲染其余目标（合成一步）；`keepBoth` = 落败方另存未跟踪 `<id>-copy`（重名 `-copy-N`）；`purge` = 真源缺失时删除所有 target，只能来自显式裁决。

### 7.4 写入与回滚

- 渲染：来源可以是真源目录或（收进真源时）某个 agent 目录；在目标父目录内建临时名完整复制后 rename 替换，不跨盘；**rename 后回读校验两侧指纹一致**，不等即失败。
- 删除：只删基线确认属于本项目的目录；符号链接、越界路径一律拒绝。
- 每个被替换/删除的目录先备份；批次内任一动作失败按备份逆序恢复，`skills.json` 不更新；JSON 原子写失败同样回滚目录；JSON 写完回读解析成功才算完成。
- 同一项目同一时间只允许一个同步任务；自动与手动共用互斥锁；批次内步骤顺序固定 `adopt（真源）→ render/delete（target）→ 台账刷新`。

### 7.5 安装 / 更新 / 卸载 / 编辑

- 安装：复用 `skill_source` 的预览/下载/解压（预览只读，staging 留在 store 卷）；**提交 staging 必须落在 `<project>/.ccswitchx/skills/` 内**（与真源同卷，项目可能在另一块盘，跨卷 rename 会失败）；先逐个校验渲染目标可写，再写任何文件；写进真源 → 写 JSON 条目（`targets`、`source`、`dirs` 基线 absent）→ 交给同步引擎渲染。渲染唯一入口永远是同步。
- 项目真源**不写 `.meta.json`**：既污染仓库，又会让每次检查都改指纹从而触发全目标重渲染。检查状态只进本机缓存。
- 更新：仅 GitHub 来源，用 `compute_dir_hash` 与上游比较；只写真源（先备份真源），不直接写 target。每个 target 仍等于基线才渲染替换；有漂移的 target 列为冲突。
- 编辑正文：写 `<project>/.ccswitchx/skills/<id>/SKILL.md`，只动这一个文件（附件保持不动）；校验 UTF-8 + ID 安全 + 写后回读字节一致；先备份真源。注意 `parse_and_validate` 对 Skill 是空实现，不能当安全闸。
- 收进真源：把某个 agent 目录的当前字节写进真源（先备份旧真源），保留 `targets`。
- 移除单个副本：删指定 target 副本（先备份），从 `targets` 移除，基线置 `present:false`；**不删条目**（真源还在）。
- 卸载：删真源与所有 target 副本（同一批次、同一时间戳、同一回滚圈，先备份），移除条目；有漂移时先确认。
- 取消跟踪：从 `targets` 移除指定目录，保留文件，不更新该目录基线。
- 被 Settings 停用的 agent 对应 target：保留在 `targets` 中标记 `unavailable`，同步跳过并提示；重新启用后先显示基线差异，由用户确认再同步。

### 7.6 首次初始化与 v1 迁移

- **新项目**：`add_project` 立即写 `version: 2` 的 `skills.json`，为扫描到的每个 skill 写 `dirs` 现状快照，`targets: []`、`canonical: absent`。只写这个 JSON，不复制、不删除任何内容；矩阵显示「未跟踪」，用户逐项「收进真源」。JSON 已存在时不覆盖。
- **v1 项目迁移**：读路径保持纯读（不在 `load_ledger` 或列表命令里写盘，否则会破坏「只读命令不加写锁」「空跑不改仓库文件」两条不变量）；列表命令只**计算**迁移计划并返回给界面；落盘只由显式命令执行。
- 计划规则（`T` = v1 targets 中 present 的目录，`U` = present 的目录全集），按序命中第一条：

| 条件 | 处置 | 写入 |
| --- | --- | --- |
| `U` 为空 | 移除条目 | 不写文件 |
| `T ≥ 1` 且哈希全等 | 采纳，来源取**字典序最小**的 key（确定性优先，不用 mtime） | 新建 `skills/<id>/**` + v2 JSON |
| `T ≥ 1` 且哈希不等 | 冲突（mtime 最新者仅作推荐） | 不写 |
| `T == 0`、`U == 1` | 提议（默认关） | 采纳才建真源 |
| `T == 0`、`U > 1` 且全等 | 提议（默认开） | 同上 |
| `T == 0`、`U > 1` 且不等 | 冲突 | 不写 |
| 磁盘有 skill 但账本无条目 | 不进任何清单 | **永不自动采纳**（不碰外来内容） |
| 真源已存在且与拟采纳内容不同 | 冲突 | 拒绝覆盖 |
| 该目录被扫描判为不可解析 | 冲突 | 不写 |

- 迁移**只新建文件**，agent 目录一个字节都不改；旧 JSON 先备份；原子写后回读解析成功才算完成；任一步失败按记录的路径精确删除本次新建的真源目录并从备份还原 JSON；冲突未全部裁决就整体拒绝、不做部分写入；幂等（已存在且等 → skip，不等 → 拒），第二次跑计划为空。
- `source` 继承 v1 条目，`kind == "github"` 时补 `treeHash = compute_dir_hash(被采纳的上游候选)`。
- 多机共享同一仓库：另一台首次打开时基线落后 → 全部 `edited` → 不自动写盘，只提供批量「以真源为准」。
- 降级保护：v2 的 `version` 字段要求旧版本程序在读到 `version > 1` 时明确报错（backport），否则它会把渲染产物当对等副本互相复制。

## 8. 后端改动

- `domain/project.rs`：`PROJECT_LEDGER_VERSION = 2`、`PROJECT_CANONICAL_RELATIVE`、`canonical_root`、`canonical_skill_dir`、`ProjectSkillEntry.canonical`、`strip_verbatim`、状态与差异类型。
- `domain/agent.rs`：项目落点映射；不改变用户级 `AgentPaths` 语义。
- `store/projects.rs`：`local/projects.json` 读写、路径规范化、原子写。
- `store/project_checks.rs`（新）：本机检查状态缓存的读写。
- `services/project_scan.rs`：`scan` + `scan_canonical`、`tree_state`（跳点文件）、账本读写与版本闸、`ensure_baseline` 写 v2。
- `services/project_registry.rs`：视图投影（`summary` 与全局 `AssetSummary` 字段对齐）、`selectable_dir_keys` 能力闸、`describe_skill` 优先读真源、`conflict_count` 真算。
- `services/project_sync.rs`：`SyncSourceRef{Canonical,Dir}`、`plan_sync`（含 `allowed_dirs` 写入侧校验）、`apply_render` + `verify_render`、`apply_adopt`、`refresh_ledger`、`set_targets`、`remove_copy`、`uninstall`、`adopt`、`read_body`、`write_body`。
- `services/project_migrate.rs`（新）：`plan_migration` / `apply_migration`。
- `services/project_install.rs`（新）：`install_project_skill` / `import_project_skill_dir` / `create_project_skill`。
- `services/skill_source.rs` / `skill_update.rs`：抽出 `SkillSink` / `UpdateSink`，让下载、校验、冲突策略可复用；项目分支不写中央 assets、不写 `.meta.json`、staging 与真源同卷。
- `commands/projects.rs`：`list_projects` / `add_project` / `remove_project` / `touch_project` / `set_project_auto_sync` / `list_project_skills` / `sync_project_skills` / `set_project_skill_targets` / `remove_project_skill_copy` / `uninstall_project_skill` / `reveal_project_folder`，新增 `preview_project_skill_migration` / `migrate_project_skills` / `preview_project_skill_source` / `install_project_skill` / `install_discovered_project_skill` / `import_project_skill_dir` / `create_project_skill` / `read_project_skill_body` / `write_project_skill_body` / `adopt_project_skill` / `check_project_skill_updates` / `update_project_skills`。写命令一律 `project_ops` → `enter_write()` → 校验项目在注册表且路径存在；网络类命令额外 `begin_skill_job()`。
- `state.rs`：项目互斥锁；不把项目状态塞进 `AppSettings`。
- `error.rs`：`Project(String)`、`SyncConflict` 等。
- `index/`：**不加表**，项目状态每次由 JSON + 文件系统现算；将来要加也必须可重建。
- `backups/`：项目级备份目录；不进 Git 同步。

## 9. 前端改动

- `components/shell/ScopeSwitcher.tsx`：项目选择器（选中项目即切范围、单一选中态、portal 菜单的外点豁免）。
- `panels/SkillsPanel.tsx` 与 `panels/ProjectSkillsPanel.tsx`：共用 `skillFilters`（搜索/来源/仓库/筛选谓词的纯函数）与 `skillScope`（作用域适配器，两个实现：中央库 / 项目真源）；行列表直接用现成的 `AssetRow` + `SkillSourceBadge`，项目行经 `projectRow()` 折算成 `AssetSummary` 形状，两个面板因此不需要知道彼此的存在。
  - 实现期修订：原计划的 `SkillInstalledView` 与 `MatrixCells` 两个抽取**没有做**。全局行列表的 DOM 是冻结的回归证明（`SkillsPanel.test.tsx` 一行不改必须绿），而项目格子的六态必须写成文字（`已同步`/`待更新`/`已改`/`无`/`未跟踪`/`待实测`）才说得清 —— 全局那颗圆点只表达「开/关」两态，把「已改（要人工裁决）」压成颜色正是最容易误导人的地方。项目格子仍占全局格子那一列宽（`w-9`），只是里面放文字 + 悬停说明。复用因此停在行组件这一层，没有再往上抽。
- `components/skills/`：`SkillInstallSheet` / `SkillDiscoveryPanel` / `SkillFormSheet` 接 `scope`；`SkillDetailSheet` 签名不变（可选 `extra` 传项目专属信息）。
- `panels/ProjectSkillsPanel.tsx`：迁移横幅与 `ProjectMigrationSheet`、立即同步 / 自动同步 / 结果条 / 冲突对话框 / 收进真源。
- `hooks/useProjects.ts`、`hooks/useProjectSkills.ts`：项目列表、当前项目、增删、自动同步设置、项目 skill 视图。
- `types.ts`：`ProjectView`、`ProjectSkillView`（含本机检查状态 `check`）、六态单元格、`SyncReport`、`ProjectInstallReport`、冲突与迁移类型；前端行数据由 `projectRow()` 折算成 `AssetSummary` 形状，不另立一套。
- `lib/api.ts`：项目相关 invoke，参数 camelCase。
- 范围与项目选择持久化在 localStorage；启动时校验项目仍存在，失效回退全局。
- 用户可见文案全部中文。

## 10. 测试与真机验证

Rust 单测（全部 `tempfile` 临时 store + 临时项目根）：

- 指纹：跳过点开头条目；真源扫描拒绝非法 ID 与链接目录；版本闸拒绝更高版本。
- 迁移：targets 一致时采纳且 agent 目录字节不变；不一致时整体拒写；无 target 时默认关；账本无条目永不自动采纳；二次跑幂等；备份旧 JSON 并回读解析；失败回滚新建目录；计划列出全部将新建文件。
- 同步：真源变更渲染进干净 target；`stale`/`edited` 判定正确；`edited` 不被覆盖；回读不符整批回滚；非 selectable 目录写入被拒；真源存在则条目不删；purge 只来自显式裁决且自动同步不产；`adopt` / `write_body` 只动 `SKILL.md`；`remove_copy` 保留条目；`uninstall` 一批删真源与全部 target；空跑不改 JSON；不可解析目录永不驱动删除。
- 安装/更新：store sink 行为与改造前逐条一致（回归锚点）；项目分支不写 `.meta.json`；staging 落在真源父目录内；检查不碰 `skills.json`；上游比较用 `compute_dir_hash`。
- 视图：`conflict_count` 真算；`describe_skill` 优先真源；新项目写 v2 空真源；能力闸拒绝 `.agents`。

前端（Vitest，mock `../lib/api`）：

- `SkillsPanel.test.tsx` **一行不改必须全绿**（抽取的回归证明）。
- `skillFilters` / `skillScope` 纯函数与适配器调用矩阵测试。
- 项目面板：范围切换与置灰、自动跳 Skills、`stale`/`edited` 标签、迁移文案（明写「只新建文件、不改 agent 目录」）、搜索与目录/状态/来源/仓库筛选、批量部署与取消部署、安装抽屉带 `targets`、`···` 里没有 cc-switch 导入、单一选中态、菜单项在 portal 下仍点得动。

真机验证（2026-10-27 已跑，结论如下）：

- 临时 store + `baseOverride` 临时 home + 临时项目目录；禁止拿用户真实 `~/.claude`、`~/.qoder-cn` 或真实仓库当试验田。
- `scripts/seed-ui-e2e.ps1 -Project` 铺项目沙箱，`scripts/drive-ui-e2e.cjs` 走「添加项目 → 收进真源 → 勾选两个目录 → 同步 → 搜索筛选 → 编辑正文 → 再同步」并逐张看截图。几点与最初写法不同，都是为了让断言真的成立：
  - 「添加项目…」开的是原生文件夹对话框，CDP 操作不了 —— driver 直接调同一条 `add_project` IPC，注册表 / 台账 / 能力闸仍全部由后端算。
  - 项目默认开着自动同步，勾完目标立刻铺盘；流程先关掉它，才验得到「勾选只改台账、同步才写盘」这条两步纪律。
  - 已断言：收进真源只新建 `.ccswitchx/skills/probe`，原 `.claude/skills/probe` 字节不变；勾选两个目录后磁盘仍无变化；同步后两份产物与真源逐字相同；编辑正文只有真源先变，两份产物保持落后直到下一次同步；`.agents/` 全程没被创建。
  - **迁移横幅没走 UI**：沙箱项目是新建的（直接 v2，无迁移可做），v1→v2 的迁移路径由 `project_migrate` 的 Rust 测试与 `#[ignore]` 集成测试覆盖。
- 用户级配置的保护：`#[ignore]` 集成测试结尾用 `compute_dir_hash` 比对真实 `~/.claude/skills`、`~/.qoder-cn/skills` 与三个用户级配置文件的字节，证明项目级同步一格都没碰到 home。
- 能力档位：`.claude/skills` 与 `.qoder/skills` 已有真机读取证据（`domain/project.rs` 注明日期与证据）；`.agents/skills` 本机取不到证（codex CLI 无 skills 概念、gemini 未安装、pi 报 Connection error），**继续 `verified: false`，测试全绿不升档**。

门禁：`pnpm build` → `cargo test --manifest-path src-tauri/Cargo.toml` → `pnpm test`（顺序不能反，`tauri::generate_context!` 编译期读 `dist/`）。

## 11. 分期

分支 `project-skills-parity`，括号内是落地提交。

- P0：本文档改口径（`e8f28b1`）。
- P1：v2 模型与扫描地基（`e9769e1`）。
- P2：v1→v2 迁移（`3025fbb`）。**preview（只读）与 apply 分两步发布**，先在真实仓库核对「将新建哪些文件」清单。
- P3：同步引擎按真源重写（`7a6a847`）。
- P4：安装/更新管道参数化到项目真源 + 本机检查缓存 + 项目命令（`02e658b`、`86bd649`、`0711a81`，回归锚点 `3d367de`）。
- P5：前端抽取（`6918f27` 筛选项 + `skillScope` 作用域适配器）。原计划的 `MatrixCells` / `SkillInstalledView` 未做，理由见 §9。
- P6：项目面板接上全量能力（`0653cc1`）。
- P7：集成测试扩段与真机取证（`c466c5e`、`52d6c4b`）；`.agents/skills` 本机取不到证，继续 `verified: false`。
- 后续：项目级 MCP 独立设计。其他资产的项目级支持不在本期。

## 12. 已定决策与实现者裁量

已定（含 2026-10-26 修订）：

- **项目内主副本（方案 B）**：`.ccswitchx/skills/<id>/` 是唯一可编辑真源，agent 目录是渲染产物。（原为「不做项目主副本」，由产品负责人推翻。）
- 侧栏选中项目即切项目范围；项目范围只有 Skills；其他资产置灰。
- 新项目直接写 v2：`targets` 空、`canonical` absent，不复制内容；旧项目走显式迁移，迁移只新建文件。
- 只有 `targets` 参与渲染；取消勾选保留文件；移除副本/卸载才删文件。
- 真源变更可安全覆盖 `stale` 目标；`edited` 与 `untracked` 永不自动覆盖或删除。
- **真源与所有 target 都消失才移除条目**（原「所有 target 删光即移除」在 B 方案下会制造孤儿真源，已改）。
- 项目状态与真源随仓库提交，不存绝对路径；易变检查状态只存本机；来源记 `repo/branch/subpath/treeHash`。
- 共享物理目录按「已在 Settings 启用的 reader agent 全部实测通过」才可作为目标；勾选侧与写入侧双重校验。
- 项目级写入只复制，不 symlink/junction；不写 `.meta.json` 与归属标记文件。
- 项目根拒绝 home、storeRoot 和用户级 agent 配置根内的路径。
- 「从 cc-switch 导入」仅全局，不做项目版。
- 安装 / 导入 / 新建**只落项目真源，不带渲染目标**：往仓库里的 agent 目录写文件必须由「勾选 + 同步」这一步显式发生。
- 检查更新的结果只写本机 `local/project-skill-checks/<pid>.json`，绝不进随仓库提交的 `skills.json`；更新只换真源，渲染留给下一次同步。
- Skills 先行，MCP 后做。

实现者可自行决定（写进实现计划即可）：

- 共享目录的列头展示方式（多 agent 名称堆叠或合并文案）。
- 自动同步的结果展示形式（结果条或 toast）。
- 同步批次内动作顺序、备份保留数量。
- 项目格子的具体视觉：实现期定为「占全局格子那一列宽（`w-9`），里面写状态文字 + 悬停说明」，不用全局的两态圆点 —— 六态里「已改（要人工裁决）」不能被压成一种颜色。（2026-10-27 定，真机截图已核对。）

## 13. 项目级 MCP 边界（后续）

- MCP 项目级需要单独立项：它是共享 JSON/TOML 里的键级合并，不适合照搬 skill 的整目录复制模型。
- 首要未决问题：项目级 MCP 配置通常会随仓库提交，真实密钥不能写进去；需要决定是用环境变量引用、项目本地未跟踪文件，还是仅允许无密钥服务器。
- 本期只保证项目范围下 MCP 置灰不可点，不改 MCP 现有全局行为。
