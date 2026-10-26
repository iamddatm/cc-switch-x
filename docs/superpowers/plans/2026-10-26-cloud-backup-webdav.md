# WebDAV 云备份 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加 WebDAV 云备份：把可移植真源打成一份最新状态包上传到固定远端对象，并能下载后走现有安全恢复流水线落回本地。

**Architecture:** `net/webdav.rs` 只负责 HTTP 传输（MKCOL / PUT / MOVE / DELETE / GET / HEAD、Basic Auth、代理、限流）；`services/cloud_backup.rs` 负责打包、上传、下载、恢复与状态；`commands/cloud.rs` 是薄 IPC 层；前端在备份面板新增 WebDAV 区块。云端固定 `ccswitchx-backup.zip`，通过临时对象 + MOVE 原子替换。

**Tech Stack:** Rust（Tauri 2、reqwest blocking、zip、chrono）、React 18、TypeScript、Vitest。

**Spec:** `docs/superpowers/specs/2026-10-26-cloud-backup-webdav-design.md`

## Global Constraints

- 云端只保留一份最新状态包；上传覆盖远端，最后上传者获胜。
- 固定对象名 `ccswitchx-backup.zip`；临时对象 `ccswitchx-backup.zip.tmp-<时间戳>`；默认远端目录 `ccswitchx`。
- 包内容只包含 `assets/`、`agents/<known>.json` 与 `.gitignore`；不得包含 `local/`、`state/`、`secrets.local.json`、`index.db`、`logs/`、`backups/`、`.git`。
- 密码只进 `secrets.local.json`（scope `cloud.webdav`，key `password`）；不得出现在设置文件、日志、错误信息或命令返回值中。
- 上传 / 下载上限 512 MiB；恢复端沿用 `ArchiveLimits::default()`：30,000 条目、4 KiB 最小计费、最大深度 32。
- 快照格式、manifest 校验、安全快照与白名单恢复全部复用 `services/backup.rs`，不新增包格式。
- 网络操作使用全局代理配置；连接超时 30 秒，整体超时 600 秒；禁止跟随重定向。
- HTTP 与 HTTPS 都允许；HTTP 在 UI 明确警告；不跳过 TLS 校验、不关闭证书验证。
- 自动上传只挂在自动快照成功之后；手动上传不创建额外本地快照。
- 锁顺序：上传 `cloud_ops` → `maintenance`（仅打包）→ 释放 → 网络；恢复 `cloud_ops` → 下载 → `maintenance` → `index`。不跨网络持有 `maintenance`、`index`、`settings`、`secrets`。
- 不新增 Rust / npm 依赖；测试使用临时 store 与本机假 WebDAV，不碰真实 `~/.claude`、`~/.codex`、`~/.qoder-cn`。
- 前端用户可见文案用中文；后端调用集中在 `src/lib/api.ts`，类型集中在 `src/types.ts`。
- `cargo check` / `cargo test` 前先运行 `pnpm build`（`tauri::generate_context!` 需要 `dist/`）。
- 提交信息用中文；每个任务完成一个可独立审查的提交。

## Review Focus

- **认证失败**：应用密码错误时必须返回“WebDAV 认证失败”，不得写本地、不得泄露密码。测试：Task 2 `wrong_password_returns_auth_error`、Task 5 `test_connection_rejects_wrong_password`、Task 6 `upload_failure_records_last_error`。
- **云端没有备份**：首次恢复必须得到“云端还没有备份，请先上传”，不能当成坏包或本地错误。测试：Task 2 `head_missing_returns_exists_false`、Task 7 `restore_missing_remote_is_friendly_error`。
- **空包覆盖**：本地没有资产 / agents 时不得覆盖云端唯一副本。测试：Task 4 `validate_archive_rejects_empty_package`、Task 6 `upload_rejects_empty_store`。
- **超过恢复上限的包**：上传前必须拒绝，避免“上传成功、换机恢复不了”。测试：Task 4 `validate_archive_rejects_too_many_entries` / `validate_archive_rejects_oversize_charge`、Task 7 `restore_rejects_oversize_remote`。
- **MOVE 不支持或部分写入**：服务器不支持 MOVE 时回退直接 PUT；失败时清理临时对象；上传后 HEAD 比对大小。测试：Task 3 `move_unsupported_falls_back_to_put`、`failed_move_cleans_temp_object`、`put_verifies_remote_size`。

---

### Task 1: 云备份设置、密钥与 URL 校验

**Files:**
- Modify: `src-tauri/src/settings/mod.rs`
- Modify: `src-tauri/src/services/settings.rs`
- Create: `src-tauri/src/net/webdav.rs`
- Modify: `src-tauri/src/net/mod.rs`
- Modify: `src-tauri/src/error.rs`
- Test: inline `#[cfg(test)] mod tests` in the files above
- Commit also: `docs/superpowers/specs/2026-10-26-cloud-backup-webdav-design.md`, `docs/superpowers/plans/2026-10-26-cloud-backup-webdav.md`

**Interfaces:**
- Consumes: existing `AppSettings`, `Secrets`, `SecretRef`, `AppError`.
- Produces:
  - `pub struct CloudBackupSettings { pub auto_upload: bool, pub base_url: String, pub username: String, pub remote_dir: String }`
  - `AppSettings.cloud: CloudBackupSettings`（`#[serde(default)]`）
  - `pub struct CloudSettingsView { pub auto_upload: bool, pub base_url: String, pub username: String, pub remote_dir: String, pub has_password: bool }`
  - `pub fn cloud_secret_ref() -> SecretRef`
  - `pub fn cloud_view(settings: &AppSettings, secrets: &Secrets) -> CloudSettingsView`
  - `pub fn set_cloud_settings_at(config_path: &Path, secrets_path: &Path, base_url: String, username: String, password: Option<String>, remote_dir: String, auto_upload: bool) -> Result<(AppSettings, Secrets), AppError>`
  - `pub fn normalize_base_url(raw: &str) -> Result<String, AppError>`
  - `pub fn validate_remote_dir(raw: &str) -> Result<String, AppError>`
  - `pub fn remote_url(base_url: &str, remote_dir: &str, object: &str) -> Result<reqwest::Url, AppError>`

- [ ] **Step 1: 写失败测试（设置默认值与迁移）**

在 `settings/mod.rs` 的测试模块加入：

```rust
#[test]
fn old_settings_files_get_default_cloud_settings() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let old = serde_json::json!({
        "storeRoot": dir.path().join("data"),
        "theme": "light",
        "enabledAgents": null
    });
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

    let loaded = load_from(&path).unwrap();
    assert!(!loaded.cloud.auto_upload);
    assert_eq!(loaded.cloud.base_url, "");
    assert_eq!(loaded.cloud.username, "");
    assert_eq!(loaded.cloud.remote_dir, "ccswitchx");
}

#[test]
fn cloud_settings_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let settings = AppSettings {
        cloud: CloudBackupSettings {
            auto_upload: true,
            base_url: "https://dav.example.com/dav".into(),
            username: "me".into(),
            remote_dir: "backups/ccswitchx".into(),
        },
        ..AppSettings::default()
    };
    save_to(&path, &settings).unwrap();
    assert_eq!(load_from(&path).unwrap(), settings);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests::old_settings_files_get_default_cloud_settings`
Expected: FAIL，提示 `CloudBackupSettings` / `cloud` 字段不存在。

- [ ] **Step 3: 实现设置模型**

在 `settings/mod.rs` 增加结构体、默认值与 `AppSettings.cloud` 字段；同步更新 `AppSettings::default()` 与所有 `AppSettings { ... }` 测试字面量。`CloudBackupSettings` 的字段使用 `#[serde(default)]`，`remote_dir` 使用 `default_cloud_remote_dir()` 返回 `"ccswitchx"`。同时在 `error.rs` 增加 `#[error("云备份失败：{0}")] Cloud(String)`，供后续上传校验使用。

- [ ] **Step 4: 写失败测试（密钥与校验）**

在 `services/settings.rs` 测试模块加入：

```rust
#[test]
fn cloud_password_semantics() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("settings.json");
    let secrets_path = dir.path().join("secrets.local.json");

    let (saved, secrets) = set_cloud_settings_at(
        &config,
        &secrets_path,
        "https://dav.example.com/dav".into(),
        "me".into(),
        Some("app-password".into()),
        "ccswitchx".into(),
        true,
    )
    .unwrap();
    assert!(saved.cloud.auto_upload);
    assert_eq!(secrets.get(&cloud_secret_ref()), Some("app-password"));
    assert!(!std::fs::read_to_string(&config).unwrap().contains("app-password"));
    assert!(cloud_view(&saved, &secrets).has_password);

    let (kept, kept_secrets) = set_cloud_settings_at(
        &config, &secrets_path,
        "https://dav.example.com/dav".into(), "me".into(), None,
        "ccswitchx".into(), false,
    ).unwrap();
    assert_eq!(kept_secrets.get(&cloud_secret_ref()), Some("app-password"));
    assert!(!kept.cloud.auto_upload);

    let cleared = set_cloud_settings_at(
        &config, &secrets_path, String::new(), String::new(), None,
        "ccswitchx".into(), true,
    ).unwrap();
    assert!(cleared.1.get(&cloud_secret_ref()).is_none());
    assert!(!cleared.0.cloud.auto_upload);

    assert!(set_cloud_settings_at(
        &config, &secrets_path,
        "https://dav.example.com/dav".into(), "me".into(), Some(String::new()),
        "ccswitchx".into(), false,
    ).is_err());
}

#[test]
fn cloud_url_and_remote_dir_validation() {
    assert_eq!(normalize_base_url(" https://dav.example.com/dav/ ").unwrap(), "https://dav.example.com/dav");
    assert_eq!(normalize_base_url("http://127.0.0.1:8099/dav").unwrap(), "http://127.0.0.1:8099/dav");
    for bad in ["ftp://example.com/dav", "https://user:pass@example.com/dav", "https://example.com/dav?x=1"] {
        assert!(normalize_base_url(bad).is_err(), "{bad}");
    }
    assert_eq!(validate_remote_dir("").unwrap(), "ccswitchx");
    assert_eq!(validate_remote_dir("backups/ccswitchx").unwrap(), "backups/ccswitchx");
    for bad in ["../x", "/abs", "a\\b", "a//b", "a/../b", "a b"] {
        assert!(validate_remote_dir(bad).is_err(), "{bad}");
    }
}

#[test]
fn remote_url_joins_base_path() {
    let url = remote_url("https://dav.example.com/dav", "ccswitchx", "ccswitchx-backup.zip").unwrap();
    assert_eq!(url.as_str(), "https://dav.example.com/dav/ccswitchx/ccswitchx-backup.zip");
}
```

- [ ] **Step 5: 实现密钥与校验**

- 在 `services/settings.rs` 加 `CLOUD_SECRET_SCOPE = "cloud.webdav"`、`CLOUD_SECRET_KEY = "password"`、`cloud_secret_ref()`、`cloud_view()`、`set_cloud_settings_at()`。
- 密码语义：`None` 保留旧密码；`Some("")` 清除；`Some(v)` 覆盖。配置了 `base_url` 与 `username` 时必须有密码，否则返回 `AppError::InvalidInput("请填写 WebDAV 密码")`。
- `base_url` 为空时清空用户名、密码与 `auto_upload`，保留规范化后的 `remote_dir`。
- 保存顺序对齐 `set_proxy_at`：先保存 secrets，再保存 settings。
- 在 `net/webdav.rs` 实现 `normalize_base_url`、`validate_remote_dir`、`remote_url`；`net/mod.rs` 增加 `pub mod webdav;`。
- 校验规则按设计 6.3：http/https、拒绝 userinfo/query/fragment；远端目录逐段 `[A-Za-z0-9._-]`，拒绝 `..`、空段、反斜杠、绝对路径；空目录回退 `ccswitchx`。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud`
Expected: PASS（新增设置 / 密钥 / URL 测试）。

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/settings/mod.rs src-tauri/src/services/settings.rs src-tauri/src/net/mod.rs src-tauri/src/net/webdav.rs src-tauri/src/error.rs docs/superpowers/specs/2026-10-26-cloud-backup-webdav-design.md docs/superpowers/plans/2026-10-26-cloud-backup-webdav.md
git commit -m "feat: 增加云备份设置与密钥保管"
```

---

### Task 2: WebDAV 基础传输

**Files:**
- Modify: `src-tauri/src/net/webdav.rs`
- Test: inline `#[cfg(test)] mod tests` + `#[cfg(test)] pub(crate) mod test_support`

**Interfaces:**
- Consumes: `ProxyConfig::apply_blocking`、`AppError`。
- Produces:
  - `pub const BACKUP_OBJECT: &str = "ccswitchx-backup.zip";`
  - `pub const MAX_CLOUD_BYTES: u64 = 512 * 1024 * 1024;`
  - `pub struct WebDavConfig { pub base_url: String, pub username: String, pub password: String, pub remote_dir: String }`（Clone，不派生 Debug）
  - `pub struct RemoteObject { pub exists: bool, pub size_bytes: Option<u64>, pub modified_at: Option<i64>, pub etag: Option<String> }`
  - `pub struct WebDavClient { ... }`
  - `pub fn new(config: WebDavConfig, proxy: Option<&ProxyConfig>) -> Result<Self, AppError>`
  - `pub fn ensure_dir(&self) -> Result<(), AppError>`
  - `pub fn head(&self) -> Result<RemoteObject, AppError>`
  - `pub fn get_to_file(&self, dest: &Path, max_bytes: u64) -> Result<RemoteObject, AppError>`
  - `pub fn test(&self) -> Result<(), AppError>`

- [ ] **Step 1: 写假 WebDAV 测试服务**

在 `net/webdav.rs` 加 `#[cfg(test)] pub(crate) mod test_support`：

```rust
pub(crate) struct FakeWebDav { /* addr + Arc<Mutex<FakeState>> + join handle */ }

impl FakeWebDav {
    pub(crate) fn start(username: &str, password: &str) -> Self;
    pub(crate) fn base_url(&self) -> String;          // http://127.0.0.1:<port>/dav
    pub(crate) fn put_object(&self, path: &str, bytes: Vec<u8>);
    pub(crate) fn object(&self, path: &str) -> Option<Vec<u8>>;
    pub(crate) fn paths(&self) -> Vec<String>;
    pub(crate) fn request_count(&self, method: &str) -> usize;
    pub(crate) fn set_reject_move(&self, reject: bool);
    pub(crate) fn set_fail_move(&self, fail: bool);
    pub(crate) fn set_redirect(&self, redirect: bool);
    pub(crate) fn set_fail_put(&self, fail: bool);
    pub(crate) fn set_head_size_override(&self, size: Option<u64>);
}
```

实现一个最小 TCP HTTP 服务：读取到 `\r\n\r\n`，按 `Content-Length` 读 body；校验 `Authorization: Basic base64(user:pass)`；支持 MKCOL、PUT、MOVE、DELETE、GET、HEAD；响应 `Connection: close`。路径包含 `/dav/ccswitchx/...`。`set_reject_move(true)` 时 MOVE 返回 405；`set_fail_move(true)` 时 MOVE 返回 500；`set_redirect(true)` 时返回 302；`set_fail_put(true)` 时 PUT 返回 500；`set_head_size_override(Some(n))` 时 HEAD 报告 `Content-Length: n`，用于模拟超限远端而不分配大文件。

- [ ] **Step 2: 写失败测试（基础传输）**

```rust
#[test]
fn mkcol_is_ok_when_directory_exists() { /* fake returns 405 for second MKCOL; assert ensure_dir ok */ }

#[test]
fn head_missing_returns_exists_false() {
    let server = FakeWebDav::start("me", "pw");
    let client = client_for(&server);
    let remote = client.head().unwrap();
    assert!(!remote.exists);
}

#[test]
fn get_downloads_object() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"zip-bytes".to_vec());
    let client = client_for(&server);
    let dir = tempdir().unwrap();
    let dest = dir.path().join("download.zip");
    let remote = client.get_to_file(&dest, MAX_CLOUD_BYTES).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"zip-bytes");
    assert_eq!(remote.size_bytes, Some(9));
}

#[test]
fn get_rejects_oversize_before_writing() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", vec![0; 32]);
    let client = client_for(&server);
    let dir = tempdir().unwrap();
    let dest = dir.path().join("download.zip");
    assert!(client.get_to_file(&dest, 8).is_err());
    assert!(!dest.exists());
}

#[test]
fn wrong_password_returns_auth_error() {
    let server = FakeWebDav::start("me", "pw");
    let mut config = config_for(&server);
    config.password = "wrong".into();
    let client = WebDavClient::new(config, None).unwrap();
    let error = client.test().unwrap_err().to_string();
    assert!(error.contains("认证失败"));
    assert!(!error.contains("wrong"));
}

#[test]
fn redirect_not_followed() {
    let server = FakeWebDav::start("me", "pw");
    server.set_redirect(true);
    let client = client_for(&server);
    assert!(client.test().is_err());
}
```

`client_for(&server)` / `config_for(&server)` 是测试内 helper，返回 `WebDavClient` 与 `WebDavConfig`，base URL 用 `server.base_url()`，用户名 `me`，密码 `pw`，远端目录 `ccswitchx`。

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml net::webdav::tests`
Expected: FAIL，传输类型 / 方法不存在。

- [ ] **Step 4: 实现传输**

- `WebDavClient::new`：`reqwest::blocking::Client::builder()` + User-Agent + connect timeout 30s + timeout 600s + `redirect(Policy::none())`；有代理时 `proxy.apply_blocking(builder)`。
- 所有请求对 `remote_url(base, remote_dir, BACKUP_OBJECT)` 或目录 URL 发起；Basic Auth 用 `.basic_auth(username, Some(password))`。
- `ensure_dir`：逐段 MKCOL；201/200/204/405 成功；409 报“父目录不存在”；401/403 报认证失败。
- `head`：200 解析 `Content-Length`、`Last-Modified`（`chrono::DateTime::parse_from_rfc2822`）与 `ETag`；404 返回 `exists: false`；401/403 认证错误；其他状态报“WebDAV HEAD 失败：HTTP {status}”。
- `get_to_file`：先看 `Content-Length`，超过 `max_bytes` 直接报错；用 `response.take(max_bytes + 1)` 写入文件，写超限则删除文件并报错；404 报“云端还没有备份，请先上传”。
- `test`：`ensure_dir` + `head`（404 视为成功）。
- 错误消息不得包含密码或 `Authorization`。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml net::webdav::tests`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/net/webdav.rs
git commit -m "feat: 增加 WebDAV 基础传输"
```

---

### Task 3: WebDAV 原子上传

**Files:**
- Modify: `src-tauri/src/net/webdav.rs`
- Test: inline `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `WebDavClient`、`RemoteObject`、`BACKUP_OBJECT`。
- Produces: `pub fn put(&self, source: &Path) -> Result<RemoteObject, AppError>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn put_replaces_object_via_move() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"old".to_vec());
    let client = client_for(&server);
    let file = temp_file(b"new-zip");
    let remote = client.put(&file).unwrap();
    assert_eq!(server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(), b"new-zip");
    assert_eq!(remote.size_bytes, Some(7));
    assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
}

#[test]
fn move_unsupported_falls_back_to_put() {
    let server = FakeWebDav::start("me", "pw");
    server.set_reject_move(true);
    let client = client_for(&server);
    let file = temp_file(b"fallback");
    client.put(&file).unwrap();
    assert_eq!(server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(), b"fallback");
    assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
}

#[test]
fn failed_move_cleans_temp_object() {
    let server = FakeWebDav::start("me", "pw");
    server.set_fail_move(true);
    let client = client_for(&server);
    let file = temp_file(b"data");
    assert!(client.put(&file).is_err());
    assert!(server.paths().iter().all(|path| !path.contains(".tmp-")));
}

#[test]
fn put_verifies_remote_size() {
    let server = FakeWebDav::start("me", "pw");
    server.set_truncate_after_put(true);
    let client = client_for(&server);
    let file = temp_file(b"0123456789");
    let error = client.put(&file).unwrap_err().to_string();
    assert!(error.contains("大小不一致"));
}
```

为 `FakeWebDav` 增加 `set_truncate_after_put(bool)`：PUT 只保存前 3 字节，用于验证上传后 HEAD 比对。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml webdav::tests::put_replaces_object_via_move`
Expected: FAIL，`put` 不存在。

- [ ] **Step 3: 实现 `put`**

1. `ensure_dir`。
2. 临时路径：`<remote_dir>/ccswitchx-backup.zip.tmp-<UTC 时间戳>`。
3. 流式 PUT 临时对象：`Body::sized(File, len)`，`Content-Type: application/zip`，显式 `Content-Length`。
4. MOVE 临时对象到最终对象：`Destination` 为最终绝对 URL，`Overwrite: T`。
5. MOVE 返回 405/501：删除临时对象（尽力），回退直接 PUT 最终对象。
6. 其他失败：尽力 DELETE 临时对象，返回错误。
7. 成功后 HEAD 最终对象，比较 `Content-Length` 与本地文件大小；不一致报“上传后远端大小不一致”。
8. 返回最终 `RemoteObject`；临时对象清理失败不覆盖主错误。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml webdav::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/net/webdav.rs
git commit -m "feat: WebDAV 上传使用临时对象原子替换"
```

---

### Task 4: 快照打包复用与上传前校验

**Files:**
- Modify: `src-tauri/src/services/backup.rs`
- Test: inline `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: existing `collect_files`, `write_zip`, `read_manifest`, `ArchiveLimits`。
- Produces:
  - `pub struct ArchiveBuild { pub created_at: i64, pub files: u64, pub size_bytes: u64 }`
  - `pub struct ArchiveSummary { pub files: u64, pub total_bytes: u64, pub max_depth: usize }`
  - `pub fn write_archive_to(store: &Store, target: &Path, kind: SnapshotKind) -> Result<ArchiveBuild, AppError>`
  - `pub fn validate_archive_for_restore(path: &Path) -> Result<ArchiveSummary, AppError>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn validate_archive_rejects_empty_package() {
    let (_dir, store, _index) = fixture();
    let info = create(&store, Some("empty"), SnapshotKind::Manual).unwrap();
    let path = snapshots_dir(&store).join(format!("{}.zip", info.name));
    let error = validate_archive_for_restore(&path).unwrap_err().to_string();
    assert!(error.contains("没有可备份的资产"));
}

#[test]
fn validate_archive_rejects_too_many_entries() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("too-many.zip");
    write_manifest_only_zip(&path, 30_001, 1, &["a"]);
    assert!(validate_archive_for_restore(&path)
        .unwrap_err()
        .to_string()
        .contains("超过可恢复上限"));
}

#[test]
fn validate_archive_rejects_oversize_charge() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("oversize.zip");
    write_manifest_only_zip(&path, 1, 600 * 1024 * 1024, &["a"]);
    assert!(validate_archive_for_restore(&path)
        .unwrap_err()
        .to_string()
        .contains("超过可恢复上限"));
}

#[test]
fn validate_archive_accepts_normal_package() {
    let (_dir, store, _index) = fixture();
    write_mcp(&store, "pw", "Playwright");
    let info = create(&store, Some("ok"), SnapshotKind::Manual).unwrap();
    let path = snapshots_dir(&store).join(format!("{}.zip", info.name));
    let summary = validate_archive_for_restore(&path).unwrap();
    assert!(summary.files >= 2);
    assert!(summary.total_bytes > 0);
}
```

`write_manifest_only_zip(path, file_count, file_size, path_components)` 是测试 helper：写一个只含 `manifest.json` 的 zip，manifest 的 `files` 数组按参数生成，`format = "ccswitchx-backup"`、`schema = 1`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::validate_archive_rejects_empty_package`
Expected: FAIL，`validate_archive_for_restore` 不存在。

- [ ] **Step 3: 实现打包复用与校验**

- 把 `create` 中“collect files → manifest → write_zip → rename”的中间步骤抽成 `write_archive_to(store, target, kind)`，返回 `ArchiveBuild`；`create` 仍负责临时文件与 rename，并用 `ArchiveBuild` 构造 `SnapshotInfo`（`name` 仍由 `create` 决定），行为不变。
- `validate_archive_for_restore(path)`：
  - 用 `read_manifest` 校验 `format` 与 `schema`。
  - 非空：除 `.gitignore` 外至少一个文件，否则 `AppError::Cloud("本地没有可备份的资产，已拒绝上传")`。
  - 条目数：`manifest.files.len() + 1`（含 manifest）不得超过 `ArchiveLimits::default().max_entries`。
  - 总字节：`sum(max(file.size, limits.min_entry_bytes)) + limits.min_entry_bytes` 不得超过 `max_total_bytes`。
  - 深度：路径按 `/` 分段数不得超过 `max_depth`。
  - 失败统一 `AppError::Cloud(format!("备份包超过可恢复上限：..."))`。
  - 返回 `ArchiveSummary`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests`
Expected: PASS，现有快照测试不回归。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/backup.rs
git commit -m "refactor: 抽取快照打包并增加上传前校验"
```

---

### Task 5: 云备份状态、连接测试与并发锁

**Files:**
- Create: `src-tauri/src/services/cloud_backup.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: inline `#[cfg(test)] mod tests` in `cloud_backup.rs`

**Interfaces:**
- Consumes: `AppState`, `Store`, `Secrets`, `WebDavClient`, `CloudBackupSettings`, `cloud_secret_ref`, `ProxyConfig`。
- Produces:
  - `AppState.cloud_ops: Arc<Mutex<()>>`
  - `pub struct CloudRuntimeStatus { pub last_upload_at: Option<i64>, pub last_error: Option<String> }`（Serialize + Deserialize + Default + PartialEq + Eq + Debug + Clone）
  - `pub struct RemoteStatus { pub exists: bool, pub size_bytes: Option<u64>, pub modified_at: Option<i64>, pub etag: Option<String> }`
  - `pub struct CloudStatus { pub configured: bool, pub has_password: bool, pub auto_upload: bool, pub remote: Option<RemoteStatus>, pub remote_error: Option<String>, pub last_upload_at: Option<i64>, pub last_error: Option<String> }`
  - 所有返回前端的结构体使用 `#[derive(Serialize)]` + `#[serde(rename_all = "camelCase")]`；`CloudRuntimeStatus` 同时可反序列化。
  - `pub fn status_path(store: &Store) -> PathBuf`
  - `pub fn load_runtime_status(store: &Store) -> CloudRuntimeStatus`
  - `pub fn save_runtime_status(store: &Store, status: &CloudRuntimeStatus) -> Result<(), AppError>`
  - `pub fn status(state: &AppState) -> Result<CloudStatus, AppError>`
  - `pub fn test_connection(state: &AppState, base_url: &str, username: &str, password: Option<&str>, remote_dir: &str) -> Result<u64, AppError>`

- [ ] **Step 1: 写失败测试（状态文件与状态查询）**

```rust
#[test]
fn runtime_status_roundtrip_and_corrupt_file_is_default() {
    let dir = tempdir().unwrap();
    let store = Store::init(dir.path().join("store")).unwrap();
    let status = CloudRuntimeStatus {
        last_upload_at: Some(123),
        last_error: Some("boom".into()),
    };
    save_runtime_status(&store, &status).unwrap();
    assert_eq!(load_runtime_status(&store), status);

    std::fs::write(status_path(&store), "{bad json").unwrap();
    assert_eq!(load_runtime_status(&store), CloudRuntimeStatus::default());
}

#[test]
fn status_without_config_does_not_probe_network() {
    let state = test_state(CloudBackupSettings::default(), None);
    let status = status(&state).unwrap();
    assert!(!status.configured);
    assert!(status.remote.is_none());
}

#[test]
fn status_reports_remote_object() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"12345".to_vec());
    let state = test_state(config_for(&server), Some("pw"));
    let status = status(&state).unwrap();
    assert!(status.configured);
    assert!(status.remote.as_ref().unwrap().exists);
    assert_eq!(status.remote.as_ref().unwrap().size_bytes, Some(5));
}

#[test]
fn status_probe_error_is_reported_without_failing() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state(config_for_with_password(&server, "wrong"), Some("pw"));
    let status = status(&state).unwrap();
    assert!(status.remote_error.is_some());
    assert!(status.remote.is_none());
}

#[test]
fn test_connection_rejects_wrong_password() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state(config_for(&server), Some("pw"));
    let error = test_connection(
        &state, &server.base_url(), "me", Some("wrong"), "ccswitchx",
    ).unwrap_err().to_string();
    assert!(error.contains("认证失败"));
    assert!(!error.contains("wrong"));
}

#[test]
fn test_connection_returns_latency() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state(config_for(&server), Some("pw"));
    let latency = test_connection(
        &state, &server.base_url(), "me", Some("pw"), "ccswitchx",
    ).unwrap();
    assert!(latency < 5_000);
}
```

`test_state(settings, password)` helper：用临时目录构造 `AppSettings`（`store_root` 指向临时 store），调用 `AppState::init`，把密码插入 `state.secrets`。`config_for` / `config_for_with_password` 返回 `CloudBackupSettings`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup`
Expected: FAIL，模块 / 类型不存在。

- [ ] **Step 3: 实现状态与锁**

- `state.rs` 增加 `pub cloud_ops: Arc<Mutex<()>>`，`AppState::init` 初始化为 `Arc::new(Mutex::new(()))`。
- `services/mod.rs` 增加 `pub mod cloud_backup;`。
- `cloud_backup.rs` 实现：
  - `status_path` = `store.root().join("state/cloud-backup.json")`；读写用 `store::atomic::write_atomic`；读取失败（缺失 / 损坏 / IO）返回默认并 `eprintln!`。
  - `configured` = `base_url` 非空 && `username` 非空 && secrets 中有非空密码。
  - `status` 克隆配置 / 密码 / 代理后释放锁；获取 `cloud_ops`；未配置直接返回；已配置构建 `WebDavClient` 并 `head`，失败写入 `remote_error`，成功写 `remote`。
  - `test_connection`：密码参数 `Some` 用参数，`None` 读 secrets；获取 `cloud_ops`；构建客户端；`client.test()`；返回 `Instant::elapsed().as_millis() as u64`。
  - 不跨网络持有 `settings` / `secrets` / `maintenance` / `index`。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/cloud_backup.rs src-tauri/src/services/mod.rs src-tauri/src/state.rs
git commit -m "feat: 增加云备份状态与连接测试"
```

---

### Task 6: 云备份上传

**Files:**
- Modify: `src-tauri/src/services/cloud_backup.rs`
- Test: inline `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `backup::write_archive_to`、`backup::validate_archive_for_restore`、`WebDavClient::put`、`CloudRuntimeStatus`。
- Produces:
  - `pub fn upload_now(state: &AppState) -> Result<CloudStatus, AppError>`
  - `pub fn upload_archive(state: &AppState, archive: &Path) -> Result<CloudStatus, AppError>`

- [ ] **Step 1: 写失败测试（上传成功、空包、失败记录）**

```rust
#[test]
fn upload_now_writes_remote_and_status() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state_with_asset(config_for(&server), Some("pw"));
    let status = upload_now(&state).unwrap();
    assert!(server.object("/dav/ccswitchx/ccswitchx-backup.zip").is_some());
    assert!(status.remote.as_ref().unwrap().exists);
    assert!(status.last_upload_at.is_some());
    assert_eq!(load_runtime_status(&state.store).last_error, None);
}

#[test]
fn upload_rejects_empty_store() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"existing".to_vec());
    let state = test_state(config_for(&server), Some("pw"));
    let error = upload_now(&state).unwrap_err().to_string();
    assert!(error.contains("没有可备份的资产"));
    assert_eq!(server.object("/dav/ccswitchx/ccswitchx-backup.zip").unwrap(), b"existing");
}

#[test]
fn upload_failure_records_last_error() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state_with_asset(config_for(&server), Some("wrong"));
    assert!(upload_now(&state).is_err());
    let runtime = load_runtime_status(&state.store);
    assert!(runtime.last_error.is_some());
    assert!(runtime.last_upload_at.is_none());
}

#[test]
fn upload_now_cleans_temp_archive() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state_with_asset(config_for(&server), Some("pw"));
    upload_now(&state).unwrap();
    let staging = state.store.root().join("state/staging");
    let leftovers = std::fs::read_dir(&staging).unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("cloud-"))
        .count();
    assert_eq!(leftovers, 0);
}
```

`test_state_with_asset` 在临时 store 写一个 MCP 资产后构造 `AppState`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests::upload_now_writes_remote_and_status`
Expected: FAIL，`upload_now` 不存在。

- [ ] **Step 3: 实现上传**

- `upload_now`：
  1. 读取配置 / 密码 / 代理；校验已配置。
  2. 获取 `cloud_ops`。
  3. 在 `maintenance` 写锁下用 `backup::write_archive_to` 写 `state/staging/cloud-<timestamp>.zip`；释放锁。
  4. 调用 `backup::validate_archive_for_restore`；失败时写 `last_error`、删除临时包、返回错误。
  5. 调用 `upload_archive`（内部完成 PUT + 状态更新）；删除临时包。
- `upload_archive`（公开，供自动上传使用）：获取 `cloud_ops` 后调用私有 `upload_archive_locked`。
- `upload_now`：已持有 `cloud_ops` 时直接调用 `upload_archive_locked`，避免二次加锁。
- `upload_archive_locked`：
  1. 校验配置 / 密码。
  2. `backup::validate_archive_for_restore(archive)`。
  3. `client.put(archive)`。
  4. 成功：`last_upload_at = now_ms`，`last_error = None`；失败：`last_error = Some(message)`，保留 `last_upload_at`；保存 runtime status。
  5. 返回 `CloudStatus`（用 `client.head()` 结果或 `put` 返回的 `RemoteObject`，避免再次加 `cloud_ops`）。
- 任何失败都不得删除或覆盖已有远端备份（除 PUT / MOVE 的正常覆盖路径）。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/cloud_backup.rs
git commit -m "feat: 实现 WebDAV 云备份上传"
```

---

### Task 7: 从云端下载并安全恢复

**Files:**
- Modify: `src-tauri/src/services/cloud_backup.rs`
- Test: inline `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `WebDavClient::head/get_to_file`、`backup::validate_archive_for_restore`、`backup::restore`、`state.notify_git`。
- Produces:
  - `pub struct CloudRestoreReport { pub safety_backup: String, pub restored_files: u64, pub removed_files: u64, pub downloaded_snapshot: String, pub remote_size_bytes: u64, pub remote_modified_at: Option<i64> }`
  - `pub fn restore_latest(state: &AppState) -> Result<CloudRestoreReport, AppError>`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn restore_latest_restores_assets_and_creates_safety_snapshot() {
    let server = FakeWebDav::start("me", "pw");
    let source = test_state_with_asset(config_for(&server), Some("pw"));
    let snapshot = build_named_snapshot(&source, "cloud-source");
    server.put_object(
        "/dav/ccswitchx/ccswitchx-backup.zip",
        std::fs::read(snapshot).unwrap(),
    );

    let target = test_state(config_for(&server), Some("pw"));
    std::fs::write(target.store.root().join("local/agents/sentinel.json"), "keep").unwrap();
    let report = restore_latest(&target).unwrap();

    assert!(target.store.asset_dir(AssetType::Mcp).join("pw.json").is_file());
    assert!(!report.safety_backup.is_empty());
    assert!(report.downloaded_snapshot.starts_with("cloud_restore_"));
    assert_eq!(
        std::fs::read_to_string(target.store.root().join("local/agents/sentinel.json")).unwrap(),
        "keep"
    );
}

#[test]
fn restore_missing_remote_is_friendly_error() {
    let server = FakeWebDav::start("me", "pw");
    let state = test_state(config_for(&server), Some("pw"));
    let error = restore_latest(&state).unwrap_err().to_string();
    assert!(error.contains("云端还没有备份"));
}

#[test]
fn restore_rejects_foreign_archive() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"not a zip".to_vec());
    let state = test_state(config_for(&server), Some("pw"));
    assert!(restore_latest(&state).is_err());
    assert!(!state.store.root().join("agents").join("claude.json").exists());
}

#[test]
fn restore_rejects_oversize_remote_without_downloading() {
    let server = FakeWebDav::start("me", "pw");
    server.put_object("/dav/ccswitchx/ccswitchx-backup.zip", b"x".to_vec());
    server.set_head_size_override(Some(MAX_CLOUD_BYTES + 1));
    let state = test_state(config_for(&server), Some("pw"));
    let before = server.request_count("GET");
    assert!(restore_latest(&state).is_err());
    assert_eq!(server.request_count("GET"), before);
}
```

`build_named_snapshot(&state, name)` 用 `backup::create(&state.store, Some(name), SnapshotKind::Manual)` 生成 zip 并返回路径。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup::tests::restore_latest_restores_assets_and_creates_safety_snapshot`
Expected: FAIL，`restore_latest` 不存在。

- [ ] **Step 3: 实现恢复**

1. 读取配置 / 密码 / 代理；未配置直接报错；获取 `cloud_ops`。
2. `client.head()`；`!exists` 返回 `AppError::Cloud("云端还没有备份，请先上传")`；`size_bytes > MAX_CLOUD_BYTES` 返回超限错误，且不发起 GET。
3. 下载到 `state/staging/cloud-<timestamp>.zip`；失败清理临时文件。
4. `backup::validate_archive_for_restore(&temp)`；失败清理并返回“云端备份无效：...”。
5. 生成唯一本地名 `cloud_restore_YYYYMMDD_HHMMSS`（冲突加 `_2`）；获取 `maintenance` 写锁与 `index` 锁；把临时包 `fs::rename` 到 `backups/snapshots/<name>.zip`。
6. `backup::restore(&state.store, &mut index, &name)`；成功 / 回滚后 `state.notify_git("restore: cloud")`。
7. 不调用 `prune`；下载包受正常保留策略约束。
8. 返回 `CloudRestoreReport`，其中 `remote_size_bytes` 来自 HEAD / GET，`remote_modified_at` 来自 HEAD。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cloud_backup`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/cloud_backup.rs
git commit -m "feat: 实现从云端下载并安全恢复"
```

---

### Task 8: 自动快照后自动上传

**Files:**
- Modify: `src-tauri/src/services/backup.rs`
- Modify: `src-tauri/src/services/cloud_backup.rs`（如需暴露 `upload_archive` 给 backup 使用）
- Test: inline `#[cfg(test)] mod tests` in `backup.rs`

**Interfaces:**
- Consumes: `CloudBackupSettings.auto_upload`、`cloud_backup::upload_archive`。
- Produces: `periodic_once(state: &AppState)` 改为 `pub(crate)`（测试可见），行为增加自动上传。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn periodic_once_auto_uploads_when_enabled() {
    let server = FakeWebDav::start("me", "pw");
    let state = state_with_asset_and_cloud(
        auto_upload: true,
        base_url: server.base_url(),
        password: "pw",
    );
    state.settings.lock().unwrap().backup.interval_hours = 1;

    periodic_once(&state);

    assert_eq!(list(&state.store).unwrap().len(), 1);
    assert!(server.object("/dav/ccswitchx/ccswitchx-backup.zip").is_some());
}

#[test]
fn periodic_once_skips_upload_when_disabled() {
    let server = FakeWebDav::start("me", "pw");
    let state = state_with_asset_and_cloud(false, server.base_url(), "pw");
    state.settings.lock().unwrap().backup.interval_hours = 1;

    periodic_once(&state);

    assert!(server.object("/dav/ccswitchx/ccswitchx-backup.zip").is_none());
}

#[test]
fn auto_upload_failure_is_recorded() {
    let server = FakeWebDav::start("me", "pw");
    server.set_fail_put(true);
    let state = state_with_asset_and_cloud(true, server.base_url(), "pw");
    state.settings.lock().unwrap().backup.interval_hours = 1;

    periodic_once(&state);

    assert!(load_runtime_status(&state.store).last_error.is_some());
}
```

`state_with_asset_and_cloud` helper 构造临时 `AppState`，写入一个 MCP 资产，设置 `cloud.auto_upload` / `base_url` / `remote_dir` / `username`，并把密码写入 `state.secrets`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::periodic_once_auto_uploads_when_enabled`
Expected: FAIL，自动上传未发生。

- [ ] **Step 3: 实现自动上传**

- 把 `periodic_once` 的 `create` + `prune` 放进独立作用域，确保 `maintenance` 写锁在上传前释放。
- `create` 成功后读取 `state.settings.cloud.auto_upload`；为真时调用 `cloud_backup::upload_archive(&state, &snapshot_path)`。
- 上传失败只 `eprintln!`，`upload_archive` 已写 `last_error`；不 return、不打断下一次调度。
- `snapshot_path` 用 `snapshots_dir(&state.store).join(format!("{}.zip", info.name))`。
- 自动上传不重复打包，直接上传刚创建的自动快照。

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::periodic_once`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/backup.rs src-tauri/src/services/cloud_backup.rs
git commit -m "feat: 自动快照后自动上传云端"
```

---

### Task 9: 接入云备份 IPC 命令

**Files:**
- Create: `src-tauri/src/commands/cloud.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `services::settings::{cloud_view, set_cloud_settings_at}`、`services::cloud_backup::{status, test_connection, upload_now, restore_latest}`、`CloudSettingsView`、`CloudStatus`、`CloudRestoreReport`。
- Produces（Tauri 命令）:
  - `get_cloud_settings() -> CloudSettingsView`
  - `set_cloud_settings(base_url: String, username: String, password: Option<String>, remote_dir: String, auto_upload: bool) -> CloudSettingsView`
  - `test_cloud_connection(base_url: String, username: String, password: Option<String>, remote_dir: String) -> u64`
  - `get_cloud_status() -> CloudStatus`
  - `cloud_upload_now() -> CloudStatus`
  - `cloud_restore_latest() -> CloudRestoreReport`

- [ ] **Step 1: 写命令**

按 `commands/backup.rs` / `commands/git.rs` 的模式：
- `get_cloud_settings` 读 `state.settings` 与 `state.secrets` 后调用 `cloud_view`。
- `set_cloud_settings` 调用 `set_cloud_settings_at(&settings_path()?, &secrets_path(state.store.root()), ...)`，成功后更新 `state.settings` 与 `state.secrets`，返回 `cloud_view`。
- `test_cloud_connection` / `get_cloud_status` / `cloud_upload_now` / `cloud_restore_latest` 用 `AppHandle` + `spawn_blocking`，在闭包里 `app.state::<AppState>()`，错误统一 `AppError::Cloud(format!("云备份任务执行失败：{error}"))`。
- 所有命令参数名保持 snake_case；前端用 camelCase 调用。

- [ ] **Step 2: 注册**

- `commands/mod.rs` 增加 `pub mod cloud;`
- `lib.rs` 的 `invoke_handler` 增加 6 个命令。

- [ ] **Step 3: 验证编译与测试**

Run: `pnpm build && cargo check --manifest-path src-tauri/Cargo.toml`
Expected: PASS。

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS（现有测试 + 新增服务测试）。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/commands/cloud.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat: 接入云备份 IPC 命令"
```

---

### Task 10: 前端类型、API 与数据 hook

**Files:**
- Modify: `src/types.ts`
- Modify: `src/lib/api.ts`
- Modify: `src/hooks/useBackups.ts`
- Modify: `src/__tests__/BackupsPanel.test.tsx`（补 cloud API mock，保持现有测试通过）
- Modify: `src/__tests__/App.test.tsx`
- Modify: `src/__tests__/SettingsPanel.test.tsx`

**Interfaces:**
- Consumes: 后端命令名与返回结构。
- Produces（TypeScript）:
  - `CloudBackupSettings { autoUpload: boolean; baseUrl: string; username: string; remoteDir: string }`
  - `CloudSettingsView extends CloudBackupSettings { hasPassword: boolean }`
  - `CloudSettingsInput = CloudBackupSettings & { password: string | null }`
  - `CloudTestInput = Omit<CloudSettingsInput, "autoUpload">`
  - `CloudRemoteStatus { exists: boolean; sizeBytes: number | null; modifiedAt: number | null; etag: string | null }`
  - `CloudStatus { configured: boolean; hasPassword: boolean; autoUpload: boolean; remote: CloudRemoteStatus | null; remoteError: string | null; lastUploadAt: number | null; lastError: string | null }`
  - `CloudRestoreReport { safetyBackup: string; restoredFiles: number; removedFiles: number; downloadedSnapshot: string; remoteSizeBytes: number; remoteModifiedAt: number | null }`
  - `AppSettings.cloud: CloudBackupSettings`
  - `api.getCloudSettings`, `api.setCloudSettings`, `api.testCloudConnection`, `api.getCloudStatus`, `api.cloudUploadNow`, `api.cloudRestoreLatest`

- [ ] **Step 1: 加类型与 API**

在 `types.ts` 加上述类型，并给 `AppSettings` 加 `cloud`。在 `api.ts` 加：

```ts
getCloudSettings: () => invoke<CloudSettingsView>("get_cloud_settings"),
setCloudSettings: (input: CloudSettingsInput) =>
  invoke<CloudSettingsView>("set_cloud_settings", input),
testCloudConnection: (input: CloudTestInput) =>
  invoke<number>("test_cloud_connection", input),
getCloudStatus: () => invoke<CloudStatus>("get_cloud_status"),
cloudUploadNow: () => invoke<CloudStatus>("cloud_upload_now"),
cloudRestoreLatest: () => invoke<CloudRestoreReport>("cloud_restore_latest"),
```

`CloudSettingsInput` 只包含可写字段与 `password`；不要把 `hasPassword` 传回后端。

- [ ] **Step 2: 扩展 useBackups**

- 新增 state：`cloud: CloudSettingsView | null`、`cloudStatus: CloudStatus | null`。
- 先定义 `refreshCloudStatus`，再定义 `refresh`，避免 `refresh` 的闭包引用尚未初始化的回调。
- 新增 `refreshCloudStatus`：调用 `api.getCloudStatus()`，失败时设置 `cloudStatus = null` 并把错误写入现有 `error`。
- `refresh` 的 `Promise.all` 增加 `api.getCloudSettings()`；设置 state 后调用 `void refreshCloudStatus()`，不阻塞首屏。
- `run` 成功后仍调用 `refresh()`，从而刷新配置与状态。
- 返回 `cloud`、`cloudStatus`、`refreshCloudStatus`。
- 不给云状态加轮询；面板手动刷新。

- [ ] **Step 2b: 补齐现有测试 mock**

- `BackupsPanel.test.tsx` 的 `api` mock 增加 `getCloudSettings` / `getCloudStatus`，默认返回未配置状态；否则 `useBackups` 新增调用会让现有测试失败。
- `App.test.tsx` / `SettingsPanel.test.tsx` 的 `getSettings` mock 补 `cloud: { autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx" }`。

- [ ] **Step 3: 验证类型与现有测试**

Run: `pnpm build`
Expected: PASS（若 TS 报 `getSettings` mock 缺 `cloud`，在测试 mock 中补默认值）。

Run: `pnpm test`
Expected: PASS（现有 119 个测试不回归；面板测试此时尚未新增云交互）。

- [ ] **Step 4: 提交**

```bash
git add src/types.ts src/lib/api.ts src/hooks/useBackups.ts src/__tests__/BackupsPanel.test.tsx src/__tests__/App.test.tsx src/__tests__/SettingsPanel.test.tsx
git commit -m "feat: 前端接入云备份数据层"
```

---

### Task 11: 备份面板云备份区块

**Files:**
- Modify: `src/panels/BackupsPanel.tsx`
- Modify: `src/__tests__/BackupsPanel.test.tsx`
- Modify: `src/__tests__/App.test.tsx`（如 Task 10 未补）
- Modify: `src/__tests__/SettingsPanel.test.tsx`（如 Task 10 未补）

**Interfaces:**
- Consumes: `useBackups` 的 `cloud`、`cloudStatus`、`refreshCloudStatus`，以及 Task 10 的 API。
- Produces: 备份面板“云备份（WebDAV）”区块。

- [ ] **Step 1: 写失败测试（面板交互）**

在 `BackupsPanel.test.tsx` 的 api mock 中加入 6 个云 API；`mockBase` 增加：

```ts
const cloudView: CloudSettingsView = {
  autoUpload: true,
  baseUrl: "https://dav.example.com/dav",
  username: "me",
  remoteDir: "ccswitchx",
  hasPassword: true,
};
const cloudStatus: CloudStatus = {
  configured: true, hasPassword: true, autoUpload: true,
  remote: { exists: true, sizeBytes: 2048, modifiedAt: Date.now() - 300_000, etag: "\"abc\"" },
  remoteError: null,
  lastUploadAt: Date.now() - 300_000,
  lastError: null,
};
vi.mocked(api.getCloudSettings).mockResolvedValue(cloudView);
vi.mocked(api.getCloudStatus).mockResolvedValue(cloudStatus);
vi.mocked(api.setCloudSettings).mockResolvedValue(cloudView);
vi.mocked(api.testCloudConnection).mockResolvedValue(42);
vi.mocked(api.cloudUploadNow).mockResolvedValue(cloudStatus);
vi.mocked(api.cloudRestoreLatest).mockResolvedValue({
  safetyBackup: "backup_safety",
  restoredFiles: 6,
  removedFiles: 1,
  downloadedSnapshot: "cloud_restore_20261026_120000",
  remoteSizeBytes: 2048,
  remoteModifiedAt: Date.now() - 300_000,
});
```

新增测试：

```ts
test("renders cloud backup section and remote status", async () => {
  render(<BackupsPanel />);
  expect(await screen.findByText("云备份（WebDAV）")).toBeInTheDocument();
  expect(screen.getByText(/ccswitchx\/ccswitchx-backup\.zip/)).toBeInTheDocument();
  expect(screen.getByText("云端有备份")).toBeInTheDocument();
});

test("saves cloud settings without overwriting the saved password", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.change(screen.getByLabelText("WebDAV 地址"), {
    target: { value: "https://dav.example.com/dav" },
  });
  fireEvent.click(screen.getByRole("button", { name: "保存云备份" }));
  await waitFor(() =>
    expect(api.setCloudSettings).toHaveBeenCalledWith({
      baseUrl: "https://dav.example.com/dav",
      username: "me",
      password: null,
      remoteDir: "ccswitchx",
      autoUpload: true,
    }),
  );
});

test("tests the cloud connection", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "测试连接" }));
  await waitFor(() => expect(api.testCloudConnection).toHaveBeenCalled());
  expect(await screen.findByText(/连接成功（42 ms）/)).toBeInTheDocument();
});

test("uploads and restores only after confirmation", async () => {
  render(<BackupsPanel />);
  await screen.findByText("云备份（WebDAV）");
  fireEvent.click(screen.getByRole("button", { name: "立即上传" }));
  await waitFor(() => expect(api.cloudUploadNow).toHaveBeenCalled());

  fireEvent.click(screen.getByRole("button", { name: "从云端恢复" }));
  const dialog = await screen.findByRole("alertdialog", { name: "从云端恢复？" });
  expect(api.cloudRestoreLatest).not.toHaveBeenCalled();
  fireEvent.click(within(dialog).getByRole("button", { name: "恢复" }));
  await waitFor(() => expect(api.cloudRestoreLatest).toHaveBeenCalled());
});

test("disables cloud actions without config", async () => {
  vi.mocked(api.getCloudSettings).mockResolvedValue({
    autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx", hasPassword: false,
  });
  vi.mocked(api.getCloudStatus).mockResolvedValue({
    configured: false, hasPassword: false, autoUpload: false,
    remote: null, remoteError: null, lastUploadAt: null, lastError: null,
  });
  render(<BackupsPanel />);
  expect(await screen.findByText("未配置")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "立即上传" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "从云端恢复" })).toBeDisabled();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test -- src/__tests__/BackupsPanel.test.tsx`
Expected: FAIL，云备份区块不存在。

- [ ] **Step 3: 实现面板**

- 在 Git 区块之后加“云备份（WebDAV）”区块，图标用 `Cloud`（lucide-react）。
- 状态徽标优先级：未配置（无地址 / 无密码）→ 连接失败（`remoteError`）→ 云端有备份（`remote.exists`）→ 已配置（其余情况）。未配置时显示“未配置”，云端无对象时显示“已配置”。
- 表单：WebDAV 地址、用户名、密码、远端目录；固定 id 为 `cloud-base-url`、`cloud-username`、`cloud-password`、`cloud-remote-dir`，`Field` 的 `htmlFor` 与 `Input` 的 `id` 对应，测试用 `getByLabelText` 定位。密码输入 `type={show ? "text" : "password"}`，`hasPassword && !passwordDirty` 时 placeholder 为“已保存，留空保持不变”；显示/隐藏按钮用现有 `IconButton`。
- 从 `cloud` 初始化表单；保存成功后清空密码输入与 dirty 标记。
- 复选框“自动快照后自动上传”。
- 按钮：`测试连接`、`保存云备份`、`立即上传`、`从云端恢复`。
- 保存调用 `api.setCloudSettings({ baseUrl: ..., username: ..., password: passwordDirty ? password : null, remoteDir: ..., autoUpload: ... })`。
- 测试连接使用草稿字段与 `passwordDirty ? password : null`，成功显示“连接成功（{latency} ms）”。
- 立即上传调用 `api.cloudUploadNow()`，成功后显示远端大小；从云端恢复先弹 `ConfirmDialog`，确认后调用 `api.cloudRestoreLatest()`，展示 `restoredFiles` / `removedFiles` / `safetyBackup`。
- 未配置（`!cloudStatus?.configured`）时禁用立即上传 / 从云端恢复；测试连接只要求地址非空。
- 显示远端路径 `${baseUrl}/${remoteDir}/ccswitchx-backup.zip`、远端大小 / 修改时间、`lastUploadAt`、`lastError`、`remoteError`。
- `baseUrl` 以 `http://` 开头时显示 `Notice tone="warning"`：HTTP 会明文传输凭据，仅建议内网使用。
- 云状态刷新按钮调用 `refreshCloudStatus`。
- 更新 `App.test.tsx` / `SettingsPanel.test.tsx` 的 `getSettings` mock，补 `cloud: { autoUpload: false, baseUrl: "", username: "", remoteDir: "ccswitchx" }`。

- [ ] **Step 4: 运行测试确认通过**

Run: `pnpm test -- src/__tests__/BackupsPanel.test.tsx`
Expected: PASS。

Run: `pnpm build && pnpm test`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/panels/BackupsPanel.tsx src/__tests__/BackupsPanel.test.tsx src/__tests__/App.test.tsx src/__tests__/SettingsPanel.test.tsx
git commit -m "feat: 备份面板增加 WebDAV 云备份"
```

---

### Task 12: 第三方声明、全量门禁与手工验证

**Files:**
- Modify: `THIRD_PARTY_NOTICES.md`
- Test: full gates + manual verification

**Interfaces:**
- Consumes: 前 11 个任务的实现。
- Produces: 可交付的 WebDAV 云备份。

- [ ] **Step 1: 更新第三方声明**

如果 WebDAV 传输参考或复用了上游 cc-switch 的 `webdav.rs` 思路，在 `THIRD_PARTY_NOTICES.md` 的 cc-switch 行补充：

- 复用内容：WebDAV MKCOL / PUT / MOVE / HEAD / GET 与 Basic Auth 传输思路。
- 落点：`src-tauri/src/net/webdav.rs`。

如果实现完全独立，也建议保留该归属，避免后续维护争议。

- [ ] **Step 2: 全量门禁**

Run: `pnpm build`
Expected: PASS。

Run: `pnpm test`
Expected: PASS，所有前端测试通过。

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS，所有 Rust 测试通过（`--ignored` 的导入测试不在本次范围）。

- [ ] **Step 3: 手工验证**

1. 启动一个本机 WebDAV 服务（例如 `rclone serve webdav --addr 127.0.0.1:8099 --user test --pass test <临时目录>`），不要使用真实云账号做破坏性测试。
2. 用临时 store 启动应用（遵循项目现有 `baseOverride` / 沙箱方式），不要使用真实 `~/.claude`、`~/.codex`、`~/.qoder-cn`。
3. 在备份面板填写 `http://127.0.0.1:8099/dav`、`test` / `test`、远端目录 `ccswitchx`，保存并测试连接。
4. 上传后确认远端出现 `ccswitchx-backup.zip`；修改本地一个资产；从云端恢复并确认恢复、创建安全快照、`local/` 与密钥未变。
5. 分别验证：错误密码、云端无备份、HTTP 警告提示。
6. 如果本机没有可用的 WebDAV 服务或无法启动桌面应用，明确记录“UI 未手工验证”，不要声称已通过。

- [ ] **Step 4: 提交**

```bash
git add THIRD_PARTY_NOTICES.md
git commit -m "docs: 补充 WebDAV 云备份第三方声明"
```

---

## Self-Review Notes

- Spec coverage：设置 / 密钥（Task 1）、传输（Task 2–3）、打包与上限（Task 4）、状态（Task 5）、上传（Task 6）、恢复（Task 7）、自动上传（Task 8）、IPC（Task 9）、前端数据层（Task 10）、面板（Task 11）、声明与门禁（Task 12）。
- Review Focus 的五类失败都有对应测试；认证错误、404、空包、超限、MOVE 回退分别落在 Task 2/3/4/5/6/7。
- 类型一致性：`CloudBackupSettings` / `CloudSettingsView` / `CloudStatus` / `CloudRestoreReport` / `RemoteObject` / `ArchiveSummary` 在前后端任务中使用同一组字段名。
- 测试全部使用临时 store 与本机假 WebDAV；没有引入新依赖。
