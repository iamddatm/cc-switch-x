# 云备份（WebDAV）设计

- 日期：2026-10-26
- 状态：待批准
- 已确认的范围：云端只保留一份最新状态包（方案 C）；首期只做 WebDAV；用途是“云端留档、换机时下载恢复”。

## 1. 背景与目标

v0.4.0 的备份面板已有两条路径：

- 本地快照：把 `assets/`、`agents/` 与 `.gitignore` 打成 zip，用于本机回滚或跨机器恢复。
- Git 备份：本地提交 + 远端推拉，属于文件级双向同步。

上游 cc-switch 还提供 WebDAV / S3 云备份。本次补齐 WebDAV 这一条，目标是把当前可移植真源上传到用户自己的 WebDAV 目录；换机器时下载云端最新包，走现有安全恢复流水线落回本地。

不引入第二套文件级合并逻辑：云端只有一份不可变的“最新状态包”，上传即覆盖，下载即恢复。多台机器共用同一远端时，最后上传者获胜；这是方案 C 明确接受的语义。

## 2. 已确认的行为

- 用户填写 WebDAV 地址、用户名、应用密码与远端目录；密码只进 `secrets.local.json`。
- “立即上传”把当前可移植真源打包并覆盖远端固定对象。
- 可选“自动上传”：每次自动快照成功后，把该快照上传到远端。
- “从云端恢复”下载远端最新包，先落成本地快照，再走现有恢复流水线；恢复前自动创建安全快照。
- 远端不保留多版本，不自动清理，不在启动时自动下载或恢复。
- 首期只支持 WebDAV Basic Auth；S3、WebDAV Digest、OAuth 等不做。

## 3. 非目标与反目标

非目标：

- S3 / 对象存储、其他网盘协议。
- 远端多版本历史、快照列表、保留份数清理。
- 文件级双向同步、按文件冲突合并。
- 自动下载 / 启动时恢复（远端内容不可信，不能静默覆盖本地）。
- 上传设备级数据：`local/`、`state/`、`secrets.local.json`、`index.db`、`logs/`、`backups/`、`.git/`。
- 替用户创建或修复远端 WebDAV 账户 / 应用密码。

反目标：

- 为了“能连上”而跳过 TLS 校验、放宽鉴权或把密码写进设置文件 / URL。
- 把云端包当作可信输入直接覆盖本地，不经过 manifest 校验与安全快照。
- 把云备份做成另一个索引型真源；包内容必须始终可由文本真源重建。

## 4. 方案比较

| 方案 | 内容 | 取舍 | 结论 |
|---|---|---|---|
| A. 快照镜像 | 云端保存多份快照 zip，可列表、下载任意一份 | 远端历史丰富，但要引入远端列表、保留与删除策略，交互更重 | 未选 |
| B. 快照镜像 + WebDAV/S3 双后端 | A 的语义，同时支持两种传输 | 能力最全，但多一套 SigV4、配置与测试，工作量约多一半 | 未选 |
| C. 单一最新状态包 | 远端固定一个对象，上传覆盖，下载最新并恢复 | 远端最干净，复用现有快照/恢复；没有云端多版本，靠本地快照保留历史 | **已选** |

首期只实现 WebDAV：现有 `reqwest` 依赖即可覆盖坚果云、Nextcloud、群晖等；S3 以后可按同一套包格式与恢复流水线追加。

## 5. 云端格式与恢复语义

### 5.1 包内容

云端对象就是一个标准 CC Switch X 快照 zip，完全复用现有格式：

- `manifest.json`：`format = "ccswitchx-backup"`、`schema = 1`、`createdAt`、`kind`、`appVersion`、逐文件 `size` + `sha256`。
- `assets/{mcp,skills,prompts,rules,subagents,hooks}/...`
- `agents/<known-agent>.json`
- `.gitignore`（存在时）

不新增第二套包格式。为此把 `services/backup.rs` 中“收集文件 + 写 manifest + 写 zip”的逻辑抽成可复用的公开函数，`create` 与云端上传共用，禁止复制粘贴。

### 5.2 远端布局

```text
<base_url>/<remote_dir>/ccswitchx-backup.zip
```

- `remote_dir` 默认 `ccswitchx`，允许安全的 `/` 分隔多级目录。
- 固定对象名 `ccswitchx-backup.zip`；上传通过临时对象 + `MOVE` 原子替换，不做远端版本管理。
- 上传先写 `ccswitchx-backup.zip.tmp-<时间戳>`，成功后 `MOVE` 覆盖最终对象；服务器不支持 `MOVE`（405/501）时回退为直接 `PUT` 最终对象，直写成功后再清理临时对象（直写失败时保留临时对象，避免两份副本同时丢失）。其他失败路径尽力 `DELETE` 临时对象。
- 不额外写 sidecar 元数据；远端大小与修改时间由 `HEAD` 获取，包内 manifest 在下载后校验。
- 多台机器使用同一路径时按“最后上传者获胜”处理；如需隔离，用户可配置不同的 `remote_dir`。

### 5.3 恢复语义

下载包先落到本地 `backups/snapshots/cloud_restore_<时间戳>.zip`，再调用现有 `backup::restore`：

- 恢复前自动创建 `preRestore` 安全快照。
- 只写白名单内容（`assets/`、`agents/`、`.gitignore`），不动 `local/`、`state/`、密钥与索引文件。
- 逐文件校验大小与 sha256，拒绝清单外文件与路径越界。
- 任一步失败按安全快照回滚。
- 恢复后清理派生 agent 状态与漂移并重建索引，通知 Git worker。

下载包会尝试保留在本地快照列表里，便于再次恢复或排查；命名 `cloud_restore_YYYYMMDD_HHMMSS`，冲突时加 `_2`、`_3` 后缀。它受现有 `retain_count` 与快照清理策略约束，可能被后续清理；需要再次恢复时重新下载云端包，不对 `cloud_restore_*` 做清理豁免。

## 6. 配置与密钥

### 6.1 设置模型

云配置放在 `AppSettings` 顶层（与 `proxy` 并列），不放进 `BackupSettings`，避免保存快照策略 / Git 开关时用旧值覆盖云配置：

```rust
pub struct CloudBackupSettings {
    pub auto_upload: bool,
    pub base_url: String,
    pub username: String,
    pub remote_dir: String,
}
```

- `auto_upload` 默认 `false`。
- `remote_dir` 默认 `ccswitchx`。
- 旧设置文件缺少 `cloud` 时取默认值。
- 前端 `CloudSettingsView` 额外返回 `has_password`，不返回密码本身。

### 6.2 密码

- 存在 `secrets.local.json` 的 scope `cloud.webdav`、key `password`。
- 语义与代理密码一致：
  - `password = None`：沿用已保存密码。
  - `password = Some("")`：清除；若同时仍配置了地址和用户名，则视为配置不完整，拒绝保存或要求同时清空地址。
  - `password = Some(v)`：覆盖。
- 配置了 `base_url` 与 `username` 时必须有密码；不做匿名 WebDAV。
- 设置文件、日志、错误信息、命令返回值中都不能出现密码。

### 6.3 校验

- `base_url`：只接受 `http` / `https`；拒绝 URL 内嵌用户名/密码；拒绝查询串与 fragment；去掉尾部 `/` 后保存。
- `remote_dir`：逐段只允许 `[A-Za-z0-9._-]`，拒绝空段、`.`、`..`、反斜杠、绝对路径、控制字符；总长度受限。
- `username`：配置地址时不能为空。
- HTTP 明确提示会明文传输凭据；不做自动降级，也不跳过 HTTPS 证书校验。自签名 HTTPS 不被信任时，用户应改用内网 HTTP 或安装受信任证书。

## 7. 上传

### 7.1 手动上传

命令：`cloud_upload_now`。

1. 读取并校验云配置、密码、全局代理快照；任一缺失直接报错。
2. 获取 `cloud_ops` 互斥锁，串行化所有云端网络操作。
3. 在 `maintenance` 写锁下，把当前真源写到 `state/staging/cloud-*.zip` 临时文件；释放写锁后再联网，避免网络慢时阻塞本地写操作。
4. 创建 WebDAV 客户端，确保远端目录存在。
5. 以 `Content-Type: application/zip`、已知 `Content-Length` 流式 `PUT` 到临时对象 `ccswitchx-backup.zip.tmp-<时间戳>`；不把整包读进内存。
6. `MOVE` 临时对象覆盖 `ccswitchx-backup.zip`（`Destination` 指向最终 URL，`Overwrite: T`）。服务器返回 405/501 时回退为直接 `PUT` 最终对象，直写成功后再清理临时对象；其他失败路径尽力 `DELETE` 临时对象。
7. `HEAD` 最终对象并比对 `Content-Length` 与本包大小；不一致视为上传失败（远端可能被写坏），`HEAD` 本身失败可降级为警告。
8. 更新本机状态文件并清理临时包。
9. 返回 `CloudStatus`。

手动上传不额外创建本地快照；本地快照仍由“立即备份”管理。

### 7.2 自动上传

- 只挂在现有自动快照之后：`periodic_once` 成功创建 `SnapshotKind::Auto` 并清理旧快照后，释放 `maintenance` 写锁，再判断 `cloud.auto_upload`。
- 直接上传刚创建的那份本地快照，不重复打包。
- 上传在备份后台线程内完成，不占用 UI 线程；失败只写状态文件与 stderr，不打断快照调度。
- `interval_hours = 0` 时不会自动创建快照，因此也不会自动上传；UI 文案要写明“自动快照后自动上传”。

### 7.3 上传前校验

上传（手动与自动）在联网前用 manifest 做两项检查，避免把“换机恢复不了”或“空包”覆盖到云端唯一副本：

- 可恢复性：文件数、按 `ArchiveLimits` 估算的总字节（每项至少计 4 KiB）与最大路径深度都不能超过恢复端限制；超过时直接拒绝并说明具体数值。这里的上限与下载上限 512 MiB 使用同一套常量。
- 非空：除 `.gitignore` 外至少有一个资产或 agents 文件；空包拒绝上传。自动上传遇到空包只记录错误并跳过，不覆盖远端。

手动上传覆盖远端唯一副本是方案 C 的既定语义，不额外加二次确认；UI 用文案明确“上传会覆盖云端备份”。

## 8. 下载与恢复

命令：`cloud_restore_latest`。

1. 读取并校验云配置、密码、代理；获取 `cloud_ops`。
2. `HEAD` 远端对象：
   - `404`：返回“云端还没有备份，请先上传”。
   - 大小超过上限（512 MiB，与现有归档限制一致）：拒绝下载。
   - 其他错误：带 HTTP 状态返回。
3. 流式 `GET` 到 `state/staging/` 临时文件，边读边计数，超过上限立即中止并删除。
4. 读取并校验 manifest 的 `format` / `schema`；格式不符直接拒绝，不进入本地快照目录。
5. 获取 `maintenance` 写锁与 index 锁，把临时包移动到 `backups/snapshots/cloud_restore_*.zip`，调用 `backup::restore`。
6. 恢复成功 / 失败回滚后通知 Git worker；返回 `CloudRestoreReport`。
7. 下载期间的本地写入不被长时间阻塞；安全快照反映的是真正执行恢复时的本地状态。

不自动下载、不启动时检查后自动恢复；用户必须点击“从云端恢复”并二次确认。

## 9. 状态与 UI

### 9.1 本机状态文件

新增 `state/cloud-backup.json`（已在 `.gitignore` 的 `state/` 下，不参与 Git / 快照 / 云包）：

```json
{
  "lastUploadAt": 1761440000000,
  "lastError": null
}
```

- 上传成功：更新 `lastUploadAt`，清空 `lastError`。
- 手动或自动上传失败：写入 `lastError`，保留上次成功时间。
- 文件损坏或缺失：按默认值处理并继续，不让云备份功能整体失败。
- 原子写。

### 9.2 状态查询

`get_cloud_status` 返回：

```text
configured / hasPassword
remote: { exists, sizeBytes, modifiedAt, etag } 或 null
remoteError: string | null
lastUploadAt: number | null
lastError: string | null
```

- 未配置时跳过网络，直接返回 `configured = false`。
- `HEAD` 失败作为 `remoteError` 返回，不把整个面板打成错误页。
- 状态查询也要拿 `cloud_ops`，避免和上传 / 恢复并发。

### 9.3 面板

在备份面板 Git 区块之后新增“云备份（WebDAV）”区块：

- 状态徽标：未配置 / 已配置 / 连接失败 / 云端无备份。
- 字段：WebDAV 地址、用户名、密码（已保存时占位提示“留空保持不变”，可显示/隐藏）、远端目录、自动上传复选框。
- 按钮：测试连接、保存、立即上传、从云端恢复。
- 远端信息：对象路径、大小、最后修改时间；本机状态：上次上传时间、上次错误。
- HTTP 地址显示醒目警告：凭据会明文传输，仅建议内网使用。
- “从云端恢复”必须弹确认框：说明会覆盖本地 `assets/` 与 `agents/`，恢复前自动创建安全快照，`local/`、密钥与索引不会被动。
- 未配置或缺少密码时，上传 / 恢复 / 自动上传相关操作禁用。
- 测试连接使用当前草稿字段；密码留空时沿用已保存密码。校验地址 / 用户名 / 密码后执行 `MKCOL`（目录已存在时 405 视为成功）并对最终对象做 `HEAD`（404 视为成功）。它验证可达性、认证与目录访问，不做探针写入，因此不会创建临时对象；真正的写权限由上传验证。测试成功显示延迟，失败显示后端错误。
- 保存密码的交互沿用网络面板：密码输入框不预填；`hasPassword` 只用于提示；不提供“单独清除密码”按钮，清空 WebDAV 地址即清除整组云端配置。

## 10. 网络、安全与错误

- 客户端：`reqwest::blocking::Client`，复用全局代理配置（代理密码从 secrets 取），`rustls`，带 User-Agent。
- 连接超时 30 秒，整体超时 10 分钟；上传 / 下载均流式，不整包进内存。
- `PUT` 使用 `reqwest::blocking::Body::sized`（或等效的已知长度流式 body），显式 `Content-Length`；部分 WebDAV 服务器拒绝 chunked。
- 禁止跟随重定向，避免 `Authorization` 被带到其他主机；需要重定向时由用户填写最终地址。
- 只使用 Basic Auth；`401/403` 提示检查用户名与应用密码。
- `MKCOL` 逐级创建 `remote_dir`；`405` 视为目录已存在；`409` 提示父目录不存在。
- 状态码错误信息包含方法、状态码与截断后的响应片段；绝不包含 `Authorization` 或密码。
- 下载上限 512 MiB；ZIP 解压沿用现有 `ArchiveLimits`（条目数、总大小、深度、越界路径、符号链接）。
- 所有命令错误走 `AppError`；新增 `AppError::Cloud(String)`，文案统一加“云备份”上下文。
- 不修改全局 TLS、CORS、鉴权或代理设置；HTTP 仅作为用户显式选择的内网方案并给出警告。

## 11. 并发与锁

- `AppState` 新增 `cloud_ops: Arc<Mutex<()>>`，所有云网络操作（状态、测试、上传、恢复）串行。
- 锁顺序：
  - 上传：`cloud_ops` → `maintenance`（仅打包）→ 释放 → 网络。
  - 恢复：`cloud_ops` → 下载 → `maintenance` → `index` → 恢复 → 释放。
  - 状态 / 测试：`cloud_ops` → 网络。
- 不跨网络持有 `maintenance`、`index`、`settings`、`secrets` 锁；配置与密码在联网前克隆。
- 自动上传在备份后台线程执行，不阻塞 UI；与手动上传通过 `cloud_ops` 互斥。
- 恢复仍遵循现有约定：`maintenance` 写锁挡住普通写操作，`index` 锁在恢复期间持有。

## 12. 代码改动

### 12.1 后端

| 文件 | 改动 |
|---|---|
| `src-tauri/src/settings/mod.rs` | 新增 `CloudBackupSettings`，`AppSettings.cloud` 与默认值；URL / 远端目录校验辅助 |
| `src-tauri/src/services/settings.rs` | 云配置视图、密码引用、保存与校验（对齐 `proxy` 模式） |
| `src-tauri/src/net/webdav.rs` | 新增 WebDAV 传输：MKCOL / PUT / MOVE / DELETE / GET / HEAD、Basic Auth、代理、限流、错误映射 |
| `src-tauri/src/services/cloud_backup.rs` | 新增上传、下载、状态文件、状态探测、自动上传入口 |
| `src-tauri/src/services/backup.rs` | 抽出可复用的打包函数；增加最小归档校验入口 |
| `src-tauri/src/commands/cloud.rs` | 新增 6 个薄命令 |
| `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs` | 模块与命令注册 |
| `src-tauri/src/state.rs` | 新增 `cloud_ops` |
| `src-tauri/src/error.rs` | 新增 `Cloud(String)` |

### 12.2 前端

| 文件 | 改动 |
|---|---|
| `src/types.ts` | `CloudBackupSettings`、`CloudSettingsView`、`CloudStatus`、`CloudTestResult`、`CloudRestoreReport`；`AppSettings.cloud` |
| `src/lib/api.ts` | `getCloudSettings`、`setCloudSettings`、`testCloudConnection`、`getCloudStatus`、`cloudUploadNow`、`cloudRestoreLatest` |
| `src/hooks/useBackups.ts` | 拉取云配置与状态；命令后刷新；状态查询不阻塞面板首屏 |
| `src/panels/BackupsPanel.tsx` | 新增云备份区块、密码交互、测试 / 上传 / 恢复确认 |
| `src/__tests__/BackupsPanel.test.tsx` | 补充云备份 mock 与交互测试 |

### 12.3 文档与声明

- 若复用上游 `webdav.rs` 的传输思路 / 代码，在 `THIRD_PARTY_NOTICES.md` 的 cc-switch 行补充 WebDAV 传输逻辑与对应文件；否则写明为独立实现。
- 本次不新增依赖；`reqwest`、`chrono`、`zip`、`sha2`、`tempfile` 已够用。

## 13. 数据模型与命令

### 13.1 设置示例

```json
{
  "storeRoot": "C:/Users/me/.ccswitchx",
  "cloud": {
    "autoUpload": true,
    "baseUrl": "https://dav.jianguoyun.com/dav",
    "username": "me@example.com",
    "remoteDir": "ccswitchx"
  }
}
```

### 13.2 命令

| 命令 | 返回 | 说明 |
|---|---|---|
| `get_cloud_settings` | `CloudSettingsView` | 本地读取，不含密码 |
| `set_cloud_settings` | `CloudSettingsView` | 保存地址 / 用户名 / 远端目录 / 自动上传；密码按 None/空串/值处理 |
| `test_cloud_connection` | `CloudTestResult` | 使用草稿字段测试；不保存 |
| `get_cloud_status` | `CloudStatus` | `HEAD` 远端 + 本机状态文件 |
| `cloud_upload_now` | `CloudStatus` | 打包当前真源并覆盖远端 |
| `cloud_restore_latest` | `CloudRestoreReport` | 下载最新包并走安全恢复 |

## 14. 测试与验收

### 14.1 Rust

- `settings`：旧文件缺 `cloud` 的默认值、序列化往返、URL / 远端目录校验、密码语义。
- `cloud_backup`：状态文件读写与损坏容错；上传成功后状态更新；上传失败保留上次成功时间。
- `webdav`：用本机临时 HTTP 服务模拟 MKCOL / PUT / GET / HEAD：
  - Basic Auth 头正确；
  - PUT 带正确 `Content-Length`；
  - 405 MKCOL 视为已存在；
  - 404 映射为“云端无备份”；
  - 401 映射为认证失败；
  - 下载超限被中止；
  - 不跟随重定向；
  - `MOVE` 成功替换最终对象；405/501 时回退直接 `PUT`，直写成功后再删临时对象；
  - 上传失败时临时对象被尽力清理；
  - 上传前拒绝空包与超过恢复限制的包；
  - 测试连接只做 `MKCOL` + `HEAD`，不写探针对象。
- 端到端（临时 store，不碰真实 `~/.claude`）：
  1. 写入一个 MCP 资产；
  2. 上传到本机假 WebDAV；
  3. 修改 / 删除本地资产；
  4. 下载并恢复；
  5. 断言资产恢复、`local/` 与 `secrets` 未变、创建了安全快照。
- 自动上传：自动快照后调用上传；上传失败不打断调度。

### 14.2 前端

- 未配置时显示引导，上传 / 恢复禁用。
- 保存配置时密码为空传 `null`，不覆盖已保存密码。
- 测试连接、立即上传、从云端恢复的调用与确认框。
- 远端状态、上次上传时间、上次错误展示。
- 更新 `BackupsPanel.test.tsx` 的 api mock；检查 `App.test.tsx` 的 `getSettings` mock 是否需要补 `cloud`。

### 14.3 手工验证

- 使用临时 store 与本机 WebDAV 服务（或用户自己的 WebDAV，但不得使用真实 `~/.claude` / `~/.codex` / `~/.qoder-cn` 作为试验田）。
- 验证：保存后 `settings.json` 无密码、`secrets.local.json` 有密码；上传覆盖远端；换一个临时 store 下载恢复；HTTP / 错误密码 / 404 / 超限分别给出清晰错误。
- 门禁：`pnpm build`、`pnpm test`、`cargo test --manifest-path src-tauri/Cargo.toml`。

### 14.4 验收标准

1. 能配置并测试 WebDAV，密码不落设置文件。
2. 能上传当前真源并在远端看到固定对象。
3. 能在另一个临时 store 下载并恢复，且恢复前有安全快照。
4. 远端无备份、认证失败、网络失败、超限包都给出可操作错误，不破坏本地。
5. 自动上传只在自动快照成功后触发，失败可见且不打断调度。
6. 不触碰设备级数据与用户真实工具目录。

## 15. 风险与取舍

- **单槽覆盖**：多台机器共用时最后上传者获胜。接受；需要隔离时改 `remote_dir`。
- **无云端历史**：只保留最新一份。接受；本地快照仍保留多份。
- **HTTP 明文**：允许用户显式使用内网 HTTP，并在 UI 警告；不做静默降级，不跳过 TLS 校验。
- **自动上传时机**：只跟自动快照，不跟每次写操作，避免频繁上传与网络抖动。
- **临时对象 + MOVE 回退**：不支持 `MOVE` 的服务器回退直接 `PUT`；极端中断仍可能留下半包，这是 WebDAV 服务端行为的边界，无法完全消除。上传后会 `HEAD` 比对大小，降低静默坏包的概率。
- **无 S3**：首期不引入 SigV4 与对象存储配置；包格式与恢复路径已为后续后端留好边界。
- **第三方声明**：若参考上游 WebDAV 传输实现，必须补充 MIT 归属。

## 16. 留给实现者的决定

- 具体函数签名、状态文件字段命名、错误文案与 UI 排版。
- 假 WebDAV 测试服务的实现方式（本机 TcpListener 或测试工具），只要不依赖真实网络。
- 是否在 UI 展示 ETag；不展示也不影响功能。
- `cloud_restore_*` 快照默认保留可见，受正常保留策略约束；如需隐藏由后续迭代决定。
- 默认扩展现有 `useBackups`，避免新增重复请求的 hook。
