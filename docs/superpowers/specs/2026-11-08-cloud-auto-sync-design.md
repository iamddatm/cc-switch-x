# 云备份自动上传 / 自动下载设计

日期：2026-11-08
状态：待审批

## 1. 背景与意图

当前 WebDAV 云备份只有「自动上传」：真源写入后 1 秒防抖，内容指纹与上次上传一致就跳过。
云端只保留一份 zip + 一份 manifest，没有版本历史，也不能合并。下载只有手动「从云端恢复」。

使用场景是「一台主工作机 + 多台从机器」：

- 主机器负责把真源推到云端；
- 从机器只希望自动拿到云端最新状态，绝不把自己的内容传上去；
- 用户使用坚果云免费版，流量和请求次数都必须省着用。

同时，进入备份面板时，「从云端恢复」按钮要等一次联网状态探测（还要抢 `cloud_ops` 锁）
才可用；网络慢或后台正在传 14 MB 包时，按钮会长时间置灰。这次一并解决。

## 2. 目标

1. 自动上传与自动下载拆成两个独立开关（与 Git 区「自动提交 / 自动拉取」同一套心智）。
2. 主机器可只开自动上传；从机器只开自动下载。两端都只依赖一个「通道启用」开关。
3. 自动下载以云端为准：云端包更新就覆盖本机，不弹冲突选择；覆盖前仍走现有恢复流水线，
   自动创建本地安全快照。
4. 流量可控，适配坚果云免费版（每月上传 1 GB、下载 3 GB，每 30 分钟 600 次请求）：
   - 自动上传：改动停下满一段时间后再传；持续编辑时按兜底上限补传，避免整天反复传整包。
   - 自动下载：只做低频元数据探测，云端没变不下载、不读 manifest。
   - 重复触发、内容相同等情况都不产生多余传输。
5. 备份面板用本地缓存秒开：恢复按钮不再等联网状态；实时状态在后台刷新。

## 3. 非目标

- 不做文件级增量同步。首次/换机要处理 1656 个文件，半小时内必然超过坚果云 600 次请求限制，
  会把账号临时限掉；整包方案请求数恒定，只受流量约束。
- 不加云端多版本历史、合并、冲突解决。
- 不改 Git 备份通道，也不改本地快照策略（自动下载触发的保留清理除外，见 §7.4）。
- 不做「上传配额自动暂停」。只在面板展示本机本月上传量，由用户调间隔。
- 不把 `local/`、`state/`、`secrets.local.json` 纳入同步包（沿用现有快照范围）。
- **不做旧数据兼容**：旧设置文件不做字段迁移；旧 schema 的云端包、没有 manifest 的裸包
  一律按「不兼容，请从源设备重新上传」处理，不保留兼容分支。

## 4. 约束与既有事实

- 当前包约 14 MB（`assets/` 44 MB，Skills 为主）；坚果云单文件上传上限 500 MB，不构成风险。
- 坚果云免费版：1 GB/月上传、3 GB/月下载；每 30 分钟 ≤600 次请求；触发限制后可能被临时封禁数小时。
- 现有恢复流水线 `backup::restore` 会：先建 `PreRestore` 安全快照 → `maintenance` 写锁下覆盖
  `assets/` 与 `agents/` → 重建索引；恢复期间用 `SuppressionGuard` 抑制变更信号。
- `auto_download` 覆盖的是可移植真源；从机器的 `local/`、`state/`、密钥不受影响。
- 云端网络任务统一持 `cloud_ops` 锁；状态探测也持锁，这是当前按钮被拖慢的原因之一。

## 5. 方案取舍

### 方案 A：整包 + 低频合并（选）

- 上传/下载各 1 次请求，靠低频与指纹比对控制流量。
- 复用现有打包、manifest 校验、安全快照与恢复流水线；改动集中在调度、设置、状态缓存与 UI。
- 代价：从机器最多滞后一个上传周期（默认约 1 小时量级）。

### 方案 B：文件级增量（否）

- 只传改动文件，流量极低、接近实时。
- 但首次/换机上千次请求会撞坚果云频率限制；远端布局、删除、原子应用、回滚、旧包迁移都要重做。
- 在免费版上不可靠，放弃；保留为未来换付费网盘或换同步后端的备选。

## 6. 设置模型

```rust
pub struct CloudBackupSettings {
    pub enabled: bool,            // 通道启用，参与「远端备份方式」单选
    pub auto_upload: bool,        // 本机改动后自动上传
    pub auto_download: bool,      // 云端更新后自动覆盖本机（新增）
    pub upload_delay_hours: u32,  // 改动停下后等待多久上传（新增，默认 1）
    pub base_url: String,
    pub username: String,
    pub remote_dir: String,
}
```

- 新字段用 `#[serde(default)]` 让旧 JSON 能反序列化，仅此而已；不写任何语义迁移。
- 删除 `load_from` 里「`auto_upload` 为真就补 `enabled`」的旧迁移逻辑。
- `upload_delay_hours` 允许 1–24；UI 提供 1 / 6 / 12 / 24 小时选项。
- 两个自动开关都必须 `enabled = true` 才能生效；关闭通道会一并关掉它们。
- 既有的 `auto_upload = true` 用户升级后语义从「1 秒防抖」变为「按延迟合并」，这是本次需求本身。

## 7. 后台同步 worker

把现有 `spawn_change_worker` 扩成「上传 + 下载 + 对账」单一 worker，避免两个线程互相抢锁、
互相不知道对方状态。worker 内状态只放内存，不持久化：

```text
pending: Option<{ first_at, last_at, retry_at }>
next_download: Instant
next_reconcile: Instant
```

### 7.1 上传触发与重试（补上 `periodic_once` 移除后的所有入口）

不再依赖 `periodic_once` 触发上传，改为三个入口：

1. **变更信号**：收到信号且 `enabled && auto_upload` → 更新 pending（首次记 `first_at`，每次都更新 `last_at`）。
2. **定期对账**：启动时立即一次，之后每 15 分钟一次。若 `auto_upload` 开启且本地指纹 !=
   `synced_fingerprint` 且当前没有 pending → 新建 pending（`first_at = last_at = now`）。
   这覆盖：启动前遗留的未同步内容、上次上传失败、恢复被抑制但内容已变、刚打开自动上传。
3. **设置保存**：`set_cloud_settings` 成功后调用 `change_signal::notify()` 唤醒 worker；
   若此时自动上传已开启，按入口 1 记 pending。下载的首次检查仍等下一个 15 分钟 tick，
   或用户点「刷新云状态」。

上传时机（满足任一）：

- `now >= last_at + upload_delay_hours`（改动停下满延迟）；
- `now >= first_at + max_wait`，`max_wait = min(upload_delay_hours × 4, 24h)`（持续编辑兜底）。

执行前重新读取设置与真源指纹：

- 通道关闭 / `auto_upload` 关闭 / Git 远端已验证并启用 → 清空 pending，跳过；
- 本地指纹 == `synced_fingerprint` → 清空 pending，不上传（内容又改回去了）；
- 连接不完整（`InvalidInput`）→ 不是故障，保留 pending，下一轮再试；
- 认证失败（见 §7.5）→ 保留 pending，暂停自动任务；
- 其他失败 → `retry_at = now + 5 分钟`，`first_at` / `last_at` 重置为 now，避免无限快重试。

上传成功后写同步基线（§8.1）、清空 pending，并累加本月上传字节。
手动「立即上传」不受延迟与暂停影响，仍立即执行。

### 7.2 下载调度与变更判定

- 启动后立即检查一次；之后每 15 分钟一次。
- 每次检查按以下顺序，任何一步不满足就停下，不产生后续流量：

1. 未配置 / 通道关闭 / `auto_download` 关闭 / 认证暂停中 → 跳过。
2. `PROPFIND` 取远端元数据（1 次请求）。判定「云端变了」的谓词：
   - 基线与当前都有 ETag → 比 ETag；
   - 否则两边都有修改时间 → 比修改时间与大小（大小未知时不比）；
   - 都没有 → 视为已变，直接进第 3 步。
   判定未变 → 只更新观测缓存，跳过，不读 manifest。
3. 判定已变 → `GET manifest`（1 次小请求）。
   - manifest 缺失、格式不符或 schema 不是 2 → 记录错误，跳过，不改本地。
   - `manifest.archive.sha256 == synced_sha256` → 只更新观测缓存与基线元数据，跳过。
   - manifest 的 `contentFingerprint` 等于本地真源指纹 → 只更新同步基线，跳过下载。
4. 本地有未上传改动（`auto_upload` 开启且本地指纹 != `synced_fingerprint`）→ 跳过本轮，
   让上传先把本机内容落定；下一轮再看云端。
   - 这条只在编辑机（开了自动上传）生效。纯下载机上即使本地被改过，云端一更新仍以云端为准。
5. 下载整包到 staging，按 manifest 校验哈希/大小/文件数；计算包内容指纹。
   - 内容指纹 == 本地指纹 → 只更新同步基线，不恢复。
   - 否则走 §7.3 的恢复。

### 7.3 恢复与快照保留

- 自动下载复用 `backup::restore`：`maintenance` 写锁 + index 锁，自动建 `PreRestore` 安全快照、
  覆盖 `assets/` / `agents/`、重建索引，期间 `SuppressionGuard` 抑制自动上传。
- 下载包只作为临时恢复源：恢复成功或回滚后都删除，**不**留下 `cloud_restore_*` 可见快照。
  手动「从云端恢复」保持现有行为（保留下载包）。
- 成功恢复后调用 `backup::prune(retain_count)`（与自动快照同一保留策略），避免
  `PreRestore` 快照随每次云端更新无限增长；`retain_count` 取当前备份设置，最小 1。
- 成功后更新同步基线、`last_download_at`，清空 `last_download_error`，发 `cloud-auto-synced` 事件。

### 7.4 上传与下载的互斥

- 两者都通过 `cloud_ops` 串行，任何时刻只有一个网络任务。
- 上传进行中到达下载检查点：本轮下载顺延到下一次，不排队抢锁。
- 恢复期间继续使用 `SuppressionGuard`，避免自动下载反过来触发自动上传。

### 7.5 认证失败与暂停

- WebDAV 的 401/403 统一归为 `AppError::CloudAuth`（新增变体），与普通网络错误分开。
- 自动任务遇到 `CloudAuth`：写 `auth_paused_until = now + 1 小时`，记录错误，暂停自动上传/下载；
  手动上传、手动恢复、测试连接、刷新云状态不受暂停影响。
- 暂停期间 worker 不发起任何自动网络请求，避免触发坚果云更久的临时封禁。
- 成功的实时状态探测/测试连接/手动操作清除 `auth_paused_until`。
- 普通网络错误不暂停，按下一轮 tick 或上传的 5 分钟重试。

## 8. 状态缓存与面板响应

### 8.1 运行状态文件（两组字段，写入边界不同）

扩展 `<storeRoot>/state/cloud-backup.json`。所有字段 `serde(default)`，读失败按空状态处理；
不做旧文件迁移。

```rust
struct CachedRemote {
    exists: bool,
    size_bytes: Option<u64>,
    modified_at: Option<i64>,
    etag: Option<String>,
    checked_at: i64,
}

struct CloudRuntimeStatus {
    // 同步基线：只在上传/恢复成功后写，自动任务据此判断「是否已同步」
    synced_fingerprint: Option<String>,
    synced_sha256: Option<String>,
    synced_meta: Option<CachedRemote>,
    last_upload_at: Option<i64>,
    last_download_at: Option<i64>,
    last_error: Option<String>,          // 自动上传失败
    last_download_error: Option<String>, // 自动下载失败

    // 观测缓存：实时状态探测与自动探测都可写，只用于展示与快速判定
    observed_remote: Option<CachedRemote>,
    observed_manifest: Option<CloudManifestInfo>,
    observed_manifest_error: Option<String>,

    // 运行控制与统计
    auth_paused_until: Option<i64>,
    upload_month: Option<String>,   // 如 "2026-11"
    uploaded_bytes_month: u64,
}
```

硬规则：

- **实时状态探测只写 `observed_*`，绝不写 `synced_*`**。否则用户打开面板探一次，
  worker 就会误以为云端已同步，永远不下载。
- 上传/恢复成功同时写同步基线与观测缓存（二者此刻一致）。
- 观测到「元数据变了但 sha 未变」时，只更新 `synced_meta` 与观测缓存，`synced_sha256` /
  `synced_fingerprint` 不动。

### 8.2 命令

- 新增 `get_cloud_status_cached()`：只读本地设置、密钥与运行状态文件，不联网、不抢 `cloud_ops`，
  立即返回 `CloudStatus`（`from_cache = true`，内容取 `observed_*`）。
- 现有 `get_cloud_status()` 保持联网语义，`from_cache = false`，并优化：
  - `PROPFIND` 结果与 `observed_remote` 相同 → 复用 `observed_manifest`，不再 `GET`；
  - 只写 `observed_*`；成功的探测清除 `auth_paused_until`。
- `CloudStatus` 增加：`auto_download`、`upload_delay_hours`、`last_download_at`、
  `last_download_error`、`uploaded_bytes_month`、`from_cache`。
- 上传/下载成功后发 `cloud-auto-synced` 事件，载荷含动作类型、恢复文件数、删除数、
  安全快照名、上传大小；备份面板监听后刷新。其他资产面板在切换时重新挂载刷新。

### 8.3 连接参数变化

保存设置时比较旧连接与新连接：`base_url`、`username`、`remote_dir`，以及密码是否变化
（`None` = 不变）。任一变化或清空地址 → 删除 `state/cloud-backup.json`，同步基线与观测缓存全部作废。
命令层负责删除（`set_cloud_settings_at` 返回 `connection_changed` 标记）。
只切换 `enabled` / `auto_upload` / `auto_download` / `upload_delay_hours` 不动基线。

### 8.4 前端

- `useBackups` 挂载时并行拉本地数据与 `get_cloud_status_cached()`，先渲染；再在后台调
  `get_cloud_status()` 刷新。刷新中保留旧状态，不清空。
- 「从云端恢复」按钮的禁用条件只认「已知」事实：
  - 已知远端没有包、或 manifest 明确不兼容 → 置灰；
  - 状态未知（首次运行、缓存为空）→ 可点，由恢复命令自行校验并报错。
- 头部显示「检查中…」/「同步中…」的轻量指示，不阻塞按钮。
- `CloudSection` 改为两个复选框，并加「上传延迟」下拉：
  - 自动上传：改动停下 N 小时后上传；持续编辑最多每 `min(4N, 24h)` 补传一次。
  - 自动下载：启动时及每 15 分钟检查云端，有更新就覆盖本机；覆盖前自动建安全快照。
  - 显示「本机本月上传约 X MB」。
- 类型 `CloudBackupSettings`、`CloudSettingsInput`、`CloudTestInput`（Omit 新字段）同步更新；
  `lib/api.ts` 增加 `getCloudStatusCached`。

## 9. manifest 与包格式

- `CLOUD_MANIFEST_SCHEMA` 升到 **2**。
- manifest 新增必填字段 `contentFingerprint`（上传时的真源内容指纹）。
- `compatible()` 要求 `format` 匹配且 `schema == 2`；不再接受旧 schema。
- `ensure_restorable()` 对 1.x、缺失 manifest、格式不符都返回「请从源设备重新上传」。
- 删除旧版裸包兼容分支与面板上的对应提示；远端没有 manifest 视为不可恢复。
- 升级后主机器需要手动「立即上传」一次，云端换成 schema 2，从机器之后才能自动下载。

## 10. 错误与边界

- 连接不完整：静默跳过自动任务，手动操作给明确错误。
- 401/403：按 §7.5 暂停自动任务 1 小时；不反复重试。
- 包与 manifest 不一致：拒绝恢复，保留本地，提示重新上传。
- 自动下载覆盖时索引重建失败：恢复流水线本身会回滚并返回错误，运行状态记录失败。
- 从机器本地有未上传改动：纯下载机以云端为准，覆盖后可从来本机安全快照找回。
- 时钟/月份变化：仅影响「本月上传」统计，不影响同步正确性。

## 11. 测试

Rust（`FakeWebDav` 可统计各方法请求数）：

- 自动下载关闭 / 通道关闭 → 不产生任何请求。
- 远端元数据未变 → 只有 1 次 PROPFIND，无 manifest GET、无下载。
- 远端变了但 sha 与基线一致 → PROPFIND + manifest GET，无下载。
- 远端变了且本地干净（无自动上传）→ 下载并恢复，生成 `PreRestore` 安全快照；
  下载包不残留为快照，`prune` 后快照数不超过保留数。
- 纯下载机本地被改过、云端更新 → 覆盖本地（云端为准）。
- 编辑机本地有未上传改动 → 跳过下载，不动本地。
- manifest 缺失 / schema 1 / 格式不符 → 记录错误，本地字节不变，不下载。
- 下载后 `synced_fingerprint` / `synced_sha256` / `synced_meta` 正确落盘。
- 实时状态探测不改变 `synced_*`（用一个「探测显示已更新、worker 仍会下载」的用例钉住）。
- 认证失败 → 设置 `auth_paused_until`，暂停期内的自动 tick 不产生请求；手动探测成功清除。
- 连接参数变化 → 运行状态文件被清空；只改开关不清空。
- 自动上传：改动后未到延迟不上传；空档满延迟上传；持续编辑在兜底上限上传；指纹一致不传。
- 启动对账：本地指纹 != 基线且无 pending → 记 pending，延迟后上传。
- 上传失败 → 5 分钟后重试；`periodic_once` 不再绕过上传延迟。
- `get_cloud_status_cached` 在 FakeWebDav 不可用时仍返回缓存，且不增加请求数。
- 本月上传字节按月重置。
- 设置序列化：旧文件缺新字段 → 默认值；`enabled=false` 时两个自动开关恒为 false；
  不再保留「旧 `autoUpload` 补 `enabled`」用例。

前端（Vitest，mock `../lib/api`）：

- 两个复选框与延迟下拉的读写。
- 缓存状态先渲染、恢复按钮不被实时请求阻塞。
- 已知无包/不兼容才置灰；未知时可点。
- 上传/下载错误与「本月上传」展示。
- 监听 `cloud-auto-synced` 后刷新状态。

真机验证：临时 store + `baseOverride`，用两个 `FakeWebDav` 或真实 WebDAV 沙箱目录跑
「主机器上传、从机器下载」往返；不得触碰真实 `~/.claude` 等配置。

## 12. 已定的取舍（实现者的调用）

- 整包而非文件级增量（坚果云请求限制）。
- 自动上传用「改动停下满延迟」而非「每小时必传」；默认 1 小时，兜底 `min(4×延迟, 24h)`。
- 上传不再由 `periodic_once` 触发；由变更信号、15 分钟对账、设置保存唤醒三个入口驱动，
  pending 只在内存，应用重启后重新计时。
- 自动下载启动即查、之后每 15 分钟；只读元数据，变了才读 manifest，再变才下载。
- 云端为准，不弹冲突确认；安全快照兜底；自动下载不留下载包，恢复后按 `retain_count` 清理。
- 运行状态拆成「同步基线」与「观测缓存」，状态探测只写观测缓存。
- 连接参数变化清空运行状态；认证失败暂停自动任务 1 小时。
- 面板缓存优先，恢复按钮不等联网。
- 不做旧数据兼容：manifest schema 2、旧包拒收、旧设置不迁移。

## 13. 留给实现者

- Rust 模块拆分：`backup::spawn_change_worker` 扩展还是另建 `cloud_sync` 模块；只要锁顺序与
  worker 单实例不变即可。
- 事件名与载荷的具体字段命名。
- `restore_latest` 与自动下载共用下载/恢复代码的重构方式（建议抽 `restore_from_zip` 内部函数）。
- 文案细节与控件布局，遵循现有 `CloudSection` 风格。
- 缓存结构里 `RemoteStatus` / `CloudManifestInfo` 是否直接加 `Deserialize` 还是建缓存专用结构。
- worker 内部 tick 的具体实现（单线程 `recv_timeout` 即可，不需要额外线程池）。
