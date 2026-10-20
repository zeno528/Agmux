//! 当前系统的 Claude Code 原生安装；网络方式只作用于本次任务。
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::cli::*;
use super::AppContext;

fn network_settings(network: &Network) -> serde_json::Value {
    serde_json::json!({ "env": network.env() })
}
pub(super) fn native_path(home: &Path) -> PathBuf {
    home.join(".local/bin").join(if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    })
}

fn source_for_path(path: &Path) -> &'static str {
    let path = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if path.contains("/.claude/local/node_modules/.bin/") || path.contains("/appdata/roaming/npm/")
    {
        return "npm";
    }
    if path.contains("/appdata/local/microsoft/winget/links/") {
        return "winget";
    }
    if path == "/opt/homebrew/bin/claude" || path == "/usr/local/bin/claude" {
        return "homebrew";
    }
    if path == "/usr/bin/claude" || path == "/bin/claude" {
        return "package_manager";
    }
    "unknown"
}

fn other_paths(home: &Path, native: &Path, dirs: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    let mut dirs: Vec<_> = dirs.into_iter().collect();
    dirs.extend([
        home.join(".claude/local/node_modules/.bin"),
        home.join("AppData/Roaming/npm"),
        home.join("AppData/Local/Microsoft/WinGet/Links"),
    ]);
    #[cfg(target_os = "macos")]
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    let native = native
        .canonicalize()
        .unwrap_or_else(|_| native.to_path_buf());
    let mut seen = HashSet::from([native]);
    let mut found = Vec::new();
    for dir in dirs {
        let names: &[&str] = if cfg!(windows) {
            &["claude.exe", "claude.cmd", "claude.ps1"]
        } else {
            &["claude"]
        };
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
                if seen.insert(resolved) {
                    found.push(path.to_string_lossy().into_owned());
                }
            }
        }
    }
    found
}

fn parse_version(text: &str) -> Option<String> {
    let version = text.split_whitespace().next()?;
    valid_version(version).then(|| version.into())
}

fn status(home: &Path, network: &Network, verify: bool) -> Result<CliStatus, Failure> {
    let path = native_path(home);
    let dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut other_paths = other_paths(home, &path, dirs);
    let present = path.symlink_metadata().is_ok();
    #[cfg(target_os = "macos")]
    let native = !present
        || path
            .canonicalize()
            .is_ok_and(|p| p.starts_with(home.join(".local/share/claude/versions")));
    #[cfg(not(target_os = "macos"))]
    let native = true;
    if !native {
        other_paths.push(path.to_string_lossy().into_owned());
    }
    let version = if present && native {
        let result = version(&path, parse_version);
        if verify {
            Some(result?)
        } else {
            result.ok()
        }
    } else {
        None
    };
    let installation = if !other_paths.is_empty() {
        if present && native {
            "conflict"
        } else {
            "other"
        }
    } else if !present {
        "missing"
    } else if version.is_some() {
        "native"
    } else {
        "broken"
    };
    let mut sources = Vec::new();
    if present && native {
        sources.push("native");
    }
    sources.extend(
        other_paths
            .iter()
            .map(|other| source_for_path(Path::new(other))),
    );
    Ok(CliStatus {
        installation,
        source: combine_sources(sources),
        version,
        path: present.then(|| path.to_string_lossy().into_owned()),
        other_paths,
        embedded_paths: Vec::new(),
        platform: current_platform()?,
        network: network.mode(),
        proxy: network.display.clone(),
        busy: false,
    })
}

fn installer_url(platform: &str) -> Result<&'static str, Failure> {
    match platform {
        "win32-x64" | "win32-arm64" => Ok("https://claude.ai/install.ps1"),
        "darwin-x64" | "darwin-arm64" => Ok("https://claude.ai/install.sh"),
        _ => Err(failure("platform", "validation_error", "不支持此系统架构")),
    }
}

fn update_channel(settings: &serde_json::Value) -> Result<&str, Failure> {
    match settings.get("autoUpdatesChannel") {
        None => Ok("latest"),
        Some(value) if value == "latest" || value == "stable" => Ok(value.as_str().unwrap()),
        _ => Err(failure(
            "fetch_version",
            "validation_error",
            "Claude Code 更新通道无效，请检查 settings.json",
        )),
    }
}

fn update_settings(claude_home: &Path) -> Result<serde_json::Value, Failure> {
    super::claude::read_json_object(&claude_home.join("settings.json"), "settings.json").map_err(
        |_| {
            failure(
                "fetch_version",
                "validation_error",
                "无法读取 Claude Code 更新设置，请检查 settings.json 的内容和权限",
            )
        },
    )
}

fn execution_command(
    install: bool,
    path: &Path,
    settings: &Path,
    network: &Network,
) -> Result<Command, Failure> {
    let mut child = if install {
        let mut child = official_installer_command(path, network, "latest", false)?;
        // 官方引导脚本不能传 --settings；临时配置目录防止用户 settings.env 覆盖本次代理。
        // 原生程序的安装入口仍由官方安装器写入用户的 .local/bin。
        child.env(
            "CLAUDE_CONFIG_DIR",
            settings.parent().unwrap_or(Path::new(".")),
        );
        child
    } else {
        let mut child = command(path);
        child.arg("--settings").arg(settings).arg("update");
        network.apply(&mut child);
        child
    };
    child.current_dir(settings.parent().unwrap_or(Path::new(".")));
    Ok(child)
}

impl AppContext {
    pub async fn claude_check_cli_update(&self) -> Result<CliUpdate, Failure> {
        let result = async {
            let mut checked = self
                .claude_cli_operation
                .try_lock()
                .map_err(|_| failure("guard", "validation_error", "已有 CLI 操作正在执行"))?;
            *checked = None;
            let network = Network::current().await?;
            let context = self.clone();
            let status_network = network.clone();
            let (before, settings) = tauri::async_runtime::spawn_blocking(move || {
                let home = context
                    .paths
                    .claude_home
                    .parent()
                    .ok_or_else(|| failure("detect", "io_error", "无法定位用户目录"))?;
                Ok::<_, Failure>((
                    status(home, &status_network, false)?,
                    update_settings(&context.paths.claude_home)?,
                ))
            })
            .await
            .map_err(|_| failure("detect", "internal", "CLI 检测任务失败"))??;
            let channel = update_channel(&settings)?;
            let url = format!("https://downloads.claude.ai/claude-code-releases/{channel}");
            let mut update = check_for_update(
                &network,
                &url,
                |text| Some(text.trim().into()),
                before,
                channel,
            )
            .await?;
            if let Some(floor) = settings
                .get("minimumVersion")
                .and_then(serde_json::Value::as_str)
            {
                update.available &= !newer_version(floor, &update.latest_version)?;
            }
            *checked = Some(update.clone());
            Ok(update)
        }
        .await;
        finish_update_check("Claude Code", result)
    }

    pub fn claude_get_cli_status(&self) -> Result<CliStatus, Failure> {
        let network = Network::detect()?;
        let home = self
            .paths
            .claude_home
            .parent()
            .ok_or_else(|| failure("detect", "io_error", "无法定位用户目录"))?;
        let mut status = status(home, &network, false)?;
        status.busy = self.claude_cli_operation.try_lock().is_err();
        let summary = status_summary("Claude Code", status.installation);
        tauri_plugin_log::log::debug!("[app.cli.status] client=\"Claude Code\" installation={} version={} outcome=success msg={summary:?}", status.installation, status.version.as_deref().unwrap_or("-"));
        Ok(status)
    }

    pub async fn claude_run_cli(&self, install: bool) -> Result<CliStatus, Failure> {
        let task_id = task_id();
        let action = if install { "install" } else { "update" };
        let started = Instant::now();
        let result = self.claude_run_cli_inner(install, &task_id, started).await;
        if let Err(error) = &result {
            log_cli_failure("Claude Code", &task_id, action, started, error);
        }
        result
    }

    async fn claude_run_cli_inner(
        &self,
        install: bool,
        task_id: &str,
        started: Instant,
    ) -> Result<CliStatus, Failure> {
        let mut checked = self
            .claude_cli_operation
            .try_lock()
            .map_err(|_| failure("guard", "validation_error", "已有 CLI 操作正在执行"))?;
        let network = Network::current().await?;
        let home = self
            .paths
            .claude_home
            .parent()
            .ok_or_else(|| failure("detect", "io_error", "无法定位用户目录"))?
            .to_path_buf();
        let network_for_status = network.clone();
        let status_home = home.clone();
        let before = tauri::async_runtime::spawn_blocking(move || {
            status(&status_home, &network_for_status, false)
        })
        .await
        .map_err(|_| failure("detect", "internal", "CLI 检测任务失败"))??;
        let action = if install { "install" } else { "update" };
        let summary = action_summary("Claude Code", action, ActionStage::Start);
        tauri_plugin_log::log::info!("[app.cli.start] client=\"Claude Code\" task_id={task_id:?} action={action} platform={:?} proxy={} version={} outcome=success msg={summary:?}", before.platform, network.display.as_deref().unwrap_or("-"), before.version.as_deref().unwrap_or("-"));
        check_installation(&before, install)?;
        let update = if install {
            *checked = None;
            None
        } else {
            let update = require_update(checked.take(), &before)?;
            if update_channel(&update_settings(&self.paths.claude_home)?)? != update.channel {
                return Err(failure(
                    "guard",
                    "validation_error",
                    "更新通道已变化，请重新检查更新",
                ));
            }
            Some(update)
        };
        let temp =
            tempfile::tempdir().map_err(|_| failure("prepare", "io_error", "无法创建临时目录"))?;
        let execution = async {
            let executable = if install {
                let (path, bytes) = fetch_installer(&network, installer_url(&before.platform)?, &["claude.ai", "downloads.claude.ai"], temp.path(), started).await?;
                tauri_plugin_log::log::info!("[app.cli.download] client=\"Claude Code\" task_id={task_id:?} bytes={bytes} outcome=success msg=\"Claude Code CLI 安装脚本已就绪，由官方安装器校验\"");
                path
            } else { native_path(&home) };
            let settings_directory = temp.path().join("claude-config");
            std::fs::create_dir(&settings_directory).map_err(|_| failure("prepare", "io_error", "无法创建临时网络设置目录"))?;
            let settings_path = settings_directory.join("settings.json");
            let mut settings = network_settings(&network);
            if let Some(update) = &update { settings["autoUpdatesChannel"] = update.channel.clone().into(); }
            let settings = serde_json::to_vec(&settings).map_err(|_| failure("prepare", "internal", "无法生成临时网络设置"))?;
            std::fs::write(&settings_path, settings).map_err(|_| failure("prepare", "io_error", "无法写入临时网络设置"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&settings_path, std::fs::Permissions::from_mode(0o600)).map_err(|_| failure("prepare", "io_error", "无法保护临时网络设置"))?;
            }
            let child_command = execution_command(install, &executable, &settings_path, &network)?;
            let timeout = remaining(started, "run_cli")?;
            let output = run_cli(child_command, timeout).await?;
            tauri_plugin_log::log::info!("[app.cli.process] client=\"Claude Code\" task_id={task_id:?} action={action} exit_code={} duration_ms={} outcome=success msg=\"Claude Code CLI 安装器命令执行完成\"", output.status.code().unwrap_or(-1), started.elapsed().as_millis());
            let status_home = home.clone();
            let status_network = network.clone();
            remaining(started, "verify_version")?;
            let after = tauri::async_runtime::spawn_blocking(move || status(&status_home, &status_network, true)).await
                .map_err(|_| failure("verify_version", "internal", "CLI 验证任务失败"))??;
            remaining(started, "verify_version")?;
            if after.installation != "native" { return Err(failure("verify_version", "protocol_error", "命令完成，但没有检测到有效原生安装")); }
            if let Some(update) = &update { verify_update(update, &after)?; }
            let summary = action_summary("Claude Code", action, ActionStage::Complete);
            tauri_plugin_log::log::info!("[app.cli.complete] client=\"Claude Code\" task_id={task_id:?} action={action} previous_version={} version={} duration_ms={} outcome=success msg={summary:?}", before.version.as_deref().unwrap_or("-"), after.version.as_deref().unwrap_or("-"), started.elapsed().as_millis());
            Ok(after)
        }.await;
        if temp.close().is_err() {
            tauri_plugin_log::log::warn!("[app.cli.cleanup] client=\"Claude Code\" task_id={task_id:?} outcome=failure failure_kind=io_error msg=\"Claude Code CLI 临时文件清理失败\"");
        }
        execution
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn update_channel_is_read_without_modifying_user_settings_and_invalid_settings_are_rejected() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            update_channel(&update_settings(home.path()).unwrap()).unwrap(),
            "latest"
        );
        let path = home.path().join("settings.json");
        let raw =
            r#"{"autoUpdatesChannel":"stable","env":{"HTTP_PROXY":"http://fixture.invalid"}}"#;
        std::fs::write(&path, raw).unwrap();
        assert_eq!(
            update_channel(&update_settings(home.path()).unwrap()).unwrap(),
            "stable"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        assert!(update_channel(&serde_json::json!({"autoUpdatesChannel":"other"})).is_err());
        std::fs::write(&path, "invalid {").unwrap();
        assert!(update_settings(home.path()).is_err());
    }

    #[test]
    fn cli_process_test_child() {
        if let Ok(mode) = std::env::var("AGMUX_CLAUDE_CLI_TEST_CHILD") {
            if mode == "failure" {
                eprintln!("network error: fixture-secret-token");
                std::process::exit(7);
            }
            let expected = if mode == "direct" { "" } else { &mode };
            if let Ok(directory) = std::env::var("AGMUX_CLAUDE_INSTALL_CONFIG") {
                assert_eq!(std::env::var("CLAUDE_CONFIG_DIR").unwrap(), directory);
                let settings: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(Path::new(&directory).join("settings.json")).unwrap(),
                )
                .unwrap();
                for key in PROXY_KEYS {
                    assert_eq!(settings["env"][key], expected);
                }
            }
            for key in PROXY_KEYS {
                assert_eq!(std::env::var(key).unwrap_or_default(), expected);
            }
            std::process::exit(0);
        }
    }

    #[tokio::test]
    async fn process_receives_network_choice_and_failure_never_returns_raw_output() {
        for proxy in [None, Some("http://127.0.0.1:12345".into())] {
            let network = Network::new(proxy).unwrap();
            let mut child = command(&std::env::current_exe().unwrap());
            child.args(["cli_process_test_child", "--nocapture"]);
            child.env(
                "AGMUX_CLAUDE_CLI_TEST_CHILD",
                network.proxy.as_deref().unwrap_or(""),
            );
            network.apply(&mut child);
            assert!(run_cli(child, Duration::from_secs(5))
                .await
                .unwrap()
                .status
                .success());
        }
        let mut child = command(&std::env::current_exe().unwrap());
        child
            .args(["cli_process_test_child", "--nocapture"])
            .env("AGMUX_CLAUDE_CLI_TEST_CHILD", "failure");
        let error = run_cli(child, Duration::from_secs(5)).await.err().unwrap();
        assert_eq!((error.stage, error.kind), ("run_cli", "network_error"));
        assert!(error.message.contains('7'));
        assert!(!error.message.contains("fixture-secret-token"));
        let error = version(&std::env::current_exe().unwrap(), parse_version).unwrap_err();
        assert_eq!((error.stage, error.kind), ("verify_version", "internal"));
    }

    #[test]
    fn platform_and_version_are_validated_before_building_urls() {
        assert_eq!(platform("windows", "arm64").unwrap(), "win32-arm64");
        assert_eq!(platform("windows", "x86_64").unwrap(), "win32-x64");
        assert_eq!(platform("macos", "aarch64").unwrap(), "darwin-arm64");
        assert_eq!(platform("macos", "x86_64").unwrap(), "darwin-x64");
        assert!(platform("windows", "x86").is_err());
        assert!(valid_version("1.2.3"));
        assert_eq!(parse_version("1.2.3 (Claude Code)"), Some("1.2.3".into()));
        for invalid in ["", "<html>", "1.2", "1.2.3/../../file", "1.2.3\nother"] {
            assert!(!valid_version(invalid));
        }
    }

    #[test]
    fn network_settings_and_process_environment_agree_without_leaking_passwords() {
        let network = Network::new(Some("http://user:secret@127.0.0.1:20080".into())).unwrap();
        assert!(!network.display.as_ref().unwrap().contains("secret"));
        assert!(!network.display.as_ref().unwrap().contains("user"));
        let mut child = command(Path::new("fixture"));
        network.apply(&mut child);
        for (key, value) in child.get_envs() {
            let key = key.to_str().unwrap();
            assert_eq!(
                value.unwrap().to_str().unwrap(),
                network_settings(&network)["env"][key].as_str().unwrap()
            );
        }
        assert_eq!(
            network_settings(&network)["env"]["NO_PROXY"],
            "localhost,127.0.0.1,::1"
        );
        let direct = Network::new(None).unwrap();
        assert_eq!(direct.mode(), "direct");
        assert_eq!(network_settings(&direct)["env"]["HTTPS_PROXY"], "");
        assert!(Network::new(Some("socks5://127.0.0.1:1".into())).is_err());
    }

    #[test]
    fn discovery_deduplicates_native_entry_and_identifies_foreign_launcher() {
        let home = tempfile::tempdir().unwrap();
        let path = native_path(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "fixture").unwrap();
        let foreign = home.path().join("foreign");
        std::fs::create_dir(&foreign).unwrap();
        let name = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };
        std::fs::write(foreign.join(name), "fixture").unwrap();
        let found = other_paths(
            home.path(),
            &path,
            [
                path.parent().unwrap().to_path_buf(),
                foreign.clone(),
                foreign,
            ],
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("foreign"));
    }

    #[test]
    fn installation_source_uses_known_paths_and_leaves_unknown_paths_unclassified() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            source_for_path(&home.path().join("AppData/Roaming/npm/claude.cmd")),
            "npm"
        );
        assert_eq!(
            source_for_path(Path::new(
                "C:/Users/test/AppData/Local/Microsoft/WinGet/Links/claude.exe"
            )),
            "winget"
        );
        assert_eq!(source_for_path(Path::new("/tmp/claude")), "unknown");
        assert_eq!(combine_sources(["native", "npm"]), Some("multiple"));
    }

    #[test]
    fn official_install_and_native_update_share_network_without_changing_update_settings_scope() {
        for os in ["windows", "macos"] {
            for arch in ["x86_64", "arm64"] {
                let url = installer_url(&platform(os, arch).unwrap()).unwrap();
                assert!(url.ends_with(if os == "windows" { ".ps1" } else { ".sh" }));
            }
        }
        assert!(installer_url("linux-x64").is_err());
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("install 'literal'.ps1");
        let settings = temp.path().join("settings.json");
        let network = Network::new(Some("http://user:secret@127.0.0.1:20080".into())).unwrap();
        let install = execution_command(true, &script, &settings, &network).unwrap();
        assert_eq!(
            install
                .get_envs()
                .find(|(key, _)| *key == "CLAUDE_CONFIG_DIR")
                .unwrap()
                .1
                .unwrap(),
            temp.path().as_os_str()
        );
        #[cfg(windows)]
        {
            assert!(install
                .get_program()
                .to_string_lossy()
                .ends_with("WindowsPowerShell/v1.0/powershell.exe"));
            assert_eq!(
                install
                    .get_envs()
                    .find(|(key, _)| *key == "AGMUX_CLI_INSTALLER_PATH")
                    .unwrap()
                    .1
                    .unwrap(),
                script.as_os_str()
            );
        }
        #[cfg(not(windows))]
        assert_eq!(install.get_program(), "/bin/bash");
        let native = native_path(temp.path());
        let update = execution_command(false, &native, &settings, &network).unwrap();
        assert_eq!(update.get_program(), native.as_os_str());
        assert_eq!(
            update.get_args().collect::<Vec<_>>(),
            [
                std::ffi::OsStr::new("--settings"),
                settings.as_os_str(),
                std::ffi::OsStr::new("update")
            ]
        );
        assert!(!update.get_envs().any(|(key, _)| key == "CLAUDE_CONFIG_DIR"));
        for child in [&install, &update] {
            assert_eq!(
                child
                    .get_envs()
                    .find(|(key, _)| *key == "HTTPS_PROXY")
                    .unwrap()
                    .1
                    .unwrap(),
                "http://user:secret@127.0.0.1:20080"
            );
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn claude_official_bootstrap_uses_system_interpreter_and_temporary_network_settings() {
        for proxy in [None, Some("http://127.0.0.1:12345".into())] {
            let network = Network::new(proxy).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let settings = temp.path().join("settings.json");
            std::fs::write(
                &settings,
                serde_json::to_vec(&network_settings(&network)).unwrap(),
            )
            .unwrap();
            let script = temp.path().join("install 'literal'.ps1");
            std::fs::write(&script, r#"param([string]$Target)
if ($Target -ne 'latest' -or $PSVersionTable.PSVersion.Major -ne 5) { throw 'invalid installer arguments' }
& $env:AGMUX_FIXTURE_BINARY cli_process_test_child --nocapture
exit $LASTEXITCODE
"#).unwrap();
            let mut child = execution_command(true, &script, &settings, &network).unwrap();
            child.env("AGMUX_FIXTURE_BINARY", std::env::current_exe().unwrap());
            child.env("AGMUX_CLAUDE_INSTALL_CONFIG", temp.path());
            child.env(
                "AGMUX_CLAUDE_CLI_TEST_CHILD",
                network.proxy.as_deref().unwrap_or("direct"),
            );
            run_cli(child, Duration::from_secs(10)).await.unwrap();
            std::fs::write(
                &script,
                "param([string]$Target)\nthrow 'Checksum verification failed fixture-secret-token'",
            )
            .unwrap();
            let error = run_cli(
                execution_command(true, &script, &settings, &network).unwrap(),
                Duration::from_secs(10),
            )
            .await
            .err()
            .unwrap();
            assert_eq!((error.stage, error.kind), ("checksum", "validation_error"));
            assert!(!error.message.contains("fixture-secret-token"));
            std::fs::write(&script, "param([string]$Target)\nexit 7").unwrap();
            let error = run_cli(
                execution_command(true, &script, &settings, &network).unwrap(),
                Duration::from_secs(10),
            )
            .await
            .err()
            .unwrap();
            assert_eq!((error.stage, error.kind), ("run_cli", "internal"));
            assert!(error.message.contains('7'));
        }
    }

    #[tokio::test]
    async fn duplicate_tasks_are_rejected_before_any_installation_work() {
        let home = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(home.path()).unwrap();
        paths.ensure().unwrap();
        let context = AppContext::new(paths).unwrap();
        let _guard = context.claude_cli_operation.lock().await;
        let error = context
            .claude_run_cli_inner(true, "fixture", Instant::now())
            .await
            .unwrap_err();
        assert_eq!((error.stage, error.kind), ("guard", "validation_error"));
        assert!(!native_path(home.path()).exists());
    }

    #[tokio::test]
    async fn direct_http_errors_and_expired_task_keep_their_exact_stage() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            std::io::Read::read(&mut stream, &mut request).unwrap();
            assert!(String::from_utf8_lossy(&request).starts_with("GET /manifest.json "));
            stream
                .write_all(
                    b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let client = Network::new(None).unwrap().client().unwrap();
        let error = fetch(
            &client,
            &format!("http://{address}/manifest.json"),
            Instant::now(),
            "fetch_manifest",
        )
        .await
        .unwrap_err();
        assert_eq!((error.stage, error.kind), ("fetch_manifest", "http_error"));
        assert!(error.message.contains("503"));
        server.join().unwrap();
        let error = fetch(
            &client,
            "http://fixture.invalid",
            Instant::now() - TASK_TIMEOUT,
            "download",
        )
        .await
        .unwrap_err();
        assert_eq!((error.stage, error.kind), ("download", "timeout"));
    }

    #[tokio::test]
    async fn explicit_proxy_carries_requests_and_has_no_direct_fallback() {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert!(request.starts_with("GET http://fixture.invalid/version "));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n1.2.3",
                )
                .unwrap();
        });
        let network = Network::new(Some(format!("http://{address}"))).unwrap();
        assert_eq!(
            fetch(
                &network.client().unwrap(),
                "http://fixture.invalid/version",
                Instant::now(),
                "fetch_version"
            )
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
            "1.2.3"
        );
        server.join().unwrap();
        let failed = fetch(
            &network.client().unwrap(),
            "http://fixture.invalid/version",
            Instant::now(),
            "fetch_version",
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failed.stage, "fetch_version");
        assert_eq!(failed.kind, "network_error");
    }
}
