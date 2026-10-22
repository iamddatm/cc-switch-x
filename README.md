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
