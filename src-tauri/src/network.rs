//! 应用共用的网络策略；按需读取代理，本次操作的 HTTP 和子进程使用同一快照。
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::RwLock;
use std::time::Duration;

use crate::models::ProxyMode;

static PROXY_SETTING: RwLock<(ProxyMode, String)> = RwLock::new((ProxyMode::Auto, String::new()));

// 设置文件已由启动流程读取；这里只保留选择，自动模式仍按每次操作现查系统代理。
pub(crate) fn set_proxy(mode: ProxyMode, url: String) {
    *PROXY_SETTING
        .write()
        .unwrap_or_else(|error| error.into_inner()) = (mode, url);
}

pub(crate) const PROXY_KEYS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];
const LOOPBACK: &str = "localhost,127.0.0.1,::1";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct NetworkError {
    pub(crate) kind: &'static str,
    pub(crate) message: &'static str,
}

#[derive(Clone)]
pub(crate) struct Network {
    pub(crate) proxy: Option<String>,
    pub(crate) display: Option<String>,
    http_proxy: Option<reqwest::Proxy>,
}

impl Network {
    pub(crate) fn new(proxy: Option<String>) -> Result<Self, NetworkError> {
        let mut display = None;
        let http_proxy = if let Some(value) = &proxy {
            let invalid = || NetworkError {
                kind: "validation_error",
                message: "代理地址无效，请填写完整的 HTTP(S) 代理地址",
            };
            let mut url = url::Url::parse(value).map_err(|_| invalid())?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(NetworkError {
                    kind: "validation_error",
                    message: "仅支持 HTTP(S) 代理，请启用代理软件的 HTTP 代理端口",
                });
            }
            let parsed = reqwest::Proxy::all(value)
                .map_err(|_| invalid())?
                .no_proxy(reqwest::NoProxy::from_string(LOOPBACK));
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_path("");
            url.set_query(None);
            url.set_fragment(None);
            // 空路径经 WHATWG 序列化必补 "/"，展示与日志统一去尾斜杠；原始 proxy 供 env 与 reqwest 使用，保持原样。
            let mut address = url.to_string();
            if address.ends_with('/') {
                address.pop();
            }
            display = Some(address);
            Some(parsed)
        } else {
            None
        };
        Ok(Self {
            proxy,
            display,
            http_proxy,
        })
    }

    pub(crate) fn detect() -> Result<Self, NetworkError> {
        let (mode, url) = PROXY_SETTING
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        Self::selected(&mode, &url, detect_system_proxy)
    }

    fn selected(
        mode: &ProxyMode,
        url: &str,
        automatic: impl FnOnce() -> Result<Option<String>, NetworkError>,
    ) -> Result<Self, NetworkError> {
        Self::new(match mode {
            ProxyMode::Auto => automatic()?,
            ProxyMode::Off => None,
            ProxyMode::Custom => Some(url.trim().to_owned()),
        })
    }

    /// 系统检测包含阻塞平台调用，异步请求按需获取，启动时不初始化。
    pub(crate) async fn current() -> Result<Self, NetworkError> {
        tauri::async_runtime::spawn_blocking(Self::detect)
            .await
            .map_err(|_| NetworkError {
                kind: "internal",
                message: "读取网络设置失败",
            })?
    }

    pub(crate) fn mode(&self) -> &'static str {
        if self.proxy.is_some() {
            "proxy"
        } else {
            "direct"
        }
    }

    pub(crate) fn configure_http(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        // 清除隐式检测，避免同一任务的下载与子进程采用不同网络方式。
        let builder = builder.no_proxy();
        match &self.http_proxy {
            Some(proxy) => builder.proxy(proxy.clone()),
            None => builder,
        }
    }

    pub(crate) fn builder(&self) -> reqwest::ClientBuilder {
        self.configure_http(reqwest::Client::builder())
    }

    #[cfg(any(windows, target_os = "macos"))]
    pub(crate) fn updater(
        self,
        builder: tauri_plugin_updater::UpdaterBuilder,
    ) -> tauri_plugin_updater::UpdaterBuilder {
        // 更新器会将此闭包保留到下载阶段，检查与下载沿用同一份网络快照。
        builder.configure_client(move |client| self.configure_http(client))
    }

    /// CLI 下载沿用原来的连接超时；其他链路在 builder 上保留各自的超时与认证。
    pub(crate) fn client(&self) -> Result<reqwest::Client, NetworkError> {
        self.builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| NetworkError {
                kind: "internal",
                message: "无法创建下载客户端",
            })
    }

    pub(crate) fn env(&self) -> BTreeMap<&'static str, &str> {
        PROXY_KEYS
            .into_iter()
            .map(|key| (key, self.proxy.as_deref().unwrap_or("")))
            .chain([("NO_PROXY", LOOPBACK), ("no_proxy", LOOPBACK)])
            .collect()
    }

    pub(crate) fn apply(&self, command: &mut Command) {
        command.envs(self.env());
    }
}

/// 仅输出脱敏地址；检测到损坏的代理也不落原文。
pub(crate) fn proxy_note(proxy: &Option<String>) -> String {
    match Network::new(proxy.clone()) {
        Ok(network) => match network.display {
            // 直连不落 proxy 字段：字段缺席即直连，写 proxy=None 是占位噪音
            Some(address) => format!(" proxy={address:?}"),
            None => String::new(),
        },
        Err(_) => " proxy=invalid".into(),
    }
}

/// 沿用现有识别顺序：环境变量优先，其次 Windows/macOS 系统代理。
pub(crate) fn detect_system_proxy() -> Result<Option<String>, NetworkError> {
    for key in [
        "https_proxy",
        "HTTPS_PROXY",
        "http_proxy",
        "HTTP_PROXY",
        "all_proxy",
        "ALL_PROXY",
    ] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return Ok(Some(value.trim().to_string()));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        // scutil --proxy 输出 "  HTTPSProxy : 127.0.0.1" 形式的键值行
        let output = std::process::Command::new("scutil")
            .arg("--proxy")
            .output()
            .map_err(|_| SYSTEM_PROXY_READ_ERROR)?;
        if !output.status.success() {
            return Err(SYSTEM_PROXY_READ_ERROR);
        }
        let text = String::from_utf8_lossy(&output.stdout);
        for (prefix, scheme) in [("HTTPS", "http"), ("HTTP", "http"), ("SOCKS", "socks5")] {
            if scutil_value(&text, &format!("{prefix}Enable")).as_deref() == Some("1") {
                if let (Some(host), Some(port)) = (
                    scutil_value(&text, &format!("{prefix}Proxy")),
                    scutil_value(&text, &format!("{prefix}Port")),
                ) {
                    return Ok(Some(format!("{scheme}://{host}:{port}")));
                }
                return Err(SYSTEM_PROXY_READ_ERROR);
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 与 run_codex_plugin 同理：GUI 进程 spawn 控制台程序必须隐藏窗口，
        // 否则插件页每次 CLI 调用都会闪出 reg.exe 黑窗
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let settings = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
        // 一次读取整个键：缺少 ProxyEnable 是未配置，命令失败才是读取故障。
        let output = std::process::Command::new("reg")
            .args(["query", settings])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|_| SYSTEM_PROXY_READ_ERROR)?;
        if !output.status.success() {
            return Err(SYSTEM_PROXY_READ_ERROR);
        }
        windows_system_proxy(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(windows))]
    Ok(None)
}

#[cfg(any(windows, target_os = "macos", test))]
const SYSTEM_PROXY_READ_ERROR: NetworkError = NetworkError {
    kind: "io_error",
    message: "读取系统代理设置失败，请检查系统代理设置后重试",
};

#[cfg(any(windows, test))]
fn windows_system_proxy(text: &str) -> Result<Option<String>, NetworkError> {
    let value = |name| {
        text.lines().find_map(|line| {
            let (key, value) = line.trim().split_once(char::is_whitespace)?;
            (key == name).then_some(value.trim())
        })
    };
    match value("ProxyEnable") {
        None => Ok(None),
        Some(enabled) => {
            let enabled = enabled
                .strip_prefix("REG_DWORD")
                .map(str::trim)
                .ok_or(SYSTEM_PROXY_READ_ERROR)?;
            match enabled {
                "0x0" => Ok(None),
                "0x1" => value("ProxyServer")
                    .and_then(|value| value.strip_prefix("REG_SZ"))
                    .and_then(windows_proxy_address)
                    .map(Some)
                    .ok_or(SYSTEM_PROXY_READ_ERROR),
                _ => Err(SYSTEM_PROXY_READ_ERROR),
            }
        }
    }
}

#[cfg(any(windows, test))]
fn windows_proxy_address(value: &str) -> Option<String> {
    let value = value.trim();
    let endpoint = if value.contains('=') {
        ["https", "http", "socks"]
            .into_iter()
            .find_map(|protocol| {
                value.split(';').find_map(|entry| {
                    let (key, address) = entry.trim().split_once('=')?;
                    (key.eq_ignore_ascii_case(protocol) && !address.trim().is_empty())
                        .then(|| (address.trim(), protocol == "socks"))
                })
            })?
    } else {
        (value, false)
    };
    if endpoint.0.is_empty() {
        return None;
    }
    Some(if endpoint.0.contains("://") {
        endpoint.0.to_string()
    } else {
        format!(
            "{}://{}",
            if endpoint.1 { "socks5" } else { "http" },
            endpoint.0
        )
    })
}

/// 解析 scutil --proxy 输出里的单行键值（"  HTTPSProxy : 127.0.0.1"）。
#[cfg(any(target_os = "macos", test))]
fn scutil_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.trim().split(" : ");
        match parts.next() {
            Some(name) if name == key => parts.next().map(|value| value.trim().to_string()),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn three_modes_choose_one_route_and_only_auto_reads_system_proxy() {
        let automatic = || Ok(Some("http://user:fixture-secret@127.0.0.1:12345".into()));
        let auto = Network::selected(&ProxyMode::Auto, "ignored", automatic).unwrap();
        assert_eq!(auto.proxy, automatic().unwrap());
        assert_eq!(auto.display.as_deref(), Some("http://127.0.0.1:12345"));
        assert!(Network::selected(&ProxyMode::Auto, "ignored", || Ok(None))
            .unwrap()
            .proxy
            .is_none());
        let off = Network::selected(&ProxyMode::Off, "invalid", || {
            panic!("关闭模式不得检测代理")
        })
        .unwrap();
        assert!(off.proxy.is_none());
        assert!(PROXY_KEYS.iter().all(|key| off.env()[key].is_empty()));
        let custom = Network::selected(&ProxyMode::Custom, " http://127.0.0.1:23456 ", || {
            panic!("自定义模式不得检测代理")
        })
        .unwrap();
        assert_eq!(custom.proxy.as_deref(), Some("http://127.0.0.1:23456"));
        assert!(PROXY_KEYS
            .iter()
            .all(|key| custom.env()[key] == "http://127.0.0.1:23456"));
        assert!(Network::selected(&ProxyMode::Custom, "", automatic).is_err());
        assert!(Network::selected(&ProxyMode::Auto, "", || Ok(Some("invalid".into()))).is_err());
        // 新的选择不会更改已构造的操作快照。
        assert_eq!(auto.proxy, automatic().unwrap());
    }

    #[test]
    fn automatic_detection_failure_does_not_become_direct() {
        let error = Network::selected(&ProxyMode::Auto, "", || Err(SYSTEM_PROXY_READ_ERROR))
            .err()
            .unwrap();
        assert_eq!(error.kind, "io_error");
        assert_eq!(error.message, SYSTEM_PROXY_READ_ERROR.message);
    }

    #[test]
    fn windows_system_proxy_distinguishes_unconfigured_disabled_and_broken_settings() {
        assert!(windows_system_proxy("HKEY_CURRENT_USER\\fixture")
            .unwrap()
            .is_none());
        assert!(windows_system_proxy("    ProxyEnable    REG_DWORD    0x0")
            .unwrap()
            .is_none());
        let enabled = "    ProxyEnable    REG_DWORD    0x1\n";
        assert_eq!(
            windows_system_proxy(&format!(
                "{enabled}    ProxyServer    REG_SZ    fixture.invalid:23456"
            ))
            .unwrap(),
            Some("http://fixture.invalid:23456".into())
        );
        for text in [
            enabled,
            "    ProxyEnable    REG_SZ    0x1",
            "    ProxyEnable    REG_DWORD    invalid",
            "    ProxyEnable    REG_DWORD    0x1\n    ProxyServer    REG_SZ    ",
        ] {
            assert_eq!(windows_system_proxy(text).unwrap_err().kind, "io_error");
        }
    }

    fn respond(listener: TcpListener, body: &'static str) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let start = std::time::Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    // macOS 的 accept 会继承 listener 的非阻塞标志，必须显式
                    // 恢复阻塞，否则请求字节未到时 read 立刻 WouldBlock。
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        break stream;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(start.elapsed() < Duration::from_secs(5), "未收到预期请求");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8(request).unwrap()
        })
    }

    #[tokio::test]
    async fn http_uses_selected_proxy_with_auth_and_direct_choice_clears_other_proxies() {
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let proxy_request = respond(proxy, "through proxy");
        let network = Network::selected(
            &ProxyMode::Custom,
            &format!("http://u%73er:p%40ss@{proxy_address}"),
            || panic!("custom 不得读取系统代理"),
        )
        .unwrap();
        let response = network
            .builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
            .get("http://fixture.invalid/download")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "through proxy");
        let request = proxy_request.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get http://fixture.invalid/download "));
        assert!(request.contains("proxy-authorization: basic dxnlcjpwqhnz"));
        assert_eq!(network.display, Some(format!("http://{proxy_address}")));
        assert!(!proxy_note(&network.proxy).contains("p%40ss"));

        let direct = TcpListener::bind("127.0.0.1:0").unwrap();
        let direct_address = direct.local_addr().unwrap();
        let direct_request = respond(direct, "direct");
        let builder = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .resolve("fixture.invalid", direct_address)
            .timeout(Duration::from_secs(3));
        let response = Network::selected(&ProxyMode::Off, "", || panic!("off 不得读取系统代理"))
            .unwrap()
            .configure_http(builder)
            .build()
            .unwrap()
            .get(format!(
                "http://fixture.invalid:{}/download",
                direct_address.port()
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "direct");
        assert!(!direct_request
            .join()
            .unwrap()
            .to_ascii_lowercase()
            .contains("proxy-authorization"));
    }

    #[tokio::test]
    async fn failed_proxy_never_falls_back_and_loopback_remains_direct() {
        let unused_proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_address = unused_proxy.local_addr().unwrap();
        drop(unused_proxy);
        let network = Network::new(Some(format!("http://{closed_address}"))).unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_address = origin.local_addr().unwrap();
        origin.set_nonblocking(true).unwrap();
        let client = network
            .builder()
            .resolve("fixture.invalid", origin_address)
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(client
            .get(format!(
                "http://fixture.invalid:{}/download",
                origin_address.port()
            ))
            .send()
            .await
            .is_err());
        assert_eq!(
            origin.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );

        let local_request = respond(origin, "local");
        let response = client
            .get(format!("http://{origin_address}/local"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "local");
        local_request.join().unwrap();
    }

    #[test]
    fn invalid_proxy_reports_error_without_credentials_and_child_env_uses_same_snapshot() {
        for proxy in ["", "not a url with fixture-secret", "socks5://127.0.0.1:1"] {
            let error = Network::new(Some(proxy.into())).err().unwrap();
            assert_eq!(error.kind, "validation_error");
            assert!(!error.to_string().contains("fixture-secret"));
        }
        let proxy = "http://user:fixture-secret@127.0.0.1:12345";
        let network = Network::new(Some(proxy.into())).unwrap();
        let mut command = Command::new("fixture");
        network.apply(&mut command);
        for key in PROXY_KEYS {
            assert_eq!(network.env()[key], proxy);
        }
        assert_eq!(network.env()["NO_PROXY"], LOOPBACK);
        // Windows 环境变量名不区分大小写，std::process::Command 会合并大小写键。
        assert_eq!(
            command.get_envs().count(),
            if cfg!(windows) { 4 } else { 8 }
        );
        let direct = Network::new(None).unwrap();
        for key in PROXY_KEYS {
            assert_eq!(direct.env()[key], "");
        }
    }

    #[test]
    fn windows_proxy_mapping_preserves_protocol_and_prioritizes_https() {
        assert_eq!(
            windows_proxy_address("http=127.0.0.1:1;https=127.0.0.1:2"),
            Some("http://127.0.0.1:2".into())
        );
        assert_eq!(
            windows_proxy_address("socks=127.0.0.1:3"),
            Some("socks5://127.0.0.1:3".into())
        );
        assert_eq!(
            windows_proxy_address("127.0.0.1:4"),
            Some("http://127.0.0.1:4".into())
        );
        assert_eq!(windows_proxy_address(""), None);
    }

    #[test]
    fn scutil_proxy_fields_parse() {
        let text = "<dictionary> {\n  HTTPEnable : 1\n  HTTPPort : 20080\n  HTTPProxy : 127.0.0.1\n  HTTPSEnable : 1\n  HTTPSPort : 20080\n  HTTPSProxy : 127.0.0.1\n}\n";
        assert_eq!(
            scutil_value(text, "HTTPSProxy").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(scutil_value(text, "HTTPSPort").as_deref(), Some("20080"));
        assert_eq!(scutil_value(text, "NoSuchKey"), None);
    }
}
