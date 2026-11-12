//! Claude Desktop 路由网关：本地 127.0.0.1 起一个 Anthropic 兼容端点。
//! 桌面端把目录名（claude-*）发过来，网关换成上游真实模型名再转发，
//! 让不认 Claude 模型名的上游（GLM/Kimi/MiniMax…）也能接入桌面端。

use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::error::{app_err, AppResult};
use crate::models::{
    ClaudeDesktopDiagnostics, ClaudeDesktopGatewayActivity, ClaudeDesktopModelUsage,
    ClaudeDesktopRequestInfo, ClaudeDesktopRequestOutcome as Outcome, ClaudeDesktopRouteInfo,
    ClaudeDesktopTokenUsage,
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayRoute {
    pub base: String,
    pub key: String,
    pub scheme: String,
    pub model: String,
    pub label: String,
    pub supports_1m: bool,
}

pub struct GatewayConfig {
    pub token: String,
    pub routes: BTreeMap<String, GatewayRoute>,
}

struct GatewayState {
    config: GatewayConfig,
    client: reqwest::Client,
    activity: Mutex<GatewayActivity>,
    running: AtomicBool,
}

pub struct GatewayHandle {
    pub port: u16,
    shutdown: oneshot::Sender<()>,
    state: Arc<GatewayState>,
}

impl std::fmt::Debug for GatewayHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayHandle")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl GatewayHandle {
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Relaxed)
    }

    pub fn diagnostics(&self, data: &mut ClaudeDesktopDiagnostics) -> AppResult<()> {
        data.routes = self
            .state
            .config
            .routes
            .iter()
            .map(|(id, route)| route_info(id, route))
            .collect();
        let (started_at_ms, requests, models) = {
            let activity = self
                .state
                .activity
                .lock()
                .map_err(|_| app_err!("网关诊断锁已损坏"))?;
            (
                activity.started_at_ms,
                activity.requests.iter().rev().cloned().collect::<Vec<_>>(),
                activity.models.values().cloned().collect(),
            )
        };
        data.activity = ClaudeDesktopGatewayActivity {
            started_at_ms: Some(started_at_ms),
            requests: requests
                .iter()
                .filter_map(|entry| entry.lock().ok().map(|entry| entry.clone()))
                .collect(),
            models,
        };
        Ok(())
    }

    pub fn stop(self) {
        // 发不出去说明服务端已退出，同样视为停掉
        let _ = self.shutdown.send(());
    }
}

pub(super) fn route_info(id: &str, route: &GatewayRoute) -> ClaudeDesktopRouteInfo {
    let host = url::Url::parse(&route.base)
        .ok()
        .and_then(|url| {
            url.host_str().map(|host| match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            })
        })
        .unwrap_or_else(|| "-".into());
    ClaudeDesktopRouteInfo {
        desktop_model: id.to_string(),
        upstream_model: route.model.clone(),
        label: route.label.clone(),
        host,
    }
}

struct GatewayActivity {
    started_at_ms: u64,
    requests: VecDeque<Arc<Mutex<ClaudeDesktopRequestInfo>>>,
    models: BTreeMap<String, ClaudeDesktopModelUsage>,
}

struct RequestTracker {
    state: Arc<GatewayState>,
    entry: Arc<Mutex<ClaudeDesktopRequestInfo>>,
    started: Instant,
    finished: bool,
    observer: ResponseObserver,
}

impl RequestTracker {
    fn new(state: &Arc<GatewayState>, path: &str, id: Option<&str>, started: Instant) -> Self {
        let route = id.and_then(|id| {
            state
                .config
                .routes
                .get(id)
                .map(|route| route_info(id, route))
        });
        let entry = Arc::new(Mutex::new(ClaudeDesktopRequestInfo {
            at_ms: crate::paths::now_ms() as u64,
            path: path.into(),
            route: route.clone(),
            status: None,
            upstream_status: None,
            header_ms: None,
            first_chunk_ms: None,
            duration_ms: None,
            outcome: Outcome::Pending,
            usage: ClaudeDesktopTokenUsage::default(),
        }));
        if let Ok(mut activity) = state.activity.lock() {
            if activity.requests.len() == 30 {
                activity.requests.pop_front();
            }
            activity.requests.push_back(entry.clone());
            // 模型发现和 count_tokens 是诊断记录，不计入推理用量。
            if path == "/v1/messages" {
                if let Some(route) = route {
                    activity
                        .models
                        .entry(route.desktop_model.clone())
                        .or_insert(ClaudeDesktopModelUsage {
                            route,
                            requests: 0,
                            completed: 0,
                            failed: 0,
                            cancelled: 0,
                            usage_reported: 0,
                            tokens: ClaudeDesktopTokenUsage::default(),
                        })
                        .requests += 1;
                }
            }
        }
        Self {
            state: state.clone(),
            entry,
            started,
            finished: false,
            observer: ResponseObserver::default(),
        }
    }

    fn headers(&mut self, status: u16, upstream: bool) {
        if let Ok(mut entry) = self.entry.lock() {
            entry.status = Some(status);
            entry.upstream_status = upstream.then_some(status);
            entry.header_ms = Some(self.started.elapsed().as_millis() as u64);
        }
    }

    fn chunk(&mut self, chunk: &[u8]) {
        self.observer.push(chunk);
        if let Ok(mut entry) = self.entry.lock() {
            if !chunk.is_empty() {
                entry
                    .first_chunk_ms
                    .get_or_insert(self.started.elapsed().as_millis() as u64);
            }
            entry.usage = self.observer.usage.clone();
        }
    }

    fn finish(&mut self, outcome: Outcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Ok(mut entry) = self.entry.lock() {
            entry.outcome = outcome;
            entry.duration_ms = Some(self.started.elapsed().as_millis() as u64);
            entry.usage = self.observer.usage.clone();
            if entry.path == "/v1/messages" {
                if let Some(route) = &entry.route {
                    if let Ok(mut activity) = self.state.activity.lock() {
                        if let Some(model) = activity.models.get_mut(&route.desktop_model) {
                            match outcome {
                                Outcome::Completed => model.completed += 1,
                                Outcome::Cancelled => model.cancelled += 1,
                                _ => model.failed += 1,
                            }
                            if entry.usage.input_tokens.is_some()
                                && entry.usage.output_tokens.is_some()
                            {
                                model.usage_reported += 1;
                            }
                            for (sum, value) in [
                                (&mut model.tokens.input_tokens, entry.usage.input_tokens),
                                (&mut model.tokens.output_tokens, entry.usage.output_tokens),
                                (
                                    &mut model.tokens.cache_read_input_tokens,
                                    entry.usage.cache_read_input_tokens,
                                ),
                                (
                                    &mut model.tokens.cache_creation_input_tokens,
                                    entry.usage.cache_creation_input_tokens,
                                ),
                            ] {
                                if let Some(value) = value {
                                    *sum = Some(sum.unwrap_or(0).saturating_add(value));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn eof(&mut self) {
        self.observer.end();
        let http_error = self
            .entry
            .lock()
            .ok()
            .and_then(|entry| entry.status)
            .is_some_and(|status| status >= 400);
        self.finish(if http_error {
            Outcome::HttpError
        } else if self.observer.failed {
            Outcome::StreamError
        } else if self.observer.sse && !self.observer.stopped {
            Outcome::Incomplete
        } else {
            Outcome::Completed
        });
    }
}

impl Drop for RequestTracker {
    fn drop(&mut self) {
        let http_error = self
            .entry
            .lock()
            .ok()
            .and_then(|entry| entry.status)
            .is_some_and(|status| status >= 400);
        let outcome = if http_error {
            Outcome::HttpError
        } else if self.observer.failed {
            Outcome::StreamError
        } else if self.observer.stopped {
            Outcome::Completed
        } else {
            Outcome::Cancelled
        };
        self.finish(outcome);
    }
}

// 只提取 usage / 结束信号，正文原样透传；采集上限不限制客户端响应。
#[derive(Default)]
struct ResponseObserver {
    sse: bool,
    line: Vec<u8>,
    data: Vec<u8>,
    overflow: bool,
    previous_cr: bool,
    failed: bool,
    stopped: bool,
    usage: ClaudeDesktopTokenUsage,
}

impl ResponseObserver {
    fn push(&mut self, bytes: &[u8]) {
        if !self.sse {
            // shortcut: 超过 1 MiB 的非流式 JSON 不采集 usage；需要覆盖时改为增量 JSON 提取。
            if self.data.len().saturating_add(bytes.len()) > 1024 * 1024 {
                self.data.clear();
                self.overflow = true;
            }
            if !self.overflow {
                self.data.extend_from_slice(bytes);
            }
            return;
        }
        for &byte in bytes {
            if byte == b'\r' || (byte == b'\n' && !self.previous_cr) {
                self.line_end();
            } else if byte != b'\n' {
                if self.line.len() < 64 * 1024 {
                    self.line.push(byte);
                } else {
                    self.overflow = true;
                }
            }
            self.previous_cr = byte == b'\r';
        }
    }

    fn line_end(&mut self) {
        if self.line.is_empty() {
            if !self.overflow {
                if let Ok(value) = serde_json::from_slice::<Value>(&self.data) {
                    self.observe(&value);
                }
            }
            self.data.clear();
            self.overflow = false;
        } else if let Some(data) = self.line.strip_prefix(b"data:") {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            if self.data.len() + data.len() < 64 * 1024 && !self.overflow {
                self.data.extend_from_slice(data);
                self.data.push(b'\n');
            } else {
                self.overflow = true;
            }
        }
        self.line.clear();
    }

    fn observe(&mut self, value: &Value) {
        match value["type"].as_str() {
            Some("error") => self.failed = true,
            Some("message_stop") => self.stopped = true,
            _ => {}
        }
        let usage = value.get("usage").or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("usage"))
        });
        if let Some(usage) = usage {
            // SSE message_delta 是累计值；只替换已提供字段，不能逐事件相加。
            for (target, key) in [
                (&mut self.usage.input_tokens, "input_tokens"),
                (&mut self.usage.output_tokens, "output_tokens"),
                (
                    &mut self.usage.cache_read_input_tokens,
                    "cache_read_input_tokens",
                ),
                (
                    &mut self.usage.cache_creation_input_tokens,
                    "cache_creation_input_tokens",
                ),
            ] {
                if let Some(value) = usage[key].as_u64() {
                    *target = Some(value);
                }
            }
        }
    }

    fn end(&mut self) {
        if !self.sse && !self.overflow {
            if let Ok(value) = serde_json::from_slice::<Value>(&self.data) {
                self.observe(&value);
            }
        }
    }
}

fn authorized(headers: &axum::http::HeaderMap, token: &str) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);
    let api_key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    bearer == Some(token) || api_key == Some(token)
}

async fn proxy(State(state): State<Arc<GatewayState>>, request: Request<Body>) -> Response {
    if !authorized(request.headers(), &state.config.token) {
        return (StatusCode::UNAUTHORIZED, "invalid gateway token").into_response();
    }
    let started = Instant::now();
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    // 桌面端的自动发现只认得出 claude-* 名，目录即映射键
    if parts.method == Method::GET && parts.uri.path() == "/v1/models" {
        let data: Vec<Value> = state
            .config
            .routes
            .iter()
            .map(|(name, route)| {
                let mut model = json!({ "id": name, "type": "model", "display_name": route.label });
                if let Some(behavior) = super::claude_desktop::alias_behavior(name) {
                    model["behaves_as"] = json!(behavior);
                }
                model
            })
            .collect();
        let mut tracker = RequestTracker::new(&state, "/v1/models", None, started);
        tracker.headers(200, false);
        tracker.finish(Outcome::Completed);
        return axum::Json(json!({ "data": data, "has_more": false })).into_response();
    }
    if !matches!(
        parts.uri.path(),
        "/v1/messages" | "/v1/messages/count_tokens"
    ) {
        return (StatusCode::NOT_FOUND, "unsupported gateway endpoint").into_response();
    }
    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::POST);
    let bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let mut tracker = RequestTracker::new(&state, parts.uri.path(), None, started);
            tracker.headers(400, false);
            tracker.finish(Outcome::HttpError);
            return (StatusCode::BAD_REQUEST, format!("读取请求体失败: {error}")).into_response();
        }
    };
    let mut payload: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            let mut tracker = RequestTracker::new(&state, parts.uri.path(), None, started);
            tracker.headers(400, false);
            tracker.finish(Outcome::HttpError);
            return (StatusCode::BAD_REQUEST, "invalid JSON body").into_response();
        }
    };
    let Some((id, route)) = payload
        .get("model")
        .and_then(Value::as_str)
        .and_then(|model| state.config.routes.get_key_value(model))
    else {
        let mut tracker = RequestTracker::new(&state, parts.uri.path(), None, started);
        tracker.headers(400, false);
        tracker.finish(Outcome::HttpError);
        return (
            StatusCode::BAD_REQUEST,
            "unknown model; refresh the model list",
        )
            .into_response();
    };
    let mut tracker = RequestTracker::new(&state, parts.uri.path(), Some(id), started);
    payload["model"] = json!(route.model);
    let upstream_url = format!("{}{}", route.base.trim_end_matches('/'), path);
    let mut upstream = state.client.request(method, &upstream_url);
    for (name, value) in parts.headers.iter() {
        // 逐跳头与鉴权不透传，鉴权换成上游真 key
        if matches!(
            name.as_str(),
            "host"
                | "authorization"
                | "x-api-key"
                | "content-length"
                | "connection"
                | "transfer-encoding"
                | "accept-encoding"
        ) {
            continue;
        }
        if let Ok(text) = value.to_str() {
            upstream = upstream.header(name.as_str(), text);
        }
    }
    upstream = match route.scheme.as_str() {
        "x-api-key" => upstream.header("x-api-key", &route.key),
        _ => upstream.header("authorization", format!("Bearer {}", route.key)),
    };
    let response = match upstream.json(&payload).send().await {
        Ok(response) => response,
        Err(error) => {
            tracker.headers(502, false);
            tracker.finish(Outcome::NetworkError);
            return (
                StatusCode::BAD_GATEWAY,
                format!("上游请求失败: {}", error.without_url()),
            )
                .into_response();
        }
    };
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    tracker.headers(status.as_u16(), true);
    tracker.observer.sse = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    let mut builder = Response::builder().status(status);
    for (name, value) in response.headers().iter() {
        if matches!(
            name.as_str(),
            "transfer-encoding" | "content-length" | "connection"
        ) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
            axum::http::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            builder = builder.header(name, value);
        }
    }
    // 按客户端拉取节奏透传，不后台预读；断开时 drop tracker 记录取消。
    let stream = futures_util::stream::unfold(
        (response, tracker),
        |(mut response, mut tracker)| async move {
            if tracker.finished {
                return None;
            }
            match response.chunk().await {
                Ok(Some(bytes)) => {
                    tracker.chunk(&bytes);
                    Some((Ok(bytes), (response, tracker)))
                }
                Ok(None) => {
                    tracker.eof();
                    None
                }
                Err(error) => {
                    tracker.finish(Outcome::StreamError);
                    Some((Err(error), (response, tracker)))
                }
            }
        },
    );
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|error| {
            (StatusCode::BAD_GATEWAY, format!("网关响应失败: {error}")).into_response()
        })
}

/// 绑定随机端口并常驻后台；调用方持有 handle，drop 前主动 stop()。
pub fn start_blocking(
    config: GatewayConfig,
    client: reqwest::Client,
    port: u16,
) -> AppResult<GatewayHandle> {
    tauri::async_runtime::block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|error| app_err!("路由网关监听失败: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| app_err!("路由网关取端口失败: {error}"))?
            .port();
        let (shutdown, receiver) = oneshot::channel::<()>();
        let state = Arc::new(GatewayState {
            config,
            client,
            activity: Mutex::new(GatewayActivity {
                started_at_ms: crate::paths::now_ms() as u64,
                requests: VecDeque::new(),
                models: BTreeMap::new(),
            }),
            running: AtomicBool::new(true),
        });
        let app = axum::Router::new()
            .fallback(any(proxy))
            .with_state(state.clone());
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = receiver.await;
        });
        let serving = state.clone();
        tauri::async_runtime::spawn(async move {
            if server.await.is_err() {
                tauri_plugin_log::log::warn!("[gateway.claude-desktop.serve] outcome=failure failure_kind=io_error msg=\"本地网关服务退出\"");
            }
            serving.running.store(false, Ordering::Relaxed);
        });
        Ok(GatewayHandle {
            port,
            shutdown,
            state,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Arc<GatewayState> {
        Arc::new(GatewayState {
            config: GatewayConfig {
                token: "local-secret".into(),
                routes: BTreeMap::from([(
                    "claude-sonnet-test".into(),
                    GatewayRoute {
                        base: "https://user:password@example.com:8443/private?secret=query".into(),
                        key: "upstream-secret".into(),
                        scheme: "bearer".into(),
                        model: "actual-model".into(),
                        label: "Provider / Model".into(),
                        supports_1m: false,
                    },
                )]),
            },
            client: reqwest::Client::new(),
            activity: Mutex::new(GatewayActivity {
                started_at_ms: 1,
                requests: VecDeque::new(),
                models: BTreeMap::new(),
            }),
            running: AtomicBool::new(true),
        })
    }

    #[test]
    fn usage_is_cumulative_and_survives_arbitrary_sse_chunk_boundaries() {
        let input = concat!(
            "event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":1,\"cache_read_input_tokens\":8}}}\r\n\r\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"private 对话\"}}\n\n",
            "data: {\"type\":\"ping\"}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":3}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9,\"cache_creation_input_tokens\":0}}\r\r",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        for size in [1, 2, 7, 4096] {
            let mut observer = ResponseObserver {
                sse: true,
                ..Default::default()
            };
            for chunk in input.as_bytes().chunks(size) {
                observer.push(chunk);
            }
            observer.end();
            assert!(observer.stopped);
            assert!(!observer.failed);
            assert_eq!(observer.usage.input_tokens, Some(12));
            assert_eq!(observer.usage.output_tokens, Some(9));
            assert_eq!(observer.usage.cache_read_input_tokens, Some(8));
            assert_eq!(observer.usage.cache_creation_input_tokens, Some(0));
        }
    }

    #[test]
    fn json_usage_and_missing_fields_are_distinct_from_zero() {
        let mut observer = ResponseObserver::default();
        observer.push(br#"{"usage":{"input_tokens":0,"output_tokens":7,"cache_read_input_tokens":-1},"content":"private"}"#);
        observer.end();
        assert_eq!(observer.usage.input_tokens, Some(0));
        assert_eq!(observer.usage.output_tokens, Some(7));
        assert_eq!(observer.usage.cache_read_input_tokens, None);
        assert_eq!(observer.usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn collection_limits_do_not_leak_content_or_lose_later_sse_usage() {
        let mut observer = ResponseObserver {
            sse: true,
            ..Default::default()
        };
        observer.push(b"data: ");
        observer.push(&vec![b'x'; 70 * 1024]);
        observer
            .push(b"\n\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\n\n");
        assert_eq!(observer.usage.output_tokens, Some(2));
        assert!(observer.line.len() <= 64 * 1024);
        let mut observer = ResponseObserver::default();
        observer.push(&vec![b'x'; 2 * 1024 * 1024]);
        observer.end();
        assert!(observer.data.is_empty());
        assert!(observer.usage.input_tokens.is_none());
    }

    #[test]
    fn lifecycle_handles_late_errors_truncation_cancellation_and_http_failure() {
        let state = state();
        for (bytes, eof, status, expected) in [
            (
                "data: {\"type\":\"message_stop\"}\n\n",
                true,
                200,
                Outcome::Completed,
            ),
            (
                "data: {\"type\":\"error\",\"error\":{\"message\":\"secret\"}}\n\n",
                true,
                200,
                Outcome::StreamError,
            ),
            (
                "data: {\"type\":\"ping\"}\n\n",
                true,
                200,
                Outcome::Incomplete,
            ),
            (
                "data: {\"type\":\"ping\"}\n\n",
                false,
                200,
                Outcome::Cancelled,
            ),
            (
                "data: {\"type\":\"message_stop\"}\n\n",
                false,
                200,
                Outcome::Completed,
            ),
            (
                "data: {\"type\":\"error\"}\n\n",
                true,
                429,
                Outcome::HttpError,
            ),
        ] {
            let mut tracker = RequestTracker::new(
                &state,
                "/v1/messages",
                Some("claude-sonnet-test"),
                Instant::now(),
            );
            let entry = tracker.entry.clone();
            tracker.headers(status, true);
            tracker.observer.sse = true;
            tracker.chunk(bytes.as_bytes());
            if eof {
                tracker.eof();
            }
            drop(tracker);
            let entry = entry.lock().unwrap();
            assert_eq!(entry.outcome, expected);
            assert!(entry.first_chunk_ms.is_some());
            assert!(entry.duration_ms.is_some());
        }
        let activity = state.activity.lock().unwrap();
        let model = &activity.models["claude-sonnet-test"];
        assert_eq!(
            (
                model.requests,
                model.completed,
                model.failed,
                model.cancelled
            ),
            (6, 2, 3, 1)
        );
    }

    #[test]
    fn session_totals_survive_recent_request_eviction_and_exclude_token_estimates() {
        let state = state();
        for _ in 0..35 {
            let mut tracker = RequestTracker::new(
                &state,
                "/v1/messages",
                Some("claude-sonnet-test"),
                Instant::now(),
            );
            tracker.headers(200, true);
            tracker.chunk(br#"{"usage":{"input_tokens":2,"output_tokens":3}}"#);
            tracker.eof();
            tracker.eof(); // Finalization must never double count.
        }
        let mut tracker = RequestTracker::new(
            &state,
            "/v1/messages/count_tokens",
            Some("claude-sonnet-test"),
            Instant::now(),
        );
        tracker.headers(200, true);
        tracker.chunk(br#"{"input_tokens":100}"#);
        tracker.eof();
        let activity = state.activity.lock().unwrap();
        assert_eq!(activity.requests.len(), 30);
        let model = &activity.models["claude-sonnet-test"];
        assert_eq!(model.requests, 35);
        assert_eq!(model.usage_reported, 35);
        assert_eq!(model.tokens.input_tokens, Some(70));
        assert_eq!(model.tokens.output_tokens, Some(105));
        assert_eq!(model.tokens.cache_read_input_tokens, None);
        let serialized = serde_json::to_string(
            &activity
                .requests
                .iter()
                .map(|entry| entry.lock().unwrap().clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(serialized.contains("example.com:8443"));
        for secret in [
            "local-secret",
            "upstream-secret",
            "password",
            "private",
            "query",
        ] {
            assert!(!serialized.contains(secret));
        }
    }

    #[test]
    fn authorization_accepts_bearer_and_x_api_key_forms() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer secret".parse().unwrap());
        assert!(authorized(&headers, "secret"));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-api-key", "secret".parse().unwrap());
        assert!(authorized(&headers, "secret"));
        headers.insert("x-api-key", "wrong".parse().unwrap());
        assert!(!authorized(&headers, "secret"));
    }
}
