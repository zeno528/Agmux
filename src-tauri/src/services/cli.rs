//! 两端 CLI 共用的进程和任务边界；网络策略复用 crate::network。
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::now_ms;
use super::plugins::wait_child_with_timeout;

pub(super) const TASK_TIMEOUT: Duration = Duration::from_secs(900);
pub(super) use crate::network::Network;
#[cfg(test)]
pub(super) use crate::network::PROXY_KEYS;

#[derive(Clone, Debug, Serialize)]
pub struct CliStatus {
    pub installation: &'static str,
    pub source: Option<&'static str>,
    pub version: Option<String>,
    pub path: Option<String>,
    pub other_paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embedded_paths: Vec<String>,
    pub platform: String,
    pub network: &'static str,
    pub proxy: Option<String>,
    pub busy: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CliUpdate {
    pub status: CliStatus,
    pub latest_version: String,
    pub channel: String,
    pub available: bool,
}

pub(super) fn newer_version(candidate: &str, current: &str) -> Result<bool, Failure> {
    let parse = |value: &str| {
        semver::Version::parse(value)
            .map_err(|_| failure("fetch_version", "parse_error", "无法识别 CLI 版本信息"))
    };
    Ok(parse(candidate)? > parse(current)?)
}

/// 只读官方版本元数据；这里不下载或执行安装器。
pub(super) async fn check_for_update(
    network: &Network,
    url: &str,
    parse: fn(&str) -> Option<String>,
    status: CliStatus,
    channel: &str,
) -> Result<CliUpdate, Failure> {
    check_installation(&status, false)?;
    let response = network
        .client()?
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| request_error("fetch_version", error))?;
    if !response.status().is_success() {
        return Err(failure(
            "fetch_version",
            "http_error",
            format!("版本查询失败，HTTP {}", response.status().as_u16()),
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|error| request_error("fetch_version", error))?;
    let latest_version = parse(&body)
        .filter(|version| valid_version(version))
        .ok_or_else(|| failure("fetch_version", "parse_error", "官方版本信息无效"))?;
    let current = status
        .version
        .as_deref()
        .ok_or_else(|| failure("detect", "not_found", "无法读取当前 CLI 版本"))?;
    let available = newer_version(&latest_version, current)?;
    Ok(CliUpdate {
        status,
        latest_version,
        channel: channel.into(),
        available,
    })
}

/// 两端共用的 CLI 检测中文结论；安装方式措辞与设置页 UI 一致，主语写全称。
pub(super) fn status_summary(client: &str, installation: &str) -> String {
    match installation {
        "missing" => format!("尚未安装 {client} CLI"),
        "native" => format!("{client} CLI 为原生安装"),
        "broken" => format!("{client} CLI 原生安装无法运行"),
        "other" => format!("{client} CLI 为其他方式安装"),
        _ => format!("检测到多份 {client} CLI 安装"),
    }
}

/// 安装/升级动作的中文结论；action 取值只有 install/update 两种。
pub(super) fn action_summary(client: &str, action: &str, stage: ActionStage) -> String {
    let verb = if action == "install" {
        "安装"
    } else {
        "升级"
    };
    match stage {
        ActionStage::Start => format!("开始{verb} {client} CLI"),
        ActionStage::Complete => format!("{client} CLI {verb}完成并通过验证"),
        ActionStage::Failed => format!("{client} CLI {verb}失败"),
    }
}

pub(super) enum ActionStage {
    Start,
    Complete,
    Failed,
}

pub(super) fn finish_update_check(
    client: &str,
    result: Result<CliUpdate, Failure>,
) -> Result<CliUpdate, Failure> {
    match result {
        Ok(update) => {
            // 有无更新同落一条 check 事件，结论按 available 分两态；
            // 空版本/空代理落 "-"，network 字段砍（proxy 在场即走了代理）。
            let summary = if update.available {
                format!("发现 {client} CLI 新版本")
            } else {
                format!("{client} CLI 已是最新版本")
            };
            tauri_plugin_log::log::info!(
                "[app.cli.check] client={client:?} version={} latest_version={} channel={} available={} proxy={} outcome=success msg={summary:?}",
                update.status.version.as_deref().unwrap_or("-"),
                update.latest_version,
                update.channel,
                update.available,
                update.status.proxy.as_deref().unwrap_or("-"),
            );
            Ok(update)
        }
        Err(error) => {
            // 失败并回同一事件标签：rg '[app.cli.check]' 必须同时命中成败两态。
            let summary = format!("{client} CLI 检查更新失败");
            tauri_plugin_log::log::warn!(
                "[app.cli.check] client={client:?} stage={} outcome=failure failure_kind={} error={:?} msg={summary:?}",
                error.stage, error.kind, error.message
            );
            Err(error)
        }
    }
}

pub(super) fn require_update(
    checked: Option<CliUpdate>,
    status: &CliStatus,
) -> Result<CliUpdate, Failure> {
    checked
        .filter(|checked| {
            checked.available
                && checked.status.version == status.version
                && checked.status.path == status.path
                && status.installation == "native"
        })
        .ok_or_else(|| {
            failure(
                "guard",
                "validation_error",
                "请先检查更新，确认有新版本后再更新；安装状态变化后需要重新检查",
            )
        })
}

pub(super) fn verify_update(checked: &CliUpdate, after: &CliStatus) -> Result<(), Failure> {
    let version = after.version.as_deref().unwrap_or_default();
    if !newer_version(
        version,
        checked.status.version.as_deref().unwrap_or_default(),
    )? || newer_version(&checked.latest_version, version)?
    {
        return Err(failure(
            "verify_version",
            "protocol_error",
            "命令完成，但未更新到已检查的新版本；请检查客户端更新策略后重试",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct Failure {
    pub stage: &'static str,
    pub kind: &'static str,
    pub message: String,
}

pub(super) fn failure(
    stage: &'static str,
    kind: &'static str,
    message: impl Into<String>,
) -> Failure {
    Failure {
        stage,
        kind,
        message: message.into(),
    }
}

impl From<crate::network::NetworkError> for Failure {
    fn from(error: crate::network::NetworkError) -> Self {
        failure("network", error.kind, error.message)
    }
}

pub(super) fn platform(os: &str, arch: &str) -> Result<String, Failure> {
    let os = match os {
        "windows" => "win32",
        "macos" => "darwin",
        _ => {
            return Err(failure(
                "platform",
                "validation_error",
                "仅支持 Windows 与 macOS",
            ))
        }
    };
    let arch = match arch {
        "x86_64" | "x64" => "x64",
        "aarch64" | "arm64" => "arm64",
        _ => return Err(failure("platform", "validation_error", "不支持此系统架构")),
    };
    Ok(format!("{os}-{arch}"))
}

pub(super) fn current_platform() -> Result<String, Failure> {
    #[cfg(windows)]
    let arch = {
        use windows::Win32::System::SystemInformation::{
            GetNativeSystemInfo, PROCESSOR_ARCHITECTURE_AMD64, PROCESSOR_ARCHITECTURE_ARM64,
            SYSTEM_INFO,
        };
        let mut info = SYSTEM_INFO::default();
        unsafe {
            GetNativeSystemInfo(&mut info);
        }
        match unsafe { info.Anonymous.Anonymous.wProcessorArchitecture } {
            PROCESSOR_ARCHITECTURE_ARM64 => "arm64".to_string(),
            PROCESSOR_ARCHITECTURE_AMD64 => "x86_64".to_string(),
            _ => "unsupported".to_string(),
        }
    };
    #[cfg(not(windows))]
    let arch = {
        let mut arch = sysinfo::System::cpu_arch();
        if cfg!(target_os = "macos") && arch == "x86_64" {
            let translated = Command::new("/usr/sbin/sysctl")
                .args(["-n", "sysctl.proc_translated"])
                .output();
            if translated.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1") {
                arch = "arm64".into();
            }
        }
        arch
    };
    platform(std::env::consts::OS, &arch)
}

pub(super) fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

pub(super) fn valid_version(version: &str) -> bool {
    let core = version.split('-').next().unwrap_or_default();
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

pub(super) fn combine_sources(
    sources: impl IntoIterator<Item = &'static str>,
) -> Option<&'static str> {
    let mut sources = sources.into_iter();
    let first = sources.next()?;
    Some(if sources.any(|source| source != first) {
        "multiple"
    } else {
        first
    })
}

pub(super) fn version(
    path: &Path,
    parse_version: fn(&str) -> Option<String>,
) -> Result<String, Failure> {
    let child = command(path)
        .arg("--version")
        .env("DISABLE_AUTOUPDATER", "1")
        .spawn()
        .map_err(|_| {
            failure(
                "verify_version",
                "io_error",
                "无法执行 CLI，请检查程序路径和权限",
            )
        })?;
    let output = wait_child_with_timeout(child, Duration::from_secs(5))
        .map_err(|_| failure("verify_version", "io_error", "无法读取 CLI 输出"))?
        .ok_or_else(|| failure("verify_version", "timeout", "CLI 版本检测超时"))?;
    if !output.status.success() {
        return Err(failure("verify_version", "internal", "CLI 版本检测失败"));
    }
    parse_version(&output.stdout)
        .ok_or_else(|| failure("verify_version", "parse_error", "CLI 没有返回有效版本号"))
}

pub(super) fn remaining(started: Instant, stage: &'static str) -> Result<Duration, Failure> {
    TASK_TIMEOUT
        .checked_sub(started.elapsed())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| failure(stage, "timeout", "CLI 操作超过 15 分钟，请检查网络后重试"))
}

/// 两端外壳共用的失败日志行；格式串只此一份，防止两处漂移。
pub(super) fn log_cli_failure(
    client: &str,
    task_id: &str,
    action: &str,
    started: Instant,
    error: &Failure,
) {
    let summary = action_summary(client, action, ActionStage::Failed);
    tauri_plugin_log::log::warn!(
        "[app.cli.failure] client={client:?} task_id={task_id:?} action={action} stage={} outcome=failure failure_kind={} duration_ms={} error={:?} msg={summary:?}",
        error.stage, error.kind, started.elapsed().as_millis(), error.message
    );
}

pub(super) async fn fetch(
    client: &reqwest::Client,
    url: &str,
    started: Instant,
    stage: &'static str,
) -> Result<reqwest::Response, Failure> {
    let response = client
        .get(url)
        .timeout(remaining(started, stage)?)
        .send()
        .await
        .map_err(|e| request_error(stage, e))?;
    if !response.status().is_success() {
        return Err(failure(
            stage,
            "http_error",
            format!("下载服务器返回 HTTP {}", response.status().as_u16()),
        ));
    }
    Ok(response)
}

/// 两端都下载官方引导脚本；包校验和安装布局由官方安装器负责。
pub(super) async fn fetch_installer(
    network: &Network,
    url: &str,
    allowed_hosts: &[&str],
    directory: &Path,
    started: Instant,
) -> Result<(PathBuf, usize), Failure> {
    let response = fetch(&network.client()?, url, started, "fetch_installer").await?;
    let source = response.url().clone();
    let script = response
        .bytes()
        .await
        .map_err(|e| request_error("fetch_installer", e))?;
    validate_installer(&source, &script, allowed_hosts)?;
    let path = directory.join(if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    });
    std::fs::write(&path, &script)
        .map_err(|_| failure("prepare", "io_error", "无法保存安装脚本"))?;
    Ok((path, script.len()))
}

fn validate_installer(
    source: &url::Url,
    script: &[u8],
    allowed_hosts: &[&str],
) -> Result<(), Failure> {
    if source.scheme() != "https"
        || !source
            .host_str()
            .is_some_and(|host| allowed_hosts.contains(&host))
    {
        return Err(failure(
            "fetch_installer",
            "validation_error",
            "安装脚本跳转到了非官方来源，已停止操作",
        ));
    }
    if script.is_empty()
        || script.len() > 1024 * 1024
        || std::str::from_utf8(script).map_or(true, |text| {
            let text = text.trim_start_matches('\u{feff}').trim_start();
            text.is_empty() || text.starts_with('<')
        })
    {
        return Err(failure(
            "fetch_installer",
            "parse_error",
            "服务器没有返回有效的官方安装脚本",
        ));
    }
    Ok(())
}

#[cfg(windows)]
const POWERSHELL_INSTALLER: &str = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if ([string]::IsNullOrEmpty($env:HTTPS_PROXY)) {
    [System.Net.WebRequest]::DefaultWebProxy = $null
} else {
    $address = [Uri]$env:HTTPS_PROXY
    $builder = New-Object System.UriBuilder($address)
    $builder.UserName = ''
    $builder.Password = ''
    $proxy = New-Object System.Net.WebProxy($builder.Uri)
    $PSDefaultParameterValues['Invoke-WebRequest:Proxy'] = $builder.Uri
    $PSDefaultParameterValues['Invoke-RestMethod:Proxy'] = $builder.Uri
    if ($address.UserInfo) {
        $parts = $address.UserInfo -split ':', 2
        $user = [Uri]::UnescapeDataString($parts[0])
        $password = if ($parts.Length -gt 1) { [Uri]::UnescapeDataString($parts[1]) } else { '' }
        $credential = New-Object System.Management.Automation.PSCredential($user, (ConvertTo-SecureString $password -AsPlainText -Force))
        $proxy.Credentials = $credential.GetNetworkCredential()
        $PSDefaultParameterValues['Invoke-WebRequest:ProxyCredential'] = $credential
        $PSDefaultParameterValues['Invoke-RestMethod:ProxyCredential'] = $credential
    }
    [System.Net.WebRequest]::DefaultWebProxy = $proxy
}
$LASTEXITCODE = 0
& $env:AGMUX_CLI_INSTALLER_PATH $env:AGMUX_CLI_INSTALLER_TARGET
exit $LASTEXITCODE
"#;

/// 产品运行时使用 Windows 自带的 5.1 或 macOS 自带的 bash，不依赖开发用 pwsh。
pub(super) fn official_installer_command(
    script: &Path,
    network: &Network,
    target: &str,
    release_flag: bool,
) -> Result<Command, Failure> {
    #[cfg(windows)]
    let mut child = {
        let powershell = std::env::var_os("WINDIR")
            .map(|dir| PathBuf::from(dir).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .filter(|path| path.is_file())
            .ok_or_else(|| {
                failure(
                    "prepare",
                    "not_found",
                    "未找到官方安装器所需的系统 PowerShell",
                )
            })?;
        let mut child = command(&powershell);
        // 防止父进程的 PowerShell 7 模块目录污染系统 5.1。
        child.env_remove("PSModulePath");
        child.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            POWERSHELL_INSTALLER,
        ]);
        child.env("AGMUX_CLI_INSTALLER_PATH", script);
        child.env("AGMUX_CLI_INSTALLER_TARGET", target);
        child
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut child = command(Path::new("/bin/bash"));
        child.arg(script);
        if release_flag {
            child.arg("--release");
        }
        child.arg(target);
        child
    };
    child.current_dir(script.parent().unwrap_or(Path::new(".")));
    #[cfg(windows)]
    let _ = release_flag;
    network.apply(&mut child);
    Ok(child)
}

pub(super) async fn run_cli(
    mut command: Command,
    timeout: Duration,
) -> Result<super::plugins::ChildOutput, Failure> {
    let output = tauri::async_runtime::spawn_blocking(move || {
        let child = command
            .spawn()
            .map_err(|_| failure("run_cli", "io_error", "无法启动 CLI，请检查安装目录权限"))?;
        wait_child_with_timeout(child, timeout)
            .map_err(|_| failure("run_cli", "io_error", "无法等待 CLI 退出"))?
            .ok_or_else(|| {
                failure(
                    "run_cli",
                    "timeout",
                    "安装或更新超时，请检查网络或代理后重试",
                )
            })
    })
    .await
    .map_err(|_| failure("run_cli", "internal", "CLI 执行任务失败"))??;
    if !output.status.success() {
        // 不落原始 CLI 输出：它可能包含用户配置或凭证，只提取已知故障类别。
        let text = format!("{} {}", output.stderr, output.stdout).to_ascii_lowercase();
        if text.contains("checksum")
            && (text.contains("not match")
                || text.contains("mismatch")
                || text.contains("verification failed"))
        {
            return Err(failure(
                "checksum",
                "validation_error",
                "官方安装器 SHA256 校验失败，未使用下载的安装包",
            ));
        }
        if text.contains("sha-256 digest") || text.contains("release metadata") {
            return Err(failure(
                "fetch_manifest",
                if text.contains("could not fetch") {
                    "network_error"
                } else {
                    "parse_error"
                },
                "官方安装器无法获取有效的发布清单或校验信息，请检查网络后重试",
            ));
        }
        let (kind, hint) = if text.contains("permission")
            || text.contains("access denied")
            || text.contains("eacces")
        {
            ("io_error", "请检查安装目录权限及程序是否被占用")
        } else if text.contains("proxy")
            || text.contains("fetch")
            || text.contains("network")
            || text.contains("connect")
            || text.contains("webcmdletwebresponseexception")
        {
            ("network_error", "请检查网络、代理或下载服务器是否可访问")
        } else {
            ("internal", "请检查安装目录、运行中的进程或下载服务器")
        };
        return Err(failure(
            "run_cli",
            kind,
            format!("CLI 退出码 {:?}；{hint}", output.status.code()),
        ));
    }
    Ok(output)
}

pub(super) fn request_error(stage: &'static str, error: reqwest::Error) -> Failure {
    let checking = stage == "fetch_version";
    let (kind, message) = if error.is_timeout() {
        (
            "timeout",
            if checking {
                "版本查询超时，请检查网络或代理"
            } else {
                "下载请求超时，请检查网络或代理"
            },
        )
    } else if error.is_decode() {
        (
            "parse_error",
            if checking {
                "无法解析服务器返回的版本信息"
            } else {
                "无法解析服务器返回的下载信息"
            },
        )
    } else {
        (
            "network_error",
            if checking {
                "版本查询失败，请检查网络或代理"
            } else {
                "下载中断，请检查网络或代理"
            },
        )
    };
    failure(stage, kind, message)
}

pub(super) fn task_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", now_ms(), SEQUENCE.fetch_add(1, Ordering::Relaxed))
}

pub(super) fn check_installation(status: &CliStatus, install: bool) -> Result<(), Failure> {
    if matches!(status.installation, "other" | "conflict") {
        return Err(failure(
            "detect",
            "validation_error",
            "检测到其他安装方式或多份安装，请先通过原安装方式处理，避免混装",
        ));
    }
    if !install && status.installation != "native" {
        return Err(failure(
            "detect",
            "not_found",
            "未找到有效的原生安装，请先安装 CLI",
        ));
    }
    if install && status.installation == "native" {
        return Err(failure(
            "detect",
            "validation_error",
            "已有原生安装，请先检查更新后选择更新",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_status() -> CliStatus {
        CliStatus {
            installation: "native",
            source: Some("native"),
            version: Some("1.9.0".into()),
            path: Some("/fixture/bin/cli".into()),
            other_paths: vec![],
            embedded_paths: vec![],
            platform: "darwin-arm64".into(),
            network: "direct",
            proxy: None,
            busy: false,
        }
    }

    #[tokio::test]
    async fn version_query_errors_never_report_a_package_download() {
        let network = Network::new(None).unwrap();
        let client = network.client().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/versions");
        // TCP 端口可连接但不返回 HTTP 响应，产生真实请求超时。
        for (stage, message) in [
            ("fetch_version", "版本查询超时，请检查网络或代理"),
            ("fetch_installer", "下载请求超时，请检查网络或代理"),
        ] {
            let error = client
                .get(&url)
                .timeout(Duration::from_millis(50))
                .send()
                .await
                .unwrap_err();
            let failed = request_error(stage, error);
            assert_eq!((failed.stage, failed.kind), (stage, "timeout"));
            assert_eq!(failed.message, message);
        }
        drop(listener);
        let failed = check_for_update(
            &network,
            &url,
            |text| Some(text.trim().into()),
            native_status(),
            "latest",
        )
        .await
        .unwrap_err();
        assert_eq!(
            (failed.stage, failed.kind),
            ("fetch_version", "network_error")
        );
        assert_eq!(failed.message, "版本查询失败，请检查网络或代理");
        let failed = finish_update_check("Claude Code", Err(failed)).unwrap_err();
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            serde_json::json!({
                "stage": "fetch_version",
                "kind": "network_error",
                "message": "版本查询失败，请检查网络或代理",
            })
        );
    }

    #[tokio::test]
    async fn checking_reads_only_metadata_and_never_treats_equal_or_older_versions_as_updates() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let replies = [
            "1.10.0",
            "1.9.0",
            "1.8.0",
            "<html>error</html>",
            "unavailable",
        ];
        let server = std::thread::spawn(move || {
            for (index, body) in replies.iter().enumerate() {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 2048];
                let count = stream.read(&mut request).unwrap();
                assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /versions "));
                let code = if index == 4 {
                    "503 Unavailable"
                } else {
                    "200 OK"
                };
                write!(
                    stream,
                    "HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let network = Network::new(None).unwrap();
        for expected in [Some(true), Some(false), Some(false), None, None] {
            let result = check_for_update(
                &network,
                &format!("http://{address}/versions"),
                |text| Some(text.trim().into()),
                native_status(),
                "latest",
            )
            .await;
            match expected {
                Some(expected) => assert_eq!(result.unwrap().available, expected),
                None => assert_eq!(result.unwrap_err().stage, "fetch_version"),
            }
        }
        server.join().unwrap();
        assert!(newer_version("1.10.0", "1.9.0").unwrap());
        assert!(newer_version("1.10.0", "1.10.0-beta.10").unwrap());
        assert!(!newer_version("1.10.0-beta.2", "1.10.0-beta.10").unwrap());
        assert!(!newer_version("1.10.0-alpha.1", "1.10.0").unwrap());
    }

    #[test]
    fn upgrade_requires_an_available_check_for_the_same_installation_and_verifies_the_result() {
        let before = native_status();
        let checked = CliUpdate {
            status: before.clone(),
            latest_version: "1.10.0".into(),
            channel: "latest".into(),
            available: true,
        };
        assert_eq!(require_update(None, &before).unwrap_err().stage, "guard");
        let mut no_update = checked.clone();
        no_update.available = false;
        assert!(require_update(Some(no_update), &before).is_err());
        for (path, version) in [("/other/bin/cli", "1.9.0"), ("/fixture/bin/cli", "1.10.0")] {
            let mut changed = before.clone();
            changed.path = Some(path.into());
            changed.version = Some(version.into());
            assert!(require_update(Some(checked.clone()), &changed).is_err());
        }
        require_update(Some(checked.clone()), &before).unwrap();
        assert!(verify_update(&checked, &before).is_err());
        for (version, valid) in [("1.9.1", false), ("1.10.0", true), ("1.11.0", true)] {
            let mut after = before.clone();
            after.version = Some(version.into());
            assert_eq!(verify_update(&checked, &after).is_ok(), valid);
        }
    }

    #[test]
    fn official_installer_rejects_foreign_redirects_and_invalid_downloads() {
        let allowed = ["publisher.invalid", "downloads.publisher.invalid"];
        for host in allowed {
            validate_installer(
                &url::Url::parse(&format!("https://{host}/install.ps1")).unwrap(),
                b"Write-Output 'fixture'",
                &allowed,
            )
            .unwrap();
        }
        for source in [
            "http://publisher.invalid/install.ps1",
            "https://publisher.invalid.evil.invalid/install.ps1",
            "https://other.invalid/install.ps1",
        ] {
            let error = validate_installer(&url::Url::parse(source).unwrap(), b"fixture", &allowed)
                .unwrap_err();
            assert_eq!(
                (error.stage, error.kind),
                ("fetch_installer", "validation_error")
            );
        }
        let source = url::Url::parse("https://publisher.invalid/install.ps1").unwrap();
        for script in [
            b"".as_slice(),
            b" \n",
            b" <html>error</html>",
            b"\xef\xbb\xbf<html>error</html>",
            b"\xff",
        ] {
            assert_eq!(
                validate_installer(&source, script, &allowed)
                    .unwrap_err()
                    .kind,
                "parse_error"
            );
        }
        assert_eq!(
            validate_installer(&source, &vec![b'x'; 1024 * 1024 + 1], &allowed)
                .unwrap_err()
                .kind,
            "parse_error"
        );
    }
}
