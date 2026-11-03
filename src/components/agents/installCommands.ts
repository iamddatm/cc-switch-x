/**
 * 各 agent CLI 的可复制安装/升级命令。
 *
 * **这些字符串永远不会被本应用执行**，只进剪贴板 —— 装什么、什么时候装由用户在终端里决定。
 * 命令文本逐条抄自上游 cc-switch（MIT，见 THIRD_PARTY_NOTICES.md）的
 * `src-tauri/src/commands/misc.rs`：npm 形式取自 `npm_install_command_for`，
 * 官方脚本取自 `CLAUDE_INSTALL_UNIX` / `CODEX_INSTALL_WINDOWS_SCRIPT`。
 * 包名在 Rust 侧另有一份（`domain/agent.rs` 的 `npm_package`，只用来查最新版），
 * 两处不一致会导致「有新版本」提示和复制到的命令对不上，改一处就要看另一处。
 */
export interface InstallCommand {
  label: string;
  command: string;
}

const AGENT_INSTALL_COMMANDS: Record<string, InstallCommand[]> = {
  claude: [
    {
      label: "npm 全局安装 / 升级",
      command:
        "npm i -g @anthropic-ai/claude-code@latest --ignore-scripts=false --include=optional --allow-scripts=@anthropic-ai/claude-code",
    },
    {
      label: "官方脚本（macOS / Linux）",
      command:
        "bash -c 'tmp=$(mktemp) && curl -fsSL https://claude.ai/install.sh -o $tmp && bash $tmp; status=$?; rm -f $tmp; exit $status'",
    },
  ],
  codex: [
    { label: "npm 全局安装 / 升级", command: "npm i -g @openai/codex@latest" },
    { label: "官方脚本（Windows PowerShell）", command: "irm https://chatgpt.com/codex/install.ps1 | iex" },
  ],
  gemini: [{ label: "npm 全局安装 / 升级", command: "npm i -g @google/gemini-cli@latest" }],
  pi: [{ label: "npm 全局安装 / 升级", command: "npm i -g @earendil-works/pi-coding-agent@latest" }],
};

/** 没核实到安装入口的 agent（Qoder 系不走 npm），UI 上直说没有，不编 URL。 */
export function installCommandsFor(agentId: string): InstallCommand[] {
  return AGENT_INSTALL_COMMANDS[agentId] ?? [];
}
