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
  await target.getByText("Playwright").first().click();
  await target.waitForTimeout(500);
  log.mcpMatrix = await target.locator("text=同步到").count();
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

  console.log(JSON.stringify(log, null, 2));
  await browser.close();
}

main().catch((error) => {
  console.error("DRIVER FAILED:", error);
  process.exit(1);
});
