//! Finder 启动的应用缺少 Shell 导出的变量；在管理操作使用时延迟读取。
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

pub(super) fn environment() -> Result<&'static BTreeMap<String, String>, String> {
    // shortcut: 每次启动只读取一次；支持运行中修改 Shell 配置时再加刷新。
    static ENVIRONMENT: OnceLock<Result<BTreeMap<String, String>, String>> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| {
            let shell = super::claude_terminal::macos_login_shell(
                std::env::var_os("SHELL").as_deref(),
                super::claude_terminal::macos_shell_record,
            )
            .map_err(|_| "无法识别用户登录 Shell".to_string())?;
            let shell = if matches!(
                shell.file_name().and_then(|name| name.to_str()),
                Some("sh" | "bash" | "zsh" | "dash" | "fish")
            ) {
                shell
            } else {
                "/bin/zsh".into()
            };
            let mut command = Command::new(shell);
            command
                .args([
                    "-lic",
                    "/usr/bin/printf '\\0AGMUX_SHELL_ENV\\0'; exec /usr/bin/env -0",
                ])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let child = command
                .spawn()
                .map_err(|_| "无法读取用户 Shell 环境".to_string())?;
            let output = super::plugins::wait_child_with_timeout(child, Duration::from_secs(5))
                .map_err(|_| "读取用户 Shell 环境失败".to_string())?
                .ok_or("读取用户 Shell 环境超时，请检查 Shell 启动配置后重启 Agmux")?;
            if !output.status.success() {
                return Err("读取用户 Shell 环境失败，请检查 Shell 启动配置后重启 Agmux".into());
            }
            let (_, values) = output
                .stdout
                .split_once("\0AGMUX_SHELL_ENV\0")
                .ok_or("用户 Shell 未返回环境变量")?;
            Ok(values
                .split('\0')
                .filter_map(|entry| entry.split_once('='))
                // 保留父进程的显式覆盖；调用方随后设置的配置与代理仍优先。
                .filter(|(name, _)| !name.is_empty() && std::env::var_os(name).is_none())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect())
        })
        .as_ref()
        .map_err(Clone::clone)
}
