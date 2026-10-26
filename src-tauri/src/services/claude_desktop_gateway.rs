//! Claude Desktop 路由网关：本地 127.0.0.1 起一个 Anthropic 兼容端点。
//! 桌面端把目录名（claude-*）发过来，网关换成上游真实模型名再转发，
//! 让不认 Claude 模型名的上游（GLM/Kimi/MiniMax…）也能接入桌面端。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::error::{app_err, AppResult};

pub struct GatewayConfig {
    /// 桌面端 profile 里存的本地令牌；请求头不匹配一律 401。
    pub token: String,
    /// 上游 Anthropic 兼容端点根（如 https://open.bigmodel.cn/api/anthropic）。
    pub upstream_base: String,
    pub upstream_key: String,
    /// "bearer" | "x-api-key"，与 profile 的 inferenceGatewayAuthScheme 一致。
    pub upstream_scheme: String,
    /// 目录名 → 上游真实模型名。
    pub mapping: BTreeMap<String, String>,
}

struct GatewayState {
    config: GatewayConfig,
    client: reqwest::Client,
}

pub struct GatewayHandle {
    pub port: u16,
    shutdown: oneshot::Sender<()>,
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
    pub fn stop(self) {
        // 发不出去说明服务端已退出，同样视为停掉
        let _ = self.shutdown.send(());
    }
}

/// 请求体里的目录名换成上游名；不认识的模型原样透传，让上游自己报错（信息更真实）。
fn rewrite_model(body: &[u8], mapping: &BTreeMap<String, String>) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return body.to_vec();
    };
    let Some(model) = value.get("model").and_then(Value::as_str) else {
        return body.to_vec();
    };
    let Some(target) = mapping.get(model) else {
        return body.to_vec();
    };
    value["model"] = Value::String(target.clone());
    serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
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
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    // 桌面端的自动发现只认得出 claude-* 名，目录即映射键
    if parts.method == Method::GET && path.starts_with("/v1/models") {
        let data: Vec<Value> = state
            .config
            .mapping
            .keys()
            .map(|name| json!({ "id": name, "type": "model", "display_name": name }))
            .collect();
        return axum::Json(json!({ "data": data, "has_more": false })).into_response();
    }
    let upstream_url = format!(
        "{}{}",
        state.config.upstream_base.trim_end_matches('/'),
        path
    );
    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::POST);
    let bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, format!("读取请求体失败: {error}")).into_response();
        }
    };
    let payload = rewrite_model(&bytes, &state.config.mapping);
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
    upstream = match state.config.upstream_scheme.as_str() {
        "x-api-key" => upstream.header("x-api-key", &state.config.upstream_key),
        _ => upstream.header(
            "authorization",
            format!("Bearer {}", state.config.upstream_key),
        ),
    };
    let response = match upstream.body(payload).send().await {
        Ok(response) => response,
        Err(error) => {
            return (StatusCode::BAD_GATEWAY, format!("上游请求失败: {error}")).into_response();
        }
    };
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
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
    // SSE 走字节流透传，不在网关聚齐
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|error| {
            (StatusCode::BAD_GATEWAY, format!("网关响应失败: {error}")).into_response()
        })
}

/// 绑定随机端口并常驻后台；调用方持有 handle，drop 前主动 stop()。
pub fn start_blocking(config: GatewayConfig, client: reqwest::Client) -> AppResult<GatewayHandle> {
    tauri::async_runtime::block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| app_err!("路由网关监听失败: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| app_err!("路由网关取端口失败: {error}"))?
            .port();
        let (shutdown, receiver) = oneshot::channel::<()>();
        let state = Arc::new(GatewayState { config, client });
        let app = axum::Router::new().fallback(any(proxy)).with_state(state);
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = receiver.await;
        });
        tauri::async_runtime::spawn(async move {
            let _ = server.await;
        });
        Ok(GatewayHandle { port, shutdown })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("claude-sonnet-5-5".into(), "glm-4.7".into()),
            ("claude-opus-5-5".into(), "glm-4.7-plus".into()),
        ])
    }

    #[test]
    fn rewrites_known_catalog_names_and_passes_the_rest_through() {
        let map = mapping();
        let body = br#"{"model":"claude-sonnet-5-5","messages":[]}"#;
        let rewritten = rewrite_model(body, &map);
        assert_eq!(
            serde_json::from_slice::<Value>(&rewritten).unwrap()["model"],
            "glm-4.7"
        );
        // 未知名、无 model 字段、非 JSON 体都原样返回
        assert_eq!(
            rewrite_model(br#"{"model":"gpt-4o"}"#, &map),
            br#"{"model":"gpt-4o"}"#.to_vec()
        );
        assert_eq!(rewrite_model(br#"{"a":1}"#, &map), br#"{"a":1}"#.to_vec());
        assert_eq!(rewrite_model(b"not-json", &map), b"not-json".to_vec());
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
