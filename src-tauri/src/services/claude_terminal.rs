//! Claude Code 供应商终端：激活项跟随 live，其他项使用独立覆盖。
//!
//! 激活项进入项目后直接执行 `claude`，使用默认全局配置；非激活项把配置写进
//! `.agmux/terminal/claude_<id>.json` 并用 `--settings` 指向该文件，只加载 project/local。
//! 非激活项的配置生成复用激活逻辑；不加载 user settings，避免当前激活项的 Hooks、权限等混入。
//! `--add-dir` 只引入共用 Skills，MCP 经 `--mcp-config ~/.claude.json` 读取。
//! 保留默认 Claude 用户目录，共用登录、历史会话与信任记录；不跳过权限检查。
//! `--settings` 同名字段按官方覆盖语义高于 project/local，不另造优先级规则。
//! 非激活项的覆盖文件只影响该供应商终端；激活项的配置加载与更新交由 Claude Code 处理。
//!
//! 只读旁路：不碰 `~/.claude/settings.json`、不改 live、不切激活、不触发回写。
//! 非激活项覆盖文件固定命名、每次覆写、不随进程删除；终端里会回显配置路径。

use std::path::Path;
#[cfg(any(windows, target_os = "macos"))]
use std::process::Command;

#[cfg(target_os = "windows")]
use base64::Engine;
use serde_json::Value;
#[cfg(target_os = "windows")]
use std::path::PathBuf;

use super::claude::claude_settings_bytes;
use super::AppContext;
#[cfg(test)]
use crate::database::StoredClaudeProfile;
use crate::error::{app_err, AppResult};
use crate::fsutil::atomic_write;
#[cfg(target_os = "macos")]
use crate::models::MacosTerminal;

// 附加目录只暴露 Skills，避免 --add-dir 用户主目录扩大工作区访问范围。
fn prepare_shared_skills(terminal: &Path, source: &Path) -> AppResult<std::path::PathBuf> {
    let shared = terminal.join("shared");
    let link = shared.join(".claude/skills");
    std::fs::create_dir_all(source)
        .map_err(|error| app_err!("无法创建共用 Skills 目录: {error}"))?;
    std::fs::create_dir_all(link.parent().expect("资源目录必须有父目录"))
        .map_err(|error| app_err!("无法创建 Skills 入口目录: {error}"))?;
    if !link.exists() {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output =
                Command::new(find_windows_shell().ok_or_else(|| app_err!("未找到 PowerShell"))?)
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        &format!(
                    "New-Item -ItemType Junction -Path {} -Target {} -ErrorAction Stop | Out-Null",
                    ps_quote(&link.to_string_lossy()), ps_quote(&source.to_string_lossy()),
                ),
                    ])
                    .creation_flags(0x08000000)
                    .output()
                    .map_err(|error| app_err!("无法连接共用 Skills 目录: {error}"))?;
            if !output.status.success() {
                return Err(app_err!("无法连接共用 Skills 目录: {}", link.display()));
            }
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(source, &link)
            .map_err(|error| app_err!("无法连接共用 Skills 目录: {error}"))?;
    }
    let linked = std::fs::canonicalize(&link)
        .map_err(|error| app_err!("无法读取 Skills 入口目录: {error}"))?;
    let original = std::fs::canonicalize(source)
        .map_err(|error| app_err!("无法读取共用 Skills 目录: {error}"))?;
    if linked != original {
        return Err(app_err!("共用 Skills 入口指向不一致: {}", link.display()));
    }
    Ok(shared)
}

fn terminal_mcp_config(path: &Path) -> AppResult<Option<String>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(app_err!("无法读取 MCP 配置 {}: {error}", path.display())),
    };
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| app_err!("MCP 配置解析失败 {}: {error}", path.display()))?;
    let root = document
        .as_object()
        .ok_or_else(|| app_err!("MCP 配置必须为 JSON 对象: {}", path.display()))?;
    match root.get("mcpServers") {
        None => Ok(None),
        Some(Value::Object(_)) => Ok(Some(path.to_string_lossy().into_owned())),
        Some(_) => Err(app_err!("mcpServers 必须为 JSON 对象: {}", path.display())),
    }
}

fn terminal_model(bytes: &[u8]) -> AppResult<String> {
    let settings: Value =
        serde_json::from_slice(bytes).map_err(|error| app_err!("终端配置解析失败: {error}"))?;
    let model = [&settings["env"]["ANTHROPIC_MODEL"], &settings["model"]]
        .into_iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .find(|model| !model.is_empty())
        .unwrap_or("default");
    Ok(model.into())
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
/// macOS 的 Terminal.app 与 Ghostty 启动路径共用。
#[cfg(not(windows))]
fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

impl AppContext {
    /// 激活项直接启动 Claude Code；非激活项写覆盖文件再启动终端。
    /// 目录缺失、终端起不来都返回 Err，由调用方透出给用户——这是用户主动点的操作。
    pub fn claude_open_terminal(&self, id: &str, cwd: Option<&str>) -> AppResult<()> {
        let profile = self.database.claude_profile(id)?;
        let dir = checked_cwd(cwd)?;

        let active = self.database.active_claude_profile()?.as_deref() == Some(id);
        let (arguments, banner) = terminal_launch_spec(active, || {
            self.paths.ensure()?;
            let live = self.paths.claude_home.join("settings.json");
            let settings = self.paths.terminal_settings(&profile.id);
            let bytes = claude_settings_bytes(
                Some(&profile),
                &live,
                &self.claude_previous_extra_keys(id)?,
            )?;
            let model = terminal_model(&bytes)?;
            atomic_write(&settings, &bytes)?;
            let shared_skills = prepare_shared_skills(
                &self.paths.terminal,
                &self.paths.claude_home.join("skills"),
            )?;
            let settings_text = settings.display().to_string();
            let mcp_path = self.paths.claude_mcp_config();
            let mcp_config = terminal_mcp_config(&mcp_path)?;
            Ok((
                claude_arguments(
                    &settings_text,
                    &model,
                    mcp_config.as_deref(),
                    &shared_skills.to_string_lossy(),
                ),
                format!("Agmux Claude terminal: {settings_text}"),
            ))
        })?;

        #[cfg(target_os = "windows")]
        launch_windows(dir, &banner, &arguments)?;
        #[cfg(target_os = "macos")]
        match self.settings()?.macos_terminal {
            MacosTerminal::Ghostty => launch_ghostty(dir, &banner, &arguments)?,
            MacosTerminal::Terminal => launch_macos(dir, &banner, &arguments)?,
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = (dir, &banner, &arguments);
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

/// 激活项沿用全局配置，终端不注入参数；非激活项才生成覆盖文件参数。
fn terminal_launch_spec(
    active: bool,
    profile_terminal: impl FnOnce() -> AppResult<(String, String)>,
) -> AppResult<(String, String)> {
    if active {
        Ok((
            String::new(),
            "Agmux Claude terminal: global settings".into(),
        ))
    } else {
        profile_terminal()
    }
}

fn claude_arguments(
    settings: &str,
    model: &str,
    mcp_config: Option<&str>,
    shared_skills: &str,
) -> String {
    #[cfg(not(windows))]
    use posix_quote as quote;
    #[cfg(windows)]
    use ps_quote as quote;
    format!(
        " --settings {} --setting-sources 'project,local' --add-dir {} {}--model {}",
        quote(settings),
        quote(shared_skills),
        mcp_config
            .map(|path| format!("--mcp-config {} ", quote(path)))
            .unwrap_or_default(),
        quote(model)
    )
}

/// ShellExecuteW 交给系统启动独立 PowerShell，不继承开发进程的控制台输入。
/// 脚本用 UTF-16LE Base64 传入，避免 ShellExecuteW 再次拆分路径和引号。
#[cfg(target_os = "windows")]
fn launch_windows(dir: Option<&str>, banner: &str, arguments: &str) -> AppResult<()> {
    let shell = find_windows_shell().ok_or_else(|| app_err!("未找到 PowerShell"))?;
    let script = windows_launch_script(dir, banner, arguments);
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
fn windows_launch_script(dir: Option<&str>, banner: &str, arguments: &str) -> String {
    // GUI 父进程可能缺少持久环境变量；只补缺失项，保留 shell/profile 的进程级覆盖。
    let mut script = String::from(
        "foreach ($scope in 'User','Machine') { \
         foreach ($entry in [Environment]::GetEnvironmentVariables($scope).GetEnumerator()) { \
         if (-not (Test-Path -LiteralPath ('Env:' + $entry.Key))) { \
         [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process') \
         } } }; Remove-Item Env:NO_COLOR,Env:TERM,Env:CLAUDE_CONFIG_DIR,Env:CLAUDE_SECURESTORAGE_CONFIG_DIR,Env:ANTHROPIC_BASE_URL,Env:ANTHROPIC_AUTH_TOKEN,Env:ANTHROPIC_API_KEY,Env:ANTHROPIC_MODEL -ErrorAction SilentlyContinue; ",
    );
    if let Some(dir) = dir {
        script.push_str(&format!("Set-Location -LiteralPath {}; ", ps_quote(dir)));
    }
    script.push_str(&format!(
        "Write-Output {}; claude{arguments}",
        ps_quote(banner),
    ));
    script
}

#[cfg(target_os = "windows")]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// 两套 macOS 启动路径共用的 claude 调用行（posix 单引号字面量）。
#[cfg(target_os = "macos")]
fn claude_invocation(arguments: &str) -> String {
    format!(
        "env -u CLAUDE_CONFIG_DIR -u CLAUDE_SECURESTORAGE_CONFIG_DIR -u ANTHROPIC_BASE_URL -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY -u ANTHROPIC_MODEL claude{arguments}"
    )
}

/// Ghostty 走官方 CLI `-e`（文档 config/reference：common -e flag）：脚本交给
/// `zsh -c`，工作目录用进程 current_dir 传递，脚本里不需要 cd。单实例时 CLI 经
/// IPC 转发给已有实例后立即退出；无实例时本进程常驻为 GUI——因此 spawn 发射
/// 不等待（与 Windows ShellExecuteW 同语义）。
#[cfg(target_os = "macos")]
fn ghostty_launch_script(banner: &str, arguments: &str) -> String {
    format!(
        "echo {}; {}",
        posix_quote(banner),
        claude_invocation(arguments)
    )
}

#[cfg(target_os = "macos")]
fn launch_ghostty(dir: Option<&str>, banner: &str, arguments: &str) -> AppResult<()> {
    const GHOSTTY_BIN: &str = "/Applications/Ghostty.app/Contents/MacOS/ghostty";
    if !Path::new(GHOSTTY_BIN).is_file() {
        return Err(app_err!("未安装 Ghostty（/Applications/Ghostty.app）"));
    }
    let mut command = Command::new(GHOSTTY_BIN);
    command.args(["-e", "zsh", "-c", &ghostty_launch_script(banner, arguments)]);
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
fn launch_macos(dir: Option<&str>, banner: &str, arguments: &str) -> AppResult<()> {
    let claude = claude_invocation(arguments);
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
    use serde_json::Map;

    fn terminal_settings_document(profile: &StoredClaudeProfile) -> AppResult<Vec<u8>> {
        let root = tempfile::tempdir().unwrap();
        claude_settings_bytes(Some(profile), &root.path().join("settings.json"), &[])
    }

    #[test]
    fn terminal_mcp_config_skips_unconfigured_servers_and_preserves_live_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".claude.json");
        assert_eq!(terminal_mcp_config(&path).unwrap(), None);
        for document in [r#"{}"#, r#"{"numStartups":1,"projects":{}}"#] {
            std::fs::write(&path, document).unwrap();
            let config = terminal_mcp_config(&path).unwrap();
            assert_eq!(config, None);
            let arguments =
                claude_arguments("settings.json", "default", config.as_deref(), "shared");
            assert!(!arguments.contains("--mcp-config"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), document);
        }
        for document in [
            r#"{"mcpServers":{}}"#,
            r#"{"mcpServers":{"fixture":{"command":"fixture"}}}"#,
        ] {
            std::fs::write(&path, document).unwrap();
            let config = terminal_mcp_config(&path).unwrap();
            assert_eq!(config.as_deref(), Some(path.to_str().unwrap()));
            assert!(
                claude_arguments("settings.json", "default", config.as_deref(), "shared")
                    .contains("--mcp-config")
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), document);
        }
        for document in [
            "broken",
            "null",
            r#"{"mcpServers":null}"#,
            r#"{"mcpServers":[]}"#,
        ] {
            std::fs::write(&path, document).unwrap();
            assert!(terminal_mcp_config(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), document);
        }
    }

    #[test]
    fn shared_skills_entry_uses_live_files_without_creating_a_claude_home() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join(".claude/skills");
        let terminal = root.path().join("terminal");
        let shared = prepare_shared_skills(&terminal, &source).unwrap();
        assert_eq!(prepare_shared_skills(&terminal, &source).unwrap(), shared);
        std::fs::write(source.join("SKILL.md"), "before").unwrap();
        assert_eq!(
            std::fs::read_to_string(shared.join(".claude/skills/SKILL.md")).unwrap(),
            "before"
        );
        std::fs::write(source.join("SKILL.md"), "after").unwrap();
        assert_eq!(
            std::fs::read_to_string(shared.join(".claude/skills/SKILL.md")).unwrap(),
            "after"
        );
        assert!(!shared.join(".claude.json").exists());
        assert!(!shared.join("settings.json").exists());
        assert!(!shared.join("projects").exists());
        let wrong = root.path().join("wrong");
        std::fs::create_dir_all(wrong.join("shared/.claude/skills")).unwrap();
        assert!(prepare_shared_skills(&wrong, &source).is_err());
    }

    #[test]
    fn empty_snapshot_uses_the_same_initial_settings_as_activation_without_changing_live() {
        let root = tempfile::tempdir().unwrap();
        let live = root.path().join("settings.json");
        std::fs::write(&live, r#"{"env":{"ANTHROPIC_API_KEY":"old","OLD_EXTRA":"old"},"hooks":{"Stop":[]},"enabledPlugins":{"fixture":true}}"#).unwrap();
        let before = std::fs::read(&live).unwrap();
        let mut target = profile(Some("deepseek"), Some("target"), None);
        target.raw_settings = Some("{}".into());
        let document: Value = serde_json::from_slice(
            &claude_settings_bytes(Some(&target), &live, &["OLD_EXTRA".into()]).unwrap(),
        )
        .unwrap();
        assert_eq!(document["env"]["ANTHROPIC_AUTH_TOKEN"], "target");
        assert!(document["env"].get("ANTHROPIC_API_KEY").is_none());
        assert!(document["env"].get("OLD_EXTRA").is_none());
        assert_eq!(document["enabledPlugins"]["fixture"], true);
        assert!(document["hooks"].get("Stop").is_some());
        assert_eq!(std::fs::read(&live).unwrap(), before);
        std::fs::write(&live, "broken").unwrap();
        assert!(claude_settings_bytes(Some(&target), &live, &[]).is_err());
        assert_eq!(std::fs::read_to_string(&live).unwrap(), "broken");
    }

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
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
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
    fn non_trivial_snapshot_carries_the_full_document() {
        // 非平凡快照整文档原样进覆盖文件（对齐 apply 的整文件替换）：hooks、
        // permissions 等非 env 键随配置走，env 保持快照原文、不二次合并表单字段。
        let mut target = profile(Some("packy"), Some("sk-x"), None);
        target.raw_settings = Some(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://captured.example","ANTHROPIC_AUTH_TOKEN":"captured-tok","CUSTOM":"keep"},"hooks":{"SessionStart":[]},"permissions":{"allow":["Bash(ls)"]}}"#
                .into(),
        );
        let document: Value =
            serde_json::from_slice(&terminal_settings_document(&target).unwrap()).unwrap();
        assert_eq!(
            document["env"]["ANTHROPIC_BASE_URL"],
            "https://captured.example"
        );
        assert_eq!(document["env"]["CUSTOM"], "keep");
        assert!(document.get("hooks").is_some());
        assert_eq!(document["permissions"]["allow"][0], "Bash(ls)");
    }

    #[test]
    fn claude_account_snapshot_drops_auth_overrides() {
        // 账号模式与 apply 同一道清理：快照里的供应商凭证键必须撤下，其余键保留。
        let mut target = profile(Some("claude-account"), None, None);
        target.raw_settings = Some(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.example","ANTHROPIC_AUTH_TOKEN":"tok"},"model":"sonnet"}"#
                .into(),
        );
        let document: Value =
            serde_json::from_slice(&terminal_settings_document(&target).unwrap()).unwrap();
        let env = document["env"].as_object().unwrap();
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(!env.contains_key("ANTHROPIC_BASE_URL"));
        assert_eq!(document["model"], "sonnet");
    }

    #[test]
    fn trivial_snapshot_stays_env_only() {
        // 没有现场文件时，平凡快照只包含该供应商的 env。
        let mut target = profile(Some("packy"), Some("sk-x"), None);
        target.raw_settings = Some("{}".into());
        let document: Value =
            serde_json::from_slice(&terminal_settings_document(&target).unwrap()).unwrap();
        assert_eq!(document["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-x");
        assert_eq!(document.as_object().unwrap().len(), 1);
    }

    #[test]
    fn terminal_model_uses_target_profile_instead_of_active_user_model() {
        let model = |target: &StoredClaudeProfile| {
            terminal_model(&terminal_settings_document(target).unwrap()).unwrap()
        };
        let mut target = profile(Some("deepseek"), Some("sk-x"), None);
        target.model = None;
        target.raw_settings = Some(r#"{"model":"sonnet"}"#.into());
        assert!(!env_of(&target).contains_key("ANTHROPIC_MODEL"));
        assert_eq!(model(&target), "sonnet");

        target.raw_settings = None;
        assert_eq!(model(&target), "default");

        target.model = Some("deepseek-flash[1M]".into());
        assert_eq!(model(&target), "deepseek-flash[1M]");
        target.extra_env =
            Some(r#"{"ANTHROPIC_DEFAULT_SONNET_MODEL":"deepseek-flash[1M]"}"#.into());
        assert_eq!(model(&target), "deepseek-flash[1M]");
        assert_eq!(env_of(&target)["ANTHROPIC_MODEL"], "deepseek-flash[1M]");
        target.raw_settings = Some(r#"{"env":{"ANTHROPIC_MODEL":"sonnet","ANTHROPIC_DEFAULT_SONNET_MODEL":"other-model[1M]"}}"#.into());
        assert_eq!(model(&target), "sonnet");
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
            "Agmux Claude terminal: /Users/u/.agmux/terminal/claude_p1.json",
            &claude_arguments(
                "/Users/u/.agmux/terminal/claude_p1.json",
                "sonnet's",
                Some("/Users/u's home/.claude.json"),
                "/Users/u/.agmux/terminal/shared",
            ),
        );
        assert!(!script.contains('\n') && !script.contains('\r'));
        // posix_quote 用 '\'' 嵌入撇号：POSIX 单引号内无转义，相邻 '' 拼接会丢撇号
        assert_eq!(
            script,
            "echo 'Agmux Claude terminal: /Users/u/.agmux/terminal/claude_p1.json'; \
             env -u CLAUDE_CONFIG_DIR -u CLAUDE_SECURESTORAGE_CONFIG_DIR -u ANTHROPIC_BASE_URL -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY -u ANTHROPIC_MODEL claude --settings '/Users/u/.agmux/terminal/claude_p1.json' \
             --setting-sources 'project,local' --add-dir '/Users/u/.agmux/terminal/shared' --mcp-config '/Users/u'\\''s home/.claude.json' --model 'sonnet'\\''s'"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn active_terminal_uses_default_claude_settings() {
        let script = ghostty_launch_script("banner", "");
        assert!(script.ends_with(" claude"));
        assert!(!script.contains("--settings"));
        assert!(!script.contains("--model"));
    }

    #[test]
    fn active_terminal_skips_the_profile_override_file() {
        // 激活项不能读 live 之外的覆盖文件；闭包被调用即说明分支选错了。
        let (arguments, banner) = terminal_launch_spec(true, || {
            unreachable!("active profile must not build override arguments")
        })
        .unwrap();
        assert!(arguments.is_empty());
        assert_eq!(banner, "Agmux Claude terminal: global settings");

        let (arguments, banner) =
            terminal_launch_spec(false, || Ok((" --settings 'x'".into(), "banner".into())))
                .unwrap();
        assert_eq!(arguments, " --settings 'x'");
        assert_eq!(banner, "banner");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn active_terminal_enters_project_and_starts_claude_without_arguments() {
        let project = tempfile::tempdir().unwrap();
        // 用目录内标记文件判断位置：CI 的 TEMP 可能是 8.3 短路径，直接比字符串不可靠。
        std::fs::write(project.path().join("agmux-terminal-marker.txt"), "here").unwrap();
        let fixture = r#"
$ErrorActionPreference = 'Stop'
function claude {
    if ($args.Count -ne 0) { throw 'Active profile must use default settings' }
    if (-not (Test-Path -LiteralPath '.\agmux-terminal-marker.txt')) { throw 'Wrong project directory' }
    Write-Output 'default claude verified'
}
"#;
        let script = format!(
            "{fixture}\n{}",
            windows_launch_script(Some(project.path().to_str().unwrap()), "banner", "")
        );
        let output = Command::new(find_windows_shell().unwrap())
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("default claude verified"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_script_is_a_single_line_so_nothing_is_silently_dropped() {
        // 回归：命令行里的换行会终止命令，-NoExit 窗口只执行第一行，
        // Set-Location 与 claude 被丢弃——表现为终端开了却只打印 banner 就回到提示符。
        let script = windows_launch_script(
            Some("D:\\GitHub project\\Agmux"),
            "Agmux Claude terminal: C:\\Users\\u\\.agmux\\terminal\\claude_p1.json",
            &claude_arguments(
                "C:\\Users\\u\\.agmux\\terminal\\claude_p1.json",
                "sonnet",
                None,
                "C:\\t\\shared",
            ),
        );
        assert!(!script.contains('\n') && !script.contains('\r'));
        // PowerShell 5.1 不支持 &&，只能 ; 串联；换目录、banner、claude 必须同在一条里
        assert!(
            script.contains("Set-Location -LiteralPath 'D:\\GitHub project\\Agmux'; Write-Output ")
        );
        assert!(script.ends_with(
            "; claude --settings 'C:\\Users\\u\\.agmux\\terminal\\claude_p1.json' \
             --setting-sources 'project,local' --add-dir 'C:\\t\\shared' --model 'sonnet'"
        ));
        assert!(!script.contains("&&"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_script_skips_set_location_when_no_directory_is_given() {
        let script = windows_launch_script(
            None,
            "banner",
            &claude_arguments("C:\\t\\claude_p1.json", "default", None, "C:\\t\\shared"),
        );
        assert!(!script.contains("Set-Location"));
        assert!(script.contains("Write-Output 'banner'; claude --settings "));
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
            return @{ AGMUX_TEST_MCP_KEY = 'user'; AGMUX_TEST_INHERITED = 'user' }
        }
        return @{ AGMUX_TEST_MCP_KEY = 'machine'; AGMUX_TEST_MACHINE = 'machine' }
    }
    static [void] SetEnvironmentVariable([string] $key, [string] $value, [string] $scope) {
        [System.Environment]::SetEnvironmentVariable($key, $value, $scope)
    }
}
function claude {
    if ($env:AGMUX_TEST_MCP_KEY -ne 'user') { throw 'Missing user environment' }
    if ($env:AGMUX_TEST_MACHINE -ne 'machine') { throw 'Missing machine environment' }
    if ($env:AGMUX_TEST_INHERITED -ne 'process') { throw 'Process override lost' }
    if (($args -join '|') -ne "--settings|C:\t\profile.json|--setting-sources|project,local|--add-dir|C:\shared skills|--mcp-config|C:\user's home\.claude.json|--model|sonnet") { throw 'Profile, Skills or MCP config changed' }
    foreach ($key in 'CLAUDE_CONFIG_DIR','CLAUDE_SECURESTORAGE_CONFIG_DIR','ANTHROPIC_BASE_URL','ANTHROPIC_AUTH_TOKEN','ANTHROPIC_API_KEY','ANTHROPIC_MODEL') {
        if (Test-Path ('Env:' + $key)) { throw 'Inherited isolation or provider leaked' }
    }
    if ((Test-Path Env:NO_COLOR) -or (Test-Path Env:TERM)) { throw 'Terminal cleanup lost' }
    Write-Output 'claude environment verified'
}
"#;
        let script = format!(
            "{fixture}\n{}",
            windows_launch_script(
                None,
                "banner",
                &claude_arguments(
                    r"C:\t\profile.json",
                    "sonnet",
                    Some(r"C:\user's home\.claude.json"),
                    r"C:\shared skills",
                ),
            )
        );
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let output = std::process::Command::new(find_windows_shell().unwrap())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &base64::engine::general_purpose::STANDARD.encode(bytes),
            ])
            .env_remove("AGMUX_TEST_MCP_KEY")
            .env_remove("AGMUX_TEST_MACHINE")
            .env("CLAUDE_CONFIG_DIR", "old-isolated-home")
            .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", "old-credential-home")
            .env("ANTHROPIC_AUTH_TOKEN", "inherited")
            .env("ANTHROPIC_API_KEY", "inherited")
            .env("ANTHROPIC_BASE_URL", "https://inherited.invalid")
            .env("ANTHROPIC_MODEL", "inherited")
            .env("AGMUX_TEST_INHERITED", "process")
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
        let script = windows_launch_script(
            Some("D:\\a&b$c"),
            "banner",
            &claude_arguments("C:\\t\\it's.json", "sonnet's", None, "C:\\t\\shared"),
        );
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
