use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{json, Value};

use super::plugins::SkillTool;
use super::{app_err, AppContext, AppResult};
use crate::models::{McpConnectionState, McpProbeResult, McpServerInfo, McpServerSpec, McpTool};

/// 我们声明支持的最高协议版本。2026-07-28 那版把 initialize 握手取消了，改成每个请求
/// 自带版本号，所以能声明的最高版就是最后一个握手式 revision。报低会被服务端降级迁就
/// （明明会说新话却只能说老话），报高等于撒谎，只有这个值是对的。
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_TOOLS: usize = 500;
const MAX_DETAIL_CHARS: usize = 240;

struct ProbeFailure {
    status: Option<u16>,
    message: String,
}

#[derive(Debug, Deserialize)]
struct RawTool {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(rename = "inputSchema", default = "empty_object")]
    input_schema: Value,
}

fn empty_object() -> Value {
    json!({})
}

fn clean_detail(detail: &str, secrets: &[String]) -> String {
    let mut text = detail.trim().to_string();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        text = text.replace(secret, "<redacted>");
    }
    text.chars().take(MAX_DETAIL_CHARS).collect()
}

fn failed_result(
    start: Instant,
    status: Option<u16>,
    message: impl Into<String>,
) -> McpProbeResult {
    McpProbeResult {
        ok: false,
        connection_state: None,
        auth_status: None,
        latency_ms: Some(start.elapsed().as_millis()),
        status,
        protocol_version: None,
        server_info: None,
        tools: Vec::new(),
        tools_truncated: false,
        error: Some(message.into()),
        tools_error: None,
    }
}

fn parse_rpc_values(body: &str) -> Vec<Value> {
    let trimmed = body.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return vec![value];
    }

    let mut values = Vec::new();
    let mut data = String::new();
    for line in body.lines() {
        if let Some(part) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(part.trim_start());
        } else if line.trim().is_empty() {
            push_sse_value(&mut data, &mut values);
        }
    }
    push_sse_value(&mut data, &mut values);
    values
}

fn push_sse_value(data: &mut String, values: &mut Vec<Value>) {
    let text = data.trim();
    if !text.is_empty() && text != "[DONE]" {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            values.push(value);
        }
    }
    data.clear();
}

/// 协议版本协商失败的错误码：服务端不接受我们报的版本，并在 data.supported 里列出它支持的。
/// 这不是"连不上"——服务端活着、说的也是 MCP，只是不接受我们报的版本号，所以单独给它一句
/// 能读懂的话："不支持"三个字同时被日志的 failure_kind 判定复用（见 log_probe_outcome）。
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

fn version_unsupported_message(error: &Value) -> String {
    let supported: Vec<&str> = error
        .get("data")
        .and_then(|data| data.get("supported"))
        .and_then(Value::as_array)
        .map(|versions| versions.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if supported.is_empty() {
        return format!("服务端不支持协议版本 {MCP_PROTOCOL_VERSION}");
    }
    format!(
        "服务端不支持协议版本 {MCP_PROTOCOL_VERSION}，它支持：{}",
        supported.join("、")
    )
}

fn rpc_result(values: Vec<Value>, id: u64, secrets: &[String]) -> Result<Value, String> {
    for value in values {
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
            if code == UNSUPPORTED_PROTOCOL_VERSION {
                return Err(clean_detail(&version_unsupported_message(error), secrets));
            }
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("服务器返回 MCP 错误");
            return Err(clean_detail(
                &format!("MCP 错误 {code}: {message}"),
                secrets,
            ));
        }
        return value
            .get("result")
            .cloned()
            .ok_or_else(|| "MCP 响应缺少 result".to_string());
    }
    Err("未收到 MCP 请求响应".to_string())
}

fn parse_server_info(result: &Value) -> Option<McpServerInfo> {
    let info = result.get("serverInfo")?;
    let name = info.get("name").and_then(Value::as_str).map(str::to_string);
    let version = info
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(McpServerInfo { name, version })
}

fn parse_tools_page(mut result: Value) -> Result<(Vec<McpTool>, Option<String>), String> {
    // take() 拿走 tools 数组所有权，免去为反序列化克隆整个数组
    let raw_tools: Vec<RawTool> = serde_json::from_value(
        result
            .get_mut("tools")
            .ok_or_else(|| "tools/list 响应缺少 tools".to_string())?
            .take(),
    )
    .map_err(|error| format!("工具列表格式无效: {error}"))?;
    let tools = raw_tools
        .into_iter()
        .map(|tool| McpTool {
            name: tool.name,
            title: tool.title,
            description: tool.description,
            input_schema: tool.input_schema,
        })
        .collect();
    let next_cursor = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((tools, next_cursor))
}

/// tools/list 翻页所需的两个动作，HTTP 与 stdio 会话各自实现，分页逻辑单源。
trait ToolsFetch {
    async fn notify_initialized(&mut self) -> Result<(), ProbeFailure>;
    async fn list_page(&mut self, cursor: Option<String>) -> Result<Value, ProbeFailure>;
}

/// 发初始化通知后按游标翻页拉取工具，封顶 MAX_TOOLS；
/// 通知或任一页失败记入 tools_error，失败前已收到的页保留返回。
async fn collect_tools(session: &mut impl ToolsFetch) -> (Vec<McpTool>, bool, Option<String>) {
    let mut tools_error = session
        .notify_initialized()
        .await
        .err()
        .map(|error| error.message);
    let mut tools = Vec::new();
    let mut tools_truncated = false;
    let mut cursor: Option<String> = None;
    while tools_error.is_none() && tools.len() < MAX_TOOLS {
        let page = match session.list_page(cursor.take()).await {
            Ok(page) => page,
            Err(error) => {
                tools_error = Some(error.message);
                break;
            }
        };
        let (mut page_tools, next_cursor) = match parse_tools_page(page) {
            Ok(value) => value,
            Err(error) => {
                tools_error = Some(error);
                break;
            }
        };
        let remaining = MAX_TOOLS - tools.len();
        if page_tools.len() > remaining {
            page_tools.truncate(remaining);
            tools_truncated = true;
        }
        tools.extend(page_tools);
        if tools_truncated || next_cursor.is_none() {
            break;
        }
        cursor = next_cursor;
    }
    if tools.len() >= MAX_TOOLS {
        tools_truncated = true;
    }
    (tools, tools_truncated, tools_error)
}

/// 进程环境优先；macOS 读取登录 Shell，Windows 兜底读取用户级注册表。
fn probe_env_var(name: &str) -> Result<Option<String>, String> {
    if let Ok(value) = std::env::var(name) {
        return Ok(Some(value));
    }
    #[cfg(target_os = "macos")]
    {
        Ok(super::shell_env::environment()?.get(name).cloned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(windows_user_environment_value(name))
    }
}

#[cfg(windows)]
fn windows_user_environment_value(name: &str) -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let subkey = HSTRING::from("Environment");
    let value_name = HSTRING::from(name);
    let read = |buffer: Option<*mut core::ffi::c_void>, size: &mut u32| unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &subkey,
            &value_name,
            RRF_RT_REG_SZ,
            None,
            buffer,
            Some(size),
        )
    };
    // 两段式：先问长度再取值。RRF_RT_REG_SZ 会顺带把 REG_EXPAND_SZ 的 %VAR% 引用展开。
    let mut size = 0u32;
    if read(None, &mut size) != ERROR_SUCCESS {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    if read(Some(buffer.as_mut_ptr().cast()), &mut size) != ERROR_SUCCESS {
        return None;
    }
    let wide: Vec<u16> = buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_ne_bytes(*pair))
        .collect();
    let text = String::from_utf16_lossy(&wide);
    Some(text.trim_end_matches('\0').to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn windows_user_environment_value(_name: &str) -> Option<String> {
    None
}

fn build_http_headers(spec: &McpServerSpec) -> Result<(HeaderMap, Vec<String>), String> {
    let mut headers = HeaderMap::new();
    let mut secrets = Vec::new();
    for (name, value) in &spec.http_headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("HTTP 头名称无效: {name}"))?;
        let value = HeaderValue::from_str(value).map_err(|_| "HTTP 头值无效".to_string())?;
        secrets.push(value.to_str().unwrap_or_default().to_string());
        headers.insert(name, value);
    }
    for (name, env_name) in &spec.env_http_headers {
        let value = probe_env_var(env_name)?
            .ok_or_else(|| format!("HTTP 头引用的环境变量未设置: {env_name}"))?;
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("HTTP 头名称无效: {name}"))?;
        let header_value =
            HeaderValue::from_str(&value).map_err(|_| "HTTP 头值无效".to_string())?;
        secrets.push(value);
        headers.insert(name, header_value);
    }
    if let Some(env_name) = spec.bearer_token_env_var.as_deref() {
        let token = probe_env_var(env_name)?
            .ok_or_else(|| format!("Bearer Token 环境变量未设置: {env_name}"))?;
        if token.trim().is_empty() {
            return Err(format!("Bearer Token 环境变量为空: {env_name}"));
        }
        secrets.push(token.clone());
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Bearer Token 无效".to_string())?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok((headers, secrets))
}

fn http_failure(status: reqwest::StatusCode, body: &str, secrets: &[String]) -> ProbeFailure {
    let generic = match status.as_u16() {
        401 => "认证失败，请检查 MCP 凭据",
        403 => "MCP 服务拒绝了请求",
        404 => "MCP 服务地址不存在",
        408 => "MCP 服务请求超时",
        429 => "MCP 服务请求过于频繁",
        500..=599 => "MCP 服务暂时不可用",
        _ => "MCP 服务请求失败",
    };
    let detail = clean_detail(body, secrets);
    ProbeFailure {
        status: Some(status.as_u16()),
        message: if detail.is_empty() {
            generic.to_string()
        } else {
            format!("{generic}：{detail}")
        },
    }
}

struct HttpSession {
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    secrets: Vec<String>,
    session_id: Option<String>,
    next_id: u64,
}

impl HttpSession {
    /// 拼 POST 请求样板：自定义 headers + JSON/Accept + 会话 ID；发送与错误映射由调用方决定。
    fn post_builder(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut request = self.client.post(&self.url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        request = request
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream");
        if let Some(session_id) = &self.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        request.json(body)
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        latency_start: Option<Instant>,
    ) -> Result<(Value, u16, Option<u128>), ProbeFailure> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let response = self
            .post_builder(&body)
            .send()
            .await
            .map_err(|error| ProbeFailure {
                status: error.status().map(|status| status.as_u16()),
                message: if error.is_timeout() {
                    "MCP 服务请求超时".to_string()
                } else if error.is_connect() {
                    "无法连接 MCP 服务".to_string()
                } else {
                    "MCP 请求失败".to_string()
                },
            })?;
        let response_latency_ms = latency_start.map(|start| start.elapsed().as_millis());
        let status = response.status();
        if let Some(value) = response.headers().get("Mcp-Session-Id") {
            self.session_id = value.to_str().ok().map(str::to_string);
        }
        let body = response.text().await.map_err(|_| ProbeFailure {
            status: Some(status.as_u16()),
            message: "读取 MCP 响应失败".to_string(),
        })?;
        if !status.is_success() {
            return Err(http_failure(status, &body, &self.secrets));
        }
        let message =
            rpc_result(parse_rpc_values(&body), id, &self.secrets).map_err(|message| {
                ProbeFailure {
                    status: Some(status.as_u16()),
                    message,
                }
            })?;
        Ok((message, status.as_u16(), response_latency_ms))
    }

    async fn notify(&mut self, method: &str) -> Result<(), ProbeFailure> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": {} });
        let response = self
            .post_builder(&body)
            .send()
            .await
            .map_err(|_| ProbeFailure {
                status: None,
                message: "发送 MCP 初始化通知失败".to_string(),
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(http_failure(status, &body, &self.secrets))
        }
    }
}

impl ToolsFetch for HttpSession {
    async fn notify_initialized(&mut self) -> Result<(), ProbeFailure> {
        self.notify("notifications/initialized").await
    }

    async fn list_page(&mut self, cursor: Option<String>) -> Result<Value, ProbeFailure> {
        let params = cursor
            .map(|cursor| json!({ "cursor": cursor }))
            .unwrap_or_else(|| json!({}));
        self.request("tools/list", params, None)
            .await
            .map(|(page, _, _)| page)
    }
}

/// 探测函数不返回 Err：失败编码在 result.ok/error 字段里，签名直接返回结果本体。
async fn probe_http(
    spec: &McpServerSpec,
    include_tools: bool,
    proxy: Option<&str>,
) -> McpProbeResult {
    let start = Instant::now();
    let result = tokio::time::timeout(
        PROBE_TIMEOUT,
        probe_http_inner(spec, start, include_tools, proxy),
    )
    .await;
    match result {
        Ok(result) => result,
        Err(_) => failed_result(start, None, "MCP 服务请求超时"),
    }
}

async fn probe_http_inner(
    spec: &McpServerSpec,
    start: Instant,
    include_tools: bool,
    proxy: Option<&str>,
) -> McpProbeResult {
    let Some(url) = spec.url.as_deref() else {
        return failed_result(start, None, "MCP 服务地址为空");
    };
    let header_spec = spec.clone();
    let headers = tauri::async_runtime::spawn_blocking(move || build_http_headers(&header_spec))
        .await
        .unwrap_or_else(|_| Err("读取 MCP HTTP 认证配置失败".into()));
    let (headers, secrets) = match headers {
        Ok(value) => value,
        Err(error) => return failed_result(start, None, error),
    };
    let network = match crate::network::Network::new(proxy.map(str::to_owned)) {
        Ok(network) => network,
        Err(error) => return failed_result(start, None, error.to_string()),
    };
    let client_builder = network
        .builder()
        .user_agent(format!("Agmux/{}", env!("CARGO_PKG_VERSION")))
        .timeout(PROBE_TIMEOUT);
    let client = match client_builder.build() {
        Ok(client) => client,
        Err(error) => {
            return failed_result(start, None, format!("创建 MCP HTTP 客户端失败: {error}"));
        }
    };
    let mut session = HttpSession {
        client,
        url: url.to_string(),
        headers,
        secrets,
        session_id: None,
        next_id: 1,
    };
    let handshake_start = Instant::now();
    let (initialize, status, handshake_latency_ms) = match session
        .request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "Agmux", "version": env!("CARGO_PKG_VERSION") },
            }),
            Some(handshake_start),
        )
        .await
    {
        Ok(value) => value,
        Err(error) => return failed_result(handshake_start, error.status, error.message),
    };
    let protocol_version = initialize
        .get("protocolVersion")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(protocol_version) = protocol_version else {
        return failed_result(
            handshake_start,
            Some(status),
            "MCP 初始化响应缺少 protocolVersion",
        );
    };
    let server_info = parse_server_info(&initialize);
    let (tools, tools_truncated, tools_error) = if include_tools {
        collect_tools(&mut session).await
    } else {
        (Vec::new(), false, None)
    };
    McpProbeResult {
        ok: true,
        connection_state: None,
        auth_status: None,
        latency_ms: handshake_latency_ms,
        status: Some(status),
        protocol_version: Some(protocol_version),
        server_info,
        tools,
        tools_truncated,
        error: None,
        tools_error,
    }
}

struct StdioSession {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<String>,
    stderr: Option<Receiver<Vec<u8>>>,
    deadline: Instant,
    secrets: Vec<String>,
    next_id: u64,
}

/// Windows 上 std Command 只按 .exe 解析无扩展名命令，找不到 npm 装的 npx.cmd
/// 这类 shim（实测报 program not found）；spawn 前按 PATH + PATHEXT 手动解析。
/// 带路径分隔符或已带扩展名的写法、以及其余平台，保持原行为。
#[cfg(not(any(windows, target_os = "macos")))]
fn resolve_command(program: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(program)
}

/// Finder 启动的 macOS 进程 PATH 只有 /usr/bin:/bin:/usr/sbin:/sbin，
/// posix_spawnp 按父进程 PATH 解析程序名，.envs() 注入的子进程 PATH 不参与查找，
/// 裸命令名（npx、node 等）报 ENOENT；spawn 前按登录 Shell 的 PATH 解析为绝对路径。
#[cfg(target_os = "macos")]
fn resolve_command(program: &str) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};

    if program.contains('/') {
        return PathBuf::from(program);
    }
    if let Ok(env) = super::shell_env::environment() {
        if let Some(path) = env.get("PATH") {
            if let Some(found) = std::env::split_paths(path)
                .map(|dir| dir.join(program))
                .find(|candidate| candidate.is_file())
            {
                return found;
            }
        }
    }
    PathBuf::from(Path::new(program))
}

#[cfg(windows)]
fn resolve_command(program: &str) -> std::path::PathBuf {
    use std::path::{Path, PathBuf};

    let has_executable_extension = Path::new(program)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "exe" | "cmd" | "bat"));
    if program.contains(['\\', '/']) || has_executable_extension {
        return PathBuf::from(program);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    command_candidates(program, &dirs)
        .into_iter()
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(program))
}

/// 候选顺序对齐 cmd.exe 的 PATHEXT 习惯：逐目录，目录内 .exe → .cmd → .bat。
#[cfg(windows)]
fn command_candidates(program: &str, dirs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    const EXTS: [&str; 3] = [".exe", ".cmd", ".bat"];
    dirs.iter()
        .flat_map(|dir| {
            EXTS.iter()
                .map(move |ext| dir.join(format!("{program}{ext}")))
        })
        .collect()
}

fn spawn_stdio(spec: &McpServerSpec, deadline: Instant) -> Result<StdioSession, String> {
    let Some(command_name) = spec.command.as_deref() else {
        return Err("MCP 启动命令为空".to_string());
    };
    let mut command = Command::new(resolve_command(command_name));
    #[cfg(target_os = "macos")]
    command.envs(super::shell_env::environment()?);
    command.args(&spec.args).envs(&spec.env);
    let secrets: Vec<String> = spec.env.values().cloned().collect();
    #[cfg(target_os = "macos")]
    let secrets = secrets
        .into_iter()
        .chain(super::shell_env::environment()?.values().cloned())
        .collect();
    spawn_session(command, deadline, secrets)
}

fn spawn_session(
    mut command: Command,
    deadline: Instant,
    secrets: Vec<String>,
) -> Result<StdioSession, String> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 MCP 进程: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "MCP 进程没有可读取的标准输出".to_string())?;
    let stderr = child.stderr.take();
    let (sender, messages) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    // stderr 经 channel 回传而非 JoinHandle：强杀直接子进程后，孙进程可能仍攥着
    // 管道写端导致读线程收不到 EOF，stop() 只能限时收割，不能无限 join。
    let stderr = stderr.map(|mut pipe| {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = tx.send(bytes);
        });
        rx
    });
    let stdin = child.stdin.take();
    Ok(StdioSession {
        child,
        stdin,
        messages,
        stderr,
        deadline,
        secrets,
        next_id: 1,
    })
}

impl StdioSession {
    // Claude Agent SDK 的控制协议；只查询状态，不发送 user 消息或执行工具。
    fn claude_request(&mut self, request: Value) -> Result<Value, String> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let body = json!({
            "type": "control_request", "request_id": id,
            "request": request,
        });
        let input = self.stdin.as_mut().ok_or("Claude 输入管道不可用")?;
        writeln!(input, "{body}")
            .and_then(|_| input.flush())
            .map_err(|_| "Claude 状态请求发送失败")?;
        loop {
            let line = self
                .messages
                .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| "Claude 状态请求超时或进程已退出")?;
            let message: Value =
                serde_json::from_str(&line).map_err(|_| "Claude 返回了无效状态消息")?;
            let response = &message["response"];
            if message["type"] != "control_response" || response["request_id"] != id {
                if Instant::now() >= self.deadline {
                    return Err("Claude 状态请求超时".into());
                }
                continue;
            }
            if response["subtype"] != "success" {
                // 客户端原文可能含配置和凭据，只显示固定提示。
                return Err("Claude 无法完成状态查询，请更新 Claude Code 后重试".into());
            }
            return response
                .get("response")
                .cloned()
                .ok_or_else(|| "Claude 状态响应缺少结果".into());
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, ProbeFailure> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let stdin = self.stdin.as_mut().ok_or_else(|| ProbeFailure {
            status: None,
            message: "MCP 进程输入管道不可用".to_string(),
        })?;
        writeln!(stdin, "{body}").map_err(|_| ProbeFailure {
            status: None,
            message: "无法向 MCP 进程发送请求".to_string(),
        })?;
        stdin.flush().map_err(|_| ProbeFailure {
            status: None,
            message: "无法刷新 MCP 进程请求".to_string(),
        })?;

        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ProbeFailure {
                    status: None,
                    message: "MCP 进程响应超时".to_string(),
                });
            }
            match self.messages.recv_timeout(remaining) {
                Ok(line) => {
                    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                        return Err(ProbeFailure {
                            status: None,
                            message: "MCP 进程输出了无效 JSON-RPC 消息".to_string(),
                        });
                    };
                    if value.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    return rpc_result(vec![value], id, &self.secrets).map_err(|message| {
                        ProbeFailure {
                            status: None,
                            message,
                        }
                    });
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(ProbeFailure {
                        status: None,
                        message: "MCP 进程响应超时".to_string(),
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(ProbeFailure {
                        status: None,
                        message: "MCP 进程已退出，未返回响应".to_string(),
                    });
                }
            }
        }
    }

    fn notify(&mut self, method: &str) -> Result<(), ProbeFailure> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": {} });
        let stdin = self.stdin.as_mut().ok_or_else(|| ProbeFailure {
            status: None,
            message: "MCP 进程输入管道不可用".to_string(),
        })?;
        writeln!(stdin, "{body}").map_err(|_| ProbeFailure {
            status: None,
            message: "无法向 MCP 进程发送初始化通知".to_string(),
        })?;
        stdin.flush().map_err(|_| ProbeFailure {
            status: None,
            message: "无法刷新 MCP 初始化通知".to_string(),
        })
    }

    fn stop(mut self) -> String {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        // 限时等 stderr 收尾：孙进程不松手时宁可丢报错细节，也不让探测命令挂死
        self.stderr
            .and_then(|stderr| stderr.recv_timeout(Duration::from_secs(2)).ok())
            .map(|bytes| clean_detail(&String::from_utf8_lossy(&bytes), &self.secrets))
            .unwrap_or_default()
    }
}

impl ToolsFetch for StdioSession {
    async fn notify_initialized(&mut self) -> Result<(), ProbeFailure> {
        self.notify("notifications/initialized")
    }

    async fn list_page(&mut self, cursor: Option<String>) -> Result<Value, ProbeFailure> {
        let params = cursor
            .map(|cursor| json!({ "cursor": cursor }))
            .unwrap_or_else(|| json!({}));
        self.request("tools/list", params)
    }
}

fn probe_stdio(spec: &McpServerSpec, include_tools: bool) -> McpProbeResult {
    let start = Instant::now();
    let deadline = start + PROBE_TIMEOUT;
    let mut session = match spawn_stdio(spec, deadline) {
        Ok(session) => session,
        Err(error) => return failed_result(start, None, error),
    };
    let handshake_start = Instant::now();
    let mut result = match session.request(
        "initialize",
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "Agmux", "version": env!("CARGO_PKG_VERSION") },
        }),
    ) {
        Err(error) => failed_result(handshake_start, error.status, error.message),
        Ok(initialize) => {
            let protocol_version = initialize
                .get("protocolVersion")
                .and_then(Value::as_str)
                .map(str::to_string);
            match protocol_version {
                None => failed_result(handshake_start, None, "MCP 初始化响应缺少 protocolVersion"),
                Some(protocol_version) => {
                    let server_info = parse_server_info(&initialize);
                    // stdio 是同步会话，collect_tools 的 Future 立即完成，block_on 直取
                    let (tools, tools_truncated, tools_error) = if include_tools {
                        tauri::async_runtime::block_on(collect_tools(&mut session))
                    } else {
                        (Vec::new(), false, None)
                    };
                    McpProbeResult {
                        ok: true,
                        connection_state: None,
                        auth_status: None,
                        latency_ms: Some(handshake_start.elapsed().as_millis()),
                        status: None,
                        protocol_version: Some(protocol_version),
                        server_info,
                        tools,
                        tools_truncated,
                        error: None,
                        tools_error,
                    }
                }
            }
        }
    };
    let stderr = session.stop();
    if !stderr.is_empty() && !result.ok {
        let detail = result.error.take().unwrap_or_default();
        result.error = Some(if detail.is_empty() {
            format!("MCP 进程错误：{stderr}")
        } else {
            format!("{detail}：{stderr}")
        });
    }
    result
}

fn codex_status_result(status: &Value, start: Instant, include_tools: bool) -> McpProbeResult {
    let auth = status.get("authStatus").and_then(Value::as_str);
    let runtime = status.get("runtimeStatus").and_then(Value::as_str);
    let initialized = status
        .get("serverCapabilities")
        .is_some_and(Value::is_object);
    let state = match runtime {
        Some("connected") => McpConnectionState::Connected,
        Some("authenticationRequired") => McpConnectionState::AuthenticationRequired,
        Some("failed") => McpConnectionState::Failed,
        // 旧版本不返回 runtimeStatus，能力对象只在握手成功后才存在。
        None if initialized => McpConnectionState::Connected,
        None if auth == Some("notLoggedIn") => McpConnectionState::AuthenticationRequired,
        None if status
            .get("toolsError")
            .is_some_and(|error| !error.is_null()) =>
        {
            McpConnectionState::Failed
        }
        _ => McpConnectionState::Unknown,
    };
    let message = match state {
        McpConnectionState::Connected => "",
        McpConnectionState::AuthenticationRequired => {
            "Codex MCP 需要授权或重新授权，请在 Codex 中完成登录后重新检测"
        }
        McpConnectionState::Failed => "Codex 无法连接此 MCP，请检查客户端中的连接错误后重试",
        McpConnectionState::Unknown => "Codex 未能确认此 MCP 的连接状态，请重新检测",
    };
    let mut result = failed_result(start, None, message);
    result.connection_state = Some(state);
    result.auth_status = auth
        .filter(|auth| {
            matches!(
                *auth,
                "oAuth" | "bearerToken" | "notLoggedIn" | "unsupported" | "unknown"
            )
        })
        .map(str::to_owned);
    result.ok = state == McpConnectionState::Connected;
    result.error = (!result.ok).then(|| message.to_owned());
    result.server_info = parse_server_info(status);
    if include_tools && result.ok {
        if let Some(tools) = status.get("tools").and_then(Value::as_object) {
            result.tools_truncated = tools.len() > MAX_TOOLS;
            for tool in tools.values().take(MAX_TOOLS) {
                let Ok(tool) = serde_json::from_value::<RawTool>(tool.clone()) else {
                    result.tools_error = Some("Codex 返回的 MCP 工具格式无效".into());
                    break;
                };
                result.tools.push(McpTool {
                    name: tool.name,
                    title: tool.title,
                    description: tool.description,
                    input_schema: tool.input_schema,
                });
            }
        } else {
            result.tools_error = Some("Codex 未返回 MCP 工具列表".into());
        }
        if status
            .get("toolsError")
            .is_some_and(|error| !error.is_null())
        {
            // 客户端错误可能内嵌配置或凭据，不传回原文。
            result.tools_error = Some("Codex 已连接，但 MCP 工具列表读取失败，请重试".into());
        }
    }
    result
}

fn probe_codex(codex_home: &std::path::Path, name: &str, include_tools: bool) -> McpProbeResult {
    let start = Instant::now();
    let read = || -> AppResult<Value> {
        let home = codex_home
            .parent()
            .ok_or_else(|| app_err!("Codex 配置目录无父目录"))?;
        let mut server =
            super::plugins::appserver::AppServer::start_with_codex_home(home, codex_home)?;
        server.request(
            "mcpServerStatus/list",
            json!({
                "serverName": name, "detail": "toolsAndAuthOnly",
            }),
        )
    };
    match read() {
        Ok(response) => {
            if let Some(status) =
                response
                    .get("data")
                    .and_then(Value::as_array)
                    .and_then(|servers| {
                        servers
                            .iter()
                            .find(|status| status.get("name").and_then(Value::as_str) == Some(name))
                    })
            {
                return codex_status_result(status, start, include_tools);
            }
            let mut result = failed_result(
                start,
                None,
                "Codex 未返回此 MCP 的状态，请更新客户端后重新检测",
            );
            result.connection_state = Some(McpConnectionState::Unknown);
            result
        }
        Err(error) => {
            let mut result = failed_result(
                start,
                None,
                format!("无法通过 Codex 确认 MCP 状态：{error}"),
            );
            result.connection_state = Some(McpConnectionState::Unknown);
            result
        }
    }
}

fn claude_status_result(status: &Value, start: Instant, include_tools: bool) -> McpProbeResult {
    let state = match status["status"].as_str() {
        Some("connected") => McpConnectionState::Connected,
        Some("needs-auth") => McpConnectionState::AuthenticationRequired,
        Some("failed") => McpConnectionState::Failed,
        _ => McpConnectionState::Unknown,
    };
    let message = match state {
        McpConnectionState::Connected => "",
        McpConnectionState::AuthenticationRequired => {
            "Claude MCP 需要授权或重新授权，请在 Claude Code 中完成登录后重新检测"
        }
        McpConnectionState::Failed => "Claude Code 无法连接此 MCP，请检查客户端中的连接错误后重试",
        McpConnectionState::Unknown => "Claude Code 未能确认此 MCP 的连接状态，请重新检测",
    };
    let mut result = failed_result(start, None, message);
    result.connection_state = Some(state);
    result.ok = state == McpConnectionState::Connected;
    result.error = (!result.ok).then(|| message.into());
    result.server_info = parse_server_info(status);
    if result.ok && include_tools {
        match parse_tools_page(status.clone()) {
            Ok((mut tools, _)) => {
                result.tools_truncated = tools.len() > MAX_TOOLS;
                tools.truncate(MAX_TOOLS);
                result.tools = tools;
            }
            Err(_) => {
                result.tools_error = Some("Claude 已连接，但未返回有效的 MCP 工具列表".into())
            }
        }
    }
    result
}

fn probe_claude(claude_home: &std::path::Path, name: &str, include_tools: bool) -> McpProbeResult {
    let start = Instant::now();
    let read = || -> Result<McpProbeResult, String> {
        let home = claude_home.parent().ok_or("Claude 配置目录无父目录")?;
        let cli =
            super::plugins::find_claude_cli(home).ok_or("未找到 Claude Code，无法确认 MCP 状态")?;
        let document: Value = serde_json::from_slice(
            &std::fs::read(home.join(".claude.json")).map_err(|_| "无法读取 Claude MCP 配置")?,
        )
        .map_err(|_| "无法解析 Claude MCP 配置")?;
        let entry = document["mcpServers"]
            .get(name)
            .ok_or("Claude MCP 配置中没有此服务器")?;
        let mut command = super::cli::command(&cli);
        #[cfg(target_os = "macos")]
        command.envs(super::shell_env::environment()?);
        command
            .args([
                "--print",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                &json!({"mcpServers": {name: entry}}).to_string(),
            ])
            .current_dir(home)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_SECURESTORAGE_CONFIG_DIR")
            .env_remove("CLAUDECODE");
        // 默认安装沿用 Claude 的配置与凭据目录，不创建另一套登录环境。
        if claude_home != crate::paths::home_dir().join(".claude") {
            command.env("CLAUDE_CONFIG_DIR", claude_home);
        }
        crate::network::Network::detect()
            .map_err(|_| "网络设置无效")?
            .apply(&mut command);
        let deadline = start + PROBE_TIMEOUT;
        #[cfg(not(target_os = "macos"))]
        let secrets = Vec::new();
        #[cfg(target_os = "macos")]
        let secrets = super::shell_env::environment()?.values().cloned().collect();
        let mut session = spawn_session(command, deadline, secrets)?;
        let result = (|| {
            session.claude_request(json!({"subtype":"initialize"}))?;
            let mut reconnected = false;
            loop {
                let response = session.claude_request(json!({"subtype":"mcp_status"}))?;
                let status = response["mcpServers"]
                    .as_array()
                    .and_then(|servers| servers.iter().find(|status| status["name"] == name))
                    .ok_or("Claude 未返回此 MCP 的状态")?;
                // Claude 可能沿用旧的 needs-auth 缓存，重连一次才是对现有授权的实际检查。
                if status["status"] == "needs-auth" && !reconnected {
                    reconnected = true;
                    session
                        .claude_request(json!({"subtype":"mcp_reconnect", "serverName":name}))?;
                    continue;
                }
                if status["status"] != "pending" || Instant::now() >= deadline {
                    let mut result = claude_status_result(status, start, include_tools);
                    if result.ok && reconnected {
                        result.auth_status = Some("oAuth".into());
                    }
                    return Ok(result);
                }
                thread::sleep(
                    Duration::from_millis(200)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        })();
        session.stop();
        result
    };
    read().unwrap_or_else(|error| {
        let mut result = failed_result(
            start,
            None,
            format!("无法通过 Claude Code 确认 MCP 状态：{error}"),
        );
        result.connection_state = Some(McpConnectionState::Unknown);
        result
    })
}

fn needs_claude_auth(spec: &McpServerSpec) -> bool {
    spec.url.is_some()
        && spec.command.is_none()
        && spec.bearer_token_env_var.is_none()
        && !spec
            .http_headers
            .keys()
            .chain(spec.env_http_headers.keys())
            .any(|name| name.eq_ignore_ascii_case("authorization"))
}

impl AppContext {
    pub async fn probe_mcp_server(
        &self,
        name: &str,
        include_tools: bool,
        manual: bool,
        tool: SkillTool,
    ) -> AppResult<McpProbeResult> {
        // 按用户所在页的引擎解析名单与开关：两侧各自标记自己引擎的禁用状态，
        // Codex 关了不影响 Claude 页探测，反之亦然。
        let server = match tool {
            SkillTool::Codex => self.codex_list_mcp_servers()?,
            SkillTool::Claude => self.claude_list_mcp_servers()?,
        }
        .into_iter()
        .find(|server| server.name == name)
        .ok_or_else(|| app_err!("找不到 MCP 服务器: {name}"))?;
        if server.enabled == Some(false) {
            return Err(app_err!("MCP 服务器已禁用: {name}"));
        }
        if matches!(tool, SkillTool::Codex) {
            let codex_home = self.paths.codex_home.clone();
            let name = name.to_owned();
            let result = tauri::async_runtime::spawn_blocking(move || {
                probe_codex(&codex_home, &name, include_tools)
            })
            .await
            .map_err(|error| app_err!("Codex MCP 测试任务失败: {error}"))?;
            log_probe_outcome(&server.name, include_tools, manual, &result, " via=codex");
            return Ok(result);
        }
        let is_http = server.url.is_some() && server.command.is_none();
        // 显式 token 仍直接探测；浏览器授权交给 Claude，不能以匿名 HTTP 判断它的认证状态。
        if needs_claude_auth(&server) {
            let claude_home = self.paths.claude_home.clone();
            let name = name.to_owned();
            let mut result = tauri::async_runtime::spawn_blocking(move || {
                probe_claude(&claude_home, &name, include_tools)
            })
            .await
            .map_err(|error| app_err!("Claude MCP 测试任务失败: {error}"))?;
            if result.ok && result.auth_status.is_none() {
                // Claude 状态接口不返回认证类型；匿名 401 + 客户端连通才足以确认沿用了 OAuth。
                if let Ok(network) = crate::network::Network::current().await {
                    if probe_http(&server, false, network.proxy.as_deref())
                        .await
                        .status
                        == Some(401)
                    {
                        result.auth_status = Some("oAuth".into());
                    }
                }
            }
            log_probe_outcome(&server.name, include_tools, manual, &result, " via=claude");
            return Ok(result);
        }
        // HTTP 探测需要代理归因；stdio 子进程的网络不受应用控制，日志不标注走向
        let proxy = if is_http {
            match crate::network::Network::current().await {
                Ok(network) => network.proxy,
                Err(error) => {
                    let result = failed_result(std::time::Instant::now(), None, error.to_string());
                    log_probe_outcome(
                        &server.name,
                        include_tools,
                        manual,
                        &result,
                        " proxy=invalid",
                    );
                    return Ok(result);
                }
            }
        } else {
            None
        };
        let route = if is_http {
            crate::network::proxy_note(&proxy)
        } else {
            String::new()
        };
        let result = if server.command.is_some() && server.url.is_none() {
            // spawn_blocking 要求 'static，克隆 spec 进闭包，server 留作日志取名字
            let spec = server.clone();
            tauri::async_runtime::spawn_blocking(move || probe_stdio(&spec, include_tools))
                .await
                .map_err(|error| app_err!("MCP 测试任务失败: {error}"))?
        } else if is_http {
            probe_http(&server, include_tools, proxy.as_deref()).await
        } else {
            return Err(app_err!("MCP 服务器必须配置启动命令或服务地址其中之一"));
        };
        log_probe_outcome(&server.name, include_tools, manual, &result, &route);
        Ok(result)
    }
}

/// 探测结果留痕：手动测试记 Info（含延迟，事后可追溯）；进页静默探测成功只记
/// Debug（全量并行一场十来条，会刷屏）；任何失败记 Warn——状态点错过仍可在日志溯源。
fn log_probe_outcome(
    name: &str,
    include_tools: bool,
    manual: bool,
    result: &McpProbeResult,
    route: &str,
) {
    let latency = result.latency_ms.unwrap_or_default();
    if !result.ok {
        // 静默探测失败也记：状态点错过仍可在日志溯源
        let error = result.error.as_deref().unwrap_or("未知错误");
        // 版本协商失败优先判：它走 HTTP 时 status 是 200，会被兜底成 http_error——那是谎报军情
        let failure_kind = if result.connection_state == Some(McpConnectionState::Unknown) {
            "unknown"
        } else if result.connection_state == Some(McpConnectionState::AuthenticationRequired) {
            "auth_error"
        } else if error.contains("不支持协议版本") {
            "protocol_error"
        } else {
            match result.status {
                Some(401 | 403) => "auth_error",
                Some(_) => "http_error",
                None if error.contains("环境变量未设置") => "validation_error",
                None => "network_error",
            }
        };
        let status = result
            .status
            .map(|status| format!(" status_code={status}"))
            .unwrap_or_default();
        tauri_plugin_log::log::warn!(
            "[mcp.probe] server={name:?} outcome=failure failure_kind={failure_kind}{status} latency_ms={latency}{route} error={error:?} msg=\"连通失败\""
        );
        return;
    }
    if let Some(error) = &result.tools_error {
        tauri_plugin_log::log::warn!(
            "[mcp.probe.tools] server={name:?} outcome=failure failure_kind=parse_error latency_ms={latency} error={error:?}{route} msg=\"工具列举失败\""
        );
        return;
    }
    // 静默探测成功记 Debug：开发时可见，release（Info 阈值）自动消失不吵用户
    let line = if include_tools {
        format!(
            "[mcp.probe.tools] server={name:?} outcome=success latency_ms={latency} count={}{} msg=\"工具列举成功\"",
            result.tools.len(),
            route
        )
    } else {
        format!(
            "[mcp.probe] server={name:?} outcome=success latency_ms={latency}{} msg=\"连通成功\"",
            route
        )
    };
    if manual {
        tauri_plugin_log::log::info!("{line}");
    } else {
        tauri_plugin_log::log::debug!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_tools, parse_rpc_values, parse_tools_page, rpc_result, ProbeFailure, ToolsFetch,
        MAX_TOOLS,
    };
    use crate::models::McpTool;
    use serde_json::{json, Value};

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_mcp_probes_load_shell_environment_after_gui_launch() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        const CHILD: &str = "AGMUX_SHELL_ENV_TEST_CHILD";
        let spec = crate::models::McpServerSpec {
            bearer_token_env_var: Some("AGMUX_FIXTURE_TOKEN".into()),
            env_http_headers: [
                ("X-Fixture".into(), "AGMUX_FIXTURE_HEADER".into()),
                ("X-Existing".into(), "AGMUX_FIXTURE_EXISTING".into()),
            ]
            .into(),
            ..Default::default()
        };
        if let Ok(mode) = std::env::var(CHILD) {
            if mode != "success" {
                let error = super::build_http_headers(&spec).unwrap_err();
                assert!(error.contains(match mode.as_str() {
                    "timeout" => "超时",
                    "missing_output" => "未返回环境变量",
                    _ => "读取用户 Shell 环境失败",
                }));
                assert!(!error.contains("secret-canary"));
                return;
            }
            let (headers, _) = super::build_http_headers(&spec).unwrap();
            assert_eq!(headers["authorization"], "Bearer fixture-token");
            assert_eq!(headers["x-fixture"], "value=with spaces");
            assert_eq!(headers["x-existing"], "process");
            assert_eq!(
                super::probe_env_var("AGMUX_FIXTURE_MULTILINE").unwrap(),
                Some("first\nsecond=last".into())
            );
            assert!(super::probe_env_var("AGMUX_FIXTURE_ABSENT")
                .unwrap()
                .is_none());

            let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
            let codex_home = home.join(".codex");
            let cli = codex_home.join("bin/codex");
            std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
            std::fs::write(&cli, r#"#!/bin/sh
[ "$AGMUX_FIXTURE_TOKEN" = fixture-token ] || exit 1
[ "$AGMUX_FIXTURE_EXISTING" = process ] || exit 1
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}' ;;
    *'"method":"mcpServerStatus/list"'*) printf '%s\n' '{"id":2,"result":{"data":[{"name":"fixture","serverCapabilities":{},"tools":{}}]}}' ;;
  esac
done
"#).unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
            let result = super::probe_codex(&codex_home, "fixture", false);
            assert!(result.ok, "{:?}", result.error);

            let stdio = crate::models::McpServerSpec {
                command: Some("/bin/sh".into()),
                args: vec![
                    "-c".into(),
                    r#"
[ "$AGMUX_FIXTURE_TOKEN" = fixture-token ] || exit 1
[ "$AGMUX_FIXTURE_HEADER" = config-override ] || exit 1
read request
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{}}}'
"#
                    .into(),
                ],
                env: [("AGMUX_FIXTURE_HEADER".into(), "config-override".into())].into(),
                ..Default::default()
            };
            let result = super::probe_stdio(&stdio, false);
            assert!(result.ok, "{:?}", result.error);
            return;
        }

        for (mode, startup) in [
            ("success", "printf 'startup noise\\n'\nexport AGMUX_FIXTURE_HEADER='value=with spaces'\nexport AGMUX_FIXTURE_EXISTING=shell\nexport AGMUX_FIXTURE_MULTILINE='first\nsecond=last'\n"),
            ("failure", "printf secret-canary; printf secret-canary >&2; exit 1\n"),
            ("missing_output", "exec /usr/bin/true\n"),
            ("timeout", "exec /bin/sleep 30\n"),
        ] {
            let home = tempfile::tempdir().unwrap();
            std::fs::write(home.path().join(".zprofile"), "export AGMUX_FIXTURE_TOKEN=fixture-token\n").unwrap();
            std::fs::write(home.path().join(".zshrc"), startup).unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "services::mcp_probe::tests::macos_mcp_probes_load_shell_environment_after_gui_launch",
                    "--nocapture",
                ])
                .env_clear()
                .env("HOME", home.path())
                .env("ZDOTDIR", home.path())
                .env("SHELL", "/bin/zsh")
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .env("AGMUX_FIXTURE_EXISTING", "process")
                .env(CHILD, mode)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let output = super::super::plugins::wait_child_with_timeout(
                child,
                std::time::Duration::from_secs(15),
            )
            .unwrap()
            .expect("Shell environment regression test timed out");
            assert!(output.status.success(), "{mode}: {}{}", output.stdout, output.stderr);
        }
    }

    #[test]
    fn claude_oauth_uses_client_state_and_separates_auth_from_connection() {
        use crate::models::McpConnectionState;
        let mut spec = crate::models::McpServerSpec {
            url: Some("https://fixture.invalid/mcp".into()),
            ..Default::default()
        };
        assert!(super::needs_claude_auth(&spec));
        spec.http_headers
            .insert("X-Custom".into(), "fixture".into());
        assert!(super::needs_claude_auth(&spec));
        spec.http_headers
            .insert("aUtHoRiZaTiOn".into(), "fixture".into());
        assert!(!super::needs_claude_auth(&spec));
        spec.http_headers.clear();
        spec.bearer_token_env_var = Some("FIXTURE_TOKEN".into());
        assert!(!super::needs_claude_auth(&spec));
        for (state, expected) in [
            ("connected", McpConnectionState::Connected),
            ("needs-auth", McpConnectionState::AuthenticationRequired),
            ("failed", McpConnectionState::Failed),
            ("pending", McpConnectionState::Unknown),
            ("disabled", McpConnectionState::Unknown),
            ("future-status", McpConnectionState::Unknown),
        ] {
            let result = super::claude_status_result(
                &json!({"status":state, "error":"secret-canary"}),
                std::time::Instant::now(),
                false,
            );
            assert_eq!(result.connection_state, Some(expected));
            assert_eq!(result.ok, expected == McpConnectionState::Connected);
            assert!(!result.error.unwrap_or_default().contains("secret-canary"));
        }
        let result = super::claude_status_result(
            &json!({"status":"connected"}),
            std::time::Instant::now(),
            true,
        );
        assert!(result.ok);
        assert!(
            result.auth_status.is_none(),
            "不能把所有连通的服务都标成 OAuth"
        );
        assert!(result.tools_error.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn claude_probe_uses_saved_auth_without_prompt_or_other_servers() {
        use std::os::unix::fs::PermissionsExt;
        let (home, context) = super::super::plugins::test_context();
        let cli = home.path().join(".local/bin/claude");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, r#"#!/bin/sh
printf '%s\n' "$@" > "$CLAUDE_CONFIG_DIR/args.txt"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$CLAUDE_CONFIG_DIR/requests.jsonl"
  id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"subtype":"initialize"'*) result='{}' ;;
    *'"subtype":"mcp_reconnect"'*) result='{}' ;;
    *'"subtype":"mcp_status"'*)
      if [ "$id" = 2 ]; then result='{"mcpServers":[{"name":"cloudflare","status":"pending"}]}'
      elif [ "$id" = 3 ] && [ ! -f "$CLAUDE_CONFIG_DIR/no-auth-cache" ]; then result='{"mcpServers":[{"name":"cloudflare","status":"needs-auth"}]}'
      else result=$(cat "$CLAUDE_CONFIG_DIR/status.json"); fi ;;
    *) exit 1 ;;
  esac
  printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":%s}}\n' "$id" "$result"
done
"#).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(context.paths.claude_mcp_config(), r#"{"mcpServers":{"cloudflare":{"type":"http","url":"http://127.0.0.1:1/mcp"},"other":{"command":"must-not-run"}}}"#).unwrap();
        let status_path = context.paths.claude_home.join("status.json");
        std::fs::create_dir_all(&context.paths.claude_home).unwrap();
        std::fs::write(&status_path, json!({"mcpServers":[{
            "name":"cloudflare", "status":"connected", "serverInfo":{"name":"fixture","version":"1"},
            "tools":[{"name":"whoami","description":"Read identity"}]
        }]}).to_string()).unwrap();
        let result = tauri::async_runtime::block_on(context.probe_mcp_server(
            "cloudflare",
            true,
            true,
            super::SkillTool::Claude,
        ))
        .unwrap();
        assert!(result.ok, "{:?}", result.error);
        assert_eq!(result.auth_status.as_deref(), Some("oAuth"));
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.tools[0].name, "whoami");
        assert_eq!(result.server_info.unwrap().name.as_deref(), Some("fixture"));
        let arguments =
            std::fs::read_to_string(context.paths.claude_home.join("args.txt")).unwrap();
        assert!(arguments.contains("--no-session-persistence"));
        assert!(arguments.contains("--strict-mcp-config"));
        assert!(!arguments.contains("must-not-run"));
        let requests =
            std::fs::read_to_string(context.paths.claude_home.join("requests.jsonl")).unwrap();
        let requests: Vec<Value> = requests
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(requests.len() >= 3, "pending 必须重查，不能直接判失败");
        assert!(requests
            .iter()
            .all(|request| request["type"] == "control_request"));
        let reconnects: Vec<_> = requests
            .iter()
            .filter(|request| request["request"]["subtype"] == "mcp_reconnect")
            .collect();
        assert_eq!(reconnects.len(), 1);
        assert_eq!(reconnects[0]["request"]["serverName"], "cloudflare");
        std::fs::write(context.paths.claude_home.join("no-auth-cache"), "").unwrap();
        for requires_auth in [false, true] {
            use std::io::{Read, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/mcp", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            std::fs::write(
                context.paths.claude_mcp_config(),
                json!({"mcpServers":{"cloudflare":{"type":"http","url":url}}}).to_string(),
            )
            .unwrap();
            let request = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(std::time::Duration::from_millis(10))
                        }
                        Err(error) => panic!("expected authentication check: {error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = [0; 4096];
                let size = socket.read(&mut bytes).unwrap();
                assert!(!String::from_utf8_lossy(&bytes[..size])
                    .to_ascii_lowercase()
                    .contains("authorization:"));
                let body = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{}}}"#;
                write!(socket, "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", if requires_auth {"401 Unauthorized"} else {"200 OK"}, body.len()).unwrap();
            });
            let result = tauri::async_runtime::block_on(context.probe_mcp_server(
                "cloudflare",
                false,
                true,
                super::SkillTool::Claude,
            ))
            .unwrap();
            request.join().unwrap();
            assert!(result.ok);
            assert_eq!(
                result.auth_status.as_deref(),
                requires_auth.then_some("oAuth")
            );
        }
        std::fs::write(&status_path, r#"{"mcpServers":[]}"#).unwrap();
        let result = super::probe_claude(&context.paths.claude_home, "cloudflare", false);
        assert!(!result.ok);
        assert_eq!(
            result.connection_state,
            Some(crate::models::McpConnectionState::Unknown)
        );
        std::fs::write(
            &status_path,
            r#"{"mcpServers":[{"name":"cloudflare","status":"needs-auth"}]}"#,
        )
        .unwrap();
        let result = super::probe_claude(&context.paths.claude_home, "cloudflare", false);
        assert!(!result.ok);
        assert_eq!(
            result.connection_state,
            Some(crate::models::McpConnectionState::AuthenticationRequired)
        );
        std::fs::write(&cli, r#"#!/bin/sh
read request
printf '%s\n' '{"type":"control_response","response":{"request_id":"1","subtype":"error","error":"secret-canary"}}'
"#).unwrap();
        let result = super::probe_claude(&context.paths.claude_home, "cloudflare", false);
        assert_eq!(
            result.connection_state,
            Some(crate::models::McpConnectionState::Unknown)
        );
        assert!(!result.error.unwrap().contains("secret-canary"));
    }

    #[cfg(unix)]
    #[test]
    fn claude_control_request_timeout_stops_the_client() {
        let start = std::time::Instant::now();
        let spec = crate::models::McpServerSpec {
            command: Some("/bin/sh".into()),
            args: vec!["-c".into(), "read first; read second".into()],
            ..Default::default()
        };
        let mut session =
            super::spawn_stdio(&spec, start + std::time::Duration::from_millis(100)).unwrap();
        assert!(session
            .claude_request(json!({"subtype":"initialize"}))
            .unwrap_err()
            .contains("超时"));
        session.stop();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn codex_probe_uses_client_protocol_and_rejects_missing_server_status() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        let cli = codex_home.join("bin/codex");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$CODEX_HOME/requests.jsonl"
  case "$line" in
    *'"method":"initialize"'*) printf '%s\n' '{"id":1,"result":{}}' ;;
    *'"method":"mcpServerStatus/list"'*) printf '{"id":2,"result":'; cat "$CODEX_HOME/status.json"; printf '}\n' ;;
  esac
done
"#).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            codex_home.join("status.json"),
            json!({"data":[{
                "name":"cloudflare", "authStatus":"oAuth", "serverCapabilities":{}, "tools":{}
            }]})
            .to_string(),
        )
        .unwrap();
        let result = super::probe_codex(&codex_home, "cloudflare", true);
        assert!(result.ok);
        assert_eq!(result.auth_status.as_deref(), Some("oAuth"));
        let requests = std::fs::read_to_string(codex_home.join("requests.jsonl")).unwrap();
        let request: Value = requests
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|request| request["method"] == "mcpServerStatus/list")
            .unwrap();
        assert_eq!(request["params"]["serverName"], "cloudflare");
        assert_eq!(request["params"]["detail"], "toolsAndAuthOnly");
        std::fs::write(codex_home.join("status.json"), r#"{"data":[]}"#).unwrap();
        let result = super::probe_codex(&codex_home, "cloudflare", false);
        assert!(!result.ok);
        assert_eq!(
            result.connection_state,
            Some(crate::models::McpConnectionState::Unknown)
        );
    }

    #[test]
    fn codex_oauth_probe_requires_connection_evidence_and_keeps_auth_separate() {
        use crate::models::McpConnectionState;
        use std::time::Instant;
        let connected = json!({
            "authStatus": "oAuth", "runtimeStatus": null,
            "serverCapabilities": {"tools": {}},
            "serverInfo": {"name": "cloudflare-api", "version": "0.1.0"},
            "tools": {"whoami": {"name": "whoami", "inputSchema": {"type":"object"}}},
            "toolsError": null,
        });
        let result = super::codex_status_result(&connected, Instant::now(), true);
        assert!(result.ok);
        assert_eq!(result.auth_status.as_deref(), Some("oAuth"));
        assert_eq!(result.tools[0].name, "whoami");
        for (runtime, expected) in [
            (
                "authenticationRequired",
                McpConnectionState::AuthenticationRequired,
            ),
            ("failed", McpConnectionState::Failed),
            ("starting", McpConnectionState::Unknown),
            ("futureStatus", McpConnectionState::Unknown),
        ] {
            let mut status = connected.clone();
            status["runtimeStatus"] = runtime.into();
            let result = super::codex_status_result(&status, Instant::now(), true);
            assert!(!result.ok);
            assert_eq!(result.connection_state, Some(expected));
            assert!(result.tools.is_empty());
        }
        let result =
            super::codex_status_result(&json!({"authStatus":"oAuth"}), Instant::now(), false);
        assert!(!result.ok, "仅保存 OAuth 凭据不能证明已连接");
        assert_eq!(result.connection_state, Some(McpConnectionState::Unknown));
        let result =
            super::codex_status_result(&json!({"authStatus":"notLoggedIn"}), Instant::now(), false);
        assert_eq!(
            result.connection_state,
            Some(McpConnectionState::AuthenticationRequired)
        );
        let mut tools_failed = connected;
        tools_failed["toolsError"] = "sensitive-client-error".into();
        let result = super::codex_status_result(&tools_failed, Instant::now(), true);
        assert!(result.ok, "工具清单错误不覆盖已成功的连接");
        assert!(!result
            .tools_error
            .unwrap()
            .contains("sensitive-client-error"));
        let startup_failed = json!({
            "authStatus": "unsupported", "runtimeStatus": null, "serverCapabilities": null,
            "tools": {}, "toolsError": "MCP startup failed: sensitive-client-error",
        });
        for include_tools in [false, true] {
            let result = super::codex_status_result(&startup_failed, Instant::now(), include_tools);
            assert!(!result.ok);
            assert_eq!(result.connection_state, Some(McpConnectionState::Failed));
            assert!(!result.error.unwrap().contains("sensitive-client-error"));
        }
        let mut needs_auth = startup_failed;
        needs_auth["authStatus"] = "notLoggedIn".into();
        let result = super::codex_status_result(&needs_auth, Instant::now(), false);
        assert_eq!(
            result.connection_state,
            Some(McpConnectionState::AuthenticationRequired)
        );
    }

    /// 假 tools/list 数据源：按顺序吐页，fail_from 起模拟请求失败。
    struct FakeTools {
        pages: Vec<(Vec<String>, Option<String>)>,
        notify_error: Option<String>,
        fail_from: usize,
        calls: usize,
        cursors: Vec<Option<String>>,
    }

    impl ToolsFetch for FakeTools {
        async fn notify_initialized(&mut self) -> Result<(), ProbeFailure> {
            match &self.notify_error {
                Some(message) => Err(ProbeFailure {
                    status: None,
                    message: message.clone(),
                }),
                None => Ok(()),
            }
        }

        async fn list_page(&mut self, cursor: Option<String>) -> Result<Value, ProbeFailure> {
            self.calls += 1;
            self.cursors.push(cursor);
            if self.fail_from > 0 && self.calls >= self.fail_from {
                return Err(ProbeFailure {
                    status: None,
                    message: "sample 页请求失败".to_string(),
                });
            }
            let (names, next) = &self.pages[self.calls - 1];
            Ok(json!({
                "tools": names.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(),
                "nextCursor": next,
            }))
        }
    }

    fn fake_tools(
        pages: Vec<(Vec<&str>, Option<&str>)>,
        notify_error: Option<&str>,
        fail_from: usize,
    ) -> FakeTools {
        FakeTools {
            pages: pages
                .into_iter()
                .map(|(names, next)| {
                    (
                        names.into_iter().map(str::to_string).collect(),
                        next.map(str::to_string),
                    )
                })
                .collect(),
            notify_error: notify_error.map(str::to_string),
            fail_from,
            calls: 0,
            cursors: Vec::new(),
        }
    }

    #[test]
    fn collect_tools_follows_cursor_pages() {
        let mut session = fake_tools(
            vec![
                (vec!["sample-a", "sample-b"], Some("page-2")),
                (vec!["sample-c"], None),
            ],
            None,
            0,
        );
        let (tools, truncated, tools_error) =
            tauri::async_runtime::block_on(collect_tools(&mut session));
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, ["sample-a", "sample-b", "sample-c"]);
        assert_eq!(session.cursors, vec![None, Some("page-2".to_string())]);
        assert!(!truncated);
        assert_eq!(tools_error, None);
    }

    #[test]
    fn collect_tools_truncates_at_limit() {
        let names: Vec<String> = (0..MAX_TOOLS + 100)
            .map(|index| format!("sample-tool-{index}"))
            .collect();
        let mut session = FakeTools {
            pages: vec![(names, Some("page-2".to_string()))],
            notify_error: None,
            fail_from: 0,
            calls: 0,
            cursors: Vec::new(),
        };
        let (tools, truncated, tools_error) =
            tauri::async_runtime::block_on(collect_tools(&mut session));
        assert_eq!(tools.len(), MAX_TOOLS);
        assert!(tools
            .last()
            .is_some_and(|tool: &McpTool| tool.name == "sample-tool-499"));
        assert!(truncated);
        assert_eq!(tools_error, None);
    }

    #[test]
    fn collect_tools_records_page_error() {
        let mut session = fake_tools(vec![(vec!["sample-a"], None)], None, 1);
        let (tools, truncated, tools_error) =
            tauri::async_runtime::block_on(collect_tools(&mut session));
        assert!(tools.is_empty());
        assert!(!truncated);
        assert_eq!(tools_error.as_deref(), Some("sample 页请求失败"));
    }

    #[test]
    fn collect_tools_short_circuits_on_notify_error() {
        let mut session = fake_tools(vec![(vec!["sample-a"], None)], Some("sample 通知失败"), 0);
        let (tools, truncated, tools_error) =
            tauri::async_runtime::block_on(collect_tools(&mut session));
        assert!(tools.is_empty());
        assert!(!truncated);
        assert_eq!(tools_error.as_deref(), Some("sample 通知失败"));
        assert_eq!(session.calls, 0);
    }

    #[test]
    fn parses_json_and_sse_rpc_messages() {
        assert_eq!(
            parse_rpc_values(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).len(),
            1
        );
        assert_eq!(
            parse_rpc_values(
                "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n"
            )
            .len(),
            1
        );
    }

    #[cfg(windows)]
    #[test]
    fn command_candidates_follow_pathext_order_per_dir() {
        use super::command_candidates;
        use std::path::PathBuf;

        let dirs = vec![
            PathBuf::from("C:\\sample-dir"),
            PathBuf::from("D:\\other-dir"),
        ];
        assert_eq!(
            command_candidates("sample-cli", &dirs),
            vec![
                PathBuf::from("C:\\sample-dir\\sample-cli.exe"),
                PathBuf::from("C:\\sample-dir\\sample-cli.cmd"),
                PathBuf::from("C:\\sample-dir\\sample-cli.bat"),
                PathBuf::from("D:\\other-dir\\sample-cli.exe"),
                PathBuf::from("D:\\other-dir\\sample-cli.cmd"),
                PathBuf::from("D:\\other-dir\\sample-cli.bat"),
            ],
        );
    }

    #[test]
    fn parses_tool_page_and_cursor() {
        let (tools, cursor) = parse_tools_page(json!({
            "tools": [{
                "name": "search",
                "description": "Search",
                "inputSchema": { "type": "object" }
            }],
            "nextCursor": "next"
        }))
        .unwrap();
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[0].input_schema, json!({ "type": "object" }));
        assert_eq!(cursor.as_deref(), Some("next"));
    }

    #[test]
    fn version_negotiation_error_names_the_versions_the_server_accepts() {
        let message = rpc_result(
            vec![json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {
                    "code": -32022,
                    "message": "Unsupported protocol version",
                    "data": { "supported": ["2026-07-28", "2025-11-25"], "requested": "1900-01-01" },
                },
            })],
            1,
            &[],
        )
        .unwrap_err();
        // 说人话：不是"连不上"，是版本对不上，并给出对方认的版本
        assert!(message.contains("不支持协议版本"));
        assert!(message.contains("2026-07-28"));
        assert!(!message.contains("-32022"));
    }

    #[test]
    fn other_rpc_errors_keep_the_raw_code() {
        let message = rpc_result(
            vec![json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": { "code": -32601, "message": "Method not found" },
            })],
            1,
            &[],
        )
        .unwrap_err();
        assert_eq!(message, "MCP 错误 -32601: Method not found");
    }

    #[test]
    #[cfg(windows)]
    fn probe_env_var_falls_back_to_user_registry_without_restart() {
        // 进程环境不可能带这个唯一命名的变量；写入用户级注册表后应能直接读到，
        // 不依赖环境变更广播。先删再测，清理上一次中断运行可能留下的残余。
        let name = "AGMUX_PROBE_ENV_FALLBACK";
        let delete = || {
            let _ = std::process::Command::new("reg")
                .args(["delete", r"HKCU\Environment", "/v", name, "/f"])
                .status();
        };
        delete();
        assert!(std::env::var(name).is_err());
        let added = std::process::Command::new("reg")
            .args([
                "add",
                r"HKCU\Environment",
                "/v",
                name,
                "/t",
                "REG_SZ",
                "/d",
                "fixture-token",
                "/f",
            ])
            .status()
            .unwrap()
            .success();
        assert!(added, "写入测试用注册表变量失败");
        let resolved = std::panic::catch_unwind(|| super::probe_env_var(name));
        delete();
        assert_eq!(
            resolved.unwrap().unwrap(),
            Some("fixture-token".to_string())
        );
    }
}
