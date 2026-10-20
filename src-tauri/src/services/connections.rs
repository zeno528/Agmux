use super::codex_profile_config::{parse_provider_detail, stored_provider_api_key};
use super::{
    app_err, atomic_write, AppContext, AppResult, AuthSource, BTreeMap, ChatgptResetCredit,
    CodexProfileKind, PathBuf, ProfileBalanceInfo,
};
use crate::auth::codex_oauth::{account_subject, parse_external_auth_json, CodexOAuthManager};
use crate::network::{proxy_note, Network};

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ChatgptModel {
    pub slug: String,
    pub display_name: String,
    pub supported_reasoning_levels: Vec<ChatgptReasoningLevel>,
    #[serde(default)]
    pub default_reasoning_level: Option<String>,
    #[serde(default, skip_serializing)]
    visibility: String,
    #[serde(default, skip_serializing)]
    priority: i64,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ChatgptReasoningLevel {
    pub effort: String,
}

pub(super) fn chatgpt_model_cache_key(source: AuthSource, account_id: Option<&str>) -> String {
    let source = match source {
        AuthSource::Desktop => "desktop",
        AuthSource::Oauth => "oauth",
    };
    format!("{source}:{}", account_id.unwrap_or(""))
}

pub(crate) fn parse_chatgpt_models(value: serde_json::Value) -> AppResult<Vec<ChatgptModel>> {
    // 缓存反序列化可省略 visibility；接口响应仍须提供这个必需字段。
    if value["models"]
        .as_array()
        .is_some_and(|models| models.iter().any(|model| !model["visibility"].is_string()))
    {
        return Err(app_err!("ChatGPT 模型目录格式无效"));
    }
    #[derive(serde::Deserialize)]
    struct Catalog {
        models: Vec<ChatgptModel>,
    }
    let mut catalog: Catalog =
        serde_json::from_value(value).map_err(|_| app_err!("ChatGPT 模型目录格式无效"))?;
    catalog
        .models
        .retain(|model| model.visibility == "list" && !model.slug.trim().is_empty());
    catalog.models.sort_by_key(|model| model.priority);
    Ok(catalog.models)
}

/// 供应商连通性测试结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodexProfileConnectionResult {
    pub ok: bool,
    pub latency_ms: Option<u128>,
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// 供应商余额/用量查询结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProfileBalance {
    pub is_available: bool,
    pub balance_infos: Vec<ProfileBalanceInfo>,
    pub latency_ms: Option<u128>,
}

/// DeepSeek 余额接口响应（接口文档：https://api-docs.deepseek.com/zh-cn/api/get-user-balance）
#[derive(Debug, serde::Deserialize)]
struct DeepSeekBalanceResponse {
    is_available: bool,
    balance_infos: Vec<ProfileBalanceInfo>,
}

/// 过滤掉 0 余额币种，有几个非 0 显示几个；全部为 0 时保底返回原列表（CNY 优先）。
fn preferred_deepseek_balance(mut balances: Vec<ProfileBalanceInfo>) -> Vec<ProfileBalanceInfo> {
    if let Some(index) = balances
        .iter()
        .position(|balance| balance.currency == "CNY")
    {
        balances.swap(0, index);
    }
    let non_zero: Vec<ProfileBalanceInfo> = balances
        .iter()
        .filter(|balance| {
            balance
                .total_balance
                .parse::<f64>()
                .is_ok_and(|total| total != 0.0)
        })
        .cloned()
        .collect();
    if non_zero.is_empty() {
        balances
    } else {
        non_zero
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn chatgpt_catalog_filters_hidden_models_and_keeps_order_and_efforts() {
        let entry = |slug: &str, visibility: &str, priority: i64| {
            serde_json::json!({
                "slug": slug, "display_name": slug, "visibility": visibility, "priority": priority,
                "supported_reasoning_levels": [{"effort": "high"}],
                "default_reasoning_level": "high",
            })
        };
        let models = super::parse_chatgpt_models(serde_json::json!({"models": [
            entry("second", "list", 2), entry("hidden", "hide", 0), entry("first", "list", 1),
        ]}))
        .unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.slug.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(models[0].supported_reasoning_levels[0].effort, "high");
        assert_eq!(models[0].default_reasoning_level.as_deref(), Some("high"));
        let mut missing_visibility = entry("invalid", "list", 0);
        missing_visibility
            .as_object_mut()
            .unwrap()
            .remove("visibility");
        assert!(
            super::parse_chatgpt_models(serde_json::json!({"models": [missing_visibility]}))
                .is_err()
        );
        assert!(super::parse_chatgpt_models(serde_json::json!({"data": []})).is_err());
        assert!(
            super::parse_chatgpt_models(serde_json::json!({"models": [{"slug": "invalid"}]}))
                .is_err()
        );
    }

    use super::{
        claude_balance_base, preferred_deepseek_balance, provider_default_balance_base,
        provider_request_error_message, quota_failure_error, subscription_http_error_message,
        subscription_request_error_message,
    };
    use crate::models::ProfileBalanceInfo;

    #[tokio::test]
    async fn warmup_waits_for_completion_and_usage_without_waiting_for_stream_close() {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/responses", listener.local_addr().unwrap());
        let (done, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(&stream);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            assert!(headers.starts_with("POST /responses "));
            assert!(headers.contains("authorization: Bearer fixture-token"));
            assert!(headers.contains("chatgpt-account-id: fixture-workspace"));
            assert!(headers.contains("originator: codex_cli_rs"));
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["model"], "gpt-6-luna");
            assert_eq!(body["reasoning"]["effort"], "low");
            assert_eq!(body["input"].as_array().unwrap().len(), 1);
            assert_eq!(body["input"][0]["content"][0]["text"], "Reply only OK.");
            assert_eq!(body["tools"], serde_json::json!([]));
            assert_eq!(body["store"], false);
            assert_eq!(body["stream"], true);
            assert!(body.get("max_output_tokens").is_none());
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            for chunk in [
                "data: {\"type\":\"response.created\"}\r\n\r\ndata: {\"ty",
                "pe\":\"response.completed\",\r\ndata: \"response\":{\"usage\":{\"total_tokens\":3}}}\r\n",
                "\r\n",
            ] {
                write!(stream, "{:x}\r\n{chunk}\r\n", chunk.len()).unwrap();
                stream.flush().unwrap();
            }
            // 不发送流结束标记；客户端必须在完成事件后主动结束读取。
            let _ = wait.recv_timeout(std::time::Duration::from_secs(3));
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = super::send_chatgpt_warmup(super::chatgpt_request(
            client.post(url),
            "fixture-token",
            Some("fixture-workspace"),
        ))
        .await;
        done.send(()).unwrap();
        server.join().unwrap();
        assert_eq!(result.unwrap(), 3);
        for event in [
            r#"{"type":"response.created"}"#,
            r#"{"type":"response.completed","response":{"usage":{"total_tokens":0}}}"#,
            r#"{"type":"response.completed","response":{}}"#,
            r#"{"type":"response.failed"}"#,
            r#"{"type":"response.incomplete"}"#,
            r#"{"type":"error"}"#,
            "invalid json",
        ] {
            assert!(!matches!(
                super::warmup_event_tokens(event.as_bytes()),
                Ok(Some(_))
            ));
        }
    }

    #[tokio::test]
    async fn connection_latency_includes_response_body_for_both_clients() {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in [200, 200, 401, 401] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(&stream);
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                let body = if status == 200 {
                    r#"{"data":[]}"#
                } else {
                    r#"{"error":"denied"}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(150));
                stream.write_all(body.as_bytes()).unwrap();
            }
        });
        for status in [200, 401] {
            let codex = super::test_provider_connection(&base, "fixture-key")
                .await
                .unwrap();
            let claude = super::super::claude::test_claude_connection(&base, "fixture-key")
                .await
                .unwrap();
            for result in [codex, claude] {
                assert_eq!(result.ok, status == 200);
                assert_eq!(result.status, Some(status));
                assert!(result.latency_ms.unwrap() >= 100, "{result:?}");
            }
        }
        server.join().unwrap();
        let invalid = super::super::claude::test_claude_connection("", "fixture-key")
            .await
            .unwrap();
        assert_eq!(invalid.latency_ms, None, "准备阶段失败不应产生请求耗时");
    }

    #[tokio::test]
    async fn opencode_probe_sends_credentials_without_inference_payload() {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut headers = String::new();
            let mut reader = std::io::BufReader::new(&stream);
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
            }
            stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 14\r\nConnection: close\r\n\r\nmodel required").unwrap();
            headers.to_ascii_lowercase()
        });
        let result = super::test_opencode_connection(&base, "fixture-key", "source=test")
            .await
            .unwrap();
        let headers = server.join().unwrap();
        assert!(headers.starts_with("post /responses "));
        assert!(headers.contains("authorization: bearer fixture-key\r\n"));
        assert!(headers
            .lines()
            .filter(|line| line.starts_with("content-length:"))
            .all(|line| line == "content-length: 0"));
        assert!(!headers.contains("transfer-encoding:"));
        assert!(result.ok);
    }

    fn balance(currency: &str, total: &str) -> ProfileBalanceInfo {
        ProfileBalanceInfo {
            currency: currency.into(),
            total_balance: total.into(),
            usage_percent: None,
            usage_reset: None,
            usage_reset_at: None,
            usage_label: None,
            weekly_usage_percent: None,
            weekly_reset: None,
            weekly_reset_at: None,
            weekly_label: None,
            reset_credits_available: None,
            reset_credits: None,
        }
    }

    #[test]
    fn filters_zero_balance_and_prefers_cny_fallback() {
        // USD 为 0 时只显示 CNY。
        let balances = vec![balance("USD", "0.00"), balance("CNY", "8.85")];
        let filtered = preferred_deepseek_balance(balances);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].currency, "CNY");

        // 两个币种都有余额时都返回，CNY 在前。
        let balances = vec![balance("USD", "5.00"), balance("CNY", "8.85")];
        let filtered = preferred_deepseek_balance(balances);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].currency, "CNY");
        assert_eq!(filtered[1].currency, "USD");

        // 全部为 0 时保底返回原列表，CNY 优先。
        let balances = vec![balance("USD", "0.00"), balance("CNY", "0.00")];
        let filtered = preferred_deepseek_balance(balances);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].currency, "CNY");
        assert_eq!(filtered[0].total_balance, "0.00");
    }

    #[test]
    fn claude_zhipu_resolves_to_the_shared_usage_vendor() {
        // Claude 侧 kind 是 zhipu（Codex 侧叫 ZAI）。两处漏配都是静默失败：
        // 归一漏 zhipu → 回退到错误默认端点；默认表漏 zhipu → 调用点无条件求值，直接撞 unreachable!()。
        assert_eq!(
            claude_balance_base("zhipu", Some("https://gateway.example.test/anthropic")),
            "https://gateway.example.test"
        );
        assert!(provider_default_balance_base("zhipu").starts_with("https://"));
    }

    #[test]
    fn provider_origin_preserves_authority_and_strips_url_suffixes() {
        for (base, expected) in [
            (" https://api.example.test/v1/ ", "https://api.example.test"),
            (
                "https://api.example.test?route=v1#models",
                "https://api.example.test",
            ),
            (
                "http://user:pass@[::1]:8080/v1",
                "http://user:pass@[::1]:8080",
            ),
        ] {
            assert_eq!(super::provider_origin(base).unwrap(), expected, "{base}");
        }
        for invalid in [
            "https://",
            "ftp://api.example.test",
            "file:///tmp/api",
            "http:api.example.test",
        ] {
            assert!(super::provider_origin(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn provider_request_errors_return_localizable_codes() {
        let error = reqwest::Client::new()
            .get("not a valid URL")
            .build()
            .unwrap_err();
        assert_eq!(provider_request_error_message(&error), "invalidEndpoint");
    }

    #[test]
    fn quota_failure_errors_preserve_user_messages() {
        // 401/403（非地区拦截）→ 登录失效；前端直接展示正常的查询失败状态。
        let error = quota_failure_error(reqwest::StatusCode::UNAUTHORIZED, "{}", "test");
        assert_eq!(error.0, "ChatGPT 登录已失效，请重新登录");

        // 地区拦截：换节点可解，不标记为需要重登
        let region = quota_failure_error(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error":"unsupported_country_region_territory"}"#,
            "test",
        );
        assert_eq!(region.0, "认证请求被地区限制拦截，请开启系统代理后重试");

        // 其他 HTTP 错误按普通失败处理
        let other = quota_failure_error(reqwest::StatusCode::TOO_MANY_REQUESTS, "", "test");
        assert!(other.0.starts_with("用量查询失败："));
    }

    #[test]
    fn subscription_connection_errors_use_subscription_specific_messages() {
        assert_eq!(
            subscription_http_error_message(reqwest::StatusCode::UNAUTHORIZED, ""),
            "ChatGPT 登录已失效，请重新登录"
        );
        assert_eq!(
            subscription_http_error_message(
                reqwest::StatusCode::FORBIDDEN,
                "unsupported_country_region_territory"
            ),
            "ChatGPT 访问受到地区限制，请开启系统代理后重试"
        );
        assert_eq!(
            subscription_http_error_message(reqwest::StatusCode::FORBIDDEN, ""),
            "ChatGPT 订阅请求被拒绝，请重新登录后重试"
        );
        assert_eq!(
            subscription_http_error_message(reqwest::StatusCode::TOO_MANY_REQUESTS, ""),
            "请求过于频繁，请稍后重试"
        );
        assert_eq!(
            subscription_http_error_message(reqwest::StatusCode::BAD_GATEWAY, ""),
            "ChatGPT 订阅服务暂时不可用，请稍后重试"
        );

        let error = reqwest::Client::new()
            .get("not a valid URL")
            .build()
            .unwrap_err();
        assert_eq!(
            subscription_request_error_message(&error),
            "ChatGPT 订阅请求未完成，请稍后重试"
        );
    }
}

/// MiniMax Coding Plan 用量接口响应（国内版：api.minimaxi.com/v1/api/openplatform/coding_plan/remains）
#[derive(Debug, serde::Deserialize)]
struct MiniMaxRemainsResponse {
    #[serde(default)]
    base_resp: MiniMaxBaseResp,
    #[serde(default)]
    model_remains: Vec<MiniMaxModelRemains>,
}

#[derive(Clone, Copy)]
enum BalanceAuth {
    Bearer,
    Raw,
}

struct ZhipuQuotaWindow {
    used_percent: u32,
    reset: Option<String>,
    reset_at: Option<i64>,
    unit: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ChatgptUsageResponse {
    pub(crate) rate_limit: Option<ChatgptRateLimit>,
    pub(crate) rate_limit_reset_credits: Option<ChatgptResetCreditsSummary>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ChatgptResetCreditsSummary {
    pub(crate) available_count: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct ChatgptResetCreditsResponse {
    #[serde(default)]
    available_count: Option<i64>,
    #[serde(default)]
    credits: Vec<ChatgptResetCreditResponse>,
}

#[derive(Debug, serde::Deserialize)]
struct ChatgptResetCreditResponse {
    id: String,
    status: Option<String>,
    reset_type: Option<String>,
    expires_at: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ChatgptRateLimit {
    pub(crate) primary_window: Option<ChatgptRateLimitWindow>,
    pub(crate) secondary_window: Option<ChatgptRateLimitWindow>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ChatgptRateLimitWindow {
    pub(crate) used_percent: Option<f64>,
    pub(crate) limit_window_seconds: Option<i64>,
    pub(crate) reset_at: Option<i64>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct MiniMaxBaseResp {
    #[serde(default)]
    status_code: Option<i64>,
    #[serde(default)]
    status_msg: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct MiniMaxModelRemains {
    #[serde(default)]
    pub(crate) model_name: String,
    /// 剩余百分比（0-100），接口语义为“剩余”，卡片显示“用量”时换算成已用。
    #[serde(default)]
    pub(crate) current_interval_remaining_percent: Option<f64>,
    /// 7 天窗口剩余百分比（0-100）。
    #[serde(default)]
    pub(crate) current_weekly_remaining_percent: Option<f64>,
    /// 5 小时窗口重置倒计时（毫秒）。
    #[serde(default)]
    pub(crate) remains_time: Option<i64>,
    /// 7 天窗口重置倒计时（毫秒）。
    #[serde(default)]
    pub(crate) weekly_remains_time: Option<i64>,
}

/// 统一的 HTTP 客户端：8 秒超时。
/// 出站请求统一入口：显式配置检测到的系统代理并返回代理上下文。
/// reqwest 的隐式系统代理只有它自己知道（只出现在 DEBUG 日志里），
/// 显式配置后「这次请求走了哪个代理」由应用自己掌握，日志才能准确归因。
/// Client 按「检测到的代理地址」缓存复用：代理没变就命中连接池，省掉每次
/// TCP→CONNECT→TLS 全套握手；代理开关/换端口时键变化，自动重建，仍反映当次走向。
/// 缓存条目：键 = 检测到的代理地址，值 = 按它构建的 Client。
type CachedHttpclient = (Option<String>, reqwest::Client);

pub(super) async fn http_client() -> AppResult<(reqwest::Client, Option<String>)> {
    let network = Network::current()
        .await
        .map_err(|error| app_err!("{error}"))?;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CachedHttpclient>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut cached = cache
        .lock()
        .map_err(|_| app_err!("HTTP 客户端缓存锁已损坏"))?;
    let proxy = network.proxy.clone();
    if let Some((key, client)) = cached.as_ref() {
        if *key == proxy {
            return Ok((client.clone(), network.display));
        }
    }
    let builder = network.builder().timeout(std::time::Duration::from_secs(8));
    let client = builder
        .build()
        .map_err(|error| app_err!("创建 HTTP 客户端失败: {error}"))?;
    *cached = Some((proxy.clone(), client.clone()));
    Ok((client, network.display))
}

/// reqwest 错误转可读提示。
fn reqwest_error_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "请求超时".to_string()
    } else if error.is_connect() {
        "连接失败".to_string()
    } else {
        error.to_string()
    }
}

pub(super) fn provider_request_error_message(error: &reqwest::Error) -> &'static str {
    if error.is_builder() {
        "invalidEndpoint"
    } else if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "unreachable"
    } else {
        "requestFailed"
    }
}

pub(super) fn provider_response_error(body: &str, key: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            connection_error_from_body(&value).or_else(|| {
                value
                    .get("message")
                    .or_else(|| value.get("msg"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
        });
    super::model_fetch::truncate_body(super::model_fetch::redact(&detail.unwrap_or_default(), key))
}

/// OpenCode Go 的 /models 不校验密钥；空请求只证明原生端点响应，不触发推理。
async fn test_opencode_connection(
    base_url: &str,
    key: &str,
    context: &str,
) -> AppResult<CodexProfileConnectionResult> {
    let (client, proxy) = http_client().await?;
    let request = client
        .post(format!("{}/responses", base_url.trim_end_matches('/')))
        .bearer_auth(key);
    let start = std::time::Instant::now();
    let response = match request
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Ok(CodexProfileConnectionResult {
                ok: false,
                latency_ms: None,
                status: error.status().map(|status| status.as_u16()),
                error: Some(provider_request_error_message(&error).to_owned()),
            })
        }
    };
    let status = response.status();
    let body = match response.text().await {
        Ok(body) => body,
        Err(error) => {
            return Ok(CodexProfileConnectionResult {
                ok: false,
                latency_ms: Some(start.elapsed().as_millis()),
                status: None,
                error: Some(provider_request_error_message(&error).to_owned()),
            })
        }
    };
    let latency_ms = start.elapsed().as_millis();
    let text = body.to_lowercase();
    let missing_parameters = matches!(status.as_u16(), 400 | 422)
        && ["model", "messages", "input", "body", "json"]
            .iter()
            .any(|field| text.contains(field))
        && [
            "required", "missing", "empty", "expect", "缺少", "必填", "为空",
        ]
        .iter()
        .any(|word| text.contains(word));
    let ok = status == reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
        || missing_parameters
        || (status.is_success()
            && serde_json::from_str(&body)
                .ok()
                .and_then(|value| connection_error_from_body(&value))
                .is_none());
    if ok {
        log_provider_connect_success(context, status, latency_ms, &proxy);
    }
    Ok(CodexProfileConnectionResult {
        ok,
        latency_ms: Some(latency_ms),
        status: Some(status.as_u16()),
        error: (!ok).then(|| provider_response_error(&body, key)),
    })
}

fn subscription_http_error_message(status: reqwest::StatusCode, body: &str) -> &'static str {
    if body.contains("unsupported_country_region_territory") {
        "ChatGPT 访问受到地区限制，请开启系统代理后重试"
    } else {
        match status {
            reqwest::StatusCode::UNAUTHORIZED => "ChatGPT 登录已失效，请重新登录",
            reqwest::StatusCode::FORBIDDEN => "ChatGPT 订阅请求被拒绝，请重新登录后重试",
            reqwest::StatusCode::TOO_MANY_REQUESTS => "请求过于频繁，请稍后重试",
            status if status.is_server_error() => "ChatGPT 订阅服务暂时不可用，请稍后重试",
            _ => "ChatGPT 订阅请求未完成，请稍后重试",
        }
    }
}

fn subscription_request_error_message(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "连接 ChatGPT 订阅服务超时，请检查系统代理设置后重试"
    } else if error.is_connect() {
        "无法访问 ChatGPT 订阅服务，请检查系统代理设置后重试"
    } else {
        "ChatGPT 订阅请求未完成，请稍后重试"
    }
}

/// 从 2xx 的 JSON 响应体里识别供应商级错误（OpenAI 风格 `error` 或智谱风格 `code/success`）。
pub(crate) fn connection_error_from_body(value: &serde_json::Value) -> Option<String> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| error.as_str());
        return Some(message.unwrap_or("requestFailed").to_string());
    }
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        let message = value
            .get("msg")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.get("message").and_then(serde_json::Value::as_str));
        return Some(message.unwrap_or("requestFailed").to_string());
    }
    if let Some(code) = value.get("code") {
        let is_error_code = match code {
            serde_json::Value::Number(number) => number.as_i64().is_some_and(|n| n >= 400),
            serde_json::Value::String(text) => text.parse::<i64>().is_ok_and(|n| n >= 400),
            _ => false,
        };
        if is_error_code {
            let message = value
                .get("msg")
                .and_then(serde_json::Value::as_str)
                .or_else(|| value.get("message").and_then(serde_json::Value::as_str));
            return Some(message.unwrap_or("requestFailed").to_string());
        }
    }
    None
}

fn log_provider_connect_success(
    context: &str,
    status: reqwest::StatusCode,
    latency_ms: u128,
    proxy: &Option<String>,
) {
    tauri_plugin_log::log::info!(
        "[provider.connect.test] {context} outcome=success status_code={} latency_ms={latency_ms} proxy={} msg=\"测试连通成功\"",
        status.as_u16(),
        proxy.as_deref().unwrap_or("-")
    );
}

fn log_provider_connect_failure(context: &str, result: &CodexProfileConnectionResult) {
    let failure_kind = match result.status {
        Some(401 | 403) => "auth_error",
        Some(_) => "http_error",
        None => "network_error",
    };
    let status = result
        .status
        .map(|status| format!(" status_code={status}"))
        .unwrap_or_default();
    let proxy = match Network::detect() {
        Ok(network) => proxy_note(&network.proxy),
        Err(_) => " proxy=invalid".into(),
    };
    let error = result.error.as_deref().unwrap_or("未知错误");
    tauri_plugin_log::log::warn!(
        "[provider.connect.test] {context} outcome=failure failure_kind={failure_kind}{status}{proxy} error={error:?} msg=\"测试连通失败\""
    );
}

/// 余额/用量请求公共骨架：统一处理鉴权、401/403、错误提取与网络错误；
/// 各家只提供 URL、鉴权方式和成功响应的解析。
async fn query_balance_endpoint(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    start: std::time::Instant,
    label: &str,
    auth: BalanceAuth,
    parse: impl FnOnce(String, Option<u128>) -> AppResult<ProfileBalance>,
) -> AppResult<ProfileBalance> {
    let request = client.get(url);
    let response = match auth {
        BalanceAuth::Bearer => request.bearer_auth(api_key).send().await,
        BalanceAuth::Raw => {
            request
                .header("Authorization", api_key)
                .header("Accept-Language", "en-US,en")
                .header("Content-Type", "application/json")
                .send()
                .await
        }
    };
    match response {
        Ok(response) => {
            let status = response.status();
            let latency_ms = Some(start.elapsed().as_millis());
            if status.is_success() {
                let body = response
                    .text()
                    .await
                    .map_err(|error| app_err!("{label}接口响应读取失败: {error}"))?;
                return parse(body, latency_ms);
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(app_err!("API Key 无效或无权查询{label}（HTTP {status}）"));
            }
            let message = response
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| format!("接口返回 HTTP {status}"));
            Err(app_err!("{label}查询失败：{message}"))
        }
        Err(error) => {
            let error_message = reqwest_error_message(&error);
            Err(app_err!("{label}查询失败：{error_message}"))
        }
    }
}

/// DeepSeek 余额查询：GET {base}/user/balance。
async fn query_deepseek_balance(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    start: std::time::Instant,
) -> AppResult<ProfileBalance> {
    let url = format!("{}/user/balance", base.trim_end_matches('/'));
    query_balance_endpoint(
        client,
        &url,
        api_key,
        start,
        "余额",
        BalanceAuth::Bearer,
        |body, latency_ms| {
            let parsed = serde_json::from_str::<DeepSeekBalanceResponse>(&body)
                .map_err(|error| app_err!("余额接口响应解析失败: {error}"))?;
            Ok(ProfileBalance {
                is_available: parsed.is_available,
                balance_infos: preferred_deepseek_balance(parsed.balance_infos),
                latency_ms,
            })
        },
    )
    .await
}

/// MiniMax Coding Plan 用量查询：GET {base}/api/openplatform/coding_plan/remains。
/// 接口形态以用户实测可用的 statusline.ps1 为准（国内版 Coding Plan）。
/// 供应商连通性测试核心；创建态表单与已保存配置复用。
async fn test_models_endpoint(
    base_url: &str,
    api_key: &str,
    context: &str,
) -> AppResult<CodexProfileConnectionResult> {
    if base_url
        .trim_end_matches('/')
        .eq_ignore_ascii_case("https://opencode.ai/zen/go/v1")
    {
        return test_opencode_connection(base_url, api_key, context).await;
    }

    let models_url = format!("{}/models", base_url.trim_end_matches('/'));
    let (client, proxy) = http_client().await?;

    let start = std::time::Instant::now();
    match client.get(&models_url).bearer_auth(api_key).send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let latency_ms = start.elapsed().as_millis();
            let error = if status.is_success() {
                // 部分服务端（如智谱 /api/v1/models）用 HTTP 200 包装认证失败，
                // 只认状态码会把“密钥错误/地址错误”误判成连通成功，必须校验响应体。
                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(json) => connection_error_from_body(&json).map(|detail| {
                        super::model_fetch::truncate_body(super::model_fetch::redact(
                            &detail, api_key,
                        ))
                    }),
                    Err(_) => Some("requestFailed".to_string()),
                }
            } else {
                Some(provider_response_error(&body, api_key))
            };
            if error.is_none() {
                log_provider_connect_success(context, status, latency_ms, &proxy);
            }
            Ok(CodexProfileConnectionResult {
                ok: error.is_none(),
                latency_ms: Some(latency_ms),
                status: Some(status.as_u16()),
                error,
            })
        }
        Err(error) => {
            let status = error.status().map(|status| status.as_u16());
            Ok(CodexProfileConnectionResult {
                ok: false,
                latency_ms: None,
                status,
                error: Some(provider_request_error_message(&error).to_string()),
            })
        }
    }
}

/// 创建态表单的连通性测试：地址/密钥实时传入，无已存 profile 可回退，空值直接报错。
pub async fn test_provider_connection(
    base_url: &str,
    api_key: &str,
) -> AppResult<CodexProfileConnectionResult> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(app_err!("请填写 API 端点"));
    }
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(app_err!("请填写 API Key"));
    }
    let result = test_models_endpoint(base_url, api_key, "source=form").await?;
    if !result.ok {
        log_provider_connect_failure("source=form", &result);
    }
    Ok(result)
}

async fn query_minimax_balance(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    start: std::time::Instant,
) -> AppResult<ProfileBalance> {
    let url = format!(
        "{}/api/openplatform/coding_plan/remains",
        base.trim_end_matches('/')
    );
    query_balance_endpoint(
        client,
        &url,
        api_key,
        start,
        "用量",
        BalanceAuth::Bearer,
        |body, latency_ms| {
            let parsed = serde_json::from_str::<MiniMaxRemainsResponse>(&body)
                .map_err(|error| app_err!("用量接口响应解析失败: {error}"))?;
            let code = parsed.base_resp.status_code.unwrap_or(-1);
            if code != 0 {
                let message = parsed.base_resp.status_msg.unwrap_or_default();
                return Err(app_err!("用量查询失败：{message}"));
            }
            let entry = parsed
                .model_remains
                .iter()
                .find(|item| item.model_name == "general")
                .or_else(|| parsed.model_remains.first())
                .ok_or_else(|| app_err!("用量查询失败：接口未返回用量数据"))?;
            let usage_percent = used_percent(entry.current_interval_remaining_percent)
                .ok_or_else(|| app_err!("用量查询失败：接口未返回用量数据"))?;
            Ok(ProfileBalance {
                is_available: true,
                balance_infos: vec![ProfileBalanceInfo {
                    currency: String::new(),
                    total_balance: String::new(),
                    usage_percent: Some(usage_percent),
                    usage_reset: entry.remains_time.and_then(|ms| format_reset(ms, false)),
                    usage_reset_at: None,
                    usage_label: None,
                    weekly_usage_percent: used_percent(entry.current_weekly_remaining_percent),
                    weekly_reset: entry
                        .weekly_remains_time
                        .and_then(|ms| format_reset(ms, true)),
                    weekly_reset_at: None,
                    weekly_label: None,
                    reset_credits_available: None,
                    reset_credits: None,
                }],
                latency_ms,
            })
        },
    )
    .await
}

/// 余额/用量查询的厂商默认端点：配置里 base_url 缺失或留空时回退到这里。
/// 两处调用方都已把 provider 限定在 deepseek/minimax（/ZAI/zhipu）集合内，其余值不可达。
fn provider_default_balance_base(provider: &str) -> &'static str {
    match provider {
        "deepseek" => "https://api.deepseek.com",
        "minimax" => "https://api.minimaxi.com/v1",
        "ZAI" => "https://open.bigmodel.cn/api/v1",
        // Claude 侧的智谱 kind；地址与 presets.ts 的 claude 预设一致
        "zhipu" => "https://open.bigmodel.cn/api/anthropic",
        _ => unreachable!("调用方已限定 provider 集合"),
    }
}

async fn query_supported_provider_balance(
    provider: &str,
    base: &str,
    api_key: &str,
) -> AppResult<ProfileBalance> {
    let (client, _proxy) = http_client().await?;
    let start = std::time::Instant::now();
    match provider {
        "deepseek" => query_deepseek_balance(&client, base, api_key, start).await,
        "minimax" => query_minimax_balance(&client, base, api_key, start).await,
        "ZAI" | "zhipu" => query_zhipu_usage(&client, base, api_key, start).await,
        _ => Err(app_err!("该供应商不支持余额/用量查询")),
    }
}

fn claude_balance_base(kind: &str, base_url: Option<&str>) -> String {
    let base = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .trim_end_matches('/');
    if base.is_empty() {
        return String::new();
    }
    let base = base.strip_suffix("/anthropic").unwrap_or(base);
    match kind {
        "deepseek" | "zhipu" => base.to_string(),
        "minimax" => {
            let base = if base.ends_with("/v1") {
                base.to_string()
            } else {
                format!("{base}/v1")
            };
            base.replacen("https://api.minimax.cn", "https://api.minimaxi.com", 1)
        }
        _ => String::new(),
    }
}

fn zhipu_number(value: &serde_json::Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .or_else(|| value.get(key)?.as_str()?.parse().ok())
}

fn zhipu_integer(value: &serde_json::Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .or_else(|| value.get(key)?.as_str()?.parse().ok())
}

/// 智谱 GLM Coding Plan 配额响应：TOKENS_LIMIT 包含 5 小时和 7 天窗口。
pub(crate) fn zhipu_quota_info(
    value: &serde_json::Value,
    now_ms: i64,
) -> AppResult<ProfileBalanceInfo> {
    let limits = value
        .get("data")
        .and_then(|data| data.get("limits"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| app_err!("用量查询失败：接口未返回限额数据"))?;
    let windows = limits
        .iter()
        .filter(|limit| {
            limit.get("type").and_then(serde_json::Value::as_str) == Some("TOKENS_LIMIT")
        })
        .filter_map(|limit| {
            let used_percent = zhipu_number(limit, "percentage")?.clamp(0.0, 100.0).round() as u32;
            let reset_at = zhipu_integer(limit, "nextResetTime").filter(|value| *value > 0);
            let remaining_ms = reset_at.map(|value| value.saturating_sub(now_ms));
            let unit = zhipu_integer(limit, "unit");
            Some(ZhipuQuotaWindow {
                used_percent,
                reset: remaining_ms
                    .and_then(|ms| format_reset(ms, unit == Some(6) || ms > 86_400_000)),
                reset_at,
                unit,
            })
        })
        .collect::<Vec<_>>();
    let primary = windows
        .iter()
        .find(|window| window.unit == Some(3))
        .ok_or_else(|| app_err!("用量查询失败：接口未返回 5 小时窗口"))?;
    let weekly = windows.iter().find(|window| window.unit == Some(6));

    Ok(ProfileBalanceInfo {
        currency: String::new(),
        total_balance: String::new(),
        usage_percent: Some(primary.used_percent),
        usage_reset: primary.reset.clone(),
        usage_reset_at: primary.reset_at,
        usage_label: Some("5小时".to_string()),
        weekly_usage_percent: weekly.map(|window| window.used_percent),
        weekly_reset: weekly.and_then(|window| window.reset.clone()),
        weekly_reset_at: weekly.and_then(|window| window.reset_at),
        weekly_label: weekly.map(|_| "7天".to_string()),
        reset_credits_available: None,
        reset_credits: None,
    })
}

fn provider_origin(base: &str) -> AppResult<String> {
    let invalid = || app_err!("用量查询失败：供应商 API 端点无效");
    let base = base.trim();
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(invalid());
    }
    let url = url::Url::parse(base).map_err(|_| invalid())?;
    // 保留端点认证信息，只剥离路径、查询和片段。
    Ok(url[..url::Position::BeforePath].to_string())
}

/// 智谱 GLM Coding Plan 用量查询：GET {origin}/api/monitor/usage/quota/limit。
/// 鉴权值直接使用供应商配置中的 experimental_bearer_token，保持官方插件的请求格式。
async fn query_zhipu_usage(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    start: std::time::Instant,
) -> AppResult<ProfileBalance> {
    let url = format!("{}/api/monitor/usage/quota/limit", provider_origin(base)?);
    query_balance_endpoint(
        client,
        &url,
        api_key,
        start,
        "用量",
        BalanceAuth::Raw,
        |body, latency_ms| {
            let value = serde_json::from_str::<serde_json::Value>(&body)
                .map_err(|error| app_err!("用量接口响应解析失败: {error}"))?;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            let info = zhipu_quota_info(&value, now_ms)?;
            Ok(ProfileBalance {
                is_available: true,
                balance_infos: vec![info],
                latency_ms,
            })
        },
    )
    .await
}

/// 接口给的是“剩余”百分比，卡片显示“用量”= 100 - 剩余。
pub(crate) fn used_percent(remaining: Option<f64>) -> Option<u32> {
    let remaining = remaining?;
    let used = 100.0 - remaining;
    Some(used.clamp(0.0, 100.0).round() as u32)
}

/// 重置倒计时格式：按窗口长度决定是否显示天数；不足 1 分钟不显示。
/// 分钟向上取整：ChatGPT 未使用窗口的重置点会随查询滑动（窗口尚未开始），
/// 截断会把“剩 4h59m50s”显示成 4h59m，看起来像窗口已走掉 1 分钟；
/// 官方 Plan limits 页同样向上取整（显示 5h 0m）。
pub(crate) fn format_reset(ms: i64, with_days: bool) -> Option<String> {
    if ms <= 60_000 {
        return None;
    }
    let ms = (ms + 59_999) / 60_000 * 60_000;
    let days = if with_days { ms / 86_400_000 } else { 0 };
    let hours = (ms % 86_400_000) / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    Some(if days > 0 {
        if hours > 0 {
            format!("{days}d{hours}h")
        } else {
            format!("{days}d")
        }
    } else if hours > 0 {
        if minutes > 0 {
            format!("{hours}h{minutes}m")
        } else {
            format!("{hours}h")
        }
    } else {
        format!("{minutes}m")
    })
}

fn chatgpt_window_label(seconds: Option<i64>, fallback: &str) -> String {
    match seconds {
        Some(18_000) => "5小时".to_string(),
        Some(604_800) => "7天".to_string(),
        Some(2_592_000) => "30天".to_string(),
        Some(value) if value > 0 && value % 86_400 == 0 => format!("{}天", value / 86_400),
        Some(value) if value > 0 && value % 3_600 == 0 => format!("{}小时", value / 3_600),
        _ => fallback.to_string(),
    }
}

fn chatgpt_reset_countdown(reset_at: Option<i64>, window_seconds: Option<i64>) -> Option<String> {
    let reset_ms = chatgpt_reset_timestamp(reset_at)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    format_reset(
        reset_ms.saturating_sub(now_ms),
        window_seconds.unwrap_or_default() >= 86_400,
    )
}

fn chatgpt_reset_timestamp(reset_at: Option<i64>) -> Option<i64> {
    let reset_at = reset_at?;
    Some(if reset_at > 1_000_000_000_000 {
        reset_at
    } else {
        reset_at.saturating_mul(1_000)
    })
}

pub(crate) fn chatgpt_quota_info(response: ChatgptUsageResponse) -> Option<ProfileBalanceInfo> {
    let reset_credits_available = response
        .rate_limit_reset_credits
        .and_then(|credits| credits.available_count);
    let rate_limit = response.rate_limit?;
    let windows = [rate_limit.primary_window, rate_limit.secondary_window];
    let mut usable_windows = windows
        .iter()
        .flatten()
        .filter(|window| window.used_percent.is_some());
    let primary = usable_windows.next()?;
    let secondary = usable_windows.next();
    let usage_percent = primary.used_percent?.clamp(0.0, 100.0).round() as u32;
    Some(ProfileBalanceInfo {
        currency: String::new(),
        total_balance: String::new(),
        usage_percent: Some(usage_percent),
        usage_reset: chatgpt_reset_countdown(primary.reset_at, primary.limit_window_seconds),
        usage_reset_at: chatgpt_reset_timestamp(primary.reset_at),
        usage_label: primary
            .limit_window_seconds
            .map(|seconds| chatgpt_window_label(Some(seconds), "额度")),
        weekly_usage_percent: secondary
            .as_ref()
            .and_then(|window| window.used_percent)
            .map(|used| used.clamp(0.0, 100.0).round() as u32),
        weekly_reset: secondary.as_ref().and_then(|window| {
            chatgpt_reset_countdown(window.reset_at, window.limit_window_seconds)
        }),
        weekly_reset_at: secondary
            .as_ref()
            .and_then(|window| chatgpt_reset_timestamp(window.reset_at)),
        weekly_label: secondary.as_ref().and_then(|window| {
            window
                .limit_window_seconds
                .map(|seconds| chatgpt_window_label(Some(seconds), "周期"))
        }),
        reset_credits_available,
        reset_credits: None,
    })
}

fn chatgpt_request(
    request: reqwest::RequestBuilder,
    access_token: &str,
    account_id: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = request
        .bearer_auth(access_token)
        .header("User-Agent", "codex-cli")
        .header("Accept", "application/json");
    if let Some(account_id) = account_id.filter(|id| *id != "codex-external") {
        request = request.header("chatgpt-account-id", account_id);
    }
    request
}

fn warmup_event_tokens(data: &[u8]) -> AppResult<Option<u64>> {
    if data.is_empty() {
        return Ok(None);
    }
    let event: serde_json::Value =
        serde_json::from_slice(data).map_err(|_| app_err!("预热响应格式无效，请重试"))?;
    match event["type"].as_str() {
        Some("response.completed") => event["response"]["usage"]["total_tokens"]
            .as_u64()
            .filter(|tokens| *tokens > 0)
            .map(Some)
            .ok_or_else(|| app_err!("预热响应未返回有效的 Token 用量")),
        Some("response.failed" | "response.incomplete" | "error") => {
            Err(app_err!("预热请求未完成，请检查账号额度或模型可用性后重试"))
        }
        _ => Ok(None),
    }
}

async fn send_chatgpt_warmup(
    request: reqwest::RequestBuilder,
) -> Result<u64, (&'static str, String)> {
    let network_error = |error: reqwest::Error| {
        (
            if error.is_timeout() {
                "timeout"
            } else {
                "network_error"
            },
            subscription_request_error_message(&error).to_string(),
        )
    };
    // 对齐 openai/codex 的 ResponsesApiRequest；不传订阅端未声明的 max_output_tokens。
    let mut response = request
        .header("Accept", "text/event-stream")
        .header("originator", "codex_cli_rs")
        .timeout(std::time::Duration::from_secs(60))
        .json(&serde_json::json!({
            "model": "gpt-6-luna",
            "input": [{"type": "message", "role": "user", "content": [
                {"type": "input_text", "text": "Reply only OK."}
            ]}],
            "reasoning": {"effort": "low"},
            "tools": [],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "store": false,
            "stream": true,
            "include": []
        }))
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.map_err(network_error)?;
        return Err((
            "http_error",
            format!(
                "{}（HTTP {}）",
                subscription_http_error_message(status, &body),
                status.as_u16()
            ),
        ));
    }
    let mut pending = Vec::new();
    let mut data = Vec::new();
    // 完成事件后立即返回：官方流可能仍保持连接；按行解析兼容分块与 CRLF。
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = &line[..line.len() - 1];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                if let Some(tokens) = warmup_event_tokens(&data)
                    .map_err(|error| ("protocol_error", error.to_string()))?
                {
                    return Ok(tokens);
                }
                data.clear();
            } else if let Some(value) = line.strip_prefix(b"data:") {
                data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
                data.push(b'\n');
            }
        }
    }
    Err((
        "protocol_error",
        "预热连接已结束，但未收到完成事件，请刷新用量后重试".to_string(),
    ))
}

pub(crate) fn chatgpt_reset_credit_expiry(value: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|time| time.timestamp_millis())
}

async fn query_chatgpt_reset_credits(
    client: &reqwest::Client,
    access_token: &str,
    account_id: Option<&str>,
) -> Option<(Option<i64>, Vec<ChatgptResetCredit>)> {
    let response = chatgpt_request(
        client.get("https://chatgpt.com/backend-api/wham/rate-limit-reset-credits"),
        access_token,
        account_id,
    )
    .send()
    .await
    .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let details = response.json::<ChatgptResetCreditsResponse>().await.ok()?;
    let credits = details
        .credits
        .into_iter()
        .filter(|credit| credit.status.as_deref() == Some("available"))
        .map(|credit| ChatgptResetCredit {
            id: credit.id,
            reset_type: credit.reset_type,
            expires_at: chatgpt_reset_credit_expiry(credit.expires_at.as_deref()),
        })
        .collect();
    Some((details.available_count, credits))
}

fn quota_failure_error(
    status: reqwest::StatusCode,
    body: &str,
    context: &str,
) -> crate::error::AppError {
    let auth_status =
        status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN;
    if auth_status && body.contains("unsupported_country_region_territory") {
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} outcome=failure failure_kind=region_blocked status_code={} msg=\"用量查询因地区限制被拦截\"",
            status.as_u16()
        );
        return app_err!("认证请求被地区限制拦截，请开启系统代理后重试");
    }
    if auth_status {
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} outcome=failure failure_kind=auth_error status_code={} msg=\"ChatGPT 登录凭证已失效，需重新登录\"",
            status.as_u16()
        );
        return app_err!("ChatGPT 登录已失效，请重新登录");
    }
    tauri_plugin_log::log::warn!(
        "[chatgpt.quota.query] client=\"Codex\" {context} outcome=failure failure_kind=http_error status_code={} msg=\"用量查询失败\"",
        status.as_u16()
    );
    app_err!("用量查询失败：接口返回 HTTP {status}")
}

async fn query_chatgpt_quota(
    access_token: &str,
    account_id: Option<&str>,
    context: &str,
) -> AppResult<ProfileBalance> {
    let (client, proxy) = http_client().await.map_err(|error| {
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} outcome=failure failure_kind=internal error={error} msg=\"用量查询客户端初始化失败\""
        );
        error
    })?;
    let start = std::time::Instant::now();
    let response = chatgpt_request(
        client.get("https://chatgpt.com/backend-api/wham/usage"),
        access_token,
        account_id,
    )
    .send()
    .await
    .map_err(|error| {
        // ChatGPT 订阅链路是唯一没有标准错误码可依赖的路径，失败必须留痕；
        // 超时落 timeout 枚举，其余网络故障才是 network_error
        let kind = if error.is_timeout() { "timeout" } else { "network_error" };
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} stage=request outcome=failure failure_kind={kind} connect={} latency_ms={} proxy={} error={:?} msg=\"用量查询网络错误\"",
            error.is_connect(),
            start.elapsed().as_millis(),
            proxy.as_deref().unwrap_or("-"),
            reqwest_error_message(&error),
        );
        app_err!("用量查询失败：{}", reqwest_error_message(&error))
    })?;
    let latency = start.elapsed().as_millis();
    let latency_ms = Some(latency);
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} stage=response_body outcome=failure failure_kind=io_error status_code={} latency_ms={} error={:?} msg=\"用量接口响应读取失败\"",
            status.as_u16(),
            start.elapsed().as_millis(),
            reqwest_error_message(&error)
        );
        app_err!("用量接口响应读取失败: {error}")
    })?;
    if !status.is_success() {
        return Err(quota_failure_error(status, &body, context));
    }
    let response = serde_json::from_str::<ChatgptUsageResponse>(&body).map_err(|error| {
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} outcome=failure failure_kind=parse_error status_code={} error={error:?} msg=\"用量接口响应解析失败\"",
            status.as_u16()
        );
        app_err!("用量接口响应解析失败: {error}")
    })?;
    let mut info = chatgpt_quota_info(response).ok_or_else(|| {
        tauri_plugin_log::log::warn!(
            "[chatgpt.quota.query] client=\"Codex\" {context} outcome=failure failure_kind=parse_error status_code={} msg=\"用量接口未返回可用的限额窗口\"",
            status.as_u16()
        );
        app_err!("用量接口未返回可用的限额窗口")
    })?;
    if let Some((available_count, credits)) =
        query_chatgpt_reset_credits(&client, access_token, account_id).await
    {
        info.reset_credits_available = available_count.or(info.reset_credits_available);
        info.reset_credits = Some(credits);
    }
    // 成功也留痕：用量数字不对/没刷新时，靠这行确认最后一次成功查询的时间
    tauri_plugin_log::log::debug!(
        "[chatgpt.quota.query] client=\"Codex\" {context} outcome=success latency_ms={latency} proxy={} msg=\"用量查询成功\"",
        proxy.as_deref().unwrap_or("-")
    );
    Ok(ProfileBalance {
        is_available: true,
        balance_infos: vec![info],
        latency_ms,
    })
}

impl AppContext {
    /// 验证供应商密钥连通性：默认请求 OpenAI 兼容的 GET {base}/models，OpenCode Go 使用鉴权探针，
    /// 2xx 视为可用，401/403 视为密钥无效，返回延迟 / HTTP 状态 / 错误信息。
    /// 表单传入的地址/密钥实时生效（传了就用传的，空的直接报错）；
    /// 不传才回退已保存值（卡片上的测试按钮走这条）。
    pub async fn codex_test_profile_connection(
        &self,
        id: &str,
        base_url_override: Option<&str>,
        api_key_override: Option<&str>,
    ) -> AppResult<CodexProfileConnectionResult> {
        let stored = self.database.codex_profile(id)?;
        let payload = &stored.payload;
        if payload.provider_id.is_none() {
            return Err(app_err!("该供应商缺少配置，无法测试连通性"));
        }
        let body = payload
            .provider_body
            .as_deref()
            .ok_or_else(|| app_err!("该供应商缺少配置数据"))?;
        let detail = parse_provider_detail(body)?;
        let base_url = match base_url_override {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(app_err!("请填写 API 端点"));
                }
                value.to_string()
            }
            None => detail
                .base_url
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| app_err!("该供应商没有配置 API 端点"))?,
        };
        let api_key = match api_key_override {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(app_err!("请填写 API Key"));
                }
                value.to_string()
            }
            None => stored_provider_api_key(payload)
                .ok_or_else(|| app_err!("该供应商没有配置 API Key，请先填写后再测试"))?,
        };

        let context = format!(
            "profile_id={} profile_name={:?} source=profile",
            stored.id, stored.name
        );
        let result = test_models_endpoint(&base_url, &api_key, &context).await?;
        if !result.ok {
            log_provider_connect_failure(&context, &result);
        }
        Ok(result)
    }

    /// 只读数据库缓存；只有显式刷新才请求网络。
    pub async fn codex_fetch_chatgpt_models(
        &self,
        id: Option<&str>,
        source: AuthSource,
        account_id: Option<&str>,
        oauth: &CodexOAuthManager,
        refresh: bool,
    ) -> AppResult<Vec<ChatgptModel>> {
        let stored = id.map(|id| self.database.codex_profile(id)).transpose()?;
        if stored
            .as_ref()
            .is_some_and(|profile| profile.kind != CodexProfileKind::Official)
        {
            return Err(app_err!("该配置不是 ChatGPT 订阅配置"));
        }
        let source = match &stored {
            Some(profile) => profile
                .payload
                .effective_auth_source(profile.kind, profile.account_id.as_deref())
                .ok_or_else(|| app_err!("官方配置缺少登录方式"))?,
            None => source,
        };
        let account_id = match &stored {
            Some(profile) => profile.account_id.as_deref(),
            None => account_id,
        };
        let desktop = if source == AuthSource::Desktop {
            stored.as_ref().and_then(|profile| {
                profile
                    .payload
                    .raw_auth
                    .as_deref()
                    .and_then(parse_external_auth_json)
            })
        } else {
            None
        };
        let account_id = desktop
            .as_ref()
            .map(|auth| auth.account_id.as_str())
            .or(account_id);
        let live = if source == AuthSource::Desktop && account_id.is_none() {
            self.read_external_codex_auth()
        } else {
            None
        };
        let account_id = account_id.or_else(|| live.as_ref().map(|auth| auth.account_id.as_str()));
        let cache_key = chatgpt_model_cache_key(source, account_id);
        if !refresh {
            return Ok(self
                .database
                .chatgpt_models(&cache_key)?
                .unwrap_or_default());
        }
        if source == AuthSource::Desktop && stored.is_some() && desktop.is_none() {
            return Err(app_err!("该 Codex 配置尚未保存有效登录"));
        }
        let credentials = match desktop {
            Some(auth) => (auth.access_token, Some(auth.account_id), String::new()),
            None => {
                self.auth_account_credentials(source, account_id, oauth)
                    .await?
            }
        };
        let (token, workspace, _) = credentials;
        let network = Network::current()
            .await
            .map_err(|error| app_err!("{error}"))?;
        let client = network
            .builder()
            .timeout(std::time::Duration::from_secs(
                super::model_fetch::FETCH_TIMEOUT_SECS,
            ))
            .build()
            .map_err(|_| app_err!("无法创建模型目录请求"))?;
        // 使用 Codex 实际缓存记录的客户端版本，不把 Agmux 版本冒充 Codex 版本。
        let version = std::fs::read_to_string(self.paths.codex_home.join("models_cache.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| value["client_version"].as_str().map(str::to_string));
        let version = match version {
            Some(version) => version,
            None => super::codex_cli::model_client_version(
                self.paths
                    .codex_home
                    .parent()
                    .ok_or_else(|| app_err!("无法定位用户目录"))?,
                &self.paths.codex_home,
            )
            .map_err(|_| app_err!("无法读取 Codex 客户端版本，请先安装或打开 Codex"))?,
        };
        let mut url = reqwest::Url::parse("https://chatgpt.com/backend-api/codex/models")
            .expect("static Codex catalog URL");
        url.query_pairs_mut()
            .append_pair("client_version", &version);
        let request = chatgpt_request(client.get(url), &token, workspace.as_deref());
        let response = request
            .send()
            .await
            .map_err(|error| app_err!("{}", subscription_request_error_message(&error)))?;
        if !response.status().is_success() {
            return Err(app_err!(
                "HTTP {}: requestFailed",
                response.status().as_u16()
            ));
        }
        let value = response
            .json()
            .await
            .map_err(|_| app_err!("ChatGPT 模型目录格式无效"))?;
        let models = parse_chatgpt_models(value)?;
        self.database.set_chatgpt_models(&cache_key, &models)?;
        tauri_plugin_log::log::info!(
            "[chatgpt.models] model_count={} outcome=success msg=\"ChatGPT 账号模型缓存已更新\"",
            models.len()
        );
        Ok(models)
    }

    /// 验证 ChatGPT 订阅认证连通性（Codex CLI 后台轮询同一个用量端点）。
    /// 2xx 可用；非 2xx 和网络错误按订阅链路分类提示。
    /// 仅手动点击测试时调用，不参与切换流程。
    pub async fn test_subscription_connection(
        &self,
        access_token: &str,
        context: &str,
    ) -> AppResult<CodexProfileConnectionResult> {
        let (client, proxy) = http_client().await.map_err(|error| {
            tauri_plugin_log::log::warn!(
                "[chatgpt.connect.test] {context} outcome=failure failure_kind=internal error={error} msg=\"测试连通客户端初始化失败\""
            );
            error
        })?;
        let start = std::time::Instant::now();
        match chatgpt_request(
            client.get("https://chatgpt.com/backend-api/wham/usage"),
            access_token,
            None,
        )
        .send()
        .await
        {
            Ok(response) => {
                let status = response.status();
                let text = match response.text().await {
                    Ok(text) => text,
                    Err(error) => {
                        tauri_plugin_log::log::warn!(
                            "[chatgpt.connect.test] {context} outcome=failure failure_kind=io_error status_code={} error={error:?} msg=\"测试连通响应读取失败\"",
                            status.as_u16(),
                        );
                        String::new()
                    }
                };
                let latency_ms = start.elapsed().as_millis();
                if status.is_success() {
                    // 成功也留痕：延迟数据只存在于弹窗，事后无从追溯
                    tauri_plugin_log::log::info!(
                        "[chatgpt.connect.test] {context} outcome=success status_code={} latency_ms={latency_ms} proxy={} msg=\"测试连通成功\"",
                        status.as_u16(),
                        proxy.as_deref().unwrap_or("None")
                    );
                    Ok(CodexProfileConnectionResult {
                        ok: true,
                        latency_ms: Some(latency_ms),
                        status: Some(status.as_u16()),
                        error: None,
                    })
                } else {
                    let diagnostic_body = if matches!(status.as_u16(), 401 | 403) {
                        &text
                    } else {
                        ""
                    };
                    let message = subscription_http_error_message(status, diagnostic_body);
                    // 测试连通的失败只以返回值形式存在（不是 Err），弹窗错过就无迹可寻
                    tauri_plugin_log::log::warn!(
                        "[chatgpt.connect.test] {context} outcome=failure failure_kind=http_error status_code={} proxy={} error={message:?} msg=\"测试连通失败\"",
                        status.as_u16(),
                        proxy.as_deref().unwrap_or("None")
                    );
                    Ok(CodexProfileConnectionResult {
                        ok: false,
                        latency_ms: Some(latency_ms),
                        status: Some(status.as_u16()),
                        error: Some(message.to_string()),
                    })
                }
            }
            Err(error) => {
                let status = error.status().map(|status| status.as_u16());
                tauri_plugin_log::log::warn!(
                    "[chatgpt.connect.test] {context} outcome=failure failure_kind=network_error proxy={} error={:?} msg=\"测试连通网络错误\"",
                    proxy.as_deref().unwrap_or("None"),
                    subscription_request_error_message(&error)
                );
                Ok(CodexProfileConnectionResult {
                    ok: false,
                    latency_ms: None,
                    status,
                    error: Some(subscription_request_error_message(&error).to_string()),
                })
            }
        }
    }

    /// 用量查询与预热共用卡片身份，不能回退到其他账号的 live 认证。
    pub(super) async fn auth_account_credentials(
        &self,
        source: AuthSource,
        account_id: Option<&str>,
        oauth: &CodexOAuthManager,
    ) -> AppResult<(String, Option<String>, String)> {
        let source_label = match &source {
            AuthSource::Desktop => "desktop",
            AuthSource::Oauth => "oauth",
        };
        let subject;
        let (access_token, account_id) = match source {
            AuthSource::Desktop => {
                // Desktop 额度优先按请求身份取数据库认证快照（与账号页同源，
                // 不随切换覆写的 live auth.json 漂移）；无匹配快照时回退 live 认证
                //（跟随 Codex 登录但尚未建 Desktop 配置的场景）。
                let db_snapshot = match account_id {
                    Some(requested) => self.desktop_auth_snapshot_for_account(requested)?,
                    None => None,
                };
                match db_snapshot {
                    Some((token, email)) => {
                        subject = account_id.map(|id| account_subject(id, email.as_deref()));
                        (token, account_id.map(str::to_string))
                    }
                    None => {
                        let account = self
                            .read_external_codex_auth()
                            .filter(|account| account_id.is_none_or(|id| id == account.account_id))
                            .ok_or_else(|| app_err!("未检测到有效的 Codex 登录"))?;
                        let token = self
                            .external_codex_access_token_for_account(&account.account_id)?
                            .ok_or_else(|| app_err!("未检测到有效的 Codex 登录"))?;
                        subject = Some(account_subject(
                            &account.account_id,
                            account.email.as_deref(),
                        ));
                        (token, Some(account.account_id))
                    }
                }
            }
            AuthSource::Oauth => {
                let row_id = account_id.ok_or_else(|| app_err!("OAuth 账号不存在"))?;
                let token = oauth
                    .get_valid_token_for_account(row_id)
                    .await
                    .map_err(|error| app_err!("{error}"))?;
                // chatgpt-account-id 头必须是 workspace ID，本地行 id 不能出站
                subject = Some(oauth.account_subject_for(row_id).await);
                (token, Some(oauth.workspace_of(row_id).await))
            }
        };
        // 两种来源共用账号日志格式，邮箱仅开发版输出。
        let context = match &subject {
            Some(subject) => format!("{subject} source={source_label}"),
            None => match account_id.as_deref() {
                Some(id) => format!("account_id={id} source={source_label}"),
                None => format!("source={source_label}"),
            },
        };
        Ok((access_token, account_id, context))
    }

    /// 读取设置页中的 Codex 登录或指定 OAuth 账号的官方额度。
    pub async fn get_auth_quota(
        &self,
        source: AuthSource,
        account_id: Option<&str>,
        oauth: &CodexOAuthManager,
    ) -> AppResult<ProfileBalance> {
        let (access_token, account_id, context) = self
            .auth_account_credentials(source, account_id, oauth)
            .await?;
        query_chatgpt_quota(&access_token, account_id.as_deref(), &context).await
    }

    pub async fn warmup_auth_account(
        &self,
        source: AuthSource,
        account_id: &str,
        oauth: &CodexOAuthManager,
    ) -> AppResult<()> {
        let source_label = match source {
            AuthSource::Desktop => "desktop",
            AuthSource::Oauth => "oauth",
        };
        let result = async {
            let (token, workspace, _) = self
                .auth_account_credentials(source, Some(account_id), oauth)
                .await
                .map_err(|_| {
                    (
                        "auth_error",
                        "ChatGPT 账号认证不可用，请检查登录状态或网络后重试".to_string(),
                    )
                })?;
            let (client, _) = http_client()
                .await
                .map_err(|error| ("internal", error.to_string()))?;
            send_chatgpt_warmup(chatgpt_request(
                client.post("https://chatgpt.com/backend-api/codex/responses"),
                &token,
                workspace.as_deref(),
            ))
            .await
        }
        .await;
        match result {
            Ok(tokens) => {
                tauri_plugin_log::log::info!(
                    "[chatgpt.account.warmup] account_id={account_id:?} source={source_label} model=gpt-6-luna reasoning=low outcome=success total_tokens={tokens} msg=\"ChatGPT 账号预热请求已完成\""
                );
                Ok(())
            }
            Err((kind, message)) => {
                tauri_plugin_log::log::warn!(
                    "[chatgpt.account.warmup] account_id={account_id:?} source={source_label} model=gpt-6-luna reasoning=low outcome=failure failure_kind={kind} error={message:?} msg=\"ChatGPT 账号预热失败\""
                );
                Err(app_err!("{message}"))
            }
        }
    }

    /// 按配置查询余额/用量；ChatGPT 配置只读自身认证来源，不读取 live auth.json。
    pub async fn codex_get_profile_balance(
        &self,
        id: &str,
        oauth: &CodexOAuthManager,
    ) -> AppResult<ProfileBalance> {
        let stored = self.database.codex_profile(id)?;
        let payload = &stored.payload;
        if stored.kind == CodexProfileKind::Official {
            // 配置主键和认证来源足以定位；中文 msg 负责快速扫读。
            let auth_source =
                payload.effective_auth_source(stored.kind, stored.account_id.as_deref());
            let source_label = match auth_source {
                Some(AuthSource::Oauth) => "oauth",
                _ => "desktop",
            };
            let mut context = format!("profile_id={id} source={source_label}");
            let (access_token, account_id) =
                match payload.effective_auth_source(stored.kind, stored.account_id.as_deref()) {
                    Some(AuthSource::Desktop) => payload
                        .raw_auth
                        .as_deref()
                        .and_then(parse_external_auth_json)
                        .map(|auth| (auth.access_token, Some(auth.account_id)))
                        .ok_or_else(|| app_err!("该 Codex 配置尚未保存有效登录"))?,
                    Some(AuthSource::Oauth) => {
                        let account_id = stored
                            .account_id
                            .as_deref()
                            .ok_or_else(|| app_err!("OAuth 配置未绑定订阅账号"))?;
                        let token = oauth
                            .get_valid_token_for_account(account_id)
                            .await
                            .map_err(|error| app_err!("{error}"))?;
                        // chatgpt-account-id 头必须是 workspace ID，本地行 id 不能出站
                        context = format!(
                            "profile_id={id} {} source={source_label}",
                            oauth.account_subject_for(account_id).await
                        );
                        (token, Some(oauth.workspace_of(account_id).await))
                    }
                    None => return Err(app_err!("官方配置缺少登录方式")),
                };
            return query_chatgpt_quota(&access_token, account_id.as_deref(), &context).await;
        }
        let provider = payload.provider_id.as_deref().unwrap_or_default();
        if provider != "deepseek" && provider != "minimax" && provider != "ZAI" {
            return Err(app_err!("该供应商不支持余额/用量查询"));
        }
        let body = payload
            .provider_body
            .as_deref()
            .ok_or_else(|| app_err!("该供应商缺少配置数据"))?;
        let detail = parse_provider_detail(body)?;
        let api_key = stored_provider_api_key(payload)
            .ok_or_else(|| app_err!("该供应商没有配置 API Key，无法查询余额/用量"))?;
        let base = detail
            .base_url
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        let default_base = provider_default_balance_base(provider);
        query_supported_provider_balance(provider, base.unwrap_or(default_base), &api_key).await
    }

    pub async fn claude_get_profile_balance(&self, id: &str) -> AppResult<ProfileBalance> {
        let profile = self.database.claude_profile(id)?;
        let kind = profile.kind.as_deref().unwrap_or_default();
        if kind != "deepseek" && kind != "minimax" && kind != "zhipu" {
            return Err(app_err!("该 Claude 供应商不支持用量查询"));
        }
        let api_key = profile
            .auth_token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| app_err!("该供应商没有配置 API Key，无法查询余额/用量"))?;
        let base = claude_balance_base(kind, profile.base_url.as_deref());
        let default_base = provider_default_balance_base(kind);
        query_supported_provider_balance(
            kind,
            if base.is_empty() { default_base } else { &base },
            api_key,
        )
        .await
    }

    /// 供应商级余额缓存：上次成功查询结果写入 ~/.agmux/balance-cache.json，
    /// 保证卡片首次渲染/切换视图时数字就在，不出现“消失→出现”的闪烁。
    pub fn set_profile_balance(
        &self,
        profile_id: &str,
        info: &ProfileBalanceInfo,
    ) -> AppResult<()> {
        let mut cache = self.load_balance_cache();
        cache.insert(profile_id.to_string(), info.clone());
        self.save_balance_cache(&cache)
    }

    pub(super) fn balance_cache_path(&self) -> PathBuf {
        self.paths.root.join("balance-cache.json")
    }

    pub(super) fn load_balance_cache(&self) -> BTreeMap<String, ProfileBalanceInfo> {
        std::fs::read_to_string(self.balance_cache_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(super) fn save_balance_cache(
        &self,
        cache: &BTreeMap<String, ProfileBalanceInfo>,
    ) -> AppResult<()> {
        let text = serde_json::to_string(cache)
            .map_err(|error| app_err!("余额缓存序列化失败: {error}"))?;
        atomic_write(&self.balance_cache_path(), text.as_bytes())
    }
}
