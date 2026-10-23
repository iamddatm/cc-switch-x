//! 直接调用工具自己的 CLI（现在只有 Qoder CN 的 `qoderclicn` 值得走这条路）。
//!
//! 两条铁律：
//! 1. **参数一律用数组传给 `Command`，不拼 shell 字符串** —— JSON 配置里什么字符都可能有了，
//!    拼字符串就是自己造注入点。
//! 2. **scope 必须显式**：`mcp add-json` / `mcp remove` 的 `--scope` 默认是 `local`
//!    （项目级），漏写就等于把用户级资产写进别人的项目目录；`mcp get` 不带 scope 会在
//!    任意 scope 里找到就算数，回读会骗人。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const PROGRAM: &str = "qoderclicn";
/// 用户级 scope。CLI 默认是 `local`，这里是刻意写死的值。
pub const SCOPE_USER: &str = "user";

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("没找到 {PROGRAM}，这条路径本来就走不了")]
    NotFound,
    #[error("启动 {PROGRAM} 失败：{0}")]
    Spawn(String),
    #[error("{PROGRAM} 退出码 {}：{message}", .code.map(|c| c.to_string()).unwrap_or_else(|| "信号终止".into()))]
    Failed { code: Option<i32>, message: String },
}

/// `mcp add-json <name> <json>`，显式 `--scope user`。
/// `--scope` 与 `user` 必须是相邻的两项，且 JSON 整块是一个参数。
pub fn add_mcp_args(name: &str, json: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "add-json".into(),
        "--scope".into(),
        SCOPE_USER.into(),
        name.into(),
        json.into(),
    ]
}

/// `mcp remove` 不写 scope 时会从「它发现定义在哪的那个 scope」删，可能删到项目级那份。
pub fn remove_mcp_args(name: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "remove".into(),
        "--scope".into(),
        SCOPE_USER.into(),
        name.into(),
    ]
}

/// `mcp get` 未命中时**退出码仍是 0**（真机 1.1.76 实测：输出
/// `Server "<name>" not found in user settings.`）。所以判断「在不在」不能看退出码，
/// 只认命中输出里那行 `<name>:`；格式一旦变了就当成没命中 —— 宁可说「只算已写入」，
/// 不能把没生效说成生效。
pub fn get_output_reports_present(name: &str, stdout: &str) -> bool {
    let expected = format!("{name}:");
    stdout.lines().any(|line| line.trim() == expected)
}

/// 回读也钉在 user scope：不带 `--scope` 时任意 scope 命中都算「存在」，会把没生效说成生效。
pub fn get_mcp_args(name: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "get".into(),
        "--scope".into(),
        SCOPE_USER.into(),
        name.into(),
    ]
}

pub fn list_mcp_args() -> Vec<String> {
    vec!["mcp".into(), "list".into()]
}

/// `skills link` 不带 `--consent` 会**弹交互确认**，非交互进程里就挂在那里了。
/// 用户在我们面板上点「启用」已经是那个同意，所以这里显式带上，并同时显式 `--scope user`。
pub fn link_skill_args(path: &Path) -> Vec<String> {
    vec![
        "skills".into(),
        "link".into(),
        "--scope".into(),
        SCOPE_USER.into(),
        "--consent".into(),
        path.to_string_lossy().to_string(),
    ]
}

pub fn uninstall_skill_args(name: &str) -> Vec<String> {
    vec![
        "skills".into(),
        "uninstall".into(),
        "--scope".into(),
        SCOPE_USER.into(),
        name.into(),
    ]
}

pub fn list_skills_args() -> Vec<String> {
    vec!["skills".into(), "list".into()]
}

/// 真正执行的样子。测试注入假的 `run`，所以这里可以是任何可执行动作；
/// 生产路径用 `spawn`。
pub type Runner = fn(&Path, &[String]) -> Result<String, CliError>;

#[derive(Debug, Clone)]
pub struct QoderCli {
    pub program: PathBuf,
    pub run: Runner,
}

impl QoderCli {
    /// 在 `PATH` 里找可执行文件，**不假设安装位置**。找不到就返回 `None`（不是错误：
    /// 没有 CLI 时照样能用文件路径落盘，只是少一层「已生效」的确认）。
    pub fn discover() -> Option<Self> {
        let program = discover_program(PROGRAM)?;
        Some(Self {
            program,
            run: spawn,
        })
    }

    /// 测试用：指定程序路径与执行动作。
    pub fn with_runner(program: PathBuf, run: Runner) -> Self {
        Self { program, run }
    }

    pub fn add_mcp(&self, name: &str, json: &str) -> Result<String, CliError> {
        (self.run)(&self.program, &add_mcp_args(name, json))
    }

    pub fn remove_mcp(&self, name: &str) -> Result<String, CliError> {
        (self.run)(&self.program, &remove_mcp_args(name))
    }

    pub fn list_mcp(&self) -> Result<String, CliError> {
        (self.run)(&self.program, &list_mcp_args())
    }

    pub fn link_skill(&self, path: &Path) -> Result<String, CliError> {
        (self.run)(&self.program, &link_skill_args(path))
    }

    pub fn uninstall_skill(&self, name: &str) -> Result<String, CliError> {
        (self.run)(&self.program, &uninstall_skill_args(name))
    }

    /// 只有输出里出现 `<name>:` 才算「CLI 认为它在 user scope 里存在」。
    /// 退出码不能当依据：未命中也是 0（见 `get_output_reports_present`）。
    pub fn mcp_present(&self, name: &str) -> bool {
        (self.run)(&self.program, &get_mcp_args(name))
            .map(|stdout| get_output_reports_present(name, &stdout))
            .unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
fn candidate_names(program: &str) -> Vec<String> {
    // Windows 上 PATH 里可能是 .exe / .cmd / .bat；PATHEXT 的顺序我们不复刻，
    // 只按这个固定顺序找，找到一个就用。
    ["exe", "cmd", "bat", ""]
        .iter()
        .map(|ext| {
            if ext.is_empty() {
                program.to_string()
            } else {
                format!("{program}.{ext}")
            }
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn candidate_names(program: &str) -> Vec<String> {
    vec![program.to_string()]
}

/// 在 PATH 里找可执行文件；找不到返回 None。
pub fn discover_program(program: &str) -> Option<PathBuf> {
    for dir in std::env::split_paths(&std::env::var("PATH").ok()?) {
        if let Some(found) = find_in_dir(&dir, program) {
            return Some(found);
        }
    }
    None
}

/// 「CLI 是否安装」徽标用：PATH 之外再看常见安装目录。
/// GUI 进程的 PATH 常常缺 shell 里的 npm / fnm 路径，只扫 PATH 会把已装的工具报成未安装。
pub fn discover_cli(program: &str, home: &Path) -> Option<PathBuf> {
    discover_program(program).or_else(|| {
        common_install_dirs(home)
            .into_iter()
            .find_map(|dir| find_in_dir(&dir, program))
    })
}

fn find_in_dir(dir: &Path, program: &str) -> Option<PathBuf> {
    candidate_names(program)
        .into_iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn common_install_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".volta/bin"),
        home.join(".cargo/bin"),
    ];
    if let Some(data) = dirs::data_dir() {
        // Windows 的 npm 全局目录在 %APPDATA%\npm；其他平台多一层兜底。
        dirs.push(data.join("npm"));
        dirs.push(data.join("pnpm"));
        collect_fnm_dirs(&data, &mut dirs);
    }
    if let Some(local) = dirs::data_local_dir() {
        dirs.push(local.join("pnpm"));
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
    }
    dirs
}

/// fnm 的全局安装目录（GUI 进程通常拿不到 multishell 的 PATH）：
/// Windows 在 `<data>/fnm/node-versions/<ver>/installation`，Unix 再下一层 `bin`。
fn collect_fnm_dirs(data_dir: &Path, out: &mut Vec<PathBuf>) {
    let root = data_dir.join("fnm").join("node-versions");
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let installation = entry.path().join("installation");
        #[cfg(target_os = "windows")]
        out.push(installation);
        #[cfg(not(target_os = "windows"))]
        out.push(installation.join("bin"));
    }
}

/// 真正的执行：`Command` + 参数数组，捕获 stdout；失败时带上 stderr。
fn spawn(program: &Path, args: &[String]) -> Result<String, CliError> {
    let output = std::process::Command::new(program)
        .args(args)
        // 绝不给非交互进程留一个等输入的提示
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| CliError::Spawn(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(CliError::Failed {
            code: output.status.code(),
            message: if stderr.is_empty() {
                stdout.trim().to_string()
            } else {
                stderr
            },
        });
    }
    Ok(stdout)
}

/// 落盘方式。报告里必须说清走了哪条：CLI 确认过的才配得上「已生效」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MethodUsed {
    Cli,
    File,
}

/// 这个 agent 的 CLI 通道情况。分成三种是为了让「没走 CLI」有可说的原因，
/// 而不是把「不适用」「没装」「装了但失败」混成一句「走了文件写入」。
#[derive(Debug, Clone, Default)]
pub enum CliChannel {
    /// 该 agent 没有官方写命令（Claude / Codex / Gemini / Pi）
    #[default]
    NotApplicable,
    /// 该 agent 该有 CLI，但本机 PATH 里找不到
    Unavailable,
    Present(QoderCli),
}

impl CliChannel {
    /// 该 agent 是否归 `qoderclicn` 管。目前只有 Qoder CN：国际版只发现 IDE 启动器，
    /// 没有独立 CLI（本机实测），所以不给它走这条路径。
    pub fn for_agent(agent_id: &str) -> Self {
        if agent_id != "qoder-cn" {
            return Self::NotApplicable;
        }
        match QoderCli::discover() {
            Some(cli) => Self::Present(cli),
            None => Self::Unavailable,
        }
    }

    pub fn cli(&self) -> Option<&QoderCli> {
        match self {
            Self::Present(cli) => Some(cli),
            _ => None,
        }
    }

    /// 没走 CLI 时该在报告里写什么。`None` = 这条路径本来就不适用，不需要解释。
    pub fn fallback_note(&self) -> Option<String> {
        match self {
            Self::NotApplicable => None,
            Self::Unavailable => Some("本机没找到 qoderclicn，走文件写入".into()),
            Self::Present(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假 runner：把收到的 argv 原样回显，测试就能断言「到底传了什么」。
    fn echo(_: &Path, args: &[String]) -> Result<String, CliError> {
        Ok(args.join("\u{1}"))
    }

    fn fail(_: &Path, _: &[String]) -> Result<String, CliError> {
        Err(CliError::Failed {
            code: Some(2),
            message: "boom".into(),
        })
    }

    fn cli(run: Runner) -> QoderCli {
        QoderCli::with_runner(PathBuf::from("qoderclicn-fake"), run)
    }

    fn argv(out: Result<String, CliError>) -> Vec<String> {
        out.unwrap().split('\u{1}').map(str::to_string).collect()
    }

    #[test]
    fn mcp_add_json_always_passes_scope_user() {
        let args = add_mcp_args("playwright", "{\"type\":\"stdio\"}");
        let scope = args
            .iter()
            .position(|a| a == "--scope")
            .expect("必须有 --scope");
        assert_eq!(args[scope + 1], SCOPE_USER, "--scope 与 user 必须相邻");
        assert_eq!(
            argv(cli(echo).add_mcp("playwright", "{\"type\":\"stdio\"}")),
            args
        );
        // 漏了 --scope 就落到 local（项目级）：这是这条路径上最容易犯的错
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--scope" && pair[1] == "user"));
    }

    #[test]
    fn every_mcp_call_pins_the_user_scope() {
        for args in [
            add_mcp_args("n", "{}"),
            remove_mcp_args("n"),
            get_mcp_args("n"),
        ] {
            let scope = args.iter().position(|a| a == "--scope");
            assert!(scope.is_some(), "{args:?} 没有钉 scope");
            assert_eq!(args[scope.unwrap() + 1], "user", "{args:?}");
        }
    }

    #[test]
    fn the_json_stays_one_argument_even_with_shell_meta_and_spaces() {
        let json = r#"{"command":"npx -y foo && echo \"hi\"","args":["a b"]}"#;
        let sent = argv(cli(echo).add_mcp("x", json));
        assert_eq!(
            sent.last().unwrap(),
            json,
            "JSON 必须整块是一个参数，不能被拆开"
        );
        assert_eq!(
            sent.iter().filter(|a| a.as_str() == json).count(),
            1,
            "出现两次说明被拼接过了"
        );
        // 拼出的 argv 里没有 shell 分隔符角色：`&&` 只出现在 JSON 值内部
        assert_eq!(sent.len(), add_mcp_args("x", json).len());
    }

    #[test]
    fn linking_a_skill_passes_consent_so_it_cannot_hang() {
        let args = link_skill_args(Path::new("C:/store/assets/skills/demo"));
        assert!(
            args.iter().any(|a| a == "--consent"),
            "不带 --consent 会弹交互确认，非进程化调用会挂在那里：{args:?}"
        );
        let scope = args.iter().position(|a| a == "--scope").unwrap();
        assert_eq!(args[scope + 1], "user");
        assert_eq!(args.last().unwrap(), "C:/store/assets/skills/demo");
        assert_eq!(
            argv(cli(echo).link_skill(Path::new("C:/store/assets/skills/demo"))),
            args
        );
    }

    #[test]
    fn uninstalling_a_skill_names_the_user_scope() {
        let args = uninstall_skill_args("demo");
        let scope = args.iter().position(|a| a == "--scope").unwrap();
        assert_eq!(args[scope + 1], "user");
        assert_eq!(args.last().unwrap(), "demo");
    }

    /// 真机 1.1.76 的命中形状（首行是 `<name>:`）。
    fn get_reports_present(_: &Path, args: &[String]) -> Result<String, CliError> {
        let name = args.last().map(String::as_str).unwrap_or_default();
        Ok(format!(
            "{name}:\n  Scope: User config (available in all your projects)\n  Type: stdio\n"
        ))
    }

    /// 真机 1.1.76 的未命中形状：**退出码 0**，文字说 not found。
    fn get_reports_absent(_: &Path, args: &[String]) -> Result<String, CliError> {
        let name = args.last().map(String::as_str).unwrap_or_default();
        Ok(format!("Server \"{name}\" not found in user settings.\n"))
    }

    #[test]
    fn a_failing_cli_surfaces_as_an_error_not_a_silent_success() {
        let error = cli(fail).add_mcp("n", "{}").unwrap_err();
        assert!(error.to_string().contains("退出码 2"), "{error}");
        assert!(error.to_string().contains("boom"), "{error}");
        // 回读也一样：失败就是「没确认」，不是「已生效」
        assert!(!cli(fail).mcp_present("n"));
        // 退出 0 但输出只是 argv 回显：同样不算命中
        assert!(!cli(echo).mcp_present("n"));
        assert!(cli(get_reports_present).mcp_present("n"));
    }

    #[test]
    fn an_exit_zero_not_found_is_not_a_confirmation() {
        // 这条是 A1 真机复核时抓到的：`mcp get` 未命中也是 0，
        // 只看退出码会把「没写进去」报成「已生效」。
        assert!(!cli(get_reports_absent).mcp_present("definitely-absent"));
        assert!(!get_output_reports_present(
            "definitely-absent",
            "Server \"definitely-absent\" not found in user settings.\n"
        ));
    }

    #[test]
    fn a_found_server_is_recognised_by_its_heading_line_only() {
        let out =
            "probe-ccsx:\n  Scope: User config (available in all your projects)\n  Command: echo\n";
        assert!(get_output_reports_present("probe-ccsx", out));
        // 名字必须整行相等：前缀相同的别的服务器不能被认成命中
        assert!(!get_output_reports_present("probe", out));
        assert!(!get_output_reports_present("probe-ccsx", "ok"));
    }

    #[test]
    fn spawning_a_missing_program_is_a_spawn_error() {
        let error = spawn(
            Path::new("this-binary-does-not-exist-xyz"),
            &["--version".into()],
        )
        .unwrap_err();
        assert!(matches!(error, CliError::Spawn(_)), "{error}");
    }

    #[test]
    fn candidate_names_cover_the_windows_suffixes() {
        let names = candidate_names(PROGRAM);
        assert!(names.iter().any(|n| n == PROGRAM));
        if cfg!(target_os = "windows") {
            assert!(names.iter().any(|n| n == "qoderclicn.exe"));
            assert!(names.iter().any(|n| n == "qoderclicn.cmd"));
        }
    }

    #[test]
    fn finds_a_platform_candidate_in_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let program = "ccsx-cli-probe";
        #[cfg(target_os = "windows")]
        let name = format!("{program}.cmd");
        #[cfg(not(target_os = "windows"))]
        let name = program.to_string();
        std::fs::write(dir.path().join(&name), b"").unwrap();

        let found = find_in_dir(dir.path(), program).expect("应找到候选文件");
        assert_eq!(found, dir.path().join(name));
        assert_eq!(find_in_dir(dir.path(), "ccsx-missing"), None);
    }
}
