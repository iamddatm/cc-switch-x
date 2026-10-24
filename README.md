# CC Switch X

跨 agent 的 AI 编程资产管理中心:用纯文本文件作为唯一真源,统一管理 MCP / Skills / Prompts / Rules / Subagents / Hooks 六类资产,并把它们分发到 Qoder、Qoder CN 等 agent 的配置里。

设计与计划文档:

- 设计:`docs/superpowers/specs/2026-10-22-cc-switch-x-design.md`
- 实现计划:`docs/superpowers/plans/`

## 技术栈

Tauri 2 + Rust(`domain` / `store` / `index` / `services` / `commands` 四层)+ React 18 + TypeScript + Vite + Tailwind。

## 开发

```bash
pnpm install        # 前端依赖
pnpm test           # 前端测试(Vitest)
pnpm build          # 前端产物 dist/
cargo test --manifest-path src-tauri/Cargo.toml   # 后端测试(需先 pnpm build,dist/ 是编译输入)
pnpm tauri dev      # 启动桌面应用(自动执行 pnpm dev)
```

## 数据目录

运行时真源默认在 `~/.ccswitchx/`:`assets/` 为文本真源,`agents/` 为启用矩阵,`index.db` 为可随时删除重建的派生索引,`local/`、`secrets.local.json`、`backups/`、`logs/`、`state/` 均不参与同步。

## 发布与更新

应用内更新使用 Tauri updater，当前只发布 Windows x64 的 NSIS 当前用户安装包，清单与安装包都放在 GitHub Releases。

首次发布前需要准备签名密钥：

```bash
pnpm tauri signer generate -w ~/.tauri/cc-switch-x.key
```

把输出的公钥填入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，并在 GitHub 仓库设置两个 secret：

- `TAURI_SIGNING_PRIVATE_KEY`：私钥文件内容（两行原文，不是路径）
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：生成密钥时设置的密码

私钥和密码只留在本机与 GitHub Secrets，**不要提交到仓库**。密钥丢失后，已安装的旧版本无法验证新签名，只能让用户手动安装一次新版本。

应用启用了 `requireSignedVersion`，手工用 `tauri signer sign` 签发安装包时必须带 `--app-version <版本>`，否则更新器会拒绝该签名。

发布流程：

1. 同步修改 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 的版本号，运行 `cargo check` 更新 `src-tauri/Cargo.lock`。
2. 新增 `docs/release-notes/v<版本>.md`。
3. 运行 `node scripts/check-release-version.mjs` 做本地门禁。
4. 提交后打 tag（如 `v0.1.1`）并推送 tag，`.github/workflows/release.yml` 会构建、签名、生成 `latest.json` 并发布为 GitHub latest release。
5. 手动触发 workflow 时只构建并上传 workflow artifact，不创建 Release，可用于干跑。

本地签名构建：

```bash
TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/cc-switch-x.key) \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<密码> \
pnpm tauri build --ci --bundles nsis
```

更新检查由前端启动后延迟 1 秒触发，也可在「Settings → 关于」手动检查。更新说明来自 `latest.json`，只做纯文本展示。
