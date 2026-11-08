# WebDAV 云备份自动同步 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 给 WebDAV 云备份加上自动上传与自动下载：主机器按延迟合并上传，从机器只自动下载；同时用本地缓存让备份面板秒开。

**Architecture:** 在现有整包上传/恢复流水线上扩展：运行状态拆成「同步基线」与「观测缓存」；单一后台 worker 负责上传防抖/对账和 15 分钟下载探测；manifest 升到 schema 2 并携带内容指纹；前端先渲染缓存状态，再后台刷新。

**Tech Stack:** Rust（Tauri 2、reqwest、serde、chrono）、React 18 + TypeScript + Vitest、FakeWebDav 测试服务器。

**Spec:** `docs/superpowers/specs/2026-11-08-cloud-auto-sync-design.md`

## Global Constraints

- 坚果云免费版：每月上传 1 GB、下载 3 GB；每 30 分钟 ≤600 次请求。自动任务必须低频。
- 不做旧数据兼容：manifest schema 固定为 2；旧 schema/裸包按不兼容处理；旧设置文件不做语义迁移。
- 自动下载以云端为准；覆盖前必须走 `backup::restore`，自动创建 `PreRestore` 安全快照。
- 所有云端网络任务持 `cloud_ops`；恢复持 `maintenance` 写锁 + index 锁；恢复期间用 `SuppressionGuard`。
- 运行状态文件里，`synced_*` 只能由上传/恢复成功写入；状态探测只能写 `observed_*`。
- 先跑 `pnpm build` 再跑 `cargo check` / `cargo test`（`tauri::generate_context!` 编译期读 `dist/`）。
- 提交信息用中文；执行本计划前先建独立分支/worktree。
- 面向用户文案用中文；代码与标识符用英文。

## Review Focus

- 打开面板触发实时探测时，不能把「云端已更新」写成「已同步」，否则自动下载永远不触发。
- 修改 WebDAV 地址/用户名/目录/密码后，旧基线必须作废，否则新远端永远收不到自动上传。
- 编辑机本地有未上传改动时，自动下载必须跳过，不能覆盖用户刚改的内容。
- 401/403 必须暂停自动任务，不能每 15 分钟重试触发坚果云临时封禁。
- 自动下载不能每次更新都留下整包快照；下载包删除并按 `retain_count` 清理。

---

### Task 1: 设置字段、命令签名与连接变化重置

**Files:**
- Modify: `src-tauri/src/settings/mod.rs`
- Modify: `src-tauri/src/services/settings.rs`
- Modify: `src-tauri/src/services/cloud_backup.rs`（新增 `reset_runtime_status`）
- Modify: `src-tauri/src/commands/cloud.rs`
- Modify: `src/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/panels/BackupsPanel.tsx`（所有 `api.setCloudSettings` 调用补新字段）
- Test: 上述 Rust 文件内的 `#[cfg(test)]`，以及 `src/__tests__/BackupsPanel.test.tsx` 的 fixture

**Interfaces:**
- Produces: `CloudBackupSettings { enabled: bool, auto_upload: bool, auto_download: bool, upload_delay_hours: u32, base_url: String, username: String, remote_dir: String }`
- Produces: `set_cloud_settings_at(config_path, secrets_path, base_url, username, password, remote_dir, enabled, auto_upload, auto_download, upload_delay_hours) -> Result<(AppSettings, Secrets, bool), AppError>`，第三个值表示连接参数是否变化。
- Produces: `cloud_backup::reset_runtime_status(store: &Store) -> Result<(), AppError>`。
- Produces: `CloudSettingsView` 增加 `auto_download: bool`、`upload_delay_hours: u32`。

- [ ] **Step 1: 写失败的设置模型测试**

在 `src-tauri/src/settings/mod.rs` 的 tests 中：

```rust
#[test]
fn cloud_settings_default_auto_flags() {
    let cloud = CloudBackupSettings::default();
    assert!(!cloud.enabled);
    assert!(!cloud.auto_upload);
    assert!(!cloud.auto_download);
    assert_eq!(cloud.upload_delay_hours, 1);
}

#[test]
fn cloud_settings_missing_new_fields_read_defaults() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        br#"{"cloud":{"enabled":true,"autoUpload":true,"baseUrl":"https://dav.example.com/dav","username":"me","remoteDir":"ccswitchx"}}"#,
    )
    .unwrap();
    let loaded = load_from(&path).unwrap();
    assert!(loaded.cloud.enabled);
    assert!(loaded.cloud.auto_upload);
    assert!(!loaded.cloud.auto_download);
    assert_eq!(loaded.cloud.upload_delay_hours, 1);
}
```

同时把 `roundtrip_preserves_values` 的 `CloudBackupSettings` 字面量补上 `auto_download: false, upload_delay_hours: 6`，并**删除** `legacy_auto_upload_setting_implies_enabled_channel`。

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm build && cargo test --manifest-path src-tauri/Cargo.toml settings::tests::cloud_settings`
Expected: 编译失败（缺少字段）/ 旧迁移测试已删除。

- [ ] **Step 3: 实现设置字段并删除旧迁移**

在 `CloudBackupSettings` 加 `auto_download`、`upload_delay_hours`（`#[serde(default)]`，默认 `1`）；`Default` 同步。删除 `load_from` 里「`auto_upload` 为真就补 `enabled`」的两行逻辑。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests`
Expected: PASS。

- [ ] **Step 5: 写失败的服务层测试**

在 `src-tauri/src/services/settings.rs` tests 中新增/改写：

```rust
#[test]
fn cloud_auto_download_requires_enabled_channel() { /* enabled=false, auto_download=true -> 落盘后为 false */ }

#[test]
fn cloud_upload_delay_must_be_1_to_24() { /* 0 和 25 都返回 InvalidInput */ }

#[test]
fn cloud_connection_changed_flag_tracks_endpoint_and_password() {
    // 首次保存 -> true；同参数再保存 -> false；
    // 改 remote_dir / username / base_url / 密码 -> true；密码传 None -> false。
}
```

把现有所有 `set_cloud_settings_at(...)` 调用补两个参数（`auto_download`、`upload_delay_hours`），并把所有 `let (settings, secrets) = ...` 解构改成三元组 `(settings, secrets, _)`。同时给所有 `CloudBackupSettings { ... }` 字面量补新字段：`services/cloud_backup.rs` 的 `config_for`、`services/backup.rs` 的 `state_with_asset_and_cloud`、`settings/mod.rs` 的 `roundtrip_preserves_values`。

- [ ] **Step 6: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::settings`
Expected: 编译失败（参数数量不匹配）。

- [ ] **Step 7: 实现服务层**

`set_cloud_settings_at` 增加参数并返回 `(AppSettings, Secrets, bool)`：
- `auto_download = enabled && auto_download`；
- `upload_delay_hours` 不在 `1..=24` 时返回 `AppError::InvalidInput("上传延迟需在 1–24 小时之间")`；
- `base_url` 为空时清空 `enabled` / `auto_upload` / `auto_download`；
- `connection_changed` 比较旧/新的 `base_url`、`username`、`remote_dir` 与最终密码（`None` 表示沿用）；
- `cloud_view` 带出新字段；
- `set_backup_settings_at` 启用 Git 时同步把 `cloud.auto_download = false`（与已有的 `auto_upload = false` 并列），并在测试里钉住。

- [ ] **Step 8: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services::settings`
Expected: PASS。

- [ ] **Step 9: 实现重置函数与命令层**

在 `cloud_backup.rs` 新增：

```rust
/// 连接参数变化后作废旧远端的所有同步基线与观测缓存。
pub fn reset_runtime_status(store: &Store) -> Result<(), AppError> {
    match fs::remove_file(status_path(store)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
```

测试 `reset_runtime_status_removes_file_and_ignores_missing`。

`commands/cloud.rs` 的 `set_cloud_settings` 增加 `auto_download: bool`、`upload_delay_hours: u32`；拿到 `connection_changed` 为真时调用 `cloud_backup::reset_runtime_status(&state.store)?`。保存成功后调用 `crate::change_signal::notify()` 唤醒 worker。

- [ ] **Step 10: 更新前端类型与调用点**

`src/types.ts`：`CloudBackupSettings` 加 `autoDownload: boolean`、`uploadDelayHours: number`；`CloudTestInput` 的 `Omit` 增加 `"autoDownload" | "uploadDelayHours"`。`src/panels/BackupsPanel.tsx` 每处 `api.setCloudSettings` 补这两个字段（沿用当前 `cloud` 的值）。测试 fixture 同步补齐：`src/__tests__/BackupsPanel.test.tsx`、`src/__tests__/App.test.tsx`、`src/__tests__/AgentsPanel.test.tsx`、`src/__tests__/SettingsPanel.test.tsx` 里的 `cloud: {...}` 字面量。

- [ ] **Step 11: 运行构建与测试**

Run: `pnpm build && cargo test --manifest-path src-tauri/Cargo.toml services::settings settings::tests`
Expected: PASS；`pnpm build` 无 TS 错误。

- [ ] **Step 12: 提交**

```bash
git add src-tauri/src/settings/mod.rs src-tauri/src/services/settings.rs src-tauri/src/services/cloud_backup.rs src-tauri/src/commands/cloud.rs src/types.ts src/lib/api.ts src/panels/BackupsPanel.tsx
git commit -m "feat: 云备份增加自动下载与上传延迟设置"
```

---

### Task 2: 运行状态拆分与缓存状态接口

**Files:**
- Modify: `src-tauri/src/services/cloud_backup.rs`
- Modify: `src-tauri/src/commands/cloud.rs`
- Modify: `src-tauri/src/lib.rs`（注册 `get_cloud_status_cached`）
- Modify: `src/lib/api.ts`、`src/types.ts`
- Test: `src-tauri/src/services/cloud_backup.rs` tests

**Interfaces:**
- Produces: `CachedRemote { exists, size_bytes, modified_at, etag, checked_at }`（derive `Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize`）。
- Produces: `CloudRuntimeStatus { synced_fingerprint, synced_sha256, synced_meta, observed_remote, observed_manifest, observed_manifest_error, last_upload_at, last_download_at, last_error, last_download_error, auth_paused_until, upload_month, uploaded_bytes_month }`。
- Produces: `cloud_backup::status_cached(state: &AppState) -> Result<CloudStatus, AppError>`。
- Produces: `CloudStatus` 增加 `auto_download`、`upload_delay_hours`、`last_download_at`、`last_download_error`、`uploaded_bytes_month`、`from_cache`。
- Produces: Tauri 命令 `get_cloud_status_cached`。

- [ ] **Step 1: 写失败的状态测试**

```rust
#[test]
fn cached_status_does_not_probe_network() {
    // 运行状态里预置 observed_remote/observed_manifest；
    // status_cached 返回 from_cache=true 且 FakeWebDav 的 PROPFIND 计数为 0。
}

#[test]
fn live_status_never_overwrites_synced_baseline() {
    // 预置 synced_fingerprint=Some("a"), synced_sha256=Some("b")；
    // 调 status() 后两者保持不变，observed_remote 被写入。
}

#[test]
fn live_status_reuses_observed_manifest_when_metadata_matches() {
    // upload_now 之后读一次 GET 计数；再 status() 一次，GET 计数不变。
}

#[test]
fn runtime_status_roundtrip_includes_new_fields() { /* 新结构逐字段相等 */ }
```

把现有 `runtime_status_roundtrip_and_corrupt_file_is_default`、`runtime_status_without_fingerprint_field_reads_none` 改成新字段名与断言。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests`
Expected: 编译失败（字段/函数不存在）。

- [ ] **Step 3: 实现运行状态结构与写入边界**

- 把 `remote_fingerprint` 改名为 `synced_fingerprint`，同时更新 `backup.rs` 与所有测试里的引用（含 `state_with_asset_and_cloud` 相关断言）；新增 §设计 8.1 的其他字段，全部 `#[serde(default)]`。
- `CachedRemote::from_remote(remote: &RemoteObject, checked_at: i64)`。
- `CloudManifestInfo` 加 `Deserialize`。
- `status_cached`：从 settings/secrets/runtime 组装；不建客户端、不抢 `cloud_ops`；`from_cache = true`。
- `status()`：只写 `observed_*`；`observed_remote` 元数据未变时复用 `observed_manifest` / `observed_manifest_error`，变了才 GET manifest；成功探测清除 `auth_paused_until`；`from_cache = false`。
- 上传成功写 `synced_fingerprint` / `synced_sha256`（manifest.archive.sha256）/ `synced_meta`，同时写 `observed_*`，并按 `chrono::Local::now().format("%Y-%m")` 累加 `uploaded_bytes_month`（月份变了先清零）。
- `restore_latest` 成功写 `synced_fingerprint`、`synced_sha256`（下载包 sha256）、`synced_meta`、`observed_*`（现有恢复已写内容指纹，补上其余字段）。
- 测试补充：`upload_increments_monthly_bytes_and_resets_on_new_month`（预置上个月的 `upload_month`，上传后计数从新月份 0 开始累加）。
- 同步更新所有 `CloudStatus { ... }` 与 `CloudRuntimeStatus { ... }` 测试字面量（`status`、`status_from_remote`、`test_state` 相关断言）；漏一个都会编译失败。

- [ ] **Step 4: 注册命令并更新前端 API**

`commands/cloud.rs` 加：

```rust
#[tauri::command]
pub fn get_cloud_status_cached(state: State<'_, AppState>) -> Result<CloudStatus, AppError> {
    cloud_backup::status_cached(&state)
}
```

`lib.rs` 注册；`src/lib/api.ts` 加 `getCloudStatusCached: () => invoke<CloudStatus>("get_cloud_status_cached")`；`src/types.ts` 的 `CloudStatus` 补新字段。

- [ ] **Step 5: 运行测试确认通过**

Run: `pnpm build && cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/services/cloud_backup.rs src-tauri/src/commands/cloud.rs src-tauri/src/lib.rs src/lib/api.ts src/types.ts
git commit -m "feat: 云备份状态拆成同步基线与观测缓存"
```

---

### Task 3: manifest schema 2 与内容指纹

**Files:**
- Modify: `src-tauri/src/services/cloud_backup.rs`
- Test: 同文件 tests

**Interfaces:**
- Produces: `CLOUD_MANIFEST_SCHEMA = 2`；`CloudManifest.content_fingerprint: String`（JSON `contentFingerprint`）。
- Produces: `CloudManifest::compatible()` 要求 `format == CLOUD_MANIFEST_FORMAT && schema == 2`。

- [ ] **Step 1: 写失败的 manifest 测试**

```rust
#[test]
fn upload_writes_schema_2_with_content_fingerprint() {
    // 上传后解析远端 manifest：
    // schema == 2；contentFingerprint == backup::content_fingerprint(&state.store)
}

#[test]
fn status_reports_missing_manifest_as_error() {
    // 远端只有 zip，没有 manifest：manifest 为 None，manifestError 包含 "manifest"。
}

#[test]
fn restore_rejects_schema_1_and_missing_manifest() {
    // schema=1 与无 manifest 都返回包含 "重新上传" 的错误，本地文件不变。
}
```

删除 `status_treats_missing_manifest_as_legacy_not_error`；把 `restore_rejects_foreign_and_newer_manifests` 的 schema 99 断言改为包含 `"重新上传"`，并加入 schema 1 用例。测试里的原始 manifest JSON 都要带 `"contentFingerprint":"..."`，否则会先被 JSON 解析拒绝，测不到 schema 判定。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests`
Expected: FAIL（schema 仍为 1 / 字段缺失）。

- [ ] **Step 3: 实现 schema 2**

- 常量改 2；`CloudManifest` 加必填 `content_fingerprint`。
- `upload_archive_locked` 写入 `backup::content_fingerprint(&state.store)`（已有 `content_fingerprint` 变量，改为写入 manifest）。
- `compatible()` 用 `schema == CLOUD_MANIFEST_SCHEMA`。
- `ensure_restorable()` 对任何非 2 的 schema 返回：`"云端备份由不兼容的版本上传（manifest schema {schema}，本机要求 2），请从源设备重新上传"`。
- `restore_latest` 与自动下载都要求 manifest 必须存在；缺失返回 `AppError::Cloud("云端备份缺少 manifest，请从源设备重新上传")`。
- `status()` 在远端存在但 manifest 缺失时设置 `manifest_error`（同样文案），不再当作旧包正常状态。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/cloud_backup.rs
git commit -m "feat: 云备份 manifest 升到 schema 2 并写入内容指纹"
```

---

### Task 4: 认证失败分类与自动任务暂停

**Files:**
- Modify: `src-tauri/src/error.rs`
- Modify: `src-tauri/src/net/webdav.rs`
- Modify: `src-tauri/src/services/cloud_backup.rs`
- Modify: `src-tauri/src/services/backup.rs`（`auto_upload_if_needed` 暂停检查）
- Test: `src-tauri/src/services/cloud_backup.rs`、`src-tauri/src/services/backup.rs`

**Interfaces:**
- Produces: `AppError::CloudAuth(String)`。
- Produces: `CloudRuntimeStatus.auth_paused_until: Option<i64>`。
- Produces: `record_failure` 在 `CloudAuth` 时把 `auth_paused_until` 设为 `now + 3_600_000`。

- [ ] **Step 1: 写失败的认证测试**

```rust
#[test]
fn auth_failure_pauses_automatic_tasks() {
    // 错误密码调用 upload_now；runtime.auth_paused_until 在未来，last_error 有值。
}

#[test]
fn paused_runtime_skips_auto_upload_without_network() {
    // 正确密码 + 预置 auth_paused_until 在未来；auto_upload_if_needed 后 PUT 计数为 0。
}

#[test]
fn successful_status_probe_clears_auth_pause() {
    // 预置暂停；status() 成功后面板状态/运行状态里暂停被清除。
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests auth`
Expected: 编译失败（变体/字段不存在）。

- [ ] **Step 3: 实现分类与暂停**

- `AppError` 加 `CloudAuth(String)`；`webdav::auth_error` 返回它。
- `record_failure`：是 `CloudAuth` 时写 `auth_paused_until = now + 3_600_000`。
- `record_download_failure` 同样处理（Task 5 使用）。
- `auto_upload_if_needed` 开头检查 `auth_paused_until`，未过期直接返回 `Ok(())`。返回值改成 `Result<Option<u64>>` 放到 Task 6，与 worker 的事件载荷一起改。
- `status()` 成功时清空 `auth_paused_until`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests backup::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/error.rs src-tauri/src/net/webdav.rs src-tauri/src/services/cloud_backup.rs src-tauri/src/services/backup.rs
git commit -m "feat: 云备份认证失败时暂停自动任务"
```

---

### Task 5: 自动下载检查与恢复

**Files:**
- Modify: `src-tauri/src/services/backup.rs`（抽出 `restore_zip`）
- Modify: `src-tauri/src/services/cloud_backup.rs`（新增 `auto_download_if_needed` 与下载辅助）
- Test: 两个文件的 tests

**Interfaces:**
- Produces: `backup::restore_zip(store: &Store, index: &mut Index, zip: &Path) -> Result<RestoreBackupReport, AppError>`；`restore()` 改为解析出快照路径后调用它。
- Produces: `cloud_backup::auto_download_if_needed(state: &AppState) -> Result<Option<CloudRestoreReport>, AppError>`。
- Produces: 内部 `fn remote_changed(prev: Option<&CachedRemote>, now: &RemoteObject) -> bool`。
- Consumes: `synced_fingerprint`、`synced_sha256`、`synced_meta`、`observed_*`、`auth_paused_until`。

- [ ] **Step 1: 写失败的自动下载测试**

在 `cloud_backup.rs` tests 中：

```rust
#[test]
fn auto_download_applies_when_remote_changed() {
    let server = FakeWebDav::start("me", "pw");
    let source = test_state_with_asset(config_for(&server), Some("pw"));
    let snapshot = build_named_snapshot(&source, "cloud-source");
    upload_archive(&source, &snapshot).unwrap();

    let mut settings = config_for(&server);
    settings.auto_download = true;
    let target = test_state(settings, Some("pw"));
    let report = auto_download_if_needed(&target).unwrap().expect("应下载并恢复");

    assert!(target.store.root().join("assets").join("mcp").join("pw.json").is_file());
    assert!(backup::list(&target.store)
        .unwrap()
        .iter()
        .any(|snapshot| snapshot.kind == SnapshotKind::PreRestore));
    assert!(!backup::list(&target.store)
        .unwrap()
        .iter()
        .any(|snapshot| snapshot.name.starts_with("cloud_")));
    let runtime = load_runtime_status(&target.store);
    assert!(runtime.last_download_at.is_some());
    assert!(runtime.synced_sha256.is_some());
    assert_eq!(
        runtime.synced_fingerprint.as_deref(),
        Some(backup::content_fingerprint(&target.store).unwrap().as_str())
    );
    assert!(report.safety_backup.len() > 0);
}

#[test]
fn auto_download_skips_when_remote_metadata_unchanged() {
    // 第一次下载后记录 server.request_count("GET")；
    // 第二次 auto_download_if_needed 返回 None，且 GET 计数不变（元数据未变不读 manifest）。
}

#[test]
fn auto_download_overwrites_local_edits_when_upload_disabled() {
    // 目标机先写入另一份 mcp；自动下载后该 id 不存在、云端来源的 pw 存在。
}

#[test]
fn auto_download_skips_when_local_has_pending_upload() {
    // settings.auto_upload = true, auto_download = true；
    // 目标机本地有资产、runtime.synced_fingerprint = None；
    // 返回 None，本地资产字节不变。
}

#[test]
fn auto_download_rejects_missing_or_incompatible_manifest() {
    // 远端只有 zip 或 schema=1 的 manifest；
    // 返回 Err 包含 "manifest"/"重新上传"，本地资产字节不变，last_download_error 有值。
}

#[test]
fn auto_download_prunes_snapshots_to_retain_count() {
    // settings.retain_count = 1；源机器改内容并上传两次；
    // 每次自动下载后 backup::list 数量 <= 1，且最后一轮仍有 PreRestore。
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests auto_download`
Expected: 编译失败（函数不存在）。

- [ ] **Step 3: 抽出 `restore_zip`**

把 `backup::restore` 的函数体移入 `pub fn restore_zip(store, index, zip: &Path) -> Result<RestoreBackupReport, AppError>`；`restore` 只做「按名字解析路径 → 调用 `restore_zip`」。现有恢复测试必须原样通过。

- [ ] **Step 4: 实现自动下载**

按设计 §7.2 的顺序实现：
- 设置/暂停/Git 检查 → `read_connection`（`InvalidInput` 返回 `Ok(None)`）→ `cloud_ops` → `head()`。
- `remote_changed(synced_meta, &remote)` 为假：写 `observed_remote`，返回 `Ok(None)`。
- `get_manifest()` 必须存在且兼容；`sha == synced_sha256` 只更新 `synced_meta` + `observed_remote`。
- 本地指纹 == `manifest.content_fingerprint`：更新同步基线，返回 `Ok(None)`。
- `auto_upload && synced_fingerprint != Some(local)`：跳过（只写 observed）。
- 下载到 staging，校验大小/哈希/文件数（复用 `restore_latest` 的校验逻辑，抽 `fn verify_downloaded(...)` 或等价私有函数）。
- 内容指纹 == 本地指纹：更新基线，删临时包，返回 `Ok(None)`。
- 否则：`maintenance` 写锁 + index 锁 → `backup::restore_zip` → `notify_git("restore: cloud")`；删临时包；`backup::prune(retain_count)`。
- 成功写同步基线 + observed + `last_download_at`，清空 `last_download_error`，返回报告。
- 失败：`record_download_failure`；`CloudAuth` 时设置暂停；返回 `Err`。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests backup::tests`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/services/backup.rs src-tauri/src/services/cloud_backup.rs
git commit -m "feat: 实现云端更新自动下载并按保留数清理"
```

---

### Task 6: worker 调度、对账与移除 periodic 上传

**Files:**
- Modify: `src-tauri/src/services/backup.rs`
- Test: `src-tauri/src/services/backup.rs` tests

**Interfaces:**
- Produces: `auto_upload_if_needed(state) -> Result<Option<u64>, AppError>`（返回上传字节数，供事件使用）。
- Produces: `fn upload_deadline(first: Instant, last: Instant, delay_hours: u32) -> Instant`。
- Produces: worker 内的 `PendingUpload { first_at, last_at, retry_at }`。
- Produces: 事件 `cloud-auto-synced`，载荷 `{ "kind": "upload"|"download", "bytes"?: u64, "restoredFiles"?: u64, "removedFiles"?: u64, "safetyBackup"?: String }`。

- [ ] **Step 1: 写失败的调度测试**

```rust
#[test]
fn upload_deadline_waits_for_delay_after_last_change() {
    let now = std::time::Instant::now();
    assert_eq!(
        upload_deadline(now - std::time::Duration::from_secs(10), now, 1),
        now + std::time::Duration::from_secs(3600)
    );
}

#[test]
fn upload_deadline_caps_continuous_edits() {
    let now = std::time::Instant::now();
    // first 已过去 10 小时：兜底上限 4 小时，deadline 已经过期，应立即上传。
    assert_eq!(
        upload_deadline(now - std::time::Duration::from_secs(36_000), now, 1),
        now - std::time::Duration::from_secs(21_600)
    );
}

#[test]
fn auto_upload_returns_uploaded_bytes_only_when_it_uploads() {
    // 首次调用返回 Some(>0)；内容未变时返回 None；暂停/未启用也返回 None。
}

#[test]
fn periodic_once_does_not_upload() {
    // auto_upload=true、interval_hours=0，periodic_once 后 PUT 计数为 0。
}
```

把原有 `periodic_once_uploads_*` / `periodic_once_auto_uploads_when_enabled` / `*_skips_upload_*` / `auto_upload_failure_is_recorded` 改成新语义：`periodic_once` 只做本地快照，不触发上传；上传失败记录改由 `auto_upload_if_needed` 测试覆盖。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests upload`
Expected: 编译失败 / 旧测试断言失败。

- [ ] **Step 3: 实现 worker**

`spawn_change_worker` 改为单线程循环（`recv_timeout`）：
- 常量：`UPLOAD_RETRY = 5min`、`DOWNLOAD_INTERVAL = 15min`、`RECONCILE_INTERVAL = 15min`。
- 收到信号：`enabled && auto_upload` 时更新 `pending`（首次记 `first_at`，每次更新 `last_at`）。
- tick 时按顺序：
  1. pending 且 `now >= retry_at.unwrap_or(upload_deadline(...))` → 调 `auto_upload_if_needed`；`Ok(_)` 清 pending；`Err` 时 `retry_at = now + 5min`，`first_at = last_at = now`。
  2. `now >= next_download` → 调 `cloud_backup::auto_download_if_needed`；`Ok(Some(report))` 发 `cloud-auto-synced` 事件；`next_download = now + 15min`。
  3. `now >= next_reconcile` → 若 `auto_upload` 且本地指纹 != `synced_fingerprint` 且无 pending，新建 pending；`next_reconcile = now + 15min`。
- 启动时 `next_download = next_reconcile = Instant::now()`。
- 从 `periodic_once` 删除 `auto_upload_if_needed` 调用。
- `auto_upload_if_needed` 返回值改为上传字节数（`CloudStatus.remote.size_bytes`），暂停时返回 `Ok(None)`；指纹一致/未启用返回 `Ok(None)`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/backup.rs
git commit -m "feat: 云备份 worker 支持延迟上传与定期对账"
```

---

### Task 7: 前端数据层与缓存优先

**Files:**
- Modify: `src/hooks/useBackups.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/types.ts`
- Modify: `src/__tests__/BackupsPanel.test.tsx`（新增 `getCloudStatusCached` mock、`CloudStatus` 新字段与缓存优先用例）

**Interfaces:**
- Produces: `api.getCloudStatusCached()`。
- Produces: `useBackups()` 返回 `cloudChecking: boolean`。
- Consumes: `CloudStatus.from_cache`、新字段。

- [ ] **Step 1: 写失败的缓存优先测试**

```tsx
test("renders cached cloud status before the live probe resolves", async () => {
  vi.mocked(api.getCloudStatusCached).mockResolvedValue({ ...cloudStatus, fromCache: true });
  vi.mocked(api.getCloudStatus).mockReturnValue(new Promise(() => {}));
  render(<BackupsPanel />);
  expect(await screen.findByText("云端有备份")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeEnabled();
});
```

在 mock 对象里加 `getCloudStatusCached: vi.fn()`；`cloudStatus` fixture 补新字段；`mockBase` 设置默认缓存返回值。

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test src/__tests__/BackupsPanel.test.tsx -t "cached cloud status"`
Expected: FAIL（API/字段不存在）。

- [ ] **Step 3: 实现数据层**

- `types.ts`：确认 Task 2 已把 `autoDownload`、`uploadDelayHours`、`lastDownloadAt`、`lastDownloadError`、`uploadedBytesMonth`、`fromCache` 补进 `CloudStatus`；本任务只消费。
- `api.ts`：加 `getCloudStatusCached`（Task 2 已加则确认存在）。
- `useBackups`：新增 `cloudChecking`；`refresh()` 先并行拉本地数据，再 `void primeCloudStatus()`；`primeCloudStatus` 先 `getCloudStatusCached()` 渲染，再 `refreshCloudStatus()` 实时刷新；实时刷新失败时保留缓存状态，只记 error，不清空 `cloudStatus`。

- [ ] **Step 4: 运行测试确认通过**

Run: `pnpm test src/__tests__/BackupsPanel.test.tsx`
Expected: PASS（含旧用例）。

- [ ] **Step 5: 提交**

```bash
git add src/hooks/useBackups.ts src/lib/api.ts src/types.ts src/__tests__/BackupsPanel.test.tsx src/__tests__/App.test.tsx src/__tests__/AgentsPanel.test.tsx src/__tests__/SettingsPanel.test.tsx
git commit -m "feat: 备份面板优先渲染缓存的云备份状态"
```

---

### Task 8: CloudSection 双开关、延迟与恢复按钮

**Files:**
- Modify: `src/components/backups/CloudSection.tsx`
- Modify: `src/panels/BackupsPanel.tsx`
- Modify: `src/__tests__/BackupsPanel.test.tsx`

**Interfaces:**
- Consumes: `CloudSettingsView.autoDownload` / `uploadDelayHours`，`CloudStatus.fromCache` / 新字段，`cloudChecking: boolean`。
- Produces: `onToggleAuto(patch: Partial<Pick<CloudSettingsView, "autoUpload" | "autoDownload" | "uploadDelayHours">>)`；`CloudSection` 新增 `cloudChecking: boolean` prop。

- [ ] **Step 1: 写失败的 UI 测试**

```tsx
test("toggles auto upload and auto download independently", async () => {
  // 通道启用时分别点击「自动上传」「自动下载」；
  // 断言 setCloudSettings 收到 autoUpload:true / autoDownload:true，另一项保持原值。
});

test("changes the upload delay", async () => {
  // 选择 6 小时；断言 setCloudSettings 收到 uploadDelayHours:6，其余字段沿用 cloud。
});

test("blocks restore when the remote manifest is missing", async () => {
  // getCloudStatus 返回 manifest:null、remote.exists:true；
  // 「从云端恢复」禁用。
});

test("keeps restore enabled while the live status is still loading", async () => {
  // getCloudStatusCached 先返回有备份；getCloudStatus 永不 resolve；
  // 「从云端恢复」仍可点。
});

test("refreshes cloud status after auto sync event", async () => {
  // 触发 "cloud-auto-synced" handler 后，getCloudStatus 调用次数增加。
});
```

删除 `explains a legacy remote package without manifest`；把「自动备份」相关用例改成「自动上传」；把 `warns when the remote manifest is incompatible` 的期望文案改成 `/请从源设备重新上传/`；新增 `shows the last auto download error`（`lastDownloadError` 非空 -> 提示可见）。

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test src/__tests__/BackupsPanel.test.tsx -t "auto download"`
Expected: FAIL（找不到 label）。

- [ ] **Step 3: 实现 UI**

- `CloudSection`：
  - 两个复选框：「自动上传」「自动下载」；`disabled={!cloud.enabled || !!busy}`。
  - 「上传延迟」`Select`，选项 1/6/12/24，「小时」。
  - 文案：
    - 自动上传：`改动停下 N 小时后上传；持续编辑最多每 min(4N, 24h) 补传一次，内容没变不重复上传。`
    - 自动下载：`启动时及每 15 分钟检查云端；有更新就覆盖本机（云端为准，覆盖前自动建安全快照）。`
  - 调用 `onToggleAuto({ autoUpload })` / `onToggleAuto({ autoDownload })` / `onToggleAuto({ uploadDelayHours })`。
  - 恢复按钮：`configured = cloudStatus?.configured ?? (!!cloud?.baseUrl && !!cloud?.username && !!cloud?.hasPassword)`；`knownMissing = cloudStatus?.remote?.exists === false`；`knownIncompatible = cloudStatus?.manifest?.compatible === false || (cloudStatus?.remote?.exists === true && !cloudStatus?.manifest)`；只在 `busy || draftDirty || !configured || knownMissing || knownIncompatible` 时禁用。
  - 状态区新增「上次下载」「本机本月上传」（`formatBytes`）。
  - 新增 Notice：`lastDownloadError` 非空时显示「上次自动下载失败：{error}」。
  - 不兼容提示改为「云端备份由不兼容的版本上传（manifest schema X），请从源设备重新上传一次。」；删除旧版裸包提示。
  - 头部在 `cloudChecking` 时显示「检查中…」，不阻塞按钮。
- `BackupsPanel`：
  - 所有 `setCloudSettings` 调用带 `autoDownload` / `uploadDelayHours`。
  - `toggleCloudAuto` 改为接收 patch 并合并当前 `cloud`。
  - 监听 `cloud-auto-synced`：刷新云状态并设置 notice（下载显示恢复文件数，上传显示已上传）。
  - 测试的事件 mock 改成 `handlers: Map<string, Function>`，支持 `cloud-restore-progress` 与 `cloud-auto-synced`；现有的「下载进度」用例改用 `handlers.get("cloud-restore-progress")` 触发。

- [ ] **Step 4: 运行测试确认通过**

Run: `pnpm test src/__tests__/BackupsPanel.test.tsx`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/components/backups/CloudSection.tsx src/panels/BackupsPanel.tsx src/__tests__/BackupsPanel.test.tsx
git commit -m "feat: 云备份面板拆出自动上传与自动下载"
```

---

### Task 9: 全量验证与真机沙箱

**Files:**
- 无新增代码；修复验证发现的问题。

- [ ] **Step 1: 前端构建**

Run: `pnpm build`
Expected: 无 TS 错误、构建成功。

- [ ] **Step 2: Rust 全量测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: 全绿，无 ignored 之外的失败。

- [ ] **Step 3: 前端全量测试**

Run: `pnpm test`
Expected: 全绿。

- [ ] **Step 4: 真机沙箱往返**

Run: `pwsh -NoProfile -File scripts/seed-ui-e2e.ps1`（验证结束必须 `pwsh -NoProfile -File scripts/seed-ui-e2e.ps1 -Restore`）。

用临时 store + `baseOverride` 铺数据：
1. 机器 A 开「自动上传」，改一个 prompt；
2. 等延迟（测试时临时把 `upload_delay_hours` 设为 1，必要时在沙箱里手动触发「立即上传」模拟到点）；
3. 机器 B（另一个 store）只开「自动下载」，启动后确认资产被覆盖、生成 `PreRestore` 快照、没有 `cloud_restore_*` 残留；
4. 确认 B 的本地改动在云端更新后被覆盖；A 的本地改动不会被 B 上传覆盖。
5. 核对真实 `~/.claude`、`~/.codex`、`~/.qoder-cn` 文件哈希前后一致。

- [ ] **Step 5: 提交验证修复（如有）**

```bash
git add -A
git commit -m "test: 云备份自动同步全量验证"
```
