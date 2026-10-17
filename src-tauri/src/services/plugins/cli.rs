//! 插件 CLI 执行层（codex 与 Claude Code 共用）：CLI 查找、超时分级与子进程输出排空。

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

/// codex CLI 子进程按操作类型分档限时：本地操作（remove 等）正常亚秒完成，
/// 卡住即异常，快速失败；联网操作（add/upgrade = git clone）给慢代理留足时间，
/// 但也不允许无限等待把 UI 卡死。
/// `plugin list` 实测并非纯本地读（2026-10-17 直跑 appserver CLI：闲置后首跑 34.7s
/// 出正确结果、随后亚秒，期间运行时缓存零写入——CLI 在出结果前先做网络新鲜度探测），
/// 归联网档，避免 20s 误杀慢网下合法完成的 list。
pub(super) const PLUGIN_CLI_TIMEOUT_FAST_SECS: u64 = 20;
pub(super) const PLUGIN_CLI_TIMEOUT_NETWORK_SECS: u64 = 60;

/// 是否联网操作（list 出结果前探测远端；add/upgrade/update 底层走 git，慢网/代理下耗时长）。
pub(super) fn is_network_plugin_op(args: &[&str]) -> bool {
    args == ["list"]
        || args
            .iter()
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
/// `is_network` 由调用方按各自引擎的动词表判定（codex 用 add/upgrade/update，Claude 用 install/update）。
pub(super) fn plugin_timeout_message(
    is_network: bool,
    timeout: Duration,
    client_cli: &str,
) -> String {
    if is_network {
        format!(
            "插件操作超时（{} 秒）：无法连通 GitHub。\
             请确认网络可访问 github.com，或在代理软件中开启系统代理 / TUN 模式后重试",
            timeout.as_secs()
        )
    } else {
        format!(
            "插件操作超时（{} 秒）：{client_cli} 长时间无响应，已终止。请重试；\
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

// ==================== Claude Code CLI 执行层 ====================

/// Claude Code CLI 探测链：`~/.local/bin` 原生安装优先，PATH 中的独立 CLI 兜底。
/// npm/winget 等包装安装不是原生形态（CLI 管理页同样不接管），这里不做它们的入口。
pub(super) fn find_claude_cli(home: &Path) -> Option<PathBuf> {
    let native = super::super::claude_cli::native_path(home);
    let file_name = native.file_name()?.to_os_string();
    if native.is_file() {
        return Some(native);
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(&file_name))
        .find(|path| path.is_file())
}

/// Claude 插件命令是否联网：install/update 底层走 git，marketplace add/update 拉取仓库。
fn is_claude_network_plugin_op(args: &[&str]) -> bool {
    matches!(
        args,
        ["install", ..]
            | ["update", ..]
            | ["marketplace", "add", ..]
            | ["marketplace", "update", ..]
    )
}

fn claude_plugin_timeout(args: &[&str]) -> Duration {
    Duration::from_secs(if is_claude_network_plugin_op(args) {
        PLUGIN_CLI_TIMEOUT_NETWORK_SECS
    } else {
        PLUGIN_CLI_TIMEOUT_FAST_SECS
    })
}

fn claude_plugin_cli_action(args: &[&str]) -> &'static str {
    match args {
        ["list", ..] => "读取已安装插件列表",
        ["install", ..] => "安装插件",
        ["uninstall", ..] => "卸载插件",
        ["enable", ..] => "启用插件",
        ["disable", ..] => "禁用插件",
        ["update", ..] => "升级插件",
        ["marketplace", "list"] => "读取插件市场列表",
        ["marketplace", "add", ..] => "添加插件市场",
        ["marketplace", "remove", ..] => "移除插件市场",
        ["marketplace", "update", ..] => "刷新插件市场",
        _ => "执行 Claude Code 插件命令",
    }
}

/// 插件 CLI 咽喉的统一留痕：成功 Debug、失败 Warn；client 打产品名区分两端。
fn log_plugin_cli_run(
    client: &str,
    action: &str,
    result: &AppResult<String>,
    start: std::time::Instant,
) {
    match result {
        Ok(_) => tauri_plugin_log::log::debug!(
            "[plugin.cli.run] client={client:?} outcome=success duration_ms={} msg=\"{action}成功\"",
            start.elapsed().as_millis()
        ),
        // 命令参数和 CLI 原文可能含 Git URL 凭证；错误仍返回界面，不写日志。
        Err(_) => tauri_plugin_log::log::warn!(
            "[plugin.cli.run] client={client:?} outcome=failure failure_kind=internal duration_ms={} msg=\"{action}失败\"",
            start.elapsed().as_millis()
        ),
    }
}

/// 插件 CLI 子进程共享内芯：进程构建、代理注入、限时与错误呈现。
/// CLI 定位、超时分档与动作标签由两端各自的包装负责（动词表不同：codex add/remove，Claude install/uninstall）。
fn run_plugin_cli_process(
    cli: &Path,
    args: &[&str],
    timeout: Duration,
    is_network: bool,
    client_cli: &str,
    cwd: Option<&Path>,
) -> AppResult<String> {
    let mut command = super::super::cli::command(cli);
    command.arg("plugin").args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    crate::network::Network::detect()
        .map_err(|error| app_err!("{error}"))?
        .apply(&mut command);
    let child = command
        .spawn()
        .map_err(|error| app_err!("执行{client_cli}失败: {error}"))?;
    let Some(output) = wait_child_with_timeout(child, timeout)
        .map_err(|error| app_err!("执行{client_cli}失败: {error}"))?
    else {
        return Err(app_err!(
            "{}",
            plugin_timeout_message(is_network, timeout, client_cli)
        ));
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
        return Err(app_err!("{client_cli} 报错：{detail}{hint}"));
    }
    Ok(stdout)
}

/// 跑 `codex plugin <args>`，返回 stdout；失败时把 CLI 的报错带出来，
/// 网络类失败追加代理提示；整体限时，防止 git 卡死拖住 UI。
/// 所有调用共用一个咽喉：成败各记一行留痕（Warn/Debug），排障不再靠猜。
pub(super) fn run_codex_plugin(home: &Path, args: &[&str]) -> AppResult<String> {
    let start = std::time::Instant::now();
    let result = run_codex_plugin_inner(home, args);
    log_plugin_cli_run("Codex", plugin_cli_action(args), &result, start);
    result
}

fn run_codex_plugin_inner(home: &Path, args: &[&str]) -> AppResult<String> {
    let cli = find_codex_cli(home).ok_or_else(|| {
        app_err!(
            "未找到 codex CLI（已尝试 ~/.codex/bin、PATH 与桌面版 appserver 目录），无法管理插件"
        )
    })?;
    run_plugin_cli_process(
        &cli,
        args,
        plugin_cli_timeout(args),
        is_network_plugin_op(args),
        "codex CLI",
        None,
    )
}

/// 跑 `claude plugin <args>`（Claude Code CLI，全部子命令带 --json 由调用方拼），
/// 形态与 `run_codex_plugin` 完全对称：限时、代理注入、错误呈现与留痕共用一套。
pub(super) fn run_claude_plugin(
    home: &Path,
    args: &[&str],
    cwd: Option<&Path>,
) -> AppResult<String> {
    let start = std::time::Instant::now();
    let result = run_claude_plugin_inner(home, args, cwd);
    log_plugin_cli_run(
        "Claude Code",
        claude_plugin_cli_action(args),
        &result,
        start,
    );
    result
}

fn run_claude_plugin_inner(home: &Path, args: &[&str], cwd: Option<&Path>) -> AppResult<String> {
    let cli = find_claude_cli(home).ok_or_else(|| {
        app_err!("未找到 Claude Code CLI（已尝试 ~/.local/bin 与 PATH），无法管理插件")
    })?;
    run_plugin_cli_process(
        &cli,
        args,
        claude_plugin_timeout(args),
        is_claude_network_plugin_op(args),
        "Claude Code CLI",
        cwd,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn plugin_process_runs_in_the_selected_project_directory() {
        let project = tempfile::tempdir().unwrap();
        let cli = project.path().join("fixture.cmd");
        std::fs::write(&cli, "@echo off\r\necho %CD%\r\n").unwrap();
        let output = run_plugin_cli_process(
            &cli,
            &["uninstall", "fixture"],
            Duration::from_secs(5),
            false,
            "fixture CLI",
            Some(project.path()),
        )
        .unwrap();
        assert_eq!(Path::new(output.trim()), project.path());
    }
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
        assert_eq!(
            plugin_cli_timeout(&["list"]).as_secs(),
            PLUGIN_CLI_TIMEOUT_NETWORK_SECS,
            "plugin list 出结果前有网络探测，冷跑可超 20s，不能按本地档误杀"
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
            true,
            Duration::from_secs(PLUGIN_CLI_TIMEOUT_NETWORK_SECS),
            "codex CLI",
        );
        assert!(network.contains("GitHub"));
        let local = plugin_timeout_message(
            false,
            Duration::from_secs(PLUGIN_CLI_TIMEOUT_FAST_SECS),
            "codex CLI",
        );
        assert!(!local.contains("GitHub"), "本地操作超时不能诊断为网络问题");
        assert!(local.contains("20"));
    }

    #[test]
    fn claude_plugin_timeout_and_actions_follow_verb_table() {
        // 联网档：install/update 与 marketplace add/update（git 拉取）
        for args in [
            &["install", "demo@ponytail"][..],
            &["update", "demo@ponytail"][..],
            &["marketplace", "add", "o/r"][..],
            &["marketplace", "update", "ponytail"][..],
        ] {
            assert_eq!(
                claude_plugin_timeout(args).as_secs(),
                PLUGIN_CLI_TIMEOUT_NETWORK_SECS,
                "{args:?} 应为联网档"
            );
        }
        // 本地档：list/enable/disable/uninstall 与 marketplace list/remove
        for args in [
            &["list", "--json", "--available"][..],
            &["enable", "demo@ponytail"][..],
            &["disable", "demo@ponytail"][..],
            &["uninstall", "demo@ponytail"][..],
            &["marketplace", "list"][..],
            &["marketplace", "remove", "ponytail"][..],
        ] {
            assert_eq!(
                claude_plugin_timeout(args).as_secs(),
                PLUGIN_CLI_TIMEOUT_FAST_SECS,
                "{args:?} 应为本地档"
            );
        }
        assert_eq!(
            claude_plugin_cli_action(&["uninstall", "demo@ponytail"]),
            "卸载插件"
        );
        assert_eq!(
            claude_plugin_cli_action(&["marketplace", "update", "ponytail"]),
            "刷新插件市场"
        );
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
