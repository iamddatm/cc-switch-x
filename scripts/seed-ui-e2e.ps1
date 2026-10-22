# 真机 UI 验证前的沙箱铺数据：store 与「agent 配置根」全在 $env:TEMP 下。
# 绝不往真实的 ~/.claude、~/.qoder-cn 写任何东西 —— 配置根靠 local/agents/<id>.json 的 baseOverride 重定向。
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$ErrorActionPreference = "Stop"

$root = Join-Path $env:TEMP "ccswitchx-ui-e2e"
$store = Join-Path $root "store"
$sandbox = Join-Path $root "sandbox"
if (Test-Path $root) { Remove-Item $root -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $store "assets/rules") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $store "assets/subagents") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $store "assets/hooks") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $store "assets/mcp") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $store "local/agents") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $store "state/applied") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $sandbox ".qoder-cn/rules") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $sandbox ".claude") -Force | Out-Null

# 真源：一条规则、一个子代理、一个 hook、一条 MCP
Set-Content -Encoding utf8NoBOM -Path (Join-Path $store "assets/rules/style.md") -Value @'
---
name: style
description: 写法约定
paths:
- src/**
---

改文件前先读邻近实现。
'@
Set-Content -Encoding utf8NoBOM -Path (Join-Path $store "assets/subagents/reviewer.yaml") -Value @'
name: reviewer
description: 代码审查
instructions: 先看 diff
model: null
tools:
- Read
tool_extras: {}
targets: []
'@
Set-Content -Encoding utf8NoBOM -Path (Join-Path $store "assets/hooks/fmt.yaml") -Value @'
name: fmt
event: PreToolUse
matcher: Bash
command: cargo fmt --all
command_windows: null
timeout: 30
'@
Set-Content -Encoding utf8NoBOM -Path (Join-Path $store "assets/mcp/playwright.json") -Value @'
{"id":"playwright","name":"Playwright","transport":"stdio","command":"npx","args":["-y","pkg"],"env":{},"headers":{},"description":"","homepage":"","docs":"","tags":[]}
'@

# 两个 agent 的配置根重定向到沙箱
foreach ($id in @("qoder-cn", "claude")) {
  $json = @{ baseOverride = ($sandbox -replace '\\', '/') } | ConvertTo-Json -Compress
  Set-Content -Encoding utf8NoBOM -Path (Join-Path $store "local/agents/$id.json") -Value $json
}

# 沙箱里那份「被用户改过」的规则文件：启动时 rescan 应当报 Modified
Set-Content -Encoding utf8NoBOM -Path (Join-Path $sandbox ".qoder-cn/rules/style.md") -Value @'
---
trigger: always_on
---

用户在 Qoder 那边补了一条
'@
$target = ((Join-Path $sandbox ".qoder-cn/rules/style.md") -replace '\\', '/')
$ledger = @{
  entries = @{
    "rule/style" = @{
      assetType   = "rule"
      assetId     = "style"
      targetPath  = $target
      hash        = "0000000000000000000000000000000000000000000000000000000000000000"
      renderedBy  = 1
    }
  }
} | ConvertTo-Json -Depth 6
Set-Content -Encoding utf8NoBOM -Path (Join-Path $store "state/applied/qoder-cn.json") -Value $ledger

# 沙箱里的 settings.json：各留一条用户自己写的 hook，验「我们不动它们」
$userHook = '{"model":"haiku","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo user-own"}]}]}}'
Set-Content -Encoding utf8NoBOM -Path (Join-Path $sandbox ".qoder-cn/settings.json") -Value $userHook
Set-Content -Encoding utf8NoBOM -Path (Join-Path $sandbox ".claude/settings.json") -Value $userHook

# 应用设置指到这个临时 store（跑完由 -Restore 清掉）
$settingsDir = Join-Path $env:APPDATA "cc-switch-x"
New-Item -ItemType Directory -Path $settingsDir -Force | Out-Null
$settingsPath = Join-Path $settingsDir "settings.json"
if (Test-Path $settingsPath) { Move-Item $settingsPath "$settingsPath.ui-e2e.bak" -Force }
Set-Content -Encoding utf8NoBOM -Path $settingsPath -Value (@{ storeRoot = ($store -replace '\\', '/'); theme = "dark" } | ConvertTo-Json -Compress)

Write-Output "STORE=$store"
Write-Output "SANDBOX=$sandbox"
Write-Output "SETTINGS=$settingsPath"
