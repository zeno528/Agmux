//! Codex 官方独立安装器；与桌面应用内置的 CLI 分开管理。
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::cli::*;
use super::AppContext;

pub(super) fn file_name() -> &'static str {
    if cfg!(windows) {
        "codex.exe"
    } else {
        "codex"
    }
}

pub(super) fn default_install_dir(home: &Path) -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("Programs/OpenAI/Codex/bin")
    } else {
        home.join(".local/bin")
    }
}

fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.trim_start_matches("//?/").to_ascii_lowercase()
    } else {
        text
    }
}

fn source_for_path(path: &Path) -> &'static str {
    let path = path_key(path).to_ascii_lowercase();
    if path.contains("/appdata/roaming/npm/") || path.contains("/node_modules/.bin/") {
        return "npm";
    }
    if path == "/opt/homebrew/bin/codex" || path == "/usr/local/bin/codex" {
        return "homebrew";
    }
    "unknown"
}

fn resolved(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    let target = std::fs::read_link(path)
        .map(|target| {
            if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or(Path::new(".")).join(target)
            }
        })
        .unwrap_or_else(|_| path.to_path_buf());
    // 损坏安装的二进制可能已被删除，仍按真实父目录去重两条入口。
    match (target.parent(), target.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map(|parent| parent.join(name))
            .unwrap_or(target),
        _ => target,
    }
}

fn owns(path: &Path, codex_home: &Path) -> bool {
    // 官方目录包含包清单和 bin；不能只因文件叫 codex 就接管。
    let target = resolved(path);
    let parent = target.parent().unwrap_or(&target);
    let parent = parent
        .canonicalize()
        .or_else(|_| std::fs::read_link(parent))
        .unwrap_or_else(|_| parent.to_path_buf());
    let root = resolved(&codex_home.join("packages/standalone"));
    let in_root = path_key(&parent).starts_with(&format!("{}/", path_key(&root)));
    in_root
        && parent.file_name().is_some_and(|name| name == "bin")
        && parent
            .parent()
            .is_some_and(|package| package.join("codex-package.json").is_file())
}

fn embedded(path: &Path, home: &Path, codex_home: &Path) -> bool {
    let path = path_key(&resolved(path));
    let bundled = path_key(&resolved(&codex_home.join("plugins/.plugin-appserver")));
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"));
    let desktop = path_key(&resolved(&local.join("OpenAI/Codex")));
    let default_desktop = path_key(&resolved(&home.join("AppData/Local/OpenAI/Codex")));
    path.starts_with(&format!("{bundled}/"))
        || path.starts_with(&format!("{desktop}/bin/"))
        || path.starts_with(&format!("{desktop}/runtimes/"))
        || path.starts_with(&format!("{default_desktop}/bin/"))
        || path.starts_with(&format!("{default_desktop}/runtimes/"))
        || (path.contains("/windowsapps/openai.codex_") && path.contains("/app/resources/"))
        || [
            "codex.app/contents/resources/",
            "openai codex.app/contents/resources/",
            "openai.codex.app/contents/resources/",
        ]
        .iter()
        .any(|suffix| path.to_ascii_lowercase().contains(suffix))
}

#[derive(Default)]
struct Installations {
    native: Vec<PathBuf>,
    other: Vec<String>,
    embedded: Vec<String>,
}

fn discover(
    home: &Path,
    codex_home: &Path,
    install_dir: &Path,
    dirs: impl IntoIterator<Item = PathBuf>,
) -> Installations {
    let mut candidates = vec![
        install_dir.join(file_name()),
        codex_home.join("bin").join(file_name()),
        codex_home
            .join("plugins/.plugin-appserver")
            .join(file_name()),
    ];
    let mut dirs: Vec<_> = dirs.into_iter().collect();
    dirs.push(home.join("AppData/Roaming/npm"));
    #[cfg(target_os = "macos")]
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    for dir in dirs {
        candidates.push(dir.join(file_name()));
        #[cfg(windows)]
        candidates.extend([dir.join("codex.cmd"), dir.join("codex.ps1")]);
    }
    candidates.push(
        codex_home
            .join("packages/standalone/current/bin")
            .join(file_name()),
    );
    let mut found = Installations::default();
    let mut seen = HashSet::new();
    for path in candidates {
        let native =
            path.file_name().is_some_and(|name| name == file_name()) && owns(&path, codex_home);
        if !path.is_file() && !native {
            continue;
        }
        if !seen.insert(path_key(&resolved(&path))) {
            continue;
        }
        if embedded(&path, home, codex_home) {
            found.embedded.push(path.to_string_lossy().into_owned());
        } else if native {
            found.native.push(path);
        } else {
            found.other.push(path.to_string_lossy().into_owned());
        }
    }
    // Windows 安装器会替换整个 bin 目录；macOS 的 .local/bin 是共享目录，
    // 只替换 Codex 自己的文件链接，其他程序不构成安装冲突。
    #[cfg(windows)]
    if !owns(&install_dir.join(file_name()), codex_home)
        && (std::fs::read_link(install_dir).is_ok()
            || std::fs::read_dir(install_dir).is_ok_and(|mut entries| entries.next().is_some()))
        && !found
            .other
            .iter()
            .any(|path| Path::new(path).parent() == Some(install_dir))
    {
        found.other.push(install_dir.to_string_lossy().into_owned());
    }
    found
}

fn path_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(dir) = std::env::var_os("CODEX_INSTALL_DIR") {
        dirs.push(dir.into());
    }
    dirs
}

pub(super) fn find_standalone(home: &Path) -> Option<PathBuf> {
    discover(
        home,
        &home.join(".codex"),
        &default_install_dir(home),
        path_dirs(),
    )
    .native
    .into_iter()
    .find(|path| path.is_file())
}

pub(super) fn model_client_version(home: &Path, codex_home: &Path) -> Result<String, Failure> {
    let found = discover(home, codex_home, &default_install_dir(home), path_dirs());
    for path in found
        .native
        .into_iter()
        .chain(found.other.into_iter().map(PathBuf::from))
        .chain(found.embedded.into_iter().map(PathBuf::from))
    {
        if let Ok(value) = version(&path, parse_version) {
            return Ok(value);
        }
    }
    Err(failure(
        "detect",
        "not_found",
        "无法读取 Codex 客户端版本，请先安装或打开 Codex",
    ))
}

fn parse_version(text: &str) -> Option<String> {
    let mut parts = text.split_whitespace();
    if parts.next()? != "codex-cli" {
        return None;
    }
    let version = parts.next()?;
    (valid_version(version) && parts.next().is_none()).then(|| version.into())
}

fn status(
    home: &Path,
    codex_home: &Path,
    network: &Network,
    verify: bool,
) -> Result<CliStatus, Failure> {
    let found = discover(home, codex_home, &default_install_dir(home), path_dirs());
    let path = found.native.first();
    let version = path.map(|path| version(path, parse_version));
    let version = if verify {
        version.transpose()?
    } else {
        version.and_then(Result::ok)
    };
    let installation =
        if found.native.len() > 1 || (!found.native.is_empty() && !found.other.is_empty()) {
            "conflict"
        } else if !found.other.is_empty() {
            "other"
        } else if path.is_none() {
            "missing"
        } else if version.is_some() {
            "native"
        } else {
            "broken"
        };
    let source = if found.native.len() > 1 || (!found.native.is_empty() && !found.other.is_empty())
    {
        Some("multiple")
    } else if !found.native.is_empty() {
        Some("standalone")
    } else if !found.other.is_empty() {
        combine_sources(
            found
                .other
                .iter()
                .map(|path| source_for_path(Path::new(path))),
        )
    } else if !found.embedded.is_empty() {
        Some("embedded")
    } else {
        None
    };
    let mut other_paths = found.other;
    other_paths.extend(
        found
            .native
            .iter()
            .skip(1)
            .map(|path| path.to_string_lossy().into_owned()),
    );
    Ok(CliStatus {
        installation,
        source,
        version,
        path: path.map(|path| path.to_string_lossy().into_owned()),
        other_paths,
        embedded_paths: found.embedded,
        platform: current_platform()?,
        network: network.mode(),
        proxy: network.display.clone(),
        busy: false,
    })
}

fn installer_url(platform: &str) -> Result<&'static str, Failure> {
    match platform {
        "win32-x64" | "win32-arm64" => Ok("https://chatgpt.com/codex/install.ps1"),
        "darwin-x64" | "darwin-arm64" => Ok("https://chatgpt.com/codex/install.sh"),
        _ => Err(failure("platform", "validation_error", "不支持此系统架构")),
    }
}

fn release_version(text: &str) -> Option<String> {
    let metadata: serde_json::Value = serde_json::from_str(text).ok()?;
    metadata
        .get("tag_name")?
        .as_str()?
        .strip_prefix("rust-v")
        .map(str::to_owned)
}

fn installer_command(
    script: &Path,
    install_dir: &Path,
    codex_home: &Path,
    network: &Network,
    target: &str,
) -> Result<Command, Failure> {
    let mut child = official_installer_command(script, network, target, true)?;
    // 避免继承用户用于守护进程或延迟切换的安装器内部选项。
    for key in [
        "CODEX_INSTALL_DAEMON_ONLY",
        "CODEX_INSTALL_DEFER_SELECTION",
        "CODEX_INSTALL_IF_LATEST",
        "CODEX_INSTALL_IF_CURRENT",
        "CODEX_UPDATE_FROM_RELEASE",
        "CODEX_RELEASE",
    ] {
        child.env_remove(key);
    }
    child
        .env("CODEX_NON_INTERACTIVE", "1")
        .env("CODEX_INSTALLER_USE_RELEASES_OPENAI_COM", "1")
        .env("CODEX_INSTALL_DIR", install_dir)
        .env("CODEX_HOME", codex_home);
    Ok(child)
}

impl AppContext {
    pub async fn codex_check_cli_update(&self) -> Result<CliUpdate, Failure> {
        let result = async {
            let mut checked = self
                .codex_cli_operation
                .try_lock()
                .map_err(|_| failure("guard", "validation_error", "已有 CLI 操作正在执行"))?;
            *checked = None;
            let network = Network::current().await?;
            let context = self.clone();
            let status_network = network.clone();
            let before = tauri::async_runtime::spawn_blocking(move || {
                let home = context
                    .paths
                    .codex_home
                    .parent()
                    .ok_or_else(|| failure("detect", "io_error", "无法定位用户目录"))?;
                status(home, &context.paths.codex_home, &status_network, false)
            })
            .await
            .map_err(|_| failure("detect", "internal", "CLI 检测任务失败"))??;
            let update = check_for_update(
                &network,
                "https://releases.openai.com/codex/channels/latest",
                release_version,
                before,
                "latest",
            )
            .await?;
            *checked = Some(update.clone());
            Ok(update)
        }
        .await;
        finish_update_check("Codex", result)
    }

    pub fn codex_get_cli_status(&self) -> Result<CliStatus, Failure> {
        let network = Network::detect()?;
        let home = self
            .paths
            .codex_home
            .parent()
            .ok_or_else(|| failure("detect", "io_error", "无法定位用户目录"))?;
        let mut status = status(home, &self.paths.codex_home, &network, false)?;
        status.busy = self.codex_cli_operation.try_lock().is_err();
        let summary = status_summary("Codex", status.installation);
        tauri_plugin_log::log::debug!("[app.cli.status] client=\"Codex\" installation={} version={} outcome=success msg={summary:?}", status.installation, status.version.as_deref().unwrap_or("-"));
        Ok(status)
    }

    pub async fn codex_run_cli(&self, install: bool) -> Result<CliStatus, Failure> {
        let task_id = task_id();
        let action = if install { "install" } else { "update" };
        let started = Instant::now();
        let result = self.codex_run_cli_inner(install, &task_id, started).await;
        if let Err(error) = &result {
            log_cli_failure("Codex", &task_id, action, started, error);
        }
        result
    }

    async fn codex_run_cli_inner(
        &self,
        install: bool,
        task_id: &str,
        started: Instant,
    ) -> Result<CliStatus, Failure> {
        let mut checked = self
            .codex_cli_operation
            .try_lock()
            .map_err(|_| failure("guard", "validation_error", "已有 CLI 操作正在执行"))?;
        let network = Network::current().await?;
        let codex_home = self.paths.codex_home.clone();
        let home = codex_home
            .parent()
            .ok_or_else(|| failure("detect", "io_error", "无法定位用户目录"))?
            .to_path_buf();
        let status_home = home.clone();
        let status_codex_home = codex_home.clone();
        let status_network = network.clone();
        let before = tauri::async_runtime::spawn_blocking(move || {
            status(&status_home, &status_codex_home, &status_network, false)
        })
        .await
        .map_err(|_| failure("detect", "internal", "CLI 检测任务失败"))??;
        let action = if install { "install" } else { "update" };
        let summary = action_summary("Codex", action, ActionStage::Start);
        tauri_plugin_log::log::info!("[app.cli.start] client=\"Codex\" task_id={task_id:?} action={action} platform={:?} proxy={} version={} outcome=success msg={summary:?}", before.platform, network.display.as_deref().unwrap_or("-"), before.version.as_deref().unwrap_or("-"));
        check_installation(&before, install)?;
        let update = if install {
            *checked = None;
            None
        } else {
            Some(require_update(checked.take(), &before)?)
        };
        let temp =
            tempfile::tempdir().map_err(|_| failure("prepare", "io_error", "无法创建临时目录"))?;
        let execution = async {
            let (script_path, bytes) = fetch_installer(&network, installer_url(&before.platform)?, &["chatgpt.com", "releases.openai.com"], temp.path(), started).await?;
            if install {
                tauri_plugin_log::log::info!("[app.cli.download] client=\"Codex\" task_id={task_id:?} bytes={bytes} outcome=success msg=\"Codex CLI 安装脚本已就绪，由官方安装器校验\"");
            }
            // 当前版本缓存目录不是可见入口；若只找到缓存，使用官方默认入口。
            let install_dir = before.path.as_deref().map(Path::new)
                .filter(|path| !path_key(path).starts_with(&format!("{}/", path_key(&codex_home.join("packages/standalone")))))
                .and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(|| default_install_dir(&home));
            let target = update.as_ref().map_or("latest", |update| update.latest_version.as_str());
            let output = run_cli(installer_command(&script_path, &install_dir, &codex_home, &network, target)?, remaining(started, "run_cli")?).await?;
            tauri_plugin_log::log::info!("[app.cli.process] client=\"Codex\" task_id={task_id:?} action={action} exit_code={} duration_ms={} outcome=success msg=\"Codex CLI 安装器命令执行完成\"", output.status.code().unwrap_or(-1), started.elapsed().as_millis());
            remaining(started, "verify_version")?;
            let status_home = home.clone();
            let status_network = network.clone();
            let after = tauri::async_runtime::spawn_blocking(move || status(&status_home, &codex_home, &status_network, true))
                .await.map_err(|_| failure("verify_version", "internal", "CLI 验证任务失败"))??;
            remaining(started, "verify_version")?;
            let installed_path = install_dir.join(file_name());
            if after.installation != "native" || !installed_path.is_file()
                || after.path.as_deref().is_none_or(|path| path_key(&resolved(Path::new(path))) != path_key(&resolved(&installed_path))) {
                return Err(failure("verify_version", "protocol_error", "命令完成，但没有检测到有效的独立 CLI 入口"));
            }
            if let Some(update) = &update { verify_update(update, &after)?; }
            let summary = action_summary("Codex", action, ActionStage::Complete);
            tauri_plugin_log::log::info!("[app.cli.complete] client=\"Codex\" task_id={task_id:?} action={action} previous_version={} version={} duration_ms={} outcome=success msg={summary:?}", before.version.as_deref().unwrap_or("-"), after.version.as_deref().unwrap_or("-"), started.elapsed().as_millis());
            Ok(after)
        }.await;
        if temp.close().is_err() {
            tauri_plugin_log::log::warn!("[app.cli.cleanup] client=\"Codex\" task_id={task_id:?} outcome=failure failure_kind=io_error msg=\"Codex CLI 临时文件清理失败\"");
        }
        execution
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::io::{BufRead, BufReader, Write};
    #[cfg(windows)]
    use std::time::Duration;

    fn fixture_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn platform_installers_and_codex_version_prefix_are_explicit() {
        assert_eq!(
            release_version(r#"{"tag_name":"rust-v1.2.3"}"#),
            Some("1.2.3".into())
        );
        for metadata in ["invalid", r#"{"tag_name":"v1.2.3"}"#, r#"{"assets":[]}"#] {
            assert!(release_version(metadata).is_none());
        }
        for os in ["windows", "macos"] {
            for arch in ["x86_64", "arm64"] {
                let platform = platform(os, arch).unwrap();
                let url = installer_url(&platform).unwrap();
                assert!(url.ends_with(if os == "windows" { ".ps1" } else { ".sh" }));
            }
        }
        assert!(installer_url("linux-x64").is_err());
        for value in ["0.1.2", "0.1.2-alpha.1", "0.1.2-beta.3"] {
            assert_eq!(
                parse_version(&format!("codex-cli {value}\n")),
                Some(value.into())
            );
        }
        for value in [
            "0.1.2 (Claude Code)",
            "codex-cli invalid",
            "codex-cli 0.1.2 extra",
            "codex 0.1.2",
        ] {
            assert!(parse_version(value).is_none());
        }
    }

    #[test]
    fn discovery_separates_standalone_foreign_multiple_and_desktop_copies() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let codex_home = home.join(".codex");
        let install_dir = home.join("visible/bin");
        let package = codex_home.join("packages/standalone/releases/fixture");
        let native = package.join("bin").join(file_name());
        fixture_file(&native, "fixture");
        // A file in the cache without the package manifest is not taken over.
        assert_eq!(
            discover(
                home,
                &codex_home,
                &install_dir,
                [native.parent().unwrap().into()]
            )
            .other
            .len(),
            1
        );
        fixture_file(&package.join("codex-package.json"), "{}");
        let desktop = home
            .join("AppData/Local/OpenAI/Codex/bin/build")
            .join(file_name());
        let bundled = codex_home
            .join("plugins/.plugin-appserver")
            .join(file_name());
        let mac_desktop = home.join("Codex.app/Contents/Resources").join(file_name());
        for path in [&desktop, &bundled, &mac_desktop] {
            fixture_file(path, "desktop");
        }
        let dirs = [
            native.parent().unwrap().into(),
            desktop.parent().unwrap().into(),
            mac_desktop.parent().unwrap().into(),
            native.parent().unwrap().into(),
        ];
        let found = discover(home, &codex_home, &install_dir, dirs);
        assert_eq!(found.native, std::slice::from_ref(&native));
        assert!(found.other.is_empty());
        assert_eq!(found.embedded.len(), 3);
        let foreign = home
            .join("npm")
            .join(if cfg!(windows) { "codex.cmd" } else { "codex" });
        fixture_file(&foreign, "npm");
        let second = codex_home.join("packages/standalone/releases/another");
        fixture_file(&second.join("codex-package.json"), "{}");
        fixture_file(&second.join("bin").join(file_name()), "fixture");
        let found = discover(
            home,
            &codex_home,
            &install_dir,
            [
                native.parent().unwrap().into(),
                second.join("bin"),
                foreign.parent().unwrap().into(),
            ],
        );
        assert_eq!(found.native.len(), 2);
        assert_eq!(found.other, [foreign.to_string_lossy()]);
        fixture_file(&install_dir.join("unrelated"), "preserve");
        let unrelated_is_conflict = discover(home, &codex_home, &install_dir, [])
            .other
            .contains(&install_dir.to_string_lossy().into_owned());
        assert_eq!(unrelated_is_conflict, cfg!(windows));
        assert!(!owns(
            &home
                .join(".codex/packages/standalone-other/bin")
                .join(file_name()),
            &codex_home
        ));
    }

    #[test]
    fn installation_source_uses_known_external_paths() {
        assert_eq!(
            source_for_path(Path::new("C:/Users/test/AppData/Roaming/npm/codex.cmd")),
            "npm"
        );
        assert_eq!(
            source_for_path(Path::new("/opt/homebrew/bin/codex")),
            "homebrew"
        );
        assert_eq!(source_for_path(Path::new("/tmp/codex")), "unknown");
    }

    #[cfg(unix)]
    #[test]
    fn shared_bin_allows_claude_but_rejects_foreign_codex_and_deduplicates_owned_symlinks() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        let package = codex_home.join("packages/standalone/releases/fixture");
        let binary = package.join("bin/codex");
        fixture_file(&binary, "fixture");
        fixture_file(&package.join("codex-package.json"), "{}");
        let custom = home.path().join(".local/bin");
        std::fs::create_dir_all(&custom).unwrap();
        fixture_file(&custom.join("claude"), "preserve Claude");
        let shared = discover(home.path(), &codex_home, &custom, []);
        assert!(shared.native.is_empty());
        assert!(shared.other.is_empty());
        fixture_file(&custom.join("codex"), "foreign Codex");
        assert_eq!(
            discover(home.path(), &codex_home, &custom, []).other,
            [custom.join("codex").to_string_lossy()]
        );
        std::fs::remove_file(custom.join("codex")).unwrap();
        symlink(&binary, custom.join("codex")).unwrap();
        let found = discover(home.path(), &codex_home, &custom, [package.join("bin")]);
        assert_eq!(found.native, [custom.join("codex")]);
        assert!(found.other.is_empty());
        assert_eq!(
            std::fs::read_to_string(custom.join("claude")).unwrap(),
            "preserve Claude"
        );
    }

    #[tokio::test]
    async fn operation_locks_are_independent_and_guard_precedes_all_work() {
        let home = tempfile::tempdir().unwrap();
        let context = AppContext::new(crate::paths::from_home(home.path()).unwrap()).unwrap();
        let guard = context.codex_cli_operation.lock().await;
        assert!(context.claude_cli_operation.try_lock().is_ok());
        let error = context
            .codex_run_cli_inner(true, "fixture", Instant::now())
            .await
            .unwrap_err();
        assert_eq!((error.stage, error.kind), ("guard", "validation_error"));
        assert!(!home.path().join(".codex").exists());
        drop(guard);
        let _guard = context.claude_cli_operation.lock().await;
        assert!(context.codex_cli_operation.try_lock().is_ok());
    }

    #[test]
    fn installed_version_is_checked_and_foreign_or_duplicate_installs_are_blocked() {
        let mut status = CliStatus {
            installation: "missing",
            source: None,
            version: None,
            path: None,
            other_paths: vec![],
            embedded_paths: vec!["desktop".into()],
            platform: "win32-x64".into(),
            network: "direct",
            proxy: None,
            busy: false,
        };
        assert!(check_installation(&status, true).is_ok());
        assert!(check_installation(&status, false).is_err());
        for installation in ["other", "conflict"] {
            status.installation = installation;
            assert!(check_installation(&status, true).is_err());
            assert!(check_installation(&status, false).is_err());
        }
        status.installation = "native";
        assert!(check_installation(&status, false).is_ok());
        assert!(check_installation(&status, true).is_err());
        // A successful test harness --version without a Codex version cannot pass.
        assert!(version(&std::env::current_exe().unwrap(), parse_version).is_err());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn installer_fixture_creates_owned_visible_entry_and_absolute_version_is_verified() {
        let temp = tempfile::tempdir().unwrap();
        let prototype = temp.path().join("fixture.exe");
        let source = temp.path().join("fixture.rs");
        fixture_file(
            &source,
            r#"fn main() { if std::env::args().nth(1).as_deref() == Some("--version") { println!("codex-cli 0.1.2"); } else { std::process::exit(1); } }"#,
        );
        let mut compiler = command(Path::new("rustc"));
        compiler.arg(&source).arg("-o").arg(&prototype);
        run_cli(compiler, Duration::from_secs(30)).await.unwrap();
        let script = r#"param([string]$Release)
if ($Release -ne '0.1.2') { throw 'checked version was not passed to installer' }
$package = Join-Path $env:CODEX_HOME 'packages/standalone/current'
$bin = Join-Path $package 'bin'
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item -LiteralPath $env:AGMUX_FIXTURE_BINARY -Destination (Join-Path $bin 'codex.exe')
Set-Content -LiteralPath (Join-Path $package 'codex-package.json') -Value '{}' -Encoding ASCII
New-Item -ItemType Junction -Path $env:CODEX_INSTALL_DIR -Target $bin | Out-Null
"#;
        let script_path = temp.path().join("fixture.ps1");
        fixture_file(&script_path, script);
        let mut child = installer_command(
            &script_path,
            &temp.path().join("visible"),
            &temp.path().join(".codex"),
            &Network::new(None).unwrap(),
            "0.1.2",
        )
        .unwrap();
        child.env("AGMUX_FIXTURE_BINARY", &prototype);
        run_cli(child, Duration::from_secs(10)).await.unwrap();
        let visible = temp.path().join("visible");
        let codex_home = temp.path().join(".codex");
        let found = discover(temp.path(), &codex_home, &visible, []);
        assert!(found.other.is_empty());
        assert_eq!(found.native.len(), 1);
        assert_eq!(
            path_key(&resolved(&found.native[0])),
            path_key(&resolved(&visible.join(file_name())))
        );
        assert_eq!(version(&found.native[0], parse_version).unwrap(), "0.1.2");
        assert_eq!(
            version(&visible.join(file_name()), parse_version).unwrap(),
            "0.1.2"
        );
        std::fs::remove_file(visible.join(file_name())).unwrap();
        assert!(version(&visible.join(file_name()), parse_version).is_err());
        let broken = discover(temp.path(), &codex_home, &visible, []);
        assert_eq!(broken.native.len(), 1);
        assert!(broken.other.is_empty());
    }

    #[cfg(windows)]
    fn fixture_script(temp: &Path, text: &str, network: &Network) -> Command {
        let script = temp.join("fixture.ps1");
        fixture_file(&script, text);
        installer_command(
            &script,
            &temp.join("visible"),
            &temp.join(".codex"),
            network,
            "latest",
        )
        .unwrap()
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn official_powershell_child_uses_direct_or_authenticated_proxy_for_both_cmdlets_and_fallback(
    ) {
        // Exercise the real product interpreter (Windows PowerShell 5.1), not a pwsh substitute.
        for proxy_mode in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let started = Instant::now();
                let mut completed = vec![];
                while completed.len() < 3 {
                    assert!(
                        started.elapsed() < Duration::from_secs(20),
                        "missing installer request"
                    );
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut headers = String::new();
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        headers.push_str(&line);
                    }
                    if proxy_mode
                        && !headers
                            .to_ascii_lowercase()
                            .contains("proxy-authorization: basic dxnlcjpwqhnz")
                    {
                        stream.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"fixture\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                        continue;
                    }
                    let first = headers.lines().next().unwrap().to_string();
                    assert!(first.starts_with(if proxy_mode {
                        "GET http://fixture.invalid/"
                    } else {
                        "GET /"
                    }));
                    let failed = first.contains("/releases ");
                    completed.push(first);
                    let response: &[u8] = if failed {
                        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    } else {
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}"
                    };
                    stream.write_all(response).unwrap();
                }
                completed
            });
            let network =
                Network::new(proxy_mode.then(|| format!("http://u%73er:p%40ss@{address}")))
                    .unwrap();
            let url = if proxy_mode {
                "http://fixture.invalid".to_string()
            } else {
                format!("http://{address}")
            };
            let temp = tempfile::tempdir().unwrap();
            let script = format!(
                r#"param([string]$Release)
if ($Release -ne 'latest' -or $env:CODEX_NON_INTERACTIVE -ne '1') {{ throw 'invalid task arguments' }}
if ($env:CODEX_INSTALL_DAEMON_ONLY) {{ throw 'daemon option leaked' }}
try {{ Invoke-RestMethod '{url}/releases' | Out-Null }} catch {{ Invoke-RestMethod '{url}/fallback' | Out-Null }}
Invoke-WebRequest -UseBasicParsing '{url}/package' | Out-Null
"#
            );
            let mut child = fixture_script(temp.path(), &script, &network);
            if !proxy_mode {
                // 在真实共享初始化之前放入错误默认代理，证明直连会清除它。
                let mut poisoned = command(Path::new(child.get_program()));
                let args: Vec<_> = child.get_args().collect();
                poisoned.args(&args[..args.len() - 1]).arg(format!(
                    "[System.Net.WebRequest]::DefaultWebProxy = New-Object System.Net.WebProxy('http://127.0.0.1:1')\n{}",
                    args.last().unwrap().to_string_lossy()
                ));
                for (key, value) in child.get_envs() {
                    if let Some(value) = value {
                        poisoned.env(key, value);
                    } else {
                        poisoned.env_remove(key);
                    }
                }
                poisoned.current_dir(child.get_current_dir().unwrap());
                child = poisoned;
            }
            let output = super::super::plugins::wait_child_with_timeout(
                child.spawn().unwrap(),
                Duration::from_secs(20),
            )
            .unwrap()
            .unwrap();
            assert!(
                output.status.success(),
                "fixture mode={proxy_mode} stderr={} stdout={}",
                output.stderr,
                output.stdout
            );
            let requests = server.join().unwrap();
            assert!(requests[0].contains("/releases "));
            assert!(requests[1].contains("/fallback "));
            assert!(requests[2].contains("/package "));
            assert!(!network.display.unwrap_or_default().contains("pass"));
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn failed_proxy_never_contacts_direct_destination_and_installer_errors_are_safe() {
        // WebRequest bypasses loopback by design; use a local non-loopback endpoint.
        let route = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        route.connect("192.0.2.1:9").unwrap();
        let direct = std::net::TcpListener::bind((route.local_addr().unwrap().ip(), 0)).unwrap();
        direct.set_nonblocking(true).unwrap();
        let closed_proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_address = closed_proxy.local_addr().unwrap();
        drop(closed_proxy);
        let network = Network::new(Some(format!("http://fixture:secret@{proxy_address}"))).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let child = fixture_script(temp.path(), &format!("param([string]$Release)\nInvoke-WebRequest -UseBasicParsing 'http://{}/package' -TimeoutSec 2", direct.local_addr().unwrap()), &network);
        let error = run_cli(child, Duration::from_secs(10)).await.err().unwrap();
        assert_eq!(error.kind, "network_error");
        assert_eq!(
            direct.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let direct_network = Network::new(None).unwrap();
        let child = fixture_script(temp.path(), "param([string]$Release)\nthrow 'Downloaded Codex archive checksum did not match expected digest. fixture-secret-token'", &direct_network);
        let error = run_cli(child, Duration::from_secs(10)).await.err().unwrap();
        assert_eq!((error.stage, error.kind), ("checksum", "validation_error"));
        assert!(!error.message.contains("fixture-secret-token"));
        let child = fixture_script(
            temp.path(),
            "param([string]$Release)\nStart-Sleep -Seconds 30",
            &direct_network,
        );
        let started = Instant::now();
        let error = run_cli(child, Duration::from_millis(150))
            .await
            .err()
            .unwrap();
        assert_eq!((error.stage, error.kind), ("run_cli", "timeout"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
