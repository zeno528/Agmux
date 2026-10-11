//! Claude Code 供应商终端：在新终端里起一个只属于某个供应商的 `claude` 会话。
//!
//! 机制是官方 `--settings` 的 per-invocation 覆盖：把该供应商的完整 env 写进
//! `.agofra/terminal/claude_<id>.json`，终端里跑 `claude --settings <该文件>`。
//! 因为是进程级 flag 而非全局文件，N 个终端就是 N 份互不干扰的配置。
//!
//! 只读旁路：不碰 `~/.claude/settings.json`、不改 live、不切激活、不触发回写。
//! 覆盖文件固定命名、每次覆写、不随进程删除——终端里能直接看到路径，便于排查。

use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "windows")]
use base64::Engine;
use serde_json::{Map, Value};
#[cfg(target_os = "windows")]
use std::path::PathBuf;

use super::claude::managed_env;
use super::AppContext;
use crate::database::StoredClaudeProfile;
use crate::error::{app_err, AppResult};
use crate::fsutil::atomic_write;
#[cfg(target_os = "macos")]
use crate::models::MacosTerminal;

/// 覆盖文件内容就是该供应商的完整 env，与「使用」写 live 同源（同一个 managed_env），
/// 所以终端里跑的供应商和卡片上显示的供应商必然一致。
fn terminal_settings_document(profile: &StoredClaudeProfile) -> AppResult<Vec<u8>> {
    let mut document = Map::new();
    document.insert("env".to_string(), Value::Object(managed_env(profile)));
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(document))
        .map_err(|error| app_err!("终端配置序列化失败: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn terminal_model(profile: &StoredClaudeProfile) -> String {
    profile
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            profile
                .raw_settings
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|settings| {
                    settings
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|model| !model.is_empty())
                        .map(str::to_owned)
                })
        })
        .unwrap_or_else(|| "default".to_string())
}

/// 终端启动脚本与 AppleScript 都会二次求值，路径里的引号和换行直接拒收。
/// 目录来自系统目录选择器，正常不会命中；命中说明调用方有 bug，宁可失败也不猜。
/// 目录不存在同样在这里报：让用户看到真实原因，而不是开出一个停在陌生目录的空终端。
fn checked_cwd(cwd: Option<&str>) -> AppResult<Option<&str>> {
    let Some(dir) = cwd.map(str::trim).filter(|dir| !dir.is_empty()) else {
        return Ok(None);
    };
    if dir.contains(['"', '\n', '\r', '\0']) {
        return Err(app_err!("工作目录包含非法字符"));
    }
    if !Path::new(dir).is_dir() {
        return Err(app_err!("工作目录已不存在: {dir}"));
    }
    Ok(Some(dir))
}

/// POSIX 单引号包裹：单引号内不做任何展开，`$`、反引号、`()` 都是字面量。
/// 只服务 macOS 的 Terminal.app 启动路径（cmd 用双引号），故与测试同 gate；
/// CI 的 test-macos job 负责在目标平台上跑它。
#[cfg(target_os = "macos")]
fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

impl AppContext {
    /// 写覆盖文件并在新终端里启动 `claude --settings <file>`。
    /// 目录缺失、终端起不来都返回 Err，由调用方透出给用户——这是用户主动点的操作。
    pub fn claude_open_terminal(&self, id: &str, cwd: Option<&str>) -> AppResult<()> {
        let profile = self.database.claude_profile(id)?;
        let dir = checked_cwd(cwd)?;

        self.paths.ensure()?;
        let settings = self.paths.terminal_settings(&profile.id);
        atomic_write(&settings, &terminal_settings_document(&profile)?)?;

        // 启动前回显覆盖文件路径：用户能立刻确认这个终端用的是哪份配置。
        let settings_text = settings.display().to_string();
        let banner = format!("Agofra Claude terminal: {settings_text}");
        let model = terminal_model(&profile);

        #[cfg(target_os = "windows")]
        launch_windows(dir, &settings_text, &banner, &model)?;
        #[cfg(target_os = "macos")]
        match self.settings()?.macos_terminal {
            MacosTerminal::Ghostty => launch_ghostty(dir, &settings_text, &banner, &model)?,
            MacosTerminal::Terminal => launch_macos(dir, &settings_text, &banner, &model)?,
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = (dir, &settings_text, &banner, &model);
            return Err(app_err!("当前平台不支持启动终端"));
        }

        tauri_plugin_log::log::info!(
            "[apply.claude_terminal] client=\"Claude Code\" profile_id={} profile={:?} outcome=success msg=\"已在新终端启动 Claude Code\"",
            profile.id,
            profile.name
        );
        Ok(())
    }
}

/// ShellExecuteW 交给系统启动独立 PowerShell，不继承开发进程的控制台输入。
/// 脚本用 UTF-16LE Base64 传入，避免 ShellExecuteW 再次拆分路径和引号。
#[cfg(target_os = "windows")]
fn launch_windows(dir: Option<&str>, settings: &str, banner: &str, model: &str) -> AppResult<()> {
    let shell = find_windows_shell().ok_or_else(|| app_err!("未找到 PowerShell"))?;
    let script = windows_launch_script(dir, settings, banner, model);
    shell_execute_script(&shell, &script)
}

#[cfg(target_os = "windows")]
fn shell_execute_script(shell: &str, script: &str) -> AppResult<()> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let args = format!("-NoExit -EncodedCommand {encoded}");
    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(shell),
            &HSTRING::from(args),
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize <= 32 {
        return Err(app_err!(
            "启动 PowerShell 失败: ShellExecuteW={}",
            result.0 as usize
        ));
    }
    Ok(())
}

/// 只从系统安装位置选择 PowerShell，避免继承开发工具 PATH 中的私有运行时。
/// 剔除 `%LOCALAPPDATA%\...\WindowsApps\pwsh.exe`（Store 执行别名）：它是 0 字节
/// reparse point，ShellExecuteW 拉起时 `-EncodedCommand` 参数会被别名重定向吞掉，
/// PowerShell 以纯交互模式启动——空窗口挂在继承的 cwd 上，脚本一行都没跑。
/// 只装 Store 版 PowerShell 的机器落到 System32 5.1（真实 exe，参数必达）。
#[cfg(target_os = "windows")]
fn find_windows_shell() -> Option<String> {
    [
        std::env::var_os("ProgramFiles")
            .map(|dir| PathBuf::from(dir).join("PowerShell/7/pwsh.exe")),
        std::env::var_os("WINDIR")
            .map(|dir| PathBuf::from(dir).join("System32/WindowsPowerShell/v1.0/powershell.exe")),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_file())
    .map(|path| path.to_string_lossy().into_owned())
}

/// `powershell -NoExit -EncodedCommand <脚本>` 的脚本文本。
///
/// **必须单行**：命令行里的换行会终止命令，-NoExit 起的窗口只执行第一行，
/// 后面的 Set-Location 与 claude 被静默丢弃（表现为窗口开了、只打印 banner、
/// 随即回到提示符）。因此一律用 `;` 串联。
///
/// 不用 `-NoProfile`：用户 profile 里的 PATH 才是 `claude` 能否被找到的关键
/// （GUI 启动的终端不继承登录 shell 的环境）。
///
/// 路径用 PowerShell 单引号（字面量，不展开 `$`），内嵌单引号按 PS 规则写两遍。
/// `&`、`|`、`$`、反引号在单引号内全部无害，只有 `'` 自身需要处理。
#[cfg(target_os = "windows")]
fn windows_launch_script(dir: Option<&str>, settings: &str, banner: &str, model: &str) -> String {
    // GUI 父进程可能缺少持久环境变量；只补缺失项，保留 shell/profile 的进程级覆盖。
    let mut script = String::from(
        "foreach ($scope in 'User','Machine') { \
         foreach ($entry in [Environment]::GetEnvironmentVariables($scope).GetEnumerator()) { \
         if (-not (Test-Path -LiteralPath ('Env:' + $entry.Key))) { \
         [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process') \
         } } }; Remove-Item Env:NO_COLOR,Env:TERM -ErrorAction SilentlyContinue; ",
    );
    if let Some(dir) = dir {
        script.push_str(&format!("Set-Location -LiteralPath {}; ", ps_quote(dir)));
    }
    script.push_str(&format!(
        "Write-Output {}; claude --settings {} --model {}",
        ps_quote(banner),
        ps_quote(settings),
        ps_quote(model)
    ));
    script
}

#[cfg(target_os = "windows")]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 两套 macOS 启动路径共用的 claude 调用行（posix 单引号字面量）。
#[cfg(target_os = "macos")]
fn claude_invocation(settings: &str, model: &str) -> String {
    format!(
        "claude --settings {} --model {}",
        posix_quote(settings),
        posix_quote(model)
    )
}

/// Ghostty 走官方 CLI `-e`（文档 config/reference：common -e flag）：脚本交给
/// `zsh -c`，工作目录用进程 current_dir 传递，脚本里不需要 cd。单实例时 CLI 经
/// IPC 转发给已有实例后立即退出；无实例时本进程常驻为 GUI——因此 spawn 发射
/// 不等待（与 Windows ShellExecuteW 同语义）。
#[cfg(target_os = "macos")]
fn ghostty_launch_script(banner: &str, settings: &str, model: &str) -> String {
    format!(
        "echo {}; {}",
        posix_quote(banner),
        claude_invocation(settings, model)
    )
}

#[cfg(target_os = "macos")]
fn launch_ghostty(dir: Option<&str>, settings: &str, banner: &str, model: &str) -> AppResult<()> {
    const GHOSTTY_BIN: &str = "/Applications/Ghostty.app/Contents/MacOS/ghostty";
    if !Path::new(GHOSTTY_BIN).is_file() {
        return Err(app_err!("未安装 Ghostty（/Applications/Ghostty.app）"));
    }
    let mut command = Command::new(GHOSTTY_BIN);
    command.args([
        "-e",
        "zsh",
        "-c",
        &ghostty_launch_script(banner, settings, model),
    ]);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command
        .spawn()
        .map_err(|error| app_err!("启动 Ghostty 失败: {error}"))?;
    Ok(())
}

/// 未装 Ghostty 时的兜底：Terminal.app 系统必带。
#[cfg(target_os = "macos")]
fn launch_macos(dir: Option<&str>, settings: &str, banner: &str, model: &str) -> AppResult<()> {
    let claude = claude_invocation(settings, model);
    let line = match dir {
        Some(dir) => format!("cd {} && {claude}", posix_quote(dir)),
        None => claude,
    };
    let script = format!("echo {}; {line}", posix_quote(banner));
    let applescript = format!(
        r#"tell application "Terminal"
    activate
    do script "{escaped}"
end tell"#,
        escaped = script.replace('\\', "\\\\").replace('"', "\\\"")
    );
    Command::new("osascript")
        .args(["-e", &applescript])
        .status()
        .map_err(|error| app_err!("启动终端失败: {error}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(app_err!("Terminal.app 启动失败"))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(
        kind: Option<&str>,
        token: Option<&str>,
        extra: Option<&str>,
    ) -> StoredClaudeProfile {
        StoredClaudeProfile {
            id: "p1".into(),
            name: "网关".into(),
            base_url: Some("https://gw.invalid".into()),
            auth_token: token.map(str::to_string),
            model: Some("claude-sonnet-5".into()),
            description: None,
            fetched_models: None,
            kind: kind.map(str::to_string),
            admin_url: None,
            extra_env: extra.map(str::to_string),
            raw_settings: None,
            icon: None,
            show_balance: false,
            sort_order: 0,
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    fn env_of(profile: &StoredClaudeProfile) -> Map<String, Value> {
        let text = String::from_utf8(terminal_settings_document(profile).unwrap()).unwrap();
        serde_json::from_str::<Value>(&text)
            .unwrap()
            .as_object()
            .unwrap()
            .get("env")
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn terminal_settings_carry_the_same_env_as_apply() {
        // 地址、模型、token 三项必须都在，终端才是那个供应商而不是默认配置。
        let env = env_of(&profile(Some("openrouter"), Some("sk-x"), None));
        assert_eq!(env["ANTHROPIC_BASE_URL"], "https://gw.invalid");
        assert_eq!(env["ANTHROPIC_MODEL"], "claude-sonnet-5");
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "sk-x");
    }

    #[test]
    fn terminal_model_uses_target_profile_instead_of_active_user_model() {
        let mut target = profile(Some("deepseek"), Some("sk-x"), None);
        target.model = None;
        target.raw_settings = Some(r#"{"model":"sonnet"}"#.into());
        assert!(!env_of(&target).contains_key("ANTHROPIC_MODEL"));
        assert_eq!(terminal_model(&target), "sonnet");

        target.raw_settings = None;
        assert_eq!(terminal_model(&target), "default");

        target.model = Some("deepseek-flash[1M]".into());
        assert_eq!(terminal_model(&target), "deepseek-flash[1M]");
    }

    #[test]
    fn api_key_kinds_never_also_carry_auth_token() {
        // 双 token 键会让 Claude Code 每次启动告警；两个形态必须互斥。
        let env = env_of(&profile(Some("anthropic"), Some("sk-x"), None));
        assert_eq!(env["ANTHROPIC_API_KEY"], "sk-x");
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));

        let env = env_of(&profile(Some("packy"), Some("sk-x"), None));
        assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], "sk-x");
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn blank_token_is_omitted_rather_than_written_empty() {
        let env = env_of(&profile(Some("packy"), Some("   "), None));
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
    }

    #[test]
    fn extra_env_merges_over_the_managed_keys() {
        let env = env_of(&profile(
            Some("packy"),
            Some("sk-x"),
            Some(r#"{"ANTHROPIC_CUSTOM_HEADERS":"X-Trace: 1"}"#),
        ));
        assert_eq!(env["ANTHROPIC_CUSTOM_HEADERS"], "X-Trace: 1");
    }

    #[test]
    fn cwd_rejects_characters_the_shell_cannot_safely_quote() {
        assert_eq!(checked_cwd(Some("  ")).unwrap(), None);
        assert_eq!(checked_cwd(None).unwrap(), None);
        assert!(checked_cwd(Some("D:\\a\"b")).is_err());
        assert!(checked_cwd(Some("D:\\a\nb")).is_err());
    }

    #[test]
    fn cwd_must_still_exist() {
        let here = std::env::temp_dir();
        assert_eq!(
            checked_cwd(Some(here.to_str().unwrap())).unwrap(),
            Some(here.to_str().unwrap())
        );
        assert!(checked_cwd(Some("D:\\definitely-not-here-9d2f")).is_err());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn posix_quote_neutralizes_expansion() {
        assert_eq!(posix_quote("/tmp/$(id)"), "'/tmp/$(id)'");
        assert_eq!(posix_quote("/tmp/it's"), r"'/tmp/it'\''s'");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn ghostty_script_quotes_paths_and_stays_single_line() {
        let script = ghostty_launch_script(
            "Agofra Claude terminal: /Users/u/.agofra/terminal/claude_p1.json",
            "/Users/u/.agofra/terminal/claude_p1.json",
            "sonnet's",
        );
        assert!(!script.contains('\n') && !script.contains('\r'));
        // posix_quote 用 '\'' 嵌入撇号：POSIX 单引号内无转义，相邻 '' 拼接会丢撇号
        assert_eq!(
            script,
            "echo 'Agofra Claude terminal: /Users/u/.agofra/terminal/claude_p1.json'; \
             claude --settings '/Users/u/.agofra/terminal/claude_p1.json' --model 'sonnet'\\''s'"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_script_is_a_single_line_so_nothing_is_silently_dropped() {
        // 回归：命令行里的换行会终止命令，-NoExit 窗口只执行第一行，
        // Set-Location 与 claude 被丢弃——表现为终端开了却只打印 banner 就回到提示符。
        let script = windows_launch_script(
            Some("D:\\GitHub project\\Agofra"),
            "C:\\Users\\u\\.agofra\\terminal\\claude_p1.json",
            "Agofra Claude terminal: C:\\Users\\u\\.agofra\\terminal\\claude_p1.json",
            "sonnet",
        );
        assert!(!script.contains('\n') && !script.contains('\r'));
        // PowerShell 5.1 不支持 &&，只能 ; 串联；换目录、banner、claude 必须同在一条里
        assert!(script
            .contains("Set-Location -LiteralPath 'D:\\GitHub project\\Agofra'; Write-Output "));
        assert!(script
            .ends_with("; claude --settings 'C:\\Users\\u\\.agofra\\terminal\\claude_p1.json' --model 'sonnet'"));
        assert!(!script.contains("&&"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_script_skips_set_location_when_no_directory_is_given() {
        let script = windows_launch_script(None, "C:\\t\\claude_p1.json", "banner", "default");
        assert!(!script.contains("Set-Location"));
        assert!(script.contains(
            "Remove-Item Env:NO_COLOR,Env:TERM -ErrorAction SilentlyContinue; Write-Output 'banner'; claude --settings "
        ));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_script_restores_missing_environment_before_starting_claude() {
        // 在系统环境读取边界提供 fixture，实际执行生成的启动脚本，不修改用户环境。
        let fixture = r#"
$ErrorActionPreference = 'Stop'
class Environment {
    static [System.Collections.IDictionary] GetEnvironmentVariables([string] $scope) {
        if ($scope -eq 'User') {
            return @{ AGOFRA_TEST_MCP_KEY = 'user'; AGOFRA_TEST_INHERITED = 'user' }
        }
        return @{ AGOFRA_TEST_MCP_KEY = 'machine'; AGOFRA_TEST_MACHINE = 'machine' }
    }
    static [void] SetEnvironmentVariable([string] $key, [string] $value, [string] $scope) {
        [System.Environment]::SetEnvironmentVariable($key, $value, $scope)
    }
}
function claude {
    if ($env:AGOFRA_TEST_MCP_KEY -ne 'user') { throw 'Missing user environment' }
    if ($env:AGOFRA_TEST_MACHINE -ne 'machine') { throw 'Missing machine environment' }
    if ($env:AGOFRA_TEST_INHERITED -ne 'process') { throw 'Process override lost' }
    if (($args -join '|') -ne '--settings|C:\t\profile.json|--model|sonnet') { throw 'Profile changed' }
    if ((Test-Path Env:NO_COLOR) -or (Test-Path Env:TERM)) { throw 'Terminal cleanup lost' }
    Write-Output 'claude environment verified'
}
"#;
        let script = format!(
            "{fixture}\n{}",
            windows_launch_script(None, r"C:\t\profile.json", "banner", "sonnet")
        );
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let output = std::process::Command::new(find_windows_shell().unwrap())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &base64::engine::general_purpose::STANDARD.encode(bytes),
            ])
            .env_remove("AGOFRA_TEST_MCP_KEY")
            .env_remove("AGOFRA_TEST_MACHINE")
            .env("AGOFRA_TEST_INHERITED", "process")
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("claude environment verified"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_paths_are_literal_and_do_not_expand() {
        // 单引号是 PowerShell 的字面量串：$ 与 & 都不展开；内嵌单引号写两遍
        let script =
            windows_launch_script(Some("D:\\a&b$c"), "C:\\t\\it's.json", "banner", "sonnet's");
        assert!(script.contains("Set-Location -LiteralPath 'D:\\a&b$c';"));
        assert!(script.contains("claude --settings 'C:\\t\\it''s.json'"));
        assert!(script.ends_with("--model 'sonnet''s'"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_shell_probe_actually_finds_a_powershell() {
        // 即使开发进程的 PATH 里有工具附带的 pwsh，也只选系统安装位置。
        let shell = find_windows_shell();
        assert!(shell.is_some(), "Windows 上必须能探测到 PowerShell");
        let shell = shell.unwrap();
        let path = Path::new(&shell);
        assert!(path.is_file());
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(matches!(name.as_ref(), "pwsh.exe" | "powershell.exe"));
        assert!(!shell.contains("codex-runtimes"));
    }
}
