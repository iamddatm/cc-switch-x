# 真机 UI 验证前的沙箱铺数据：store 与「agent 配置根」全在 $env:TEMP 下。
# 绝不往真实的 ~/.claude、~/.qoder-cn 写任何东西 —— 配置根靠 local/agents/<id>.json 的 baseOverride 重定向。
# 加 -Project 时再铺一个临时项目仓库，用于项目级 Skills 的真机流程（添加项目 → 收进真源 → 渲染 → 编辑）。
#
# 这个脚本会临时接管 %APPDATA%/cc-switch-x/settings.json 指到沙箱 store：
#   - 每次把当时的设置存成带时间戳的 .ui-e2e.<时间>.bak，绝不覆盖已有的备份（连续 seed 会把真设置挤掉）；
#   - 跑完必须 `-Restore` 还原最早那份未被还原过的备份。
param(
  [switch]$Project,
  [switch]$Restore
)

[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new()
$ErrorActionPreference = "Stop"

$root = Join-Path $env:TEMP "ccswitchx-ui-e2e"
$settingsDir = Join-Path $env:APPDATA "cc-switch-x"
$settingsPath = Join-Path $settingsDir "settings.json"

if ($Restore) {
  # 最早的那份才是用户的设置；后面几次 seed 存的都是上一次的沙箱设置。
  $backups = @(Get-ChildItem "$settingsDir" -Filter "settings.json.ui-e2e.*.bak" -ErrorAction SilentlyContinue | Sort-Object Name)
  if ($backups.Count -eq 0) {
    $legacy = Join-Path $settingsDir "settings.json.ui-e2e.bak"
    if (Test-Path $legacy) { $backups = @(Get-Item $legacy) }
  }
  if ($backups.Count -eq 0) { Write-Output "RESTORE=没有可还原的备份，settings.json 保持原样"; exit 0 }
  $oldest = $backups[0]
  Move-Item $oldest.FullName $settingsPath -Force
  foreach ($extra in $backups | Select-Object -Skip 1) { Remove-Item $extra.FullName -Force }
  if (Test-Path $root) { Remove-Item $root -Recurse -Force }
  Write-Output "RESTORE=$($oldest.Name) -> $settingsPath"
  exit 0
}

$store = Join-Path $root "store"
$sandbox = Join-Path $root "sandbox"
$projectRoot = Join-Path $root "home/work/demo-project"
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

# 临时项目仓库：只给 Claude 写了一份 skill，正是「忘了同步到 Qoder」的真实痛点。
# 不建 .ccswitchx/，让应用自己去登记 v2 台账；此时 .claude/skills/probe 属于「未跟踪」，
# UI 上应提供「收进真源」而不是替用户改动内容。
if ($Project) {
  New-Item -ItemType Directory -Path (Join-Path $projectRoot ".claude/skills/probe") -Force | Out-Null
  Set-Content -Encoding utf8NoBOM -Path (Join-Path $projectRoot ".claude/skills/probe/SKILL.md") -Value @'
---
name: probe
description: 只给 Claude 写的那份 skill
---

探针正文。
'@
  New-Item -ItemType Directory -Path (Join-Path $projectRoot "src") -Force | Out-Null
  Set-Content -Encoding utf8NoBOM -Path (Join-Path $projectRoot "README.md") -Value "# demo-project`n"
}

# 应用设置指到这个临时 store（跑完必须 -Restore 还原）
New-Item -ItemType Directory -Path $settingsDir -Force | Out-Null
if (Test-Path $settingsPath) {
  # 带时间戳，且已存在就绝不覆盖：连续 seed 两次也不能把用户的设置挤掉。
  $backup = Join-Path $settingsDir ("settings.json.ui-e2e.{0}.bak" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
  if (Test-Path $backup) { $backup = "$backup.$(Get-Random -Maximum 99)" }
  Move-Item $settingsPath $backup
  Write-Output "BACKUP=$backup"
}
# 带项目时只启用 Claude 与 Qoder CN：国际版 Qoder 的用户级 Skill 能力仍未实测，
# 它一起启用会把 .qoder/skills 这一列重新挡回「待实测」，项目流程就没法验证渲染。
$settings = if ($Project) {
  @{ storeRoot = ($store -replace '\\', '/'); theme = "dark"; enabledAgents = @("claude", "qoder-cn") }
} else {
  @{ storeRoot = ($store -replace '\\', '/'); theme = "dark" }
}
Set-Content -Encoding utf8NoBOM -Path $settingsPath -Value ($settings | ConvertTo-Json -Compress)

Write-Output "STORE=$store"
Write-Output "SANDBOX=$sandbox"
Write-Output "SETTINGS=$settingsPath"
if ($Project) { Write-Output "PROJECT=$projectRoot" }
