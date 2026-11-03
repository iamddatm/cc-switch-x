//! Agent CLI 的版本探测与上游最新版查询。**只读**：不安装、不升级、不碰任何真源或落盘。
//!
//! 两条查询都可能在用户机器上留下副作用，所以边界写死在这里：
//! - 跑外部程序只可能是**我们注册表里的 `cli_program`** 解析出的绝对路径，参数固定
//!   `--version` 一个，绝不接受调用方传进来的程序名或参数。
//! - 联网只打 npm 的 dist-tags 端点（不是 `/{package}`，那份 packument 几十 MB）。

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::adapters::cli::discover_cli;
use crate::domain::agent::{builtin_agents, AgentDescriptor};
use crate::error::AppError;
use crate::net::ProxyConfig;
use crate::process::hidden_command;
use crate::services::version;

/// 单个 agent 的版本账。`error` 有值表示「这次没问出来」，不等于「没有更新」。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVersionStatus {
    pub id: String,
    pub display_name: String,
    /// 在 PATH 与常见安装目录里找得到这个 CLI。
    pub cli_found: bool,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVersionReport {
    pub agents: Vec<AgentVersionStatus>,
    pub checked_at: i64,
}

/// 版本来源抽成 trait，是为了单测既不 spawn 真 CLI 也不联网。
pub trait VersionSource {
    /// 执行 `<program> --version`，拿回原始输出（stdout 为空时给 stderr）。
    fn run_version(&self, program: &Path) -> Result<String, AppError>;
    /// npm 上该包的 `latest`；包没发 npm 时返回 None。
    fn npm_latest(&self, package: &str) -> Result<Option<String>, AppError>;
}

/// 生产用的版本来源。客户端一轮只建一个：每个 agent 都要发一次 dist-tags 查询，
/// 现建现丢等于每次重新做 TCP + TLS 握手，还各带一条后台线程（与 `net::HttpFetcher`
/// 把 client 存进结构体同一口径）。
pub struct SystemVersionSource {
    client: reqwest::blocking::Client,
}

impl SystemVersionSource {
    pub fn new(proxy: ProxyConfig) -> Result<Self, AppError> {
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(concat!("cc-switch-x/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15));
        builder = proxy.apply_blocking(builder)?;
        Ok(Self {
            client: builder
                .build()
                .map_err(|error| AppError::Network(format!("创建版本查询客户端失败：{error}")))?,
        })
    }
}

impl VersionSource for SystemVersionSource {
    fn run_version(&self, program: &Path) -> Result<String, AppError> {
        // 参数只有这一个字面量，且程序路径由我们自己解析 —— 见模块头的边界说明。
        let output = hidden_command(program)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output()?;
        let text = if output.stdout.is_empty() {
            String::from_utf8_lossy(&output.stderr).to_string()
        } else {
            String::from_utf8_lossy(&output.stdout).to_string()
        };
        if !output.status.success() && text.trim().is_empty() {
            return Err(AppError::Update(format!(
                "{} --version 退出码 {}",
                program.display(),
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "未知".into())
            )));
        }
        Ok(text)
    }

    fn npm_latest(&self, package: &str) -> Result<Option<String>, AppError> {
        let url = npm_dist_tags_url(package);
        let response = self
            .client
            .get(&url)
            .send()
            .map_err(|error| AppError::Network(format!("查询 {package} 最新版失败：{error}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Network(format!(
                "查询 {package} 最新版失败：HTTP {status}"
            )));
        }
        let tags: serde_json::Value = response
            .json()
            .map_err(|error| AppError::Network(format!("{package} 的 dist-tags 读不懂：{error}")))?;
        Ok(tags
            .get("latest")
            .and_then(|value| value.as_str())
            .map(str::to_string))
    }
}

/// `@openai/codex` → `@openai%2fcodex`：scope 里的斜杠必须转义，否则被当成路径分隔。
fn npm_dist_tags_url(package: &str) -> String {
    format!(
        "https://registry.npmjs.org/-/package/{}/dist-tags",
        package.replace('/', "%2f")
    )
}

/// 查一轮版本：注册表里每个 agent 各探一次本机 CLI、各查一次上游。
/// 没有按 id 挑着查的入口 —— 会被执行的程序名只有注册表里的 `cli_program`。
pub fn check(home: &Path, source: &(dyn VersionSource + Sync)) -> AgentVersionReport {
    let agents = builtin_agents();
    // 每个 agent 都要等一个子进程退出，可能再等一次网络超时：串行等于把六段互不相关的
    // 等待首尾相接，npm 直连不通时最坏能吃满每个请求的 15s。各 agent 之间不共享可变状态，
    // 按 services::skill_discovery 同一口径用 scoped 线程并发，结果仍按注册表顺序收。
    let statuses = std::thread::scope(|scope| {
        let handles: Vec<_> = agents
            .iter()
            .map(|agent| {
                scope.spawn(move || {
                    // 解析路径这一步留在外面，check_one 因此可以在单测里被直接喂假路径。
                    let program = discover_cli(agent.cli_program, home);
                    check_one(agent, program.as_deref(), source)
                })
            })
            .collect();
        handles
            .into_iter()
            .zip(agents.iter())
            .map(|(handle, agent)| handle.join().unwrap_or_else(|_| aborted(agent)))
            .collect()
    });
    AgentVersionReport {
        agents: statuses,
        checked_at: chrono::Utc::now().timestamp_millis(),
    }
}

/// 一条账的起点：只有身份，其余全是「还不知道」。
fn blank_status(agent: &AgentDescriptor) -> AgentVersionStatus {
    AgentVersionStatus {
        id: agent.id.clone(),
        display_name: agent.display_name.clone(),
        cli_found: false,
        installed_version: None,
        latest_version: None,
        update_available: false,
        error: None,
    }
}

/// 探测线程 panic 时的兜底：这一格留下失败原因，其余照旧是「不知道」，
/// 不让整轮报告因为一个 agent 崩掉而消失。
fn aborted(agent: &AgentDescriptor) -> AgentVersionStatus {
    let mut status = blank_status(agent);
    status.error = Some("版本探测任务异常退出".into());
    status
}

fn check_one(
    agent: &AgentDescriptor,
    program: Option<&Path>,
    source: &dyn VersionSource,
) -> AgentVersionStatus {
    let mut status = blank_status(agent);
    status.cli_found = program.is_some();

    if let Some(path) = program {
        match source.run_version(path) {
            Ok(raw) => {
                status.installed_version = version::first_version_token(&raw).map(str::to_string)
            }
            Err(error) => status.error = Some(error.to_string()),
        }
    }

    if let Some(package) = agent.npm_package {
        match source.npm_latest(package) {
            Ok(latest) => status.latest_version = latest,
            // 已有失败原因时不覆盖：CLI 跑不起来比查不到最新版更值得让用户先看到。
            Err(error) => {
                if status.error.is_none() {
                    status.error = Some(error.to_string());
                }
            }
        }
    }

    if let (Some(installed), Some(latest)) = (&status.installed_version, &status.latest_version) {
        status.update_available = version::is_update_available(installed, latest);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// 假版本来源：不碰进程也不碰网络，只记被调用了几次。计数器要能跨线程用，
    /// 因为 `check` 会并发探测每个 agent。
    struct FakeSource {
        version_text: Option<String>,
        /// `None` 表示这一路查询失败（AppError 不是 Clone，没法直接存 Result）。
        latest: Option<String>,
        latest_failed: bool,
        spawned: AtomicUsize,
        requested: Mutex<Vec<String>>,
    }

    impl FakeSource {
        fn new(version_text: Option<&str>, latest: Option<&str>) -> Self {
            Self {
                version_text: version_text.map(str::to_string),
                latest: latest.map(str::to_string),
                latest_failed: false,
                spawned: AtomicUsize::new(0),
                requested: Mutex::new(Vec::new()),
            }
        }

        fn with_failed_lookup(version_text: Option<&str>) -> Self {
            Self {
                latest_failed: true,
                ..Self::new(version_text, None)
            }
        }

        fn spawned_count(&self) -> usize {
            self.spawned.load(Ordering::SeqCst)
        }

        fn requested_packages(&self) -> Vec<String> {
            self.requested.lock().unwrap().clone()
        }
    }

    impl VersionSource for FakeSource {
        fn run_version(&self, _program: &Path) -> Result<String, AppError> {
            self.spawned.fetch_add(1, Ordering::SeqCst);
            self.version_text
                .clone()
                .ok_or_else(|| AppError::Update("假来源没有输出".into()))
        }

        fn npm_latest(&self, package: &str) -> Result<Option<String>, AppError> {
            self.requested.lock().unwrap().push(package.to_string());
            if self.latest_failed {
                return Err(AppError::Network("查不到".into()));
            }
            Ok(self.latest.clone())
        }
    }

    fn descriptor(id: &str) -> AgentDescriptor {
        builtin_agents()
            .into_iter()
            .find(|agent| agent.id == id)
            .expect("注册表里应有该 agent")
    }

    #[test]
    fn a_missing_cli_is_never_spawned_and_never_claims_an_update() {
        let source = FakeSource::new(Some("9.9.9"), Some("9.9.9"));
        let status = check_one(&descriptor("claude"), None, &source);

        assert!(!status.cli_found);
        assert_eq!(source.spawned_count(), 0, "找不到命令就不该执行任何东西");
        assert_eq!(status.installed_version, None);
        // 没装也照样报上游最新版，但不说「有新版本」——本机没有可比较的基准。
        assert_eq!(status.latest_version.as_deref(), Some("9.9.9"));
        assert!(!status.update_available);
    }

    #[test]
    fn an_installed_cli_behind_the_registry_lights_the_update_flag() {
        let source = FakeSource::new(Some("claude 2.0.0 (Claude Code)"), Some("2.1.0"));
        let status = check_one(&descriptor("claude"), Some(Path::new("/x/claude")), &source);

        assert_eq!(status.installed_version.as_deref(), Some("2.0.0"));
        assert!(status.update_available);
        assert_eq!(status.error, None);
    }

    #[test]
    fn a_dev_build_is_not_pushed_to_upgrade() {
        let source = FakeSource::new(Some("0.22.1+238.gdeadbeef"), Some("9.9.9"));
        let status = check_one(&descriptor("codex"), Some(Path::new("/x/codex")), &source);

        assert_eq!(status.installed_version.as_deref(), Some("0.22.1+238.gdeadbeef"));
        assert!(!status.update_available, "开发构建和 latest 不同源，不能催升级");
    }

    #[test]
    fn a_failed_registry_call_keeps_the_installed_version_and_records_the_reason() {
        let source = FakeSource::with_failed_lookup(Some("1.0.0"));
        let status = check_one(&descriptor("codex"), Some(Path::new("/x/codex")), &source);

        assert_eq!(status.installed_version.as_deref(), Some("1.0.0"));
        assert_eq!(status.latest_version, None);
        assert!(!status.update_available);
        assert!(status.error.is_some(), "查不到要留下原因，不能静默当「已最新」");
    }

    #[test]
    fn one_round_covers_every_registered_agent_in_registry_order() {
        let temp = tempfile::tempdir().unwrap();
        let source = FakeSource::new(None, None);
        let report = check(temp.path(), &source);

        // 并发探测后仍要按注册表顺序交出去：面板按这个顺序逐行配对徽标。
        let ids: Vec<String> = report.agents.iter().map(|agent| agent.id.clone()).collect();
        assert_eq!(
            ids,
            builtin_agents()
                .into_iter()
                .map(|agent| agent.id)
                .collect::<Vec<String>>()
        );
    }

    #[test]
    fn an_agent_without_an_npm_package_never_asks_the_registry() {
        let source = FakeSource::new(None, Some("1.0.0"));
        let status = check_one(&descriptor("qoder-cn"), None, &source);

        assert_eq!(status.latest_version, None);
        assert!(
            source.requested_packages().is_empty(),
            "Qoder 不走 npm，不该发请求"
        );
    }

    #[test]
    fn scoped_package_names_are_escaped_in_the_url() {
        assert_eq!(
            npm_dist_tags_url("@openai/codex"),
            "https://registry.npmjs.org/-/package/@openai%2fcodex/dist-tags"
        );
    }
}
