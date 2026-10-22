# CC Switch X 基础骨架实现计划（Plan 1/5）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 做出一个可启动的 Tauri 桌面骨架，具备文本优先真源、可重建的 SQLite 索引、六类资产面板外壳，以及 MCP 资产的端到端增删改查。

**Architecture:** Rust 侧分四层：`domain`（canonical 类型与校验）、`store`（文件真源与原子写）、`index`（SQLite 派生索引）、`services/commands`（Tauri 命令）。前端用 React 单页壳，侧栏复刻 cc-switch 的面板结构；本计划只把 MCP 面板做通，其余面板留占位。

**Tech Stack:** Tauri 2、Rust（serde、rusqlite bundled、sha2、tempfile、thiserror、dirs）、React 18、TypeScript、Vite 6、Tailwind 3、Vitest + Testing Library。

**Spec:** `docs/superpowers/specs/2026-10-22-cc-switch-x-design.md`

## 计划拆分

本 spec 覆盖多个可独立交付的子系统，按依赖顺序拆成 5 份计划：

1. **Plan 1（本文件）**：工程骨架、文本真源、索引、UI 壳、MCP 端到端。
2. Plan 2：cc-switch 本地/云端导入、既有 agent 适配、Skills 与 Prompts。
3. Plan 3：Rules / Subagents / Hooks、Qoder 三入口适配、漂移检测。
4. Plan 4：历史对话（本地浏览 + 恢复）。
5. Plan 5：Git / WebDAV / S3 同步、打包与发布。

每份计划结束时都应得到一个可启动、可测试的版本。

## Global Constraints

- Windows 优先；路径一律用 `PathBuf`，不在代码里拼 `/` 或 `\`。
- 所有用户可见文案用中文；代码标识符用英文。
- 文本文件是唯一真源；`index.db` 必须可随时删除并从文件重建。
- 设备级设置、路径覆盖、密钥不进入 Git / WebDAV / S3；首版只管理用户级资产。
- 运行时不依赖 Node / npx；Node 只用于前端开发工具链。
- 每个 commit 必须可独立构建；提交信息用中文。
- 复用 cc-switch / teamai-cli / vercel-labs/skills 的代码或格式时保留 MIT 版权声明。
- 未实测的 agent 能力一律先隐藏开关，不硬写。

## Review Focus

以下是最容易让用户踩坑、但单体测试容易漏掉的输入/状态；每条都在对应任务的测试里钉住：

1. **Windows 覆盖写**：目标文件已存在时，原子写必须完整替换，且不留下临时文件。
2. **ID 路径穿越**：`../evil`、`a/b`、空 ID 必须被拒绝，任何写入不得逃出真源目录。
3. **索引残留**：删除或重命名资产文件后，重建索引必须移除旧行。
4. **存储根切换**：修改 store root 后重新加载设置必须指向新根，并明确提示需要重启。
5. **文本编码**：带 UTF-8 BOM、CRLF 换行、中文 frontmatter 的文件必须能正确解析。

---

## File Structure

```
package.json, index.html, vite.config.ts, tsconfig*.json,
tailwind.config.cjs, postcss.config.cjs, .gitignore, README.md
src/
  main.tsx, App.tsx, styles.css
  types.ts, lib/api.ts
  components/shell/{AppShell,Sidebar}.tsx
  panels/{McpPanel,SkillsPanel,PromptsPanel,RulesPanel,SubagentsPanel,HooksPanel,SessionsPanel,SettingsPanel}.tsx
  hooks/{useAssets,useTheme}.ts
  test/setup.ts
  __tests__/App.test.tsx, __tests__/McpPanel.test.tsx, __tests__/SettingsPanel.test.tsx
src-tauri/
  Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json
  src/main.rs, src/lib.rs, src/error.rs, src/state.rs
  src/domain/{mod,asset,agent,frontmatter,mcp,prompt,rule,subagent,hook}.rs
  src/store/{mod,atomic}.rs
  src/index/{mod,schema}.rs
  src/settings/mod.rs
  src/services/{assets,settings}.rs
  src/commands/{mod,assets,settings}.rs
```

---

### Task 1: 工程骨架与可启动壳

**Files:**
- Create: `package.json`, `index.html`, `vite.config.ts`, `tsconfig.json`, `tsconfig.node.json`, `tailwind.config.cjs`, `postcss.config.cjs`, `.gitignore`, `README.md`
- Create: `src/main.tsx`, `src/App.tsx`, `src/styles.css`, `src/test/setup.ts`, `src/__tests__/App.test.tsx`
- Create: `src-tauri/Cargo.toml`, `src-tauri/build.rs`, `src-tauri/tauri.conf.json`, `src-tauri/capabilities/default.json`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `ping() -> &'static str`（Tauri command，返回 `"pong"`）；React 默认导出 `App`。

- [ ] **Step 1: 创建 `package.json` 并写前端失败测试**

先按 Step 3 的内容创建 `package.json`；同时创建 `vite.config.ts` 与 `src/test/setup.ts`（测试需要 jsdom 与 jest-dom）。其余配置稍后再建。

`src/__tests__/App.test.tsx`：

```tsx
import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "../App";

test("renders app title", () => {
  render(<App />);
  expect(screen.getByRole("heading", { name: "CC Switch X" })).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm install && pnpm test`
Expected: FAIL，提示 `App` 模块不存在或找不到标题。

- [ ] **Step 3: 创建其余工程配置与最小 App**

`package.json` 已在 Step 1 创建，内容如下（若 Step 1 没建，按此补上）：

```json
{
  "name": "cc-switch-x",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@testing-library/jest-dom": "^6.6.0",
    "@testing-library/react": "^16.1.0",
    "@types/react": "^18.3.12",
    "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.4",
    "autoprefixer": "^10.4.20",
    "jsdom": "^25.0.1",
    "postcss": "^8.4.49",
    "tailwindcss": "^3.4.17",
    "typescript": "^5.7.2",
    "vite": "^6.0.3",
    "vitest": "^3.0.0"
  }
}
```

`vite.config.ts`：

```ts
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  test: { environment: "jsdom", setupFiles: "./src/test/setup.ts" },
});
```

`tsconfig.json`：

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noEmit": true,
    "skipLibCheck": true,
    "types": ["vitest/globals", "@testing-library/jest-dom"]
  },
  "include": ["src", "vite.config.ts"]
}
```

`tsconfig.node.json`：

```json
{
  "compilerOptions": {
    "composite": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true
  },
  "include": ["vite.config.ts"]
}
```

`tailwind.config.cjs`：

```js
/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: { extend: {} },
  plugins: [],
};
```

`postcss.config.cjs`：

```js
module.exports = { plugins: { tailwindcss: {}, autoprefixer: {} } };
```

`index.html`：

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>CC Switch X</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

`src/styles.css`：

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

html, body, #root { height: 100%; }
body { margin: 0; font-family: system-ui, "Microsoft YaHei", sans-serif; }
```

`src/test/setup.ts`：

```ts
import "@testing-library/jest-dom/vitest";
```

`src/main.tsx`：

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
```

`src/App.tsx`：

```tsx
export default function App() {
  return <h1 className="p-6 text-2xl font-semibold">CC Switch X</h1>;
}
```

- [ ] **Step 4: 创建 Tauri 配置与 `ping`**

`src-tauri/Cargo.toml`：

```toml
[package]
name = "cc-switch-x"
version = "0.1.0"
edition = "2021"

[lib]
name = "cc_switch_x_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
rusqlite = { version = "0.32", features = ["bundled"] }
sha2 = "0.10"
thiserror = "2"
dirs = "6"
chrono = { version = "0.4", features = ["serde"] }
tempfile = "3"
```

`src-tauri/build.rs`：

```rust
fn main() {
    tauri_build::build()
}
```

`src-tauri/tauri.conf.json`：

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "CC Switch X",
  "version": "0.1.0",
  "identifier": "com.ccswitchx.app",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [{ "title": "CC Switch X", "width": 1200, "height": 800 }],
    "security": { "csp": null }
  },
  "bundle": { "active": false }
}
```

`src-tauri/capabilities/default.json`：

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability",
  "windows": ["main"],
  "permissions": ["core:default"]
}
```

`src-tauri/src/main.rs`：

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    cc_switch_x_lib::run()
}
```

`src-tauri/src/lib.rs`：

```rust
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ping])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn ping_returns_pong() {
        assert_eq!(super::ping(), "pong");
    }
}
```

- [ ] **Step 5: 运行前后端测试**

Run: `pnpm test`
Expected: PASS，`renders app title` 通过。

Run: `pnpm build`
Expected: 构建成功（`cargo test` 里的 `generate_context!` 需要 `dist` 存在）。

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS，`ping_returns_pong` 通过。

- [ ] **Step 6: 构建与人工启动**

Run: `pnpm build`
Expected: 构建成功。

Run: `pnpm tauri dev`
Expected: 桌面窗口打开，显示「CC Switch X」。确认后关闭窗口。

- [ ] **Step 7: 初始化 Git 并提交**

`.gitignore`：

```
node_modules/
dist/
src-tauri/target/
references/
index.db
index.db-*
secrets.local.json
local/
backups/
logs/
state/
```

Run: `git init && git add . && git commit -m "chore: 初始化 CC Switch X 工程骨架"`

---

### Task 2: 六类资产的领域模型

**Files:**
- Create: `src-tauri/src/domain/mod.rs`, `asset.rs`, `agent.rs`, `frontmatter.rs`, `mcp.rs`, `prompt.rs`, `rule.rs`, `subagent.rs`, `hook.rs`
- Modify: `src-tauri/src/lib.rs`（加入 `pub mod domain;`）

**Interfaces:**
- Produces:
  - `AssetType::{Mcp,Skill,Prompt,Rule,Subagent,Hook}`，`AssetType::ALL`，`AssetType::dir_name() -> &'static str`，`AssetType::file_extension() -> Option<&'static str>`。
  - `AssetSummary { asset_type: AssetType, id: String, name: String, updated_at: i64, hash: String }`（serde camelCase）。
  - `domain::parse_and_validate(asset_type, id, bytes) -> Result<(), DomainError>`。
  - `domain::display_name(asset_type, id, bytes) -> Result<String, DomainError>`。
  - `AgentDescriptor { id, display_name, config_root, supports: AgentCapabilities }`，`agent::builtin_agents() -> Vec<AgentDescriptor>`。

- [ ] **Step 1: 写失败测试**

`src-tauri/src/domain/mod.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_type_dirs_are_stable() {
        assert_eq!(AssetType::Mcp.dir_name(), "mcp");
        assert_eq!(AssetType::Skill.dir_name(), "skills");
        assert_eq!(AssetType::Prompt.file_extension(), Some("md"));
    }

    #[test]
    fn parses_mcp_and_rejects_missing_command() {
        let ok = br#"{"id":"pw","name":"playwright","transport":"stdio","command":"npx","args":[]}"#;
        assert!(parse_and_validate(AssetType::Mcp, "pw", ok).is_ok());

        let bad = br#"{"id":"pw","name":"playwright","transport":"stdio"}"#;
        assert_eq!(
            parse_and_validate(AssetType::Mcp, "pw", bad),
            Err(DomainError::McpCommandRequired)
        );
    }

    #[test]
    fn parses_prompt_with_bom_and_crlf() {
        let raw = "\u{feff}---\r\ntitle: 团队提示词\r\n---\r\n\r\n正文\r\n";
        let prompt = prompt::parse_prompt("team", raw).unwrap();
        assert_eq!(prompt.meta.title.as_deref(), Some("团队提示词"));
        assert_eq!(prompt.body, "正文\n");
    }
}
```

（`Skills_dir_for_test` 只是占位断言；实现时直接断言 `AssetType::Skill.dir_name() == "skills"`，不要新增测试专用方法。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml domain`
Expected: FAIL，`AssetType` / `parse_and_validate` 未定义。

- [ ] **Step 3: 实现类型与解析**

创建 `domain/asset.rs`：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Mcp,
    Skill,
    Prompt,
    Rule,
    Subagent,
    Hook,
}

impl AssetType {
    pub const ALL: [AssetType; 6] = [
        Self::Mcp, Self::Skill, Self::Prompt, Self::Rule, Self::Subagent, Self::Hook,
    ];

    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skill => "skills",
            Self::Prompt => "prompts",
            Self::Rule => "rules",
            Self::Subagent => "subagents",
            Self::Hook => "hooks",
        }
    }

    pub fn file_extension(self) -> Option<&'static str> {
        match self {
            Self::Mcp => Some("json"),
            Self::Skill => None,
            Self::Prompt | Self::Rule => Some("md"),
            Self::Subagent | Self::Hook => Some("yaml"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
    pub asset_type: AssetType,
    pub id: String,
    pub name: String,
    pub updated_at: i64,
    pub hash: String,
}
```

`domain/mod.rs`：

```rust
pub mod agent;
pub mod asset;
pub mod frontmatter;
pub mod hook;
pub mod mcp;
pub mod prompt;
pub mod rule;
pub mod subagent;

pub use asset::{AssetSummary, AssetType};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("id 不能为空")]
    EmptyId,
    #[error("name 不能为空")]
    EmptyName,
    #[error("MCP stdio 必须提供 command")]
    McpCommandRequired,
    #[error("MCP 非 stdio 必须提供 url")]
    McpUrlRequired,
    #[error("MCP id 与文件名不一致：{0} != {1}")]
    IdMismatch(String, String),
    #[error("frontmatter 未闭合")]
    UnclosedFrontmatter,
    #[error("YAML 解析失败：{0}")]
    Yaml(String),
    #[error("JSON 解析失败：{0}")]
    Json(String),
}

pub fn parse_and_validate(
    asset_type: AssetType,
    id: &str,
    bytes: &[u8],
) -> Result<(), DomainError> {
    if id.trim().is_empty() {
        return Err(DomainError::EmptyId);
    }
    match asset_type {
        AssetType::Mcp => {
            let server: mcp::McpServer = serde_json::from_slice(bytes)
                .map_err(|e| DomainError::Json(e.to_string()))?;
            if server.id != id {
                return Err(DomainError::IdMismatch(server.id, id.to_string()));
            }
            server.validate()
        }
        AssetType::Prompt => {
            prompt::parse_prompt(id, std::str::from_utf8(bytes).unwrap_or(""))?;
            Ok(())
        }
        AssetType::Rule => {
            rule::parse_rule(id, std::str::from_utf8(bytes).unwrap_or(""))?;
            Ok(())
        }
        AssetType::Subagent => {
            subagent::parse_subagent(std::str::from_utf8(bytes).unwrap_or(""))?;
            Ok(())
        }
        AssetType::Hook => {
            hook::parse_hook(std::str::from_utf8(bytes).unwrap_or(""))?;
            Ok(())
        }
        AssetType::Skill => Ok(()),
    }
}

pub fn display_name(
    asset_type: AssetType,
    id: &str,
    bytes: &[u8],
) -> Result<String, DomainError> {
    match asset_type {
        AssetType::Mcp => {
            let server: mcp::McpServer = serde_json::from_slice(bytes)
                .map_err(|e| DomainError::Json(e.to_string()))?;
            Ok(server.name)
        }
        AssetType::Prompt => Ok(prompt::parse_prompt(id, std::str::from_utf8(bytes).unwrap_or(""))?
            .meta
            .title
            .unwrap_or_else(|| id.to_string())),
        AssetType::Rule => Ok(rule::parse_rule(id, std::str::from_utf8(bytes).unwrap_or(""))?
            .meta
            .name),
        AssetType::Subagent => Ok(subagent::parse_subagent(
            std::str::from_utf8(bytes).unwrap_or(""),
        )?
        .name),
        AssetType::Hook => Ok(hook::parse_hook(std::str::from_utf8(bytes).unwrap_or(""))?.name),
        AssetType::Skill => Ok(id.to_string()),
    }
}
```

创建 `domain/frontmatter.rs`：

```rust
use super::DomainError;

pub struct Frontmatter {
    pub yaml: Option<String>,
    pub body: String,
}

/// 支持 UTF-8 BOM 与 CRLF；返回的 body 统一为 LF。
pub fn split_frontmatter(raw: &str) -> Result<Frontmatter, DomainError> {
    let raw = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    if !raw.starts_with("---\n") {
        return Ok(Frontmatter { yaml: None, body: raw });
    }
    let rest = &raw[4..];
    let Some(end) = rest.find("\n---") else {
        return Err(DomainError::UnclosedFrontmatter);
    };
    let yaml = rest[..end].to_string();
    let body = rest[end + 4..].trim_start_matches('\n');
    Ok(Frontmatter { yaml: Some(yaml), body: body.to_string() })
}
```

创建 `domain/mcp.rs`：

```rust
use super::DomainError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport { Stdio, Http, Sse, Ws }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl McpServer {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id.trim().is_empty() { return Err(DomainError::EmptyId); }
        if self.name.trim().is_empty() { return Err(DomainError::EmptyName); }
        match self.transport {
            Transport::Stdio => {
                if self.command.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(DomainError::McpCommandRequired);
                }
            }
            _ => {
                if self.url.as_deref().unwrap_or("").trim().is_empty() {
                    return Err(DomainError::McpUrlRequired);
                }
            }
        }
        Ok(())
    }
}
```

创建 `domain/prompt.rs`：

```rust
use super::frontmatter::split_frontmatter;
use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PromptMetadata {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub id: String,
    pub meta: PromptMetadata,
    pub body: String,
}

pub fn parse_prompt(id: &str, raw: &str) -> Result<Prompt, DomainError> {
    if id.trim().is_empty() { return Err(DomainError::EmptyId); }
    let parts = split_frontmatter(raw)?;
    let meta = match parts.yaml {
        Some(yaml) => serde_yaml::from_str(&yaml).map_err(|e| DomainError::Yaml(e.to_string()))?,
        None => PromptMetadata::default(),
    };
    Ok(Prompt { id: id.to_string(), meta, body: parts.body })
}

pub fn render_prompt(prompt: &Prompt) -> String {
    let yaml = serde_yaml::to_string(&prompt.meta).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, prompt.body)
}
```

创建 `domain/rule.rs`：

```rust
use super::frontmatter::split_frontmatter;
use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleFrontmatter {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub meta: RuleFrontmatter,
    pub body: String,
}

pub fn parse_rule(id: &str, raw: &str) -> Result<Rule, DomainError> {
    if id.trim().is_empty() { return Err(DomainError::EmptyId); }
    let parts = split_frontmatter(raw)?;
    let Some(yaml) = parts.yaml else {
        return Err(DomainError::Yaml("rule 缺少 frontmatter".into()));
    };
    let meta: RuleFrontmatter =
        serde_yaml::from_str(&yaml).map_err(|e| DomainError::Yaml(e.to_string()))?;
    Ok(Rule { id: id.to_string(), meta, body: parts.body })
}

pub fn render_rule(rule: &Rule) -> String {
    let yaml = serde_yaml::to_string(&rule.meta).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, rule.body)
}
```

创建 `domain/subagent.rs`：

```rust
use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subagent {
    pub name: String,
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tool_extras: serde_json::Value,
    #[serde(default)]
    pub targets: Vec<String>,
}

pub fn parse_subagent(raw: &str) -> Result<Subagent, DomainError> {
    let parsed: Subagent =
        serde_yaml::from_str(raw).map_err(|e| DomainError::Yaml(e.to_string()))?;
    if parsed.name.trim().is_empty() { return Err(DomainError::EmptyName); }
    Ok(parsed)
}
```

创建 `domain/hook.rs`：

```rust
use super::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    pub name: String,
    pub event: String,
    #[serde(default)]
    pub matcher: String,
    pub command: String,
    #[serde(default)]
    pub command_windows: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

pub fn parse_hook(raw: &str) -> Result<Hook, DomainError> {
    let parsed: Hook =
        serde_yaml::from_str(raw).map_err(|e| DomainError::Yaml(e.to_string()))?;
    if parsed.name.trim().is_empty() { return Err(DomainError::EmptyName); }
    if parsed.event.trim().is_empty() { return Err(DomainError::EmptyName); }
    if parsed.command.trim().is_empty() { return Err(DomainError::EmptyName); }
    Ok(parsed)
}
```

创建 `domain/agent.rs`：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub mcp: bool,
    pub skills: bool,
    pub prompts: bool,
    pub rules: bool,
    pub subagents: bool,
    pub hooks: bool,
    pub sessions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub display_name: String,
    pub config_root: String,
    pub supports: AgentCapabilities,
}

pub fn builtin_agents() -> Vec<AgentDescriptor> {
    let all = |sessions: bool| AgentCapabilities {
        mcp: true, skills: true, prompts: true, rules: true,
        subagents: true, hooks: true, sessions,
    };
    vec![
        AgentDescriptor {
            id: "qoder-cn".into(),
            display_name: "Qoder CN".into(),
            config_root: "~/.qoder-cn".into(),
            supports: all(true),
        },
        AgentDescriptor {
            id: "qoder".into(),
            display_name: "Qoder".into(),
            config_root: "~/.qoder".into(),
            supports: all(false),
        },
    ]
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml domain`
Expected: PASS。

- [ ] **Step 5: 提交**

Run: `git add src-tauri/src/domain src-tauri/src/lib.rs && git commit -m "feat: 添加六类资产领域模型与校验"`

---

### Task 3: 文本真源与原子写

**Files:**
- Create: `src-tauri/src/store/mod.rs`, `src-tauri/src/store/atomic.rs`
- Modify: `src-tauri/src/lib.rs`（加入 `pub mod store;`）

**Interfaces:**
- Produces:
  - `Store::init(root: PathBuf) -> Result<Self, StoreError>`
  - `Store::root(&self) -> &Path`
  - `Store::asset_dir(&self, AssetType) -> PathBuf`
  - `Store::entry_path(&self, AssetType, id: &str) -> Result<PathBuf, StoreError>`
  - `Store::write(&self, AssetType, id: &str, bytes: &[u8]) -> Result<(), StoreError>`
  - `Store::read(&self, AssetType, id: &str) -> Result<Vec<u8>, StoreError>`
  - `Store::delete(&self, AssetType, id: &str) -> Result<bool, StoreError>`
  - `Store::list_ids(&self, AssetType) -> Result<Vec<String>, StoreError>`
  - `Store::backup(&self, AssetType, id: &str) -> Result<Option<PathBuf>, StoreError>`

- [ ] **Step 1: 写失败测试**

`src-tauri/src/store/mod.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use tempfile::tempdir;

    #[test]
    fn init_creates_layout_and_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        for name in ["mcp", "skills", "prompts", "rules", "subagents", "hooks"] {
            assert!(store.root().join("assets").join(name).is_dir());
        }
        let ignore = std::fs::read_to_string(store.root().join(".gitignore")).unwrap();
        assert!(ignore.contains("node_modules/"));
        assert!(ignore.contains("index.db"));
        assert!(ignore.contains("local/"));
        assert!(ignore.contains("secrets.local.json"));
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        store.write(AssetType::Mcp, "pw", b"{\"id\":\"pw\"}").unwrap();
        assert_eq!(store.read(AssetType::Mcp, "pw").unwrap(), b"{\"id\":\"pw\"}");
        assert_eq!(store.list_ids(AssetType::Mcp).unwrap(), vec!["pw"]);
        assert!(store.delete(AssetType::Mcp, "pw").unwrap());
        assert!(!store.read(AssetType::Mcp, "pw").is_ok());
    }

    #[test]
    fn rejects_unsafe_ids() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        for id in ["../evil", "a/b", "", ".."] {
            assert!(store.entry_path(AssetType::Mcp, id).is_err(), "id={id}");
        }
    }

    #[test]
    fn overwrite_replaces_content_without_temp_files() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        store.write(AssetType::Mcp, "pw", b"one").unwrap();
        store.write(AssetType::Mcp, "pw", b"two").unwrap();
        assert_eq!(store.read(AssetType::Mcp, "pw").unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(store.asset_dir(AssetType::Mcp))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn backup_preserves_previous_content() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().to_path_buf()).unwrap();
        store.write(AssetType::Mcp, "pw", b"one").unwrap();
        let backup = store.backup(AssetType::Mcp, "pw").unwrap().unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), b"one");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store`
Expected: FAIL，`Store` 未定义。

- [ ] **Step 3: 实现 Store 与原子写**

`store/atomic.rs`：

```rust
use std::fs;
use std::io::Write;
use std::path::Path;

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
```

`store/mod.rs`：

```rust
pub mod atomic;

use crate::domain::AssetType;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("非法资产 ID：{0}")]
    InvalidId(String),
    #[error("暂不支持该资产类型的文件写入：{0:?}")]
    Unsupported(AssetType),
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn init(root: PathBuf) -> Result<Self, StoreError> {
        for name in ["mcp", "skills", "prompts", "rules", "subagents", "hooks"] {
            fs::create_dir_all(root.join("assets").join(name))?;
        }
        for name in ["agents", "local/agents", "state", "backups", "logs"] {
            fs::create_dir_all(root.join(name))?;
        }
        let ignore = root.join(".gitignore");
        let required = [
            "index.db", "index.db-*", "secrets.local.json", "local/", "backups/", "logs/", "state/",
        ];
        let existing = fs::read_to_string(&ignore).unwrap_or_default();
        let mut content = existing.clone();
        for line in required {
            if !existing.lines().any(|l| l.trim() == line) {
                if !content.is_empty() && !content.ends_with('\n') { content.push('\n'); }
                content.push_str(line);
                content.push('\n');
            }
        }
        fs::write(&ignore, content)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path { &self.root }

    pub fn asset_dir(&self, asset_type: AssetType) -> PathBuf {
        self.root.join("assets").join(asset_type.dir_name())
    }

    pub fn entry_path(&self, asset_type: AssetType, id: &str) -> Result<PathBuf, StoreError> {
        if !is_safe_id(id) {
            return Err(StoreError::InvalidId(id.to_string()));
        }
        let base = self.asset_dir(asset_type);
        Ok(match asset_type.file_extension() {
            Some(ext) => base.join(format!("{id}.{ext}")),
            None => base.join(id),
        })
    }

    pub fn write(&self, asset_type: AssetType, id: &str, bytes: &[u8]) -> Result<(), StoreError> {
        if asset_type == AssetType::Skill {
            return Err(StoreError::Unsupported(asset_type));
        }
        let path = self.entry_path(asset_type, id)?;
        atomic::write_atomic(&path, bytes)?;
        Ok(())
    }

    pub fn read(&self, asset_type: AssetType, id: &str) -> Result<Vec<u8>, StoreError> {
        let path = self.entry_path(asset_type, id)?;
        Ok(fs::read(path)?)
    }

    pub fn delete(&self, asset_type: AssetType, id: &str) -> Result<bool, StoreError> {
        let path = self.entry_path(asset_type, id)?;
        if !path.exists() { return Ok(false); }
        if path.is_dir() { fs::remove_dir_all(path)?; } else { fs::remove_file(path)?; }
        Ok(true)
    }

    pub fn list_ids(&self, asset_type: AssetType) -> Result<Vec<String>, StoreError> {
        let dir = self.asset_dir(asset_type);
        let mut ids = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().to_string();
            let id = match asset_type.file_extension() {
                Some(ext) => {
                    if !file_type.is_file() { continue; }
                    match name.strip_suffix(&format!(".{ext}")) {
                        Some(stem) => stem.to_string(),
                        None => continue,
                    }
                }
                None => {
                    if !file_type.is_dir() { continue; }
                    name
                }
            };
            ids.push(id);
        }
        ids.sort();
        Ok(ids)
    }

    pub fn backup(&self, asset_type: AssetType, id: &str) -> Result<Option<PathBuf>, StoreError> {
        let source = self.entry_path(asset_type, id)?;
        if !source.exists() { return Ok(None); }
        let stamp = chrono::Utc::now().timestamp_millis();
        let target = self
            .root
            .join("backups")
            .join(stamp.to_string())
            .join(asset_type.dir_name())
            .join(source.file_name().unwrap_or_default());
        if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
        if source.is_dir() { copy_dir(&source, &target)?; } else { fs::copy(&source, &target)?; }
        Ok(Some(target))
    }
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() { copy_dir(&entry.path(), &target)?; }
        else { fs::copy(entry.path(), target)?; }
    }
    Ok(())
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml store`
Expected: PASS。

- [ ] **Step 5: 提交**

Run: `git add src-tauri/src/store src-tauri/src/lib.rs && git commit -m "feat: 添加文本真源存储与原子写"`

---

### Task 4: SQLite 派生索引

**Files:**
- Create: `src-tauri/src/index/mod.rs`, `src-tauri/src/index/schema.rs`
- Modify: `src-tauri/src/lib.rs`（加入 `pub mod index;`）

**Interfaces:**
- Produces:
  - `Index::open(path: &Path) -> Result<Self, IndexError>`
  - `Index::rebuild(&mut self, store: &Store) -> Result<(), IndexError>`
  - `Index::list(&self, Option<AssetType>) -> Result<Vec<AssetSummary>, IndexError>`
  - `Index::search(&self, query: &str) -> Result<Vec<AssetSummary>, IndexError>`
  - `Index::upsert(&mut self, &AssetSummary) -> Result<(), IndexError>`
  - `Index::remove(&mut self, AssetType, id: &str) -> Result<(), IndexError>`
  - `index::sha256_hex(bytes: &[u8]) -> String`

- [ ] **Step 1: 写失败测试**

`src-tauri/src/index/mod.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::store::Store;
    use tempfile::tempdir;

    #[test]
    fn rebuild_indexes_files_and_removes_deleted_rows() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        store.write(AssetType::Mcp, "pw", br#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#).unwrap();
        store.write(AssetType::Mcp, "ctx", br#"{"id":"ctx","name":"Context7","transport":"http","url":"https://x"}"#).unwrap();

        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        let all = index.list(Some(AssetType::Mcp)).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|a| a.name == "Playwright"));

        store.delete(AssetType::Mcp, "pw").unwrap();
        index.rebuild(&store).unwrap();
        let all = index.list(Some(AssetType::Mcp)).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "ctx");
    }

    #[test]
    fn search_matches_name_and_id() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        store.write(AssetType::Mcp, "pw", br#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#).unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        assert_eq!(index.search("play").unwrap().len(), 1);
        assert_eq!(index.search("pw").unwrap().len(), 1);
    }

    #[test]
    fn rebuild_indexes_skill_directories() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let skill_dir = store.asset_dir(AssetType::Skill).join("lark-wiki");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Lark Wiki\n").unwrap();

        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        let skills = index.list(Some(AssetType::Skill)).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "lark-wiki");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml index`
Expected: FAIL，`Index` 未定义。

- [ ] **Step 3: 实现索引**

`index/schema.rs`：

```rust
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS assets (
    asset_type TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    hash TEXT NOT NULL,
    PRIMARY KEY (asset_type, id)
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;
```

`index/mod.rs`：

```rust
mod schema;

use crate::domain::{display_name, AssetSummary, AssetType};
use crate::store::Store;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("数据库错误：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("存储错误：{0}")]
    Store(#[from] crate::store::StoreError),
    #[error("资产解析失败：{0}")]
    Domain(#[from] crate::domain::DomainError),
}

pub struct Index { conn: Connection }

impl Index {
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let conn = Connection::open(path)?;
        conn.execute_batch(schema::SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn rebuild(&mut self, store: &Store) -> Result<(), IndexError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM assets", [])?;
        for asset_type in AssetType::ALL {
            for id in store.list_ids(asset_type)? {
                let bytes = match asset_type {
                    AssetType::Skill => {
                        let Ok(dir) = store.entry_path(asset_type, &id) else { continue };
                        std::fs::read(dir.join("SKILL.md")).unwrap_or_default()
                    }
                    _ => match store.read(asset_type, &id) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    },
                };
                let name = match display_name(asset_type, &id, &bytes) {
                    Ok(name) => name,
                    Err(_) => continue,
                };
                let hash = sha256_hex(&bytes);
                tx.execute(
                    "INSERT INTO assets (asset_type,id,name,updated_at,hash) VALUES (?1,?2,?3,?4,?5)",
                    params![asset_type_key(asset_type), id, name, now_millis(), hash],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list(&self, asset_type: Option<AssetType>) -> Result<Vec<AssetSummary>, IndexError> {
        let mut rows = Vec::new();
        match asset_type {
            Some(t) => {
                let mut stmt = self.conn.prepare(
                    "SELECT asset_type,id,name,updated_at,hash FROM assets WHERE asset_type=?1 ORDER BY name",
                )?;
                let iter = stmt.query_map([asset_type_key(t)], row_to_summary)?;
                for row in iter { rows.push(row?); }
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT asset_type,id,name,updated_at,hash FROM assets ORDER BY asset_type,name",
                )?;
                let iter = stmt.query_map([], row_to_summary)?;
                for row in iter { rows.push(row?); }
            }
        }
        Ok(rows)
    }

    pub fn search(&self, query: &str) -> Result<Vec<AssetSummary>, IndexError> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT asset_type,id,name,updated_at,hash FROM assets
             WHERE name LIKE ?1 OR id LIKE ?1 ORDER BY name",
        )?;
        let iter = stmt.query_map([pattern], row_to_summary)?;
        let mut rows = Vec::new();
        for row in iter { rows.push(row?); }
        Ok(rows)
    }

    pub fn upsert(&mut self, summary: &AssetSummary) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO assets (asset_type,id,name,updated_at,hash) VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(asset_type,id) DO UPDATE SET name=?3,updated_at=?4,hash=?5",
            params![
                asset_type_key(summary.asset_type),
                summary.id,
                summary.name,
                summary.updated_at,
                summary.hash
            ],
        )?;
        Ok(())
    }

    pub fn remove(&mut self, asset_type: AssetType, id: &str) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM assets WHERE asset_type=?1 AND id=?2",
            params![asset_type_key(asset_type), id],
        )?;
        Ok(())
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn now_millis() -> i64 { chrono::Utc::now().timestamp_millis() }

fn asset_type_key(asset_type: AssetType) -> &'static str {
    match asset_type {
        AssetType::Mcp => "mcp",
        AssetType::Skill => "skill",
        AssetType::Prompt => "prompt",
        AssetType::Rule => "rule",
        AssetType::Subagent => "subagent",
        AssetType::Hook => "hook",
    }
}

fn asset_type_from_key(key: &str) -> AssetType {
    match key {
        "mcp" => AssetType::Mcp,
        "skill" => AssetType::Skill,
        "prompt" => AssetType::Prompt,
        "rule" => AssetType::Rule,
        "subagent" => AssetType::Subagent,
        "hook" => AssetType::Hook,
        _ => AssetType::Mcp,
    }
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetSummary> {
    Ok(AssetSummary {
        asset_type: asset_type_from_key(&row.get::<_, String>(0)?),
        id: row.get(1)?,
        name: row.get(2)?,
        updated_at: row.get(3)?,
        hash: row.get(4)?,
    })
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml index`
Expected: PASS。

- [ ] **Step 5: 提交**

Run: `git add src-tauri/src/index src-tauri/src/lib.rs && git commit -m "feat: 添加可重建的 SQLite 索引"`

---

### Task 5: 本机应用设置

**Files:**
- Create: `src-tauri/src/settings/mod.rs`
- Modify: `src-tauri/src/lib.rs`（加入 `pub mod settings;`）

**Interfaces:**
- Produces:
  - `AppSettings { store_root: PathBuf, theme: Theme }`（serde camelCase）
  - `Theme::{Light,Dark,System}`
  - `settings::load() / save(&AppSettings)`
  - `settings::load_from(path: &Path) / save_to(path, &AppSettings)`
  - `settings::settings_path() -> Result<PathBuf, SettingsError>`

- [ ] **Step 1: 写失败测试**

`src-tauri/src/settings/mod.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let settings = load_from(&dir.path().join("settings.json")).unwrap();
        assert_eq!(settings.theme, Theme::System);
        assert!(settings.store_root.ends_with(".ccswitchx"));
    }

    #[test]
    fn roundtrip_preserves_values() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let settings = AppSettings { store_root: dir.path().join("data"), theme: Theme::Dark };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path).unwrap(), settings);
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_from(&path).is_err());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings`
Expected: FAIL，`AppSettings` 未定义。

- [ ] **Step 3: 实现设置**

```rust
use crate::store::atomic::write_atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub store_root: PathBuf,
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme { Light, Dark, System }

impl Default for AppSettings {
    fn default() -> Self {
        Self { store_root: default_store_root(), theme: Theme::System }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

pub fn default_store_root() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".ccswitchx")
}

pub fn settings_path() -> Result<PathBuf, SettingsError> {
    let base = dirs::config_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir"))?;
    Ok(base.join("cc-switch-x").join("settings.json"))
}

pub fn load() -> Result<AppSettings, SettingsError> {
    load_from(&settings_path()?)
}

pub fn load_from(path: &Path) -> Result<AppSettings, SettingsError> {
    if !path.exists() { return Ok(AppSettings::default()); }
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn save(settings: &AppSettings) -> Result<(), SettingsError> {
    save_to(&settings_path()?, settings)
}

pub fn save_to(path: &Path, settings: &AppSettings) -> Result<(), SettingsError> {
    let bytes = serde_json::to_vec_pretty(settings)?;
    write_atomic(path, &bytes)?;
    Ok(())
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings`
Expected: PASS。

- [ ] **Step 5: 提交**

Run: `git add src-tauri/src/settings src-tauri/src/lib.rs && git commit -m "feat: 添加本机应用设置"`

---

### Task 6: 服务层与 Tauri 命令

**Files:**
- Create: `src-tauri/src/error.rs`, `src-tauri/src/state.rs`
- Create: `src-tauri/src/services/mod.rs`, `src-tauri/src/services/assets.rs`, `src-tauri/src/services/settings.rs`
- Create: `src-tauri/src/commands/mod.rs`, `src-tauri/src/commands/assets.rs`, `src-tauri/src/commands/settings.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces:
  - `AssetService::list(&Store, &Index, Option<AssetType>) -> Result<Vec<AssetSummary>, AppError>`
  - `AssetService::read(&Store, AssetType, &str) -> Result<String, AppError>`
  - `AssetService::write(&Store, &mut Index, AssetType, &str, &str) -> Result<AssetSummary, AppError>`
  - `AssetService::delete(&Store, &mut Index, AssetType, &str) -> Result<bool, AppError>`
  - Tauri commands: `list_assets`、`read_asset`、`write_asset`、`delete_asset`、`get_settings`、`set_theme`、`set_store_root`。

- [ ] **Step 1: 写失败测试**

`src-tauri/src/services/assets.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::index::Index;
    use crate::store::Store;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, Store, Index) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        (dir, store, index)
    }

    #[test]
    fn write_then_list_returns_summary() {
        let (_dir, store, mut index) = fixture();
        let summary = AssetService::write(
            &store, &mut index, AssetType::Mcp, "pw",
            r#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
        ).unwrap();
        assert_eq!(summary.name, "Playwright");
        assert_eq!(AssetService::list(&store, &index, Some(AssetType::Mcp)).unwrap().len(), 1);
    }

    #[test]
    fn invalid_content_is_rejected_before_write() {
        let (_dir, store, mut index) = fixture();
        let result = AssetService::write(&store, &mut index, AssetType::Mcp, "pw", "{}");
        assert!(result.is_err());
        assert!(store.list_ids(AssetType::Mcp).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_file_and_index_row() {
        let (_dir, store, mut index) = fixture();
        AssetService::write(
            &store, &mut index, AssetType::Mcp, "pw",
            r#"{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}"#,
        ).unwrap();
        assert!(AssetService::delete(&store, &mut index, AssetType::Mcp, "pw").unwrap());
        assert!(AssetService::list(&store, &index, Some(AssetType::Mcp)).unwrap().is_empty());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml services`
Expected: FAIL，`AssetService` 未定义。

- [ ] **Step 3: 实现错误、状态、服务与命令**

`error.rs`：

```rust
use crate::domain::DomainError;
use crate::index::IndexError;
use crate::settings::SettingsError;
use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("存储错误：{0}")]
    Store(#[from] StoreError),
    #[error("索引错误：{0}")]
    Index(#[from] IndexError),
    #[error("资产错误：{0}")]
    Domain(#[from] DomainError),
    #[error("设置错误：{0}")]
    Settings(#[from] SettingsError),
    #[error("未找到：{0}")]
    NotFound(String),
    #[error("非法输入：{0}")]
    InvalidInput(String),
    #[error("暂不支持：{0}")]
    Unsupported(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
```

`services/assets.rs`：

```rust
use crate::domain::{display_name, parse_and_validate, AssetSummary, AssetType};
use crate::error::AppError;
use crate::index::{sha256_hex, Index};
use crate::store::Store;

pub struct AssetService;

impl AssetService {
    pub fn list(
        _store: &Store,
        index: &Index,
        asset_type: Option<AssetType>,
    ) -> Result<Vec<AssetSummary>, AppError> {
        Ok(index.list(asset_type)?)
    }

    pub fn read(store: &Store, asset_type: AssetType, id: &str) -> Result<String, AppError> {
        let bytes = store.read(asset_type, id)?;
        String::from_utf8(bytes)
            .map_err(|_| AppError::InvalidInput(format!("{id} 不是 UTF-8 文件")))
    }

    pub fn write(
        store: &Store,
        index: &mut Index,
        asset_type: AssetType,
        id: &str,
        content: &str,
    ) -> Result<AssetSummary, AppError> {
        let bytes = content.as_bytes();
        parse_and_validate(asset_type, id, bytes)?;
        store.backup(asset_type, id)?;
        store.write(asset_type, id, bytes)?;
        let name = display_name(asset_type, id, bytes)?;
        let summary = AssetSummary {
            asset_type,
            id: id.to_string(),
            name,
            updated_at: chrono::Utc::now().timestamp_millis(),
            hash: sha256_hex(bytes),
        };
        index.upsert(&summary)?;
        Ok(summary)
    }

    pub fn delete(
        store: &Store,
        index: &mut Index,
        asset_type: AssetType,
        id: &str,
    ) -> Result<bool, AppError> {
        let deleted = store.delete(asset_type, id)?;
        if deleted { index.remove(asset_type, id)?; }
        Ok(deleted)
    }
}
```

`services/settings.rs`：

```rust
use crate::error::AppError;
use crate::settings::{self, AppSettings, Theme};

pub fn get() -> Result<AppSettings, AppError> {
    Ok(settings::load()?)
}

pub fn set_theme(theme: Theme) -> Result<AppSettings, AppError> {
    let mut current = settings::load()?;
    current.theme = theme;
    settings::save(&current)?;
    Ok(current)
}

pub fn set_store_root(store_root: std::path::PathBuf) -> Result<AppSettings, AppError> {
    set_store_root_at(&settings::settings_path()?, store_root)
}

pub fn set_store_root_at(
    config_path: &std::path::Path,
    store_root: std::path::PathBuf,
) -> Result<AppSettings, AppError> {
    if store_root.as_os_str().is_empty() {
        return Err(AppError::InvalidInput("store root 不能为空".into()));
    }
    std::fs::create_dir_all(&store_root)
        .map_err(|e| AppError::InvalidInput(format!("无法创建目录：{e}")))?;
    let mut current = settings::load_from(config_path)?;
    current.store_root = store_root;
    settings::save_to(config_path, &current)?;
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn set_store_root_persists_and_points_to_new_root() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("settings.json");
        let new_root = dir.path().join("moved-store");
        let updated = set_store_root_at(&config, new_root.clone()).unwrap();
        assert_eq!(updated.store_root, new_root);
        assert_eq!(settings::load_from(&config).unwrap().store_root, new_root);
    }
}
```

`services/mod.rs`：

```rust
pub mod assets;
pub mod settings;
```

`state.rs`：

```rust
use crate::error::AppError;
use crate::index::Index;
use crate::settings::AppSettings;
use crate::store::Store;
use std::sync::Mutex;

pub struct AppState {
    pub store: Store,
    pub index: Mutex<Index>,
    pub settings: Mutex<AppSettings>,
}

impl AppState {
    pub fn init(settings: AppSettings) -> Result<Self, AppError> {
        let store = Store::init(settings.store_root.clone())?;
        let mut index = Index::open(&settings.store_root.join("index.db"))?;
        index.rebuild(&store)?;
        Ok(Self { store, index: Mutex::new(index), settings: Mutex::new(settings) })
    }
}
```

`commands/assets.rs`：

```rust
use crate::domain::{AssetSummary, AssetType};
use crate::error::AppError;
use crate::services::assets::AssetService;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn list_assets(
    state: State<'_, AppState>,
    asset_type: Option<AssetType>,
) -> Result<Vec<AssetSummary>, AppError> {
    let index = state.index.lock().map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    AssetService::list(&state.store, &index, asset_type)
}

#[tauri::command]
pub fn read_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
) -> Result<String, AppError> {
    AssetService::read(&state.store, asset_type, &id)
}

#[tauri::command]
pub fn write_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
    content: String,
) -> Result<AssetSummary, AppError> {
    let mut index = state.index.lock().map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    AssetService::write(&state.store, &mut index, asset_type, &id, &content)
}

#[tauri::command]
pub fn delete_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
) -> Result<bool, AppError> {
    let mut index = state.index.lock().map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    AssetService::delete(&state.store, &mut index, asset_type, &id)
}
```

`commands/settings.rs`：

```rust
use crate::error::AppError;
use crate::settings::{AppSettings, Theme};
use crate::state::AppState;
use crate::services::settings as settings_service;
use tauri::State;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let settings = state.settings.lock().map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok(settings.clone())
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<AppSettings, AppError> {
    let updated = settings_service::set_theme(theme)?;
    let mut settings = state.settings.lock().map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(updated)
}

#[tauri::command]
pub fn set_store_root(
    state: State<'_, AppState>,
    store_root: std::path::PathBuf,
) -> Result<bool, AppError> {
    let updated = settings_service::set_store_root(store_root)?;
    let mut settings = state.settings.lock().map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated;
    Ok(true)
}
```

`commands/mod.rs`：

```rust
pub mod assets;
pub mod settings;
```

修改 `lib.rs`：加入模块声明与状态初始化，并把命令注册进去：

```rust
pub mod commands;
pub mod domain;
pub mod error;
pub mod index;
pub mod services;
pub mod settings;
pub mod state;
pub mod store;

#[tauri::command]
fn ping() -> &'static str { "pong" }

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings = settings::load().unwrap_or_default();
    let state = state::AppState::init(settings).expect("初始化应用状态失败");
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            ping,
            commands::assets::list_assets,
            commands::assets::read_asset,
            commands::assets::write_asset,
            commands::assets::delete_asset,
            commands::settings::get_settings,
            commands::settings::set_theme,
            commands::settings::set_store_root,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 4: 运行测试与构建**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS（Task 1–6 的全部测试）。

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: 编译成功。

- [ ] **Step 5: 提交**

Run: `git add src-tauri/src && git commit -m "feat: 添加资产服务与 Tauri 命令"`

---

### Task 7: 前端壳与侧栏导航

**Files:**
- Create: `src/types.ts`, `src/lib/api.ts`, `src/components/shell/AppShell.tsx`, `src/components/shell/Sidebar.tsx`
- Create: `src/panels/{McpPanel,SkillsPanel,PromptsPanel,RulesPanel,SubagentsPanel,HooksPanel,SessionsPanel,SettingsPanel}.tsx`
- Modify: `src/App.tsx`
- Modify: `src/__tests__/App.test.tsx`（替换 Task 1 的快速断言）

**Interfaces:**
- Consumes: 后端命令 `list_assets` / `read_asset` / `write_asset` / `delete_asset` / `get_settings` / `set_theme` / `set_store_root`。
- Produces:
  - `AssetType = "mcp" | "skill" | "prompt" | "rule" | "subagent" | "hook"`
  - `Pane = AssetType | "sessions" | "settings"`
  - `api.listAssets(assetType?)`, `api.readAsset(...)`, `api.writeAsset(...)`, `api.deleteAsset(...)`, `api.getSettings()`, `api.setTheme(...)`, `api.setStoreRoot(...)`

- [ ] **Step 1: 写失败测试**

`src/__tests__/App.test.tsx`：

```tsx
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "../App";

test("renders all sidebar entries and switches panes", () => {
  render(<App />);
  for (const label of ["MCP", "Skills", "Prompts", "Rules", "Subagents", "Hooks", "Sessions", "Settings"]) {
    expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
  }
  fireEvent.click(screen.getByRole("button", { name: "Rules" }));
  expect(screen.getByRole("heading", { name: "Rules" })).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test`
Expected: FAIL，找不到侧栏按钮。

- [ ] **Step 3: 实现类型、API 与壳**

`src/types.ts`：

```ts
export type AssetType = "mcp" | "skill" | "prompt" | "rule" | "subagent" | "hook";
export type Pane = AssetType | "sessions" | "settings";
export type Theme = "light" | "dark" | "system";

export interface AssetSummary {
  assetType: AssetType;
  id: string;
  name: string;
  updatedAt: number;
  hash: string;
}

export interface AppSettings {
  storeRoot: string;
  theme: Theme;
}
```

`src/lib/api.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, AssetSummary, AssetType, Theme } from "../types";

export const api = {
  listAssets: (assetType?: AssetType) =>
    invoke<AssetSummary[]>("list_assets", { assetType }),
  readAsset: (assetType: AssetType, id: string) =>
    invoke<string>("read_asset", { assetType, id }),
  writeAsset: (assetType: AssetType, id: string, content: string) =>
    invoke<AssetSummary>("write_asset", { assetType, id, content }),
  deleteAsset: (assetType: AssetType, id: string) =>
    invoke<boolean>("delete_asset", { assetType, id }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setTheme: (theme: Theme) => invoke<AppSettings>("set_theme", { theme }),
  setStoreRoot: (storeRoot: string) => invoke<boolean>("set_store_root", { storeRoot }),
};
```

`src/components/shell/Sidebar.tsx`：

```tsx
import type { Pane } from "../../types";

const ITEMS: { pane: Pane; label: string }[] = [
  { pane: "mcp", label: "MCP" },
  { pane: "skill", label: "Skills" },
  { pane: "prompt", label: "Prompts" },
  { pane: "rule", label: "Rules" },
  { pane: "subagent", label: "Subagents" },
  { pane: "hook", label: "Hooks" },
  { pane: "sessions", label: "Sessions" },
  { pane: "settings", label: "Settings" },
];

export function Sidebar({ pane, onSelect }: { pane: Pane; onSelect: (pane: Pane) => void }) {
  return (
    <nav className="w-48 border-r border-slate-200 bg-slate-50 p-3">
      <div className="mb-4 px-2 text-lg font-semibold">CC Switch X</div>
      <ul className="space-y-1">
        {ITEMS.map((item) => (
          <li key={item.pane}>
            <button
              type="button"
              onClick={() => onSelect(item.pane)}
              aria-current={pane === item.pane ? "page" : undefined}
              className={`w-full rounded px-2 py-1.5 text-left text-sm ${
                pane === item.pane ? "bg-slate-800 text-white" : "hover:bg-slate-200"
              }`}
            >
              {item.label}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}
```

`src/components/shell/AppShell.tsx`：

```tsx
import { useState } from "react";
import type { Pane } from "../../types";
import { Sidebar } from "./Sidebar";
import { HooksPanel } from "../../panels/HooksPanel";
import { McpPanel } from "../../panels/McpPanel";
import { PromptsPanel } from "../../panels/PromptsPanel";
import { RulesPanel } from "../../panels/RulesPanel";
import { SessionsPanel } from "../../panels/SessionsPanel";
import { SettingsPanel } from "../../panels/SettingsPanel";
import { SkillsPanel } from "../../panels/SkillsPanel";
import { SubagentsPanel } from "../../panels/SubagentsPanel";

export function AppShell() {
  const [pane, setPane] = useState<Pane>("mcp");
  const content = {
    mcp: <McpPanel />,
    skill: <SkillsPanel />,
    prompt: <PromptsPanel />,
    rule: <RulesPanel />,
    subagent: <SubagentsPanel />,
    hook: <HooksPanel />,
    sessions: <SessionsPanel />,
    settings: <SettingsPanel />,
  }[pane];
  return (
    <div className="flex h-full">
      <Sidebar pane={pane} onSelect={setPane} />
      <main className="flex-1 overflow-auto p-6">{content}</main>
    </div>
  );
}
```

占位面板统一形态，例如 `src/panels/RulesPanel.tsx`：

```tsx
export function RulesPanel() {
  return <h2 className="text-xl font-semibold">Rules</h2>;
}
```

`src/App.tsx`：

```tsx
import { AppShell } from "./components/shell/AppShell";

export default function App() {
  return <AppShell />;
}
```

其余占位面板（Skills / Prompts / Subagents / Hooks / Sessions）按 `RulesPanel` 的形态创建，标题分别为 Skills / Prompts / Subagents / Hooks / Sessions。`McpPanel` 与 `SettingsPanel` 也先创建占位版本，分别在 Task 8 / Task 9 实现。

- [ ] **Step 4: 运行测试**

Run: `pnpm test`
Expected: PASS。

- [ ] **Step 5: 提交**

Run: `git add src && git commit -m "feat: 添加前端壳与六面板导航"`

---

### Task 8: MCP 面板端到端

**Files:**
- Create: `src/hooks/useAssets.ts`, `src/components/JsonEditor.tsx`
- Modify: `src/panels/McpPanel.tsx`
- Create: `src/__tests__/McpPanel.test.tsx`

**Interfaces:**
- Consumes: `api.listAssets("mcp")`、`api.readAsset("mcp", id)`、`api.writeAsset("mcp", id, content)`、`api.deleteAsset("mcp", id)`。
- Produces: `useAssets(assetType) -> { items, refresh }`；`JsonEditor({ value, onChange })`。

- [ ] **Step 1: 写失败测试**

`src/__tests__/McpPanel.test.tsx`：

```tsx
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { McpPanel } from "../panels/McpPanel";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    listAssets: vi.fn(),
    readAsset: vi.fn(),
    writeAsset: vi.fn(),
    deleteAsset: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.listAssets).mockResolvedValue([
    { assetType: "mcp", id: "pw", name: "Playwright", updatedAt: 1, hash: "h" },
  ]);
  vi.mocked(api.readAsset).mockResolvedValue('{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}');
  vi.mocked(api.writeAsset).mockResolvedValue({
    assetType: "mcp", id: "pw", name: "Playwright", updatedAt: 2, hash: "h2",
  });
});

test("lists, edits and saves an MCP server", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  const editor = await screen.findByRole("textbox");
  fireEvent.change(editor, { target: { value: '{"id":"pw","name":"Playwright","transport":"stdio","command":"npx"}' } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() => expect(api.writeAsset).toHaveBeenCalledWith("mcp", "pw", expect.any(String)));
});

test("shows an error for invalid JSON", async () => {
  render(<McpPanel />);
  fireEvent.click(await screen.findByText("Playwright"));
  fireEvent.change(await screen.findByRole("textbox"), { target: { value: "{" } });
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByText("JSON 无效")).toBeInTheDocument();
  expect(api.writeAsset).not.toHaveBeenCalled();
});

test("creates a new MCP entry", async () => {
  render(<McpPanel />);
  fireEvent.change(await screen.findByLabelText("新 MCP ID"), { target: { value: "ctx" } });
  fireEvent.click(screen.getByRole("button", { name: "新建" }));
  fireEvent.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(api.writeAsset).toHaveBeenCalledWith("mcp", "ctx", expect.stringContaining('"id": "ctx"')),
  );
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test -- McpPanel`
Expected: FAIL，`McpPanel` 没有列表/编辑器。

- [ ] **Step 3: 实现 hook、编辑器与面板**

`src/hooks/useAssets.ts`：

```ts
import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { AssetSummary, AssetType } from "../types";

export function useAssets(assetType: AssetType) {
  const [items, setItems] = useState<AssetSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    try {
      setItems(await api.listAssets(assetType));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [assetType]);
  useEffect(() => { void refresh(); }, [refresh]);
  return { items, error, refresh };
}
```

`src/components/JsonEditor.tsx`：

```tsx
export function JsonEditor({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <textarea
      aria-label="JSON 编辑器"
      className="h-64 w-full rounded border border-slate-300 p-3 font-mono text-sm"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
```

`src/panels/McpPanel.tsx`：

```tsx
import { useEffect, useState } from "react";
import { JsonEditor } from "../components/JsonEditor";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";

export function McpPanel() {
  const { items, error, refresh } = useAssets("mcp");
  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [newId, setNewId] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected) { setContent(""); return; }
    if (!items.some((item) => item.id === selected)) return;
    void api.readAsset("mcp", selected).then(setContent).catch((e) => setSaveError(String(e)));
  }, [selected, items]);

  const create = () => {
    const id = newId.trim();
    if (!id) return;
    setSelected(id);
    setContent(JSON.stringify({ id, name: id, transport: "stdio", command: "", args: [] }, null, 2));
    setNewId("");
  };

  const save = async () => {
    if (!selected) return;
    try { JSON.parse(content); } catch { setSaveError("JSON 无效"); return; }
    try {
      await api.writeAsset("mcp", selected, content);
      setSaveError(null);
      await refresh();
    } catch (e) { setSaveError(String(e)); }
  };

  const remove = async () => {
    if (!selected || !window.confirm("确定删除该 MCP 服务器？")) return;
    await api.deleteAsset("mcp", selected);
    setSelected(null);
    await refresh();
  };

  return (
    <div className="grid grid-cols-[240px_1fr] gap-6">
      <section>
        <h2 className="mb-3 text-xl font-semibold">MCP</h2>
        {error && <p className="text-sm text-red-600">{error}</p>}
        <ul className="space-y-1">
          {items.map((item) => (
            <li key={item.id}>
              <button type="button" className="w-full rounded px-2 py-1 text-left hover:bg-slate-100" onClick={() => setSelected(item.id)}>
                {item.name}
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-4 flex gap-2">
          <input
            aria-label="新 MCP ID"
            className="min-w-0 flex-1 rounded border border-slate-300 px-2 py-1 text-sm"
            placeholder="新 MCP ID"
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
          />
          <button type="button" className="rounded border border-slate-300 px-2 py-1 text-sm" onClick={create}>
            新建
          </button>
        </div>
      </section>
      <section>
        {selected ? (
          <>
            <div className="mb-3 flex gap-2">
              <button type="button" className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white" onClick={save}>保存</button>
              <button type="button" className="rounded border border-red-300 px-3 py-1.5 text-sm text-red-700" onClick={remove}>删除</button>
            </div>
            {saveError && <p className="mb-2 text-sm text-red-600">{saveError}</p>}
            <JsonEditor value={content} onChange={setContent} />
          </>
        ) : (
          <p className="text-sm text-slate-500">选择左侧 MCP 服务器进行编辑。</p>
        )}
      </section>
    </div>
  );
}
```

- [ ] **Step 4: 运行测试**

Run: `pnpm test`
Expected: PASS（App 与 McpPanel 测试）。

- [ ] **Step 5: 提交**

Run: `git add src && git commit -m "feat: 打通 MCP 资产列表与编辑"`

---

### Task 9: 设置面板、主题与存储根

**Files:**
- Create: `src/hooks/useTheme.ts`
- Modify: `src/panels/SettingsPanel.tsx`
- Create: `src/__tests__/SettingsPanel.test.tsx`

**Interfaces:**
- Consumes: `api.getSettings()`、`api.setTheme(theme)`、`api.setStoreRoot(path)`。
- Produces: `useTheme(theme) -> void`（把 `light` / `dark` 应用到 `document.documentElement`）。

- [ ] **Step 1: 写失败测试**

`src/__tests__/SettingsPanel.test.tsx`：

```tsx
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, test, vi } from "vitest";
import { SettingsPanel } from "../panels/SettingsPanel";
import { api } from "../lib/api";

vi.mock("../lib/api", () => ({
  api: {
    getSettings: vi.fn(),
    setTheme: vi.fn(),
    setStoreRoot: vi.fn(),
  },
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getSettings).mockResolvedValue({ storeRoot: "C:/Users/me/.ccswitchx", theme: "system" });
  vi.mocked(api.setTheme).mockResolvedValue({ storeRoot: "C:/Users/me/.ccswitchx", theme: "dark" });
  vi.mocked(api.setStoreRoot).mockResolvedValue(true);
});

test("changes theme and persists it", async () => {
  render(<SettingsPanel />);
  const select = await screen.findByLabelText("主题");
  fireEvent.change(select, { target: { value: "dark" } });
  await waitFor(() => expect(api.setTheme).toHaveBeenCalledWith("dark"));
});

test("changes store root and warns about restart", async () => {
  render(<SettingsPanel />);
  const input = await screen.findByLabelText("存储位置");
  fireEvent.change(input, { target: { value: "D:/data/ccswitchx" } });
  fireEvent.click(screen.getByRole("button", { name: "保存存储位置" }));
  await waitFor(() => expect(api.setStoreRoot).toHaveBeenCalledWith("D:/data/ccswitchx"));
  expect(await screen.findByText("存储位置已保存，重启应用后生效。")).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `pnpm test -- SettingsPanel`
Expected: FAIL，设置面板没有对应控件。

- [ ] **Step 3: 实现主题与设置面板**

`src/hooks/useTheme.ts`：

```ts
import { useEffect } from "react";
import type { Theme } from "../types";

export function useTheme(theme: Theme) {
  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    root.dataset.theme = theme;
  }, [theme]);
}
```

`src/panels/SettingsPanel.tsx`：

```tsx
import { useEffect, useState } from "react";
import { useTheme } from "../hooks/useTheme";
import { api } from "../lib/api";
import type { AppSettings, Theme } from "../types";

export function SettingsPanel() {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [storeRoot, setStoreRoot] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useTheme(settings?.theme ?? "system");

  useEffect(() => {
    void api.getSettings().then((value) => {
      setSettings(value);
      setStoreRoot(value.storeRoot);
    }).catch((e) => setError(String(e)));
  }, []);

  const changeTheme = async (theme: Theme) => {
    try {
      const updated = await api.setTheme(theme);
      setSettings(updated);
      setError(null);
    } catch (e) { setError(String(e)); }
  };

  const saveStoreRoot = async () => {
    try {
      await api.setStoreRoot(storeRoot);
      setMessage("存储位置已保存，重启应用后生效。");
      setError(null);
    } catch (e) { setError(String(e)); }
  };

  return (
    <div className="max-w-2xl space-y-8">
      <h2 className="text-xl font-semibold">Settings</h2>
      {error && <p className="text-sm text-red-600">{error}</p>}
      <section>
        <label className="mb-2 block text-sm font-medium" htmlFor="theme">主题</label>
        <select
          id="theme"
          className="rounded border border-slate-300 px-3 py-1.5"
          value={settings?.theme ?? "system"}
          onChange={(e) => void changeTheme(e.target.value as Theme)}
        >
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </section>
      <section>
        <label className="mb-2 block text-sm font-medium" htmlFor="store-root">存储位置</label>
        <div className="flex gap-2">
          <input
            id="store-root"
            className="flex-1 rounded border border-slate-300 px-3 py-1.5 font-mono text-sm"
            value={storeRoot}
            onChange={(e) => setStoreRoot(e.target.value)}
          />
          <button type="button" className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white" onClick={saveStoreRoot}>
            保存存储位置
          </button>
        </div>
        {message && <p className="mt-2 text-sm text-green-700">{message}</p>}
      </section>
    </div>
  );
}
```

- [ ] **Step 4: 运行全部测试与构建**

Run: `pnpm test`
Expected: PASS。

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS。

Run: `pnpm build && cargo build --manifest-path src-tauri/Cargo.toml`
Expected: 构建成功。

Run: `pnpm tauri dev`
Expected: 窗口打开；MCP 面板可新建/编辑/删除条目；Settings 可切换主题、保存存储位置；重启后指向新目录。

- [ ] **Step 5: 提交**

Run: `git add src src-tauri && git commit -m "feat: 添加设置面板、主题与存储位置切换"`

---

## 自检

- **Spec 覆盖（Plan 1 范围）**：M0 的骨架、文本真源、索引、六面板壳、设置；M1 的 MCP 端到端。其余能力在 Plan 2–5。
- **类型一致性**：`AssetType` 的 serde 名为 `snake_case`，`AssetSummary` / `AppSettings` 的 serde 名为 `camelCase`，与前端 `types.ts` 一致；Tauri 命令参数使用 camelCase（`assetType`、`storeRoot`）。
- **Review Focus 对应**：Windows 覆盖写在 Task 3；路径穿越在 Task 3；索引残留在 Task 4；存储根切换在 Task 9；编码问题在 Task 2。
- **未决项**：无。Plan 2 开始时再展开导入与适配器细节。