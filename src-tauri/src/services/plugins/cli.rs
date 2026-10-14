//! codex CLI 执行层：CLI 查找、超时分级与子进程输出排空。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{app_err, AppResult};
// ==================== codex CLI 执行层 ====================

pub(super) fn cli_candidates(
    home: &Path,
    path_dirs: impl IntoIterator<Item = PathBuf>,
) -> Vec<PathBuf> {
    let file_name = super::super::codex_cli::file_name();
    let mut candidates = vec![home.join(".codex").join("bin").join(file_name)];
    candidates.extend(path_dirs.into_iter().map(|dir| dir.join(file_name)));
    candidates.push(
        home.join(".codex")
            .join("plugins")
            .join(".plugin-appserver")
            .join(file_name),
    );
    candidates
}

/// codex CLI 探测链：`~/.codex/bin`、PATH 中的独立 CLI、Desktop appserver 自带副本兜底。
pub(super) fn find_codex_cli(home: &Path) -> Option<PathBuf> {
    if let Some(path) = super::super::codex_cli::find_standalone(home) {
        return Some(path);
    }
    let path_dirs = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    cli_candidates(home, path_dirs)
        .into_iter()
        .find(|path| path.is_file())
}

// ==================== CLI 超时 ====================

/// codex CLI 子进程按操作类型分档限时：本地操作（list/remove）正常亚秒完成，
/// 卡住即异常，快速失败；联网操作（add/upgrade = git clone）给慢代理留足时间，
/// 但也不允许无限等待把 UI 卡死。
pub(super) const PLUGIN_CLI_TIMEOUT_FAST_SECS: u64 = 20;
pub(super) const PLUGIN_CLI_TIMEOUT_NETWORK_SECS: u64 = 60;

/// 是否联网操作（add/upgrade/update 底层走 git，慢网/代理下耗时长）。
pub(super) fn is_network_plugin_op(args: &[&str]) -> bool {
    args.iter()
        .any(|arg| matches!(*arg, "add" | "upgrade" | "update"))
}

pub(super) fn plugin_cli_timeout(args: &[&str]) -> Duration {
    Duration::from_secs(if is_network_plugin_op(args) {
        PLUGIN_CLI_TIMEOUT_NETWORK_SECS
    } else {
        PLUGIN_CLI_TIMEOUT_FAST_SECS
    })
}

/// 超时报错按档位区分：联网操作大概率是 GitHub 连通问题；
/// 本地操作（list/remove）卡住与网络无关，不能误导用户去查代理。
pub(super) fn plugin_timeout_message(args: &[&str], timeout: Duration) -> String {
    if is_network_plugin_op(args) {
        format!(
            "插件操作超时（{} 秒）：无法连通 GitHub。\
             请确认网络可访问 github.com，或在代理软件中开启系统代理 / TUN 模式后重试",
            timeout.as_secs()
        )
    } else {
        format!(
            "插件操作超时（{} 秒）：codex CLI 长时间无响应，已终止。请重试；\
             若持续出现请重启 Agmux 后再试",
            timeout.as_secs()
        )
    }
}

/// CLI 报错文本是否呈现为网络不可达（用于追加友好提示）。
pub(super) fn looks_like_network_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    [
        "unable to access",
        "could not resolve",
        "failed to connect",
        "connection refused",
        "timed out",
        "ssl",
        "terminated",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

/// `wait_child_with_timeout` 收集到的子进程输出。
pub(crate) struct ChildOutput {
    pub(crate) status: std::process::ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// 读空一根管道（在独立线程里跑，直到 EOF）。
pub(super) fn drain_pipe(mut pipe: Option<impl Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe.as_mut() {
        let _ = pipe.read_to_end(&mut bytes);
    }
    bytes
}

/// 等待子进程退出并收集 stdout/stderr，超时则 kill 并返回 `None`。
/// 排水线程必须在轮询前启动：OS 管道缓冲只有几 KB～几十 KB，
/// 若等进程退出后再读，大输出（如 `list --available --json` 的市场全量目录）
/// 会写满管道令子进程永远无法退出，只能等到超时被 kill。
pub(crate) fn wait_child_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> std::io::Result<Option<ChildOutput>> {
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || drain_pipe(stdout_pipe));
    let stderr_reader = std::thread::spawn(move || drain_pipe(stderr_pipe));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) if stdout_reader.is_finished() && stderr_reader.is_finished() => {
                break status
            }
            Some(_) if Instant::now() >= deadline => {
                // ponytail: 已退出父进程的孤儿无法可靠按 PID 归属，排水线程等 EOF；
                // 不无限 join 或误杀用户会话，需要处理此类 CLI 时再加进程组/Job Object。
                return Ok(None);
            }
            Some(_) => std::thread::sleep(Duration::from_millis(10)),
            None if Instant::now() >= deadline => {
                // 只终止本次子进程的后代；子进程继承管道时，仅 kill 父进程会让 join 卡住。
                let system = sysinfo::System::new_all();
                let mut owned =
                    std::collections::HashSet::from([sysinfo::Pid::from_u32(child.id())]);
                loop {
                    let previous = owned.len();
                    for (pid, process) in system.processes() {
                        if process
                            .parent()
                            .is_some_and(|parent| owned.contains(&parent))
                        {
                            owned.insert(*pid);
                        }
                    }
                    if owned.len() == previous {
                        break;
                    }
                }
                for pid in owned {
                    if pid.as_u32() != child.id() {
                        if let Some(process) = system.process(pid) {
                            let _ = process.kill();
                        }
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                // 不在超时分支无限 join：逃逸的后代也可能持有管道。
                return Ok(None);
            }
            None => std::thread::sleep(Duration::from_millis(150)),
        }
    };
    let stdout = String::from_utf8_lossy(&stdout_reader.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&stderr_reader.join().unwrap_or_default()).to_string();
    Ok(Some(ChildOutput {
        status,
        stdout,
        stderr,
    }))
}

fn plugin_cli_action(args: &[&str]) -> &'static str {
    match args {
        ["list"] => "读取已安装插件列表",
        ["marketplace", "list"] => "读取插件市场列表",
        ["marketplace", "add", ..] => "添加插件市场",
        ["marketplace", "remove", ..] => "移除插件市场",
        ["marketplace", "upgrade", ..] => "升级插件市场",
        ["add", ..] => "安装插件",
        ["remove", ..] => "卸载插件",
        _ => "执行 Codex 插件命令",
    }
}

/// 跑 `codex plugin <args>`，返回 stdout；失败时把 CLI 的报错带出来，
/// 网络类失败追加代理提示；整体限时，防止 git 卡死拖住 UI。
/// 所有调用共用一个咽喉：成败各记一行留痕（Warn/Debug），排障不再靠猜。
pub(super) fn run_codex_plugin(home: &Path, args: &[&str]) -> AppResult<String> {
    let start = std::time::Instant::now();
    let result = run_codex_plugin_inner(home, args);
    let command = format!("plugin {}", args.join(" "));
    let action = plugin_cli_action(args);
    match &result {
        Ok(_) => tauri_plugin_log::log::debug!(
            "[plugin.cli.run] command={command:?} outcome=success duration_ms={} msg=\"{action}成功\"",
            start.elapsed().as_millis()
        ),
        Err(error) => tauri_plugin_log::log::warn!(
            "[plugin.cli.run] command={command:?} outcome=failure failure_kind=internal duration_ms={} error={error:?} msg=\"{action}失败\"",
            start.elapsed().as_millis()
        ),
    }
    result
}

pub(super) fn run_codex_plugin_inner(home: &Path, args: &[&str]) -> AppResult<String> {
    let cli = find_codex_cli(home).ok_or_else(|| {
        app_err!(
            "未找到 codex CLI（已尝试 ~/.codex/bin、PATH 与桌面版 appserver 目录），无法管理插件"
        )
    })?;
    let mut command = super::super::cli::command(&cli);
    command.arg("plugin").args(args);
    crate::network::Network::detect()
        .map_err(|error| app_err!("{error}"))?
        .apply(&mut command);
    let child = command
        .spawn()
        .map_err(|error| app_err!("执行 codex CLI 失败: {error}"))?;
    let timeout = plugin_cli_timeout(args);
    let Some(output) = wait_child_with_timeout(child, timeout)
        .map_err(|error| app_err!("执行 codex CLI 失败: {error}"))?
    else {
        return Err(app_err!("{}", plugin_timeout_message(args, timeout)));
    };
    let ChildOutput {
        status,
        stdout,
        stderr,
    } = output;
    if !status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        let hint = if looks_like_network_error(detail) {
            "（疑似无法连通 GitHub：已尝试注入系统代理，若仍失败请确认代理软件已开启，或切换 TUN 模式后重试）"
        } else {
            ""
        };
        return Err(app_err!("codex CLI 报错：{detail}{hint}"));
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    #[test]
    fn cli_candidates_prefer_path_before_desktop_appserver() {
        let home = Path::new("/home/user");
        let candidates = cli_candidates(
            home,
            vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")],
        );
        let filename = crate::services::codex_cli::file_name();
        assert_eq!(candidates[0], home.join(".codex/bin").join(filename));
        assert_eq!(candidates[1], Path::new("/usr/local/bin").join(filename));
        assert_eq!(candidates[2], Path::new("/usr/bin").join(filename));
        assert_eq!(
            candidates[3],
            home.join(".codex/plugins/.plugin-appserver").join(filename)
        );
    }

    #[test]
    fn plugin_cli_timeout_and_network_error_detection() {
        assert_eq!(
            plugin_cli_timeout(&["marketplace", "add", "o/r"]).as_secs(),
            PLUGIN_CLI_TIMEOUT_NETWORK_SECS
        );
        assert_eq!(
            plugin_cli_timeout(&["marketplace", "list"]).as_secs(),
            PLUGIN_CLI_TIMEOUT_FAST_SECS
        );
        assert!(looks_like_network_error(
            "fatal: unable to access 'https://github.com/': Failed to connect"
        ));
        assert!(!looks_like_network_error("marketplace already added"));
    }

    #[test]
    fn plugin_cli_action_describes_the_command() {
        assert_eq!(plugin_cli_action(&["list"]), "读取已安装插件列表");
        assert_eq!(plugin_cli_action(&["add", "demo@market"]), "安装插件");
        assert_eq!(
            plugin_cli_action(&["marketplace", "remove", "market"]),
            "移除插件市场"
        );
    }

    #[test]
    fn plugin_timeout_message_matches_operation_tier() {
        let network = plugin_timeout_message(
            &["marketplace", "add", "o/r"],
            Duration::from_secs(PLUGIN_CLI_TIMEOUT_NETWORK_SECS),
        );
        assert!(network.contains("GitHub"));
        let local =
            plugin_timeout_message(&["list"], Duration::from_secs(PLUGIN_CLI_TIMEOUT_FAST_SECS));
        assert!(!local.contains("GitHub"), "本地操作超时不能诊断为网络问题");
        assert!(local.contains("20"));
    }

    /// 管道排水回归测试的子进程入口：父测试以本环境变量拉起当前测试二进制，
    /// 正常跑测试时无此变量，直接通过。
    const TEST_CHILD_MODE_ENV: &str = "AGMUX_PLUGINS_TEST_CHILD_MODE";
    #[test]
    fn plugins_test_child_entry() {
        let Some(mode) = std::env::var(TEST_CHILD_MODE_ENV).ok() else {
            return;
        };
        match mode.as_str() {
            // 向 stdout 写 2 MB，远超任何 OS 管道缓冲
            "firehose" => {
                use std::io::Write;
                let chunk = "x".repeat(1024);
                let mut stdout = std::io::stdout().lock();
                for _ in 0..2048 {
                    writeln!(stdout, "{chunk}").ok();
                }
                stdout.flush().ok();
            }
            // 挂住不退出，验证超时 kill 路径
            "stall" => std::thread::sleep(Duration::from_secs(60)),
            "inherited_pipe" => {
                let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["plugins_test_child_entry", "--nocapture"])
                    .env(TEST_CHILD_MODE_ENV, "brief_stall")
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .unwrap();
                // fixture 父进程立刻退出；后代继承管道，随后自行结束。
                let _ = child.try_wait();
            }
            "brief_stall" => std::thread::sleep(Duration::from_secs(2)),
            _ => {}
        }
        std::process::exit(0);
    }

    fn spawn_test_child(mode: &str) -> std::process::Child {
        std::process::Command::new(std::env::current_exe().unwrap())
            .arg("plugins_test_child_entry")
            .arg("--nocapture")
            .env(TEST_CHILD_MODE_ENV, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn wait_child_drains_large_output_without_deadlock() {
        let started = Instant::now();
        let output = wait_child_with_timeout(spawn_test_child("firehose"), Duration::from_secs(15))
            .unwrap()
            .expect("大输出必须靠预排水线程收完，否则子进程写满管道永不退出");
        assert!(output.status.success());
        // 子进程输出 2048 行 × 1025 字节 ≈ 2 MB（测试框架自身会附加少量字节，
        // 故用阈值断言：写满管道若未预排水，只能收到 ~64 KB 残留且走超时路径）
        assert!(
            output.stdout.len() > 2_000_000,
            "应完整收下全部输出，实际 {} 字节",
            output.stdout.len()
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "不应等到超时才返回"
        );
    }

    #[test]
    fn exited_parent_with_inherited_pipes_cannot_bypass_deadline() {
        let started = Instant::now();
        assert!(wait_child_with_timeout(
            spawn_test_child("inherited_pipe"),
            Duration::from_millis(500)
        )
        .unwrap()
        .is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn wait_child_kills_stalled_process_on_deadline() {
        let started = Instant::now();
        let outcome =
            wait_child_with_timeout(spawn_test_child("stall"), Duration::from_secs(2)).unwrap();
        assert!(outcome.is_none(), "挂死的子进程应按超时终止");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "应在超时时间点附近返回"
        );
    }
}
