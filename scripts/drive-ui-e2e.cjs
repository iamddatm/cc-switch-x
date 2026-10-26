// 真机 UI 验证：连 WebView2 的 CDP，把三个新面板点一遍并截图。
// 用法：NODE_PATH="$(npm root -g)" node scripts/drive-ui-e2e.cjs
const { chromium } = require("playwright-core");
const fs = require("node:fs");
const path = require("node:path");

const SHOTS = process.env.SHOT_DIR || path.join(process.env.TEMP || "/tmp", "ccswitchx-ui-e2e", "shots");
const SANDBOX = path.join(process.env.TEMP || "/tmp", "ccswitchx-ui-e2e", "sandbox");
const STORE = path.join(process.env.TEMP || "/tmp", "ccswitchx-ui-e2e", "store");

const read = (file) => fs.readFileSync(file, "utf8");

async function main() {
  fs.mkdirSync(SHOTS, { recursive: true });
  const browser = await chromium.connectOverCDP("http://127.0.0.1:9224");
  const context = browser.contexts()[0];
  const page = await context.newPage().catch(() => context.pages()[0]);
  const pages = context.pages();
  const target = pages.find((p) => (p.url() || "").includes("localhost")) || pages[0];
  await target.bringToFront();
  const log = {};

  const shot = async (name) => {
    await target.screenshot({ path: path.join(SHOTS, `${name}.png`), fullPage: true });
  };
  const sidebar = async (label) => {
    await target.getByRole("button", { name: label, exact: true }).first().click();
    await target.waitForTimeout(400);
  };
  await target.reload({ waitUntil: "networkidle" });
  await target.waitForTimeout(800);

  // ── MCP ────────────────────────────────────────────
  await sidebar("MCP");
  await target.waitForTimeout(500);
  // 新列表把勾选直接放在行右侧，不再需要先点开详情
  log.mcpMatrix = await target.getByRole("checkbox", { name: /启用 playwright/ }).count();
  await shot("01-mcp");

  // 勾 Claude 的 MCP（配置根已重定向到沙箱，绝不影响真实 ~/.claude.json）
  const mcpBox = target.getByRole("checkbox", { name: "Claude Code 启用 playwright" });
  log.mcpClaudeBoxEnabled = await mcpBox.isEnabled().catch(() => false);
  if (log.mcpClaudeBoxEnabled) {
    // 复选框是受控的：状态要等后端返回才翻，所以用 click + 落盘结果断言，而不是 check()
    await mcpBox.click();
    await target.waitForTimeout(1500);
    log.mcpBoxStateAfterClick = await mcpBox.isChecked();
    log.mcpErrorText = await target.locator("text=/失败|未做任何改动|漂移|Error/i").allTextContents().catch(() => []);
    const claudeJson = path.join(SANDBOX, ".claude.json");
    log.claudeJsonAfterMcp = fs.existsSync(claudeJson) ? JSON.parse(read(claudeJson)) : null;
    log.mcpReportText = await target.locator("text=/实测：/").first().textContent().catch(() => null);
    await shot("02-mcp-enabled");
  }

  // ── Rules：漂移条 + 未实测 ────────────────────────────
  await sidebar("Rules");
  await target.getByText("style").first().click();
  await target.waitForTimeout(600);
  const banner = target.getByRole("region", { name: "落盘漂移" });
  log.driftBannerVisible = await banner.isVisible().catch(() => false);
  log.driftText = log.driftBannerVisible ? await banner.textContent() : null;
  await shot("03-rules-drift");
  log.ruleUnverifiedNote = await target
    .getByText("Qoder CN：未实测，暂不提供开关")
    .count()
    .catch(() => 0);
  log.ruleBodyInForm = await target.getByLabel("规则正文").inputValue().catch(() => null);

  if (log.driftBannerVisible) {
    const before = read(path.join(SANDBOX, ".qoder-cn/rules/style.md"));
    await target.getByRole("button", { name: "采纳磁盘" }).click();
    await target.waitForTimeout(1200);
    const after = read(path.join(SANDBOX, ".qoder-cn/rules/style.md"));
    log.diskUntouchedByAdopt = before === after;
    const canonical = read(path.join(STORE, "assets/rules/style.md"));
    log.canonicalHasUserEdit = canonical.includes("用户在 Qoder 那边补了一条");
    log.canonicalKeepsPaths = canonical.includes("src/**");
    log.driftAfterAdopt = await target.getByRole("region", { name: "落盘漂移" }).count();
    await shot("04-rules-adopted");
  }

  // ── Subagents ─────────────────────────────────────
  await sidebar("Subagents");
  await target.getByText("reviewer").first().click();
  await target.waitForTimeout(500);
  log.subagentInstructions = await target.getByLabel("子代理指令").inputValue().catch(() => null);
  await shot("05-subagents");

  // ── Hooks ─────────────────────────────────────────
  await sidebar("Hooks");
  await target.getByText("fmt").first().click();
  await target.waitForTimeout(500);
  log.hookCommandShown = await target.getByTestId("hook-command").textContent().catch(() => null);
  log.hookEventOptions = await target.getByLabel("hook 事件").locator("option").count();
  log.hookEventValue = await target.getByLabel("hook 事件").inputValue().catch(() => null);
  log.foreignHookNote = await target.getByText(/不是本工具写的 hook/).count().catch(() => 0);
  await shot("06-hooks");

  const hookBox = target.getByRole("checkbox", { name: "Claude Code 启用 fmt" });
  log.hookBoxEnabled = await hookBox.isEnabled().catch(() => false);
  if (log.hookBoxEnabled) {
    await hookBox.click();
    await target.waitForTimeout(1500);
    log.hookBoxStateAfterClick = await hookBox.isChecked();
    log.hookErrorText = await target.locator("text=/失败|未做任何改动|漂移|Error/i").allTextContents().catch(() => []);
    const settings = JSON.parse(read(path.join(SANDBOX, ".claude/settings.json")));
    log.hooksAfterEnable = settings.hooks;
    log.modelKeyKept = settings.model;
    await shot("07-hooks-enabled");
    await hookBox.click();
    await target.waitForTimeout(1500);
    log.hookBoxStateAfterDisable = await hookBox.isChecked();
    const after = JSON.parse(read(path.join(SANDBOX, ".claude/settings.json")));
    log.hooksAfterDisable = after.hooks;
    await shot("08-hooks-disabled");
  }

  // ── 项目级 Skills（只有 seed 带 -Project 时才跑）───────────
  const PROJECT = process.env.PROJECT_PATH;
  if (PROJECT) {
    const claudeFile = path.join(PROJECT, ".claude/skills/probe/SKILL.md");
    const canonicalFile = path.join(PROJECT, ".ccswitchx/skills/probe/SKILL.md");
    const qoderFile = path.join(PROJECT, ".qoder/skills/probe/SKILL.md");
    const claudeBefore = read(claudeFile);

    // 「添加项目…」开的是原生文件夹对话框，CDP 操作不了 —— 直接调同一条 IPC，
    // 注册表、台账、能力闸全部由后端自己算，不走任何测试后门。
    log.projectAdded = await target.evaluate(
      async (p) => await window.__TAURI_INTERNALS__.invoke("add_project", { path: p }),
      PROJECT,
    );
    await target.reload({ waitUntil: "networkidle" });
    await target.waitForTimeout(1000);

    // 切到项目范围：侧栏顶部的范围选择器 → 选中 demo-project → Skills 面板变成项目面板
    await target.getByRole("button", { name: /^配置范围/ }).click();
    // 行本身是 aria-pressed 的可选按钮；同名的「⋯」菜单按钮没有这个属性，别点错。
    await target.locator('button[aria-pressed]', { hasText: "demo-project" }).click();
    await sidebar("Skills");
    await target.waitForTimeout(1200);
    // 先关掉自动同步：开着的话勾完目标立刻铺盘，「勾选只记账、同步才写盘」这条两步纪律就验不到。
    const autoSync = target.getByRole("checkbox", { name: "自动同步" });
    if (await autoSync.isChecked()) {
      await autoSync.click();
      await target.waitForTimeout(800);
    }
    await shot("09-project-list");
    log.projectRowVisible = await target.getByRole("button", { name: "查看 probe" }).count();
    // 未实测的 .agents/skills 只能显示「待实测」，不给开关
    log.agentsCellUnverified = await target.getByText("待实测").count();
    log.agentsCheckboxAbsent = await target.getByRole("checkbox", { name: /Agents|Codex/i }).count();

    // 1. 收进真源：只新建 .ccswitchx/skills/probe，Claude 的目录一个字节都不动
    await target.getByRole("button", { name: "probe 更多操作" }).click();
    await target.getByRole("menuitem", { name: /把 Claude Code 的副本收进真源/ }).click();
    await target.waitForTimeout(1500);
    log.canonicalCreated = fs.existsSync(canonicalFile);
    log.claudeUntouchedByAdopt = read(claudeFile) === claudeBefore;
    log.qoderNotWrittenYet = !fs.existsSync(qoderFile);
    await shot("10-project-adopted");

    // 2. 勾选两个渲染目录：只改台账不写盘；按「立即同步」才铺开
    await target.getByRole("checkbox", { name: /同步到 Claude/ }).click();
    await target.waitForTimeout(600);
    await target.getByRole("checkbox", { name: /同步到 Qoder/ }).click();
    await target.waitForTimeout(800);
    log.qoderStillAbsentAfterCheck = !fs.existsSync(qoderFile);
    log.claudeStillOriginalAfterCheck = read(claudeFile) === claudeBefore;
    await target.getByRole("button", { name: "立即同步" }).click();
    await target.waitForTimeout(2000);
    log.qoderRendered = fs.existsSync(qoderFile);
    log.renderMatchesCanonical =
      fs.existsSync(qoderFile) && read(qoderFile) === read(canonicalFile);
    log.agentsDirStillAbsent = !fs.existsSync(path.join(PROJECT, ".agents"));
    await shot("11-project-rendered");

    // 3. 筛选与搜索在项目里真的可用
    await target.getByLabel("搜索 Skill").fill("对不上的关键词");
    await target.waitForTimeout(500);
    log.projectSearchEmpty = await target.getByText("没有匹配的 Skill").count();
    await target.getByLabel("搜索 Skill").fill("");
    await target.waitForTimeout(500);
    log.projectSearchRestored = await target.getByRole("button", { name: "查看 probe" }).count();
    await shot("12-project-filtered");

    // 4. 编辑正文：只改真源，两个渲染产物都保持落后，等下一次同步
    await target.getByRole("button", { name: "probe 更多操作" }).click();
    await target.getByRole("menuitem", { name: "编辑" }).click();
    await target.getByLabel(/正文/).fill("# 项目里改过的正文\n");
    await target.getByRole("button", { name: "保存" }).click();
    await target.waitForTimeout(2000);
    log.canonicalHasEdit = read(canonicalFile).includes("项目里改过的正文");
    log.copiesStaleUntilSync =
      !read(claudeFile).includes("项目里改过的正文") &&
      !read(qoderFile).includes("项目里改过的正文");
    await shot("13-project-edited");

    await target.getByRole("button", { name: "立即同步" }).click();
    await target.waitForTimeout(2000);
    log.claudeAfterSync = read(claudeFile).includes("项目里改过的正文");
    log.qoderAfterSync = read(qoderFile).includes("项目里改过的正文");
    await shot("14-project-synced");

    // 5. 项目里没有 cc-switch 导入这条来源
    await target.getByRole("button", { name: "添加 Skill" }).click();
    await target.waitForTimeout(400);
    log.projectAddItems = await target.getByRole("menuitem").allTextContents();
    await shot("15-project-add-menu");
    await target.keyboard.press("Escape");
  }

  console.log(JSON.stringify(log, null, 2));
  await browser.close();
}

main().catch((error) => {
  console.error("DRIVER FAILED:", error);
  process.exit(1);
});

