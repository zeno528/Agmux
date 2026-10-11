//! 模型列表获取服务
//!
//! 通过 OpenAI 兼容的 GET /models 端点拉取供应商可用模型列表。
//! URL 候选策略参考 cc-switch 的 model_fetch.rs，按 Agofra 只做
//! Responses（原生）直连的场景做了简化：无 Anthropic/Google 协议头，
//! 无兼容子路径剥离。

use serde::Deserialize;

/// OpenAI 兼容的 /models 响应格式
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Option<Vec<ModelEntry>>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

pub(super) const FETCH_TIMEOUT_SECS: u64 = 15;
/// 404/405 响应体截断长度：避免把几十 KB 的 HTML 404 页整页保留到错误串里。
const ERROR_BODY_MAX_CHARS: usize = 512;

/// 获取供应商的可用模型 ID 列表（Bearer 认证，按候选 URL 顺序尝试）。
pub async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let candidates = build_models_url_candidates(base_url)?;
    if api_key.trim().is_empty() {
        return Err("请先填写 API Key 再获取模型列表".to_string());
    }

    let client = crate::network::Network::current()
        .await
        .map_err(|error| error.to_string())?
        .builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .map_err(|error| super::connections::provider_request_error_message(&error).to_string())?;

    let mut last_err: Option<String> = None;
    for url in &candidates {
        let response = match client.get(url).bearer_auth(api_key).send().await {
            Ok(response) => response,
            Err(error) => {
                return Err(super::connections::provider_request_error_message(&error).to_string())
            }
        };

        let status = response.status();
        if status.is_success() {
            let parsed: ModelsResponse = response
                .json()
                .await
                .map_err(|_| format!("HTTP {}: requestFailed", status.as_u16()))?;
            let mut models: Vec<String> = parsed
                .data
                .unwrap_or_default()
                .into_iter()
                .map(|entry| entry.id)
                .collect();
            models.sort();
            return Ok(models);
        }

        let body = super::connections::provider_response_error(
            &response.text().await.unwrap_or_default(),
            api_key,
        );
        // 404/405：路径猜错，试下一候选；其他错误（401/403/5xx）直接返回
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
        {
            last_err = Some(format!("HTTP {}: {body}", status.as_u16()));
            continue;
        }
        return Err(format!("HTTP {}: {body}", status.as_u16()));
    }

    Err(last_err.unwrap_or_else(|| "requestFailed".to_string()))
}

/// 已知供应商的模型列表端点特例：base_url 与模型端点不同族、无法从 URL 推导。
/// 智谱的 Codex Responses 端点是 `/api/v1`，而通用 OpenAI 兼容端点
/// （含模型列表）在 `/api/paas/v4`。
const KNOWN_MODELS_URLS: &[(&str, &str)] = &[(
    "open.bigmodel.cn",
    "https://open.bigmodel.cn/api/paas/v4/models",
)];

/// 构造模型列表端点的候选 URL：
/// - 命中已知供应商映射 → 特例端点排在最前（推导候选留作兜底）
/// - baseURL 以版本段 `/v{N}` 结尾（`/v1`、OpenCode `/zen/go/v1` 等）→ `{base}/models`
///   （版本号已在路径里，不能再补 `/v1`，否则 `.../v4/v1/models` → 404）
/// - 版本段非 `/v1` 时追加 `/v1/models` 兜底；无版本段 → `{base}/v1/models`
pub fn build_models_url_candidates(base_url: &str) -> Result<Vec<String>, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("API 端点为空".to_string());
    }

    let mut candidates = Vec::new();
    for (host, url) in KNOWN_MODELS_URLS {
        if trimmed.contains(host) {
            candidates.push((*url).to_string());
        }
    }
    if ends_with_version_segment(trimmed) {
        candidates.push(format!("{trimmed}/models"));
        if !trimmed.ends_with("/v1") {
            candidates.push(format!("{trimmed}/v1/models"));
        }
    } else {
        candidates.push(format!("{trimmed}/v1/models"));
    }
    // 线性去重（候选最多 3 条），保持首次出现顺序
    let mut unique: Vec<String> = Vec::with_capacity(candidates.len());
    for url in candidates {
        if !unique.contains(&url) {
            unique.push(url);
        }
    }
    Ok(unique)
}

/// 是否以 OpenAI 风格的版本段 `/v{N}` 结尾（`/v1`、`.../paas/v4`）。
pub(super) fn ends_with_version_segment(url: &str) -> bool {
    let last = url.rsplit('/').next().unwrap_or("");
    last.strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

pub(super) fn redact(body: &str, secret: &str) -> String {
    if secret.is_empty() {
        body.to_string()
    } else {
        body.replace(secret, "[REDACTED]")
    }
}

pub(super) fn truncate_body(body: String) -> String {
    if body.chars().count() <= ERROR_BODY_MAX_CHARS {
        body
    } else {
        let mut truncated: String = body.chars().take(ERROR_BODY_MAX_CHARS).collect();
        truncated.push('…');
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_plain_root_appends_v1() {
        let c = build_models_url_candidates("https://api.siliconflow.cn").unwrap();
        assert_eq!(c, vec!["https://api.siliconflow.cn/v1/models"]);
    }

    #[test]
    fn candidates_trailing_slash_is_trimmed() {
        let c = build_models_url_candidates("https://api.example.com/").unwrap();
        assert_eq!(c, vec!["https://api.example.com/v1/models"]);
    }

    #[test]
    fn candidates_v1_suffix_uses_models_directly() {
        // 以 /v1 结尾的端点：版本段已就位，拼 /models
        let c = build_models_url_candidates("https://opencode.ai/zen/go/v1").unwrap();
        assert_eq!(c, vec!["https://opencode.ai/zen/go/v1/models"]);
        let c = build_models_url_candidates("https://api.minimaxi.com/v1").unwrap();
        assert_eq!(c, vec!["https://api.minimaxi.com/v1/models"]);
    }

    #[test]
    fn candidates_non_v1_version_keeps_fallback() {
        let c = build_models_url_candidates("https://open.bigmodel.cn/api/paas/v4").unwrap();
        assert_eq!(
            c,
            vec![
                "https://open.bigmodel.cn/api/paas/v4/models",
                "https://open.bigmodel.cn/api/paas/v4/v1/models",
            ]
        );
    }

    #[test]
    fn candidates_rejects_empty_base() {
        assert!(build_models_url_candidates("  ").is_err());
    }

    #[test]
    fn candidates_zhipu_uses_known_models_endpoint_first() {
        // 智谱 Responses 端点 /api/v1 与通用端点 /api/paas/v4 不同族：
        // 特例端点排最前，推导候选（404）留作兜底
        let c = build_models_url_candidates("https://open.bigmodel.cn/api/v1").unwrap();
        assert_eq!(
            c,
            vec![
                "https://open.bigmodel.cn/api/paas/v4/models",
                "https://open.bigmodel.cn/api/v1/models",
            ]
        );
    }

    #[test]
    fn error_body_is_truncated_and_redacted() {
        let long = "x".repeat(600).replace("xxxxx", "key-1");
        let out = truncate_body(redact(&format!("key-1{long}"), "key-1"));
        assert!(out.chars().count() <= ERROR_BODY_MAX_CHARS + 1);
        assert!(!out.contains("key-1"));
        assert!(out.contains("[REDACTED]"));
    }
}
