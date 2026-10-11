//! ChatGPT 官方订阅的 OAuth 认证（对齐官方 Codex CLI 的登录流程）。
//!
//! 浏览器授权码登录：PKCE + 本地 127.0.0.1:1455 回环回调，`prompt=login`
//! 强制弹登录/选号页。
//!
//! 账号持久化（只存 refresh_token 与账号标识），access_token 内存缓存、到期前自动刷新。
//!
//! 认证一次后账号常驻，后续添加 ChatGPT 供应商无需重复认证。

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use crate::database::{Database, StoredAccount};
use crate::error::AppResult;

/// OpenAI OAuth 客户端 ID（与官方 Codex CLI 相同）
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;
const CODEX_USER_AGENT: &str = "agofra-codex-oauth";
const REGION_BLOCKED_MARKER: &str = "unsupported_country_region_territory";

// 浏览器授权码登录（PKCE + 本地回环回调，与官方 Codex CLI 的 `codex login` 同款）。
// redirect_uri 是官方 client_id 的注册白名单项，不可更换端口或路径。
const BROWSER_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const BROWSER_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const BROWSER_CALLBACK_PORT: u16 = 1455;
const BROWSER_LOGIN_TIMEOUT_SECS: u64 = 300;
const BROWSER_POLL_SLICE_MS: u64 = 400;
const BROWSER_BIND_RETRIES: u32 = 3;
const BROWSER_BIND_RETRY_DELAY_MS: u64 = 150;
const OAUTH_SCOPE_BROWSER: &str = "openid email profile offline_access";

#[derive(Debug, thiserror::Error)]
pub enum CodexOAuthError {
    #[error("OAuth 请求失败: {0}")]
    RequestFailed(String),
    #[error("Refresh Token 失效或已过期")]
    RefreshTokenInvalid,
    #[error("网络错误: {0}")]
    NetworkError(String),
    #[error("解析错误: {0}")]
    ParseError(String),
    #[error("账号不存在: {0}")]
    AccountNotFound(String),
    #[error("IO 错误: {0}")]
    IoError(String),
}

impl From<reqwest::Error> for CodexOAuthError {
    fn from(error: reqwest::Error) -> Self {
        CodexOAuthError::NetworkError(error.to_string())
    }
}

/// 返回给前端的浏览器授权入口
#[derive(Debug, Clone, Serialize)]
pub struct BrowserLoginStart {
    pub authorize_url: String,
    pub expires_in: u64,
}

/// 本地回调线程送回的结果
#[derive(Debug, Clone)]
enum BrowserCallback {
    Code(String),
    /// 用户在浏览器取消/拒绝授权（回调带 error 参数）
    Denied(String),
    Timeout,
}

/// 进行中的浏览器登录：PKCE verifier 与回调接收端一起保管，
/// poll 借走 receiver、超时后放回，完成或取消时整个槽位清空。
struct PendingBrowserLogin {
    code_verifier: String,
    code_rx: Option<tokio::sync::oneshot::Receiver<BrowserCallback>>,
}

/// 已认证账号摘要
#[derive(Debug, Clone, Serialize)]
pub struct ManagedAccount {
    pub id: String,
    pub login: String,
    pub authenticated_at: i64,
    pub is_default: bool,
    /// 订阅套餐（free/plus/pro/team…），未知为 null
    pub plan_type: Option<String>,
    /// ChatGPT 订阅到期时间（Unix 毫秒）；来自官方 id_token，未知为 null。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_active_until: Option<i64>,
}

/// 一次登录得到的凭证与身份：`complete_login` 从 token 响应提取，
/// 落库（新增行）与幂等更新（`reauthenticate_account`）共用。
struct LoginCredentials {
    chatgpt_account_id: String,
    refresh_token: String,
    email: Option<String>,
    id_token: Option<String>,
    user_identity: Option<String>,
    plan_type: Option<String>,
}

/// 认证状态摘要
#[derive(Debug, Clone, Serialize, Default)]
pub struct AuthStatus {
    pub authenticated: bool,
    pub default_account_id: Option<String>,
    pub accounts: Vec<ManagedAccount>,
    /// Desktop 来源账号（由 commands 层从数据库认证快照派生填充）。
    pub external: Vec<ManagedAccount>,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct IdTokenClaims {
    /// 跨刷新稳定的用户身份：同一 workspace 的不同用户 sub 不同
    #[serde(default)]
    sub: Option<String>,
    /// JWT 签发时间（NumericDate，秒）
    #[serde(default)]
    iat: Option<i64>,
    /// JWT access token 的过期时间（NumericDate，秒）。
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    organizations: Vec<OrgClaim>,
    #[serde(default, rename = "https://api.openai.com/auth")]
    openai_auth: Option<OpenAiAuthClaim>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OrgClaim {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OpenAiAuthClaim {
    #[serde(default)]
    chatgpt_account_id: Option<String>,
    /// 订阅套餐（free/plus/pro/team/business/go/k12…），官方原词
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    /// CLIProxyAPI 同名 JWT claim；官方返回可能是 Unix 时间戳或 RFC 3339 文本。
    #[serde(default)]
    chatgpt_subscription_active_until: Option<serde_json::Value>,
}

/// 内存缓存的 access_token
#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    expires_at_ms: i64,
}

impl CachedAccessToken {
    fn is_expiring_soon(&self) -> bool {
        self.expires_at_ms - now_ms() < TOKEN_REFRESH_BUFFER_MS
    }
}

/// 持久化的账号数据（refresh_token/id_token + 上次生成的 auth.json 缓存，access_token 不落盘）
#[derive(Debug, Clone)]
struct CodexAccountData {
    /// 本地主键：存量行是 workspace ID，新行由登录流程生成（acc-<毫秒>-<自增>）
    account_id: String,
    email: Option<String>,
    id_token: Option<String>,
    refresh_token: String,
    auth_json: Option<String>,
    authenticated_at: i64,
    /// ChatGPT workspace ID（出站语义），存量行迁移时回填为行 id
    chatgpt_account_id: Option<String>,
    /// id_token 的 sub：判重与 live auth.json 所有权校验
    user_identity: Option<String>,
    /// 订阅套餐（free/plus/pro/team…），随 id_token 刷新而更新
    plan_type: Option<String>,
}

impl CodexAccountData {
    /// 出站用的 workspace ID；未回填的旧行回退行 id
    fn workspace_id(&self) -> Option<String> {
        self.chatgpt_account_id
            .clone()
            .or_else(|| Some(self.account_id.clone()))
    }
}

impl From<StoredAccount> for CodexAccountData {
    fn from(account: StoredAccount) -> Self {
        Self {
            account_id: account.id,
            email: account.email,
            id_token: account.id_token,
            refresh_token: account.refresh_token,
            auth_json: account.auth_json,
            authenticated_at: account.authenticated_at,
            chatgpt_account_id: account.chatgpt_account_id,
            user_identity: account.user_identity,
            plan_type: account.plan_type,
        }
    }
}

impl From<&CodexAccountData> for StoredAccount {
    fn from(account: &CodexAccountData) -> Self {
        Self {
            id: account.account_id.clone(),
            email: account.email.clone(),
            id_token: account.id_token.clone(),
            refresh_token: account.refresh_token.clone(),
            auth_json: account.auth_json.clone(),
            authenticated_at: account.authenticated_at,
            chatgpt_account_id: account.chatgpt_account_id.clone(),
            user_identity: account.user_identity.clone(),
            plan_type: account.plan_type.clone(),
        }
    }
}

/// 账号本地主键的进程内自增后缀：同一毫秒内登录多个账号也不会撞主键
static ACCOUNT_ID_SEQ: AtomicU64 = AtomicU64::new(0);

fn should_log_unmanaged_live_sync(last_workspace: &mut Option<String>, workspace: &str) -> bool {
    last_workspace.replace(workspace.to_string()).as_deref() != Some(workspace)
}

/// 账号日志定位字段：account_id 恒输出；email 仅 debug 构建附加（release 绝不出现）
pub(crate) fn account_subject(account_id: &str, email: Option<&str>) -> String {
    match email {
        Some(email) if cfg!(debug_assertions) => {
            format!("account_id={account_id} email={email:?}")
        }
        _ => format!("account_id={account_id}"),
    }
}

/// 多账号认证管理器
pub struct CodexOAuthManager {
    client: Mutex<Option<(Option<String>, reqwest::Client)>>,
    accounts: Arc<RwLock<HashMap<String, CodexAccountData>>>,
    default_account_id: Arc<RwLock<Option<String>>>,
    access_tokens: Arc<RwLock<HashMap<String, CachedAccessToken>>>,
    refresh_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// live auth.json 因「不比账号新」被拒时记一次 workspace，避免轮询周期反复刷同一条 Debug
    last_stale_live_sync_workspace: std::sync::Mutex<Option<String>>,
    /// 浏览器登录同一时刻只有一个；无跨 await 持锁，用同步锁避免 Send 问题
    pending_browser: std::sync::Mutex<Option<PendingBrowserLogin>>,
    /// 非托管 live auth.json 是常态；同一 workspace 每个进程只记一次诊断行。
    last_unmanaged_live_sync_workspace: std::sync::Mutex<Option<String>>,
    /// workspace 匹配但身份对不上的归属失败：状态会持续整个轮询周期，同 workspace 去重防 Warn 刷屏。
    last_mismatched_live_sync_workspace: std::sync::Mutex<Option<String>>,
    database: Arc<Database>,
}

impl CodexOAuthManager {
    fn build_client(network: &crate::network::Network) -> Result<reqwest::Client, CodexOAuthError> {
        network
            .builder()
            .user_agent(CODEX_USER_AGENT)
            .build()
            .map_err(CodexOAuthError::from)
    }

    /// 发送请求；响应为地区拦截 403 时，重建客户端（重新读取系统代理）后重试一次。
    /// 正常路径复用缓存客户端；系统代理变化时按需重建，启动时不读取网络设置。
    async fn request_with_proxy_retry(
        &self,
        operation: &str,
        context: &str,
        make: impl Fn(reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Result<(reqwest::StatusCode, String), CodexOAuthError> {
        let mut retried = false;
        loop {
            let retry_prefix = if retried { " 代理重试" } else { " " };
            let network = crate::network::Network::current()
                .await
                .map_err(|error| {
                    tauri_plugin_log::log::warn!(
                        "[auth.request] op={operation:?} subject={context:?} stage=network_setup outcome=failure failure_kind={} error={:?} msg=\"OAuth 网络设置读取失败\"",
                        error.kind,
                        error.message
                    );
                    CodexOAuthError::NetworkError(error.to_string())
                })?;
            if context == "login" {
                tauri_plugin_log::log::info!(
                    "[auth.request] op={operation:?} subject={context:?} stage=token_exchange retry={retried} route={}{} outcome=success msg=\"OAuth 开始换取 Token\"",
                    network.mode(),
                    crate::network::proxy_note(&network.proxy)
                );
            }
            let client = {
                let mut cached = self.client.lock().await;
                if cached
                    .as_ref()
                    .is_none_or(|(proxy, _)| *proxy != network.proxy)
                {
                    let client = Self::build_client(&network).inspect_err(|_| {
                        tauri_plugin_log::log::warn!(
                            "[auth.request] op={operation:?} subject={context:?} stage=client_setup outcome=failure failure_kind=internal msg=\"OAuth HTTP 客户端创建失败\""
                        );
                    })?;
                    *cached = Some((network.proxy.clone(), client));
                }
                cached.as_ref().expect("HTTP 客户端已初始化").1.clone()
            };
            let started = std::time::Instant::now();
            let response = make(client).send().await.map_err(|error| {
                tauri_plugin_log::log::warn!(
                    "[auth.request] op={:?} subject={context:?} stage=request outcome=failure failure_kind={} connect={} latency_ms={} msg=\"OAuth 网络请求失败\"",
                    format!("{operation}{retry_prefix}"),
                    if error.is_timeout() { "timeout" } else { "network_error" },
                    error.is_connect(),
                    started.elapsed().as_millis()
                );
                CodexOAuthError::from(error)
            })?;
            let status = response.status();
            let text = response.text().await.map_err(|error| {
                tauri_plugin_log::log::warn!(
                    "[auth.request] op={:?} subject={context:?} stage=response_body outcome=failure failure_kind={} status_code={} latency_ms={} msg=\"OAuth 响应读取失败\"",
                    format!("{operation}{retry_prefix}"),
                    if error.is_timeout() { "timeout" } else { "io_error" },
                    status.as_u16(),
                    started.elapsed().as_millis()
                );
                CodexOAuthError::from(error)
            })?;
            if !retried
                && status == reqwest::StatusCode::FORBIDDEN
                && text.contains(REGION_BLOCKED_MARKER)
            {
                tauri_plugin_log::log::info!(
                    "[auth.request] op={operation:?} subject={context:?} outcome=skipped reason=region_blocked retry=true msg=\"OAuth 地区限制触发代理重试\""
                );
                *self.client.lock().await = None;
                retried = true;
                continue;
            }
            return Ok((status, text));
        }
    }

    pub fn new(database: Arc<Database>) -> Self {
        let manager = Self {
            client: Mutex::new(None),
            accounts: Arc::new(RwLock::new(HashMap::new())),
            default_account_id: Arc::new(RwLock::new(None)),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
            refresh_locks: Arc::new(RwLock::new(HashMap::new())),
            last_stale_live_sync_workspace: std::sync::Mutex::new(None),
            pending_browser: std::sync::Mutex::new(None),
            last_unmanaged_live_sync_workspace: std::sync::Mutex::new(None),
            last_mismatched_live_sync_workspace: std::sync::Mutex::new(None),
            database,
        };
        if let Err(error) = manager.load_accounts() {
            tauri_plugin_log::log::warn!(
                "[auth.account.load] outcome=failure failure_kind=io_error error={error:?} msg=\"加载认证账号失败\""
            );
        }
        manager
    }

    /// 数据库恢复/导入后，重新从 SQLite 加载账号（不再触发旧 JSON 导入）。
    pub fn reload_from_database(&self) -> AppResult<()> {
        self.load_accounts()
    }

    async fn exchange_code_for_tokens(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        let (status, text) = self
            .request_with_proxy_retry("授权码换 Token", "login", |client| {
                client
                    .post(OAUTH_TOKEN_URL)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(&[
                        ("grant_type", "authorization_code"),
                        ("code", code),
                        ("redirect_uri", redirect_uri),
                        ("client_id", CODEX_CLIENT_ID),
                        ("code_verifier", code_verifier),
                    ])
            })
            .await?;
        if !status.is_success() {
            tauri_plugin_log::log::warn!(
                "[auth.login.token] outcome=failure failure_kind=http_error status_code={} msg=\"授权码换 Token 失败\"",
                status.as_u16()
            );
            return Err(CodexOAuthError::RequestFailed(format!(
                "换取 Token 失败: {status} - {text}"
            )));
        }
        serde_json::from_str(&text).map_err(|error| {
            tauri_plugin_log::log::warn!(
                "[auth.login.token] outcome=failure failure_kind=parse_error error={error:?} msg=\"Token 响应解析失败\""
            );
            CodexOAuthError::ParseError(error.to_string())
        })
    }

    /// 浏览器登录的收尾：落库 + 登录响应自带的 access_token 入缓存
    async fn complete_login(
        &self,
        tokens: OAuthTokenResponse,
    ) -> Result<ManagedAccount, CodexOAuthError> {
        let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
            tauri_plugin_log::log::warn!(
                "[auth.login.token] outcome=failure failure_kind=validation_error error=\"missing refresh_token\" msg=\"登录响应缺少 refresh_token\""
            );
            CodexOAuthError::RequestFailed("响应缺少 refresh_token".to_string())
        })?;
        let (chatgpt_account_id, email) = extract_identity_from_tokens(&tokens);
        let chatgpt_account_id = chatgpt_account_id.ok_or_else(|| {
            tauri_plugin_log::log::warn!(
                "[auth.login.token] outcome=failure failure_kind=validation_error error=\"missing chatgpt account id\" msg=\"Token 响应缺少 ChatGPT 账号标识\""
            );
            CodexOAuthError::ParseError("无法从 token 中提取账号标识".to_string())
        })?;
        let id_token = tokens.id_token.clone();
        let credentials = LoginCredentials {
            chatgpt_account_id,
            refresh_token,
            email,
            user_identity: id_token.as_deref().and_then(extract_user_identity),
            plan_type: id_token.as_deref().and_then(extract_plan_type),
            id_token,
        };

        let account = self.add_account_internal(credentials).await.inspect_err(|error| {
            tauri_plugin_log::log::warn!(
                "[auth.login.account] stage=persist outcome=failure failure_kind={} msg=\"OAuth 登录账号保存失败\"",
                if matches!(error, CodexOAuthError::IoError(_)) { "io_error" } else { "internal" }
            );
        })?;
        // 登录响应自带的 access_token 按"本地行 id"入缓存（行 id 与 workspace 解耦后两者不同）
        self.access_tokens.write().await.insert(
            account.id.clone(),
            CachedAccessToken {
                token: tokens.access_token.clone(),
                expires_at_ms: compute_expires_at_ms(tokens.expires_in),
            },
        );
        Ok(account)
    }

    // ==================== 浏览器授权码登录（PKCE 回环回调） ====================

    /// 槽位锁的守卫。锁毒化只在持锁线程 panic 时出现，而临界区全是纯内存
    /// 的 Option 取放，即使发生，槽位数据也依然结构合法，直接取回继续用。
    fn browser_pending_lock(&self) -> std::sync::MutexGuard<'_, Option<PendingBrowserLogin>> {
        self.pending_browser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 启动浏览器授权：生成 PKCE，本地 127.0.0.1:1455 监听回调，
    /// 返回授权 URL 交给前端用系统浏览器打开。
    pub async fn start_browser_login(&self) -> Result<BrowserLoginStart, CodexOAuthError> {
        // 先结束上一次未完成的浏览器登录，旧监听线程 ≤150ms 内感知取消并退出
        self.cancel_browser_login();
        let state = random_urlsafe_token();
        let code_verifier = random_urlsafe_token();
        // 旧线程退出前端口尚未释放，AddrInUse 时短暂重试几次；其他错误立即失败
        let mut listener = None;
        for _ in 0..BROWSER_BIND_RETRIES {
            match std::net::TcpListener::bind((
                std::net::Ipv4Addr::LOCALHOST,
                BROWSER_CALLBACK_PORT,
            )) {
                Ok(bound) => {
                    listener = Some(bound);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        BROWSER_BIND_RETRY_DELAY_MS,
                    ))
                    .await;
                }
                Err(error) => {
                    tauri_plugin_log::log::warn!(
                    "[auth.login.browser] stage=listener_setup outcome=failure failure_kind=io_error error={error:?} msg=\"浏览器登录回调端口绑定失败\""
                    );
                    return Err(CodexOAuthError::RequestFailed(format!(
                        "本地端口 {BROWSER_CALLBACK_PORT} 绑定失败: {error}"
                    )));
                }
            }
        }
        let listener = listener.ok_or_else(|| {
            tauri_plugin_log::log::warn!(
                "[auth.login.browser] stage=listener_setup outcome=failure failure_kind=io_error reason=port_in_use callback_port={BROWSER_CALLBACK_PORT} msg=\"浏览器登录回调端口持续被占用\""
            );
            CodexOAuthError::RequestFailed(format!(
                "本地端口 {BROWSER_CALLBACK_PORT} 被占用（可能其他程序正在登录），请稍后重试"
            ))
        })?;
        let (code_tx, code_rx) = tokio::sync::oneshot::channel();
        let callback_state = state.clone();
        {
            let mut pending = self.browser_pending_lock();
            *pending = Some(PendingBrowserLogin {
                code_verifier: code_verifier.clone(),
                code_rx: Some(code_rx),
            });
        }
        std::thread::spawn(move || {
            serve_browser_callback(listener, callback_state, code_tx);
        });
        let authorize_url = build_authorize_url(&state, &pkce_code_challenge(&code_verifier));
        tauri_plugin_log::log::info!(
            "[auth.login.browser] stage=awaiting_callback callback_port={BROWSER_CALLBACK_PORT} timeout_secs={BROWSER_LOGIN_TIMEOUT_SECS} outcome=success msg=\"浏览器登录已启动，等待授权回调\""
        );
        Ok(BrowserLoginStart {
            authorize_url,
            expires_in: BROWSER_LOGIN_TIMEOUT_SECS,
        })
    }

    /// 轮询浏览器登录结果：用户尚未在浏览器完成授权时返回 `Ok(None)`。授权完成即换 Token 并落库。
    pub async fn poll_browser_login(&self) -> Result<Option<ManagedAccount>, CodexOAuthError> {
        let mut code_rx = {
            let mut pending = self.browser_pending_lock();
            pending
                .as_mut()
                .and_then(|slot| slot.code_rx.take())
                .ok_or_else(|| {
                    tauri_plugin_log::log::warn!(
                        "[auth.login.browser] stage=awaiting_callback outcome=failure failure_kind=internal reason=missing_receiver msg=\"OAuth 登录等待上下文不存在或正在被其他请求使用\""
                    );
                    CodexOAuthError::RequestFailed(
                        "没有进行中的浏览器登录，请重新启动登录".to_string(),
                    )
                })?
        };
        let code = match tokio::time::timeout(
            std::time::Duration::from_millis(BROWSER_POLL_SLICE_MS),
            &mut code_rx,
        )
        .await
        {
            // 尚未回调：放回 receiver，等下一次轮询
            Err(_elapsed) => {
                let mut pending = self.browser_pending_lock();
                if let Some(slot) = pending.as_mut() {
                    slot.code_rx = Some(code_rx);
                }
                return Ok(None);
            }
            // 发送端已丢弃：登录被取消
            Ok(Err(_cancelled)) => {
                self.clear_pending_browser_login();
                tauri_plugin_log::log::info!(
                    "[auth.login.browser] stage=awaiting_callback outcome=denied reason=listener_closed msg=\"OAuth 回调监听已结束，未取得授权码\""
                );
                return Err(CodexOAuthError::RequestFailed(
                    "浏览器登录已取消".to_string(),
                ));
            }
            Ok(Ok(BrowserCallback::Timeout)) => {
                self.clear_pending_browser_login();
                return Err(CodexOAuthError::RequestFailed(
                    "浏览器登录等待超时，请重试".to_string(),
                ));
            }
            Ok(Ok(BrowserCallback::Denied(message))) => {
                self.clear_pending_browser_login();
                tauri_plugin_log::log::info!(
                    "[auth.login.browser] stage=callback outcome=denied reason=authorization_denied msg=\"OAuth 浏览器授权被拒绝或中断\""
                );
                return Err(CodexOAuthError::RequestFailed(format!(
                    "浏览器授权未完成: {message}"
                )));
            }
            Ok(Ok(BrowserCallback::Code(code))) => code,
        };
        tauri_plugin_log::log::info!(
            "[auth.login.browser] stage=callback outcome=success msg=\"OAuth 已收到有效授权回调\""
        );
        let code_verifier = {
            let mut pending = self.browser_pending_lock();
            match pending.take() {
                Some(slot) => slot.code_verifier,
                None => {
                    tauri_plugin_log::log::warn!(
                        "[auth.login.browser] stage=callback outcome=failure failure_kind=internal reason=missing_pending_login msg=\"OAuth 收到回调后登录上下文已失效\""
                    );
                    return Err(CodexOAuthError::RequestFailed(
                        "没有进行中的浏览器登录，请重新启动登录".to_string(),
                    ));
                }
            }
        };
        let tokens = self
            .exchange_code_for_tokens(&code, &code_verifier, BROWSER_REDIRECT_URI)
            .await?;
        tauri_plugin_log::log::info!(
            "[auth.login.token] outcome=success msg=\"OAuth 换取 Token 成功，开始保存账号\""
        );
        let account = self.complete_login(tokens).await?;
        let email = self
            .accounts
            .read()
            .await
            .get(&account.id)
            .and_then(|data| data.email.clone());
        tauri_plugin_log::log::info!(
            "[auth.login.browser] {} source=oauth stage=complete outcome=success msg=\"浏览器授权完成\"",
            account_subject(&account.id, email.as_deref())
        );
        Ok(Some(account))
    }

    /// 取消进行中的浏览器登录：丢弃 receiver 使监听线程退出并释放回调端口。
    pub fn cancel_browser_login(&self) {
        if self.clear_pending_browser_login() {
            tauri_plugin_log::log::info!(
                "[auth.login.browser] stage=awaiting_callback outcome=denied reason=user_cancelled msg=\"OAuth 等待已取消，尚未取得有效授权回调\""
            );
        }
    }

    /// 清空浏览器登录槽位；返回是否存在未完成的登录。
    /// 槽位清空即丢弃 receiver，监听线程借 `Sender::is_closed` 在 ≤150ms 内
    /// 感知取消并退出，回调端口随之释放。
    fn clear_pending_browser_login(&self) -> bool {
        self.browser_pending_lock().take().is_some()
    }

    async fn refresh_with_token(
        &self,
        account_id: &str,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        let (status, text) = self
            .request_with_proxy_retry("Token 刷新", account_id, |client| {
                client
                    .post(OAUTH_TOKEN_URL)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .form(&[
                        ("grant_type", "refresh_token"),
                        ("refresh_token", refresh_token),
                        ("client_id", CODEX_CLIENT_ID),
                        ("scope", "openid profile email"),
                    ])
            })
            .await?;
        #[cfg(debug_assertions)]
        let subject = {
            let email = self
                .accounts
                .read()
                .await
                .get(account_id)
                .and_then(|account| account.email.clone());
            account_subject(account_id, email.as_deref())
        };
        #[cfg(not(debug_assertions))]
        let subject = account_subject(account_id, None);
        if text.contains(REGION_BLOCKED_MARKER) {
            tauri_plugin_log::log::warn!(
                "[auth.token.refresh] {subject} outcome=failure failure_kind=region_blocked status_code={} msg=\"Token 刷新被地区限制拦截\"",
                status.as_u16()
            );
            return Err(CodexOAuthError::RequestFailed(format!(
                "刷新 Token 失败: {status} - {text}"
            )));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // 服务端吊销/拒绝（refresh_token_invalidated）：掉登录的唯一信号源，必须留痕
            tauri_plugin_log::log::warn!(
                "[auth.token.refresh] {subject} outcome=failure failure_kind=auth_error status_code={} msg=\"refresh_token 被服务端拒绝，该账号需要重新登录\"",
                status.as_u16()
            );
            return Err(CodexOAuthError::RefreshTokenInvalid);
        }
        if !status.is_success() {
            tauri_plugin_log::log::warn!(
                "[auth.token.refresh] {subject} outcome=failure failure_kind=http_error status_code={} msg=\"token 刷新请求失败\"",
                status.as_u16()
            );
            return Err(CodexOAuthError::RequestFailed(format!(
                "刷新 Token 失败: {status} - {text}"
            )));
        }
        let tokens = serde_json::from_str::<OAuthTokenResponse>(&text).map_err(|error| {
            tauri_plugin_log::log::warn!(
                "[auth.token.refresh] {subject} outcome=failure failure_kind=parse_error error={error:?} msg=\"token 刷新响应解析失败\""
            );
            CodexOAuthError::ParseError(error.to_string())
        })?;
        tauri_plugin_log::log::info!(
            "[auth.token.refresh] {subject} outcome=success expires_in={} msg=\"token 刷新成功，新 access_token 已取得\"",
            tokens.expires_in.unwrap_or(0)
        );
        Ok(tokens)
    }

    // ==================== Token 获取（含自动刷新） ====================

    /// 获取账号的有效 access_token，临近过期时自动刷新
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _guard = refresh_lock.lock().await;
        self.get_valid_token_for_account_locked(account_id).await
    }

    async fn get_valid_token_for_account_locked(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        if let Some(cached) = self.access_tokens.read().await.get(account_id) {
            if !cached.is_expiring_soon() {
                return Ok(cached.token.clone());
            }
        }

        let account = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .cloned()
                .ok_or_else(|| {
                    tauri_plugin_log::log::warn!(
                        "[auth.token.refresh] account_id={account_id} outcome=failure failure_kind=not_found msg=\"Token 刷新找不到托管账号\""
                    );
                    CodexOAuthError::AccountNotFound(account_id.to_string())
                })?
        };

        // 账号 auth.json 是 access_token 的持久化快照。进程缓存丢失后，若快照中的
        // token 尚未过期，继续使用它；不要因 refresh_token 已失效而提前阻断切换/查额。
        if let Some(cached) = reusable_auth_snapshot_access_token(&account) {
            let token = cached.token.clone();
            self.access_tokens
                .write()
                .await
                .insert(account_id.to_string(), cached);
            return Ok(token);
        }

        let new_tokens = self
            .refresh_with_token(account_id, &account.refresh_token)
            .await?;

        let new_refresh = new_tokens.refresh_token.clone();
        let new_id_token = new_tokens.id_token.clone();
        let new_plan_type = new_id_token.as_deref().and_then(extract_plan_type);
        if new_refresh.is_some() || new_id_token.is_some() {
            let mut accounts = self.accounts.write().await;
            if let Some(account) = accounts.get_mut(account_id) {
                let mut changed = false;
                if let Some(token) = new_refresh {
                    if account.refresh_token != token {
                        account.refresh_token = token;
                        changed = true;
                    }
                }
                if let Some(token) = new_id_token {
                    if account.id_token.as_deref() != Some(token.as_str()) {
                        account.id_token = Some(token);
                        changed = true;
                    }
                }
                // 套餐可能升级/降级（free→plus）：跟随最新 id_token 刷新
                if let Some(plan) = new_plan_type {
                    if account.plan_type.as_deref() != Some(plan.as_str()) {
                        account.plan_type = Some(plan);
                        changed = true;
                    }
                }
                if changed {
                    // refresh_token 轮换落库是后续离线复用 auth.json 的依据，必须留痕
                    let subject = account_subject(account_id, account.email.as_deref());
                    self.save_account(account)?;
                    tauri_plugin_log::log::info!(
                        "[auth.token.rotate] {subject} outcome=success msg=\"凭证已轮换并持久化到数据库\""
                    );
                }
            }
        }

        self.access_tokens.write().await.insert(
            account_id.to_string(),
            CachedAccessToken {
                token: new_tokens.access_token.clone(),
                expires_at_ms: compute_expires_at_ms(new_tokens.expires_in),
            },
        );
        Ok(new_tokens.access_token)
    }

    /// 生成官方 Codex CLI 的 auth.json 内容（ChatGPT 订阅登录格式）。
    pub async fn codex_auth_json(&self, account_id: &str) -> Result<String, CodexOAuthError> {
        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _guard = refresh_lock.lock().await;
        self.codex_auth_json_locked(account_id).await
    }

    async fn codex_auth_json_locked(&self, account_id: &str) -> Result<String, CodexOAuthError> {
        let access_token = self.get_valid_token_for_account_locked(account_id).await?;
        let (refresh_token, id_token, chatgpt_account_id) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
            (
                account.refresh_token.clone(),
                account.id_token.clone(),
                account.workspace_id(),
            )
        };
        let id_token = id_token.ok_or_else(|| {
            CodexOAuthError::RequestFailed("账号缺少 id_token，请重新登录".to_string())
        })?;
        // tokens.account_id 必须是 ChatGPT workspace ID（Codex CLI 出站语义），不是本地行 id
        let auth = serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": refresh_token,
                "account_id": chatgpt_account_id.unwrap_or_else(|| account_id.to_string()),
            },
            "last_refresh": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        });
        let text = serde_json::to_string_pretty(&auth)
            .map_err(|error| CodexOAuthError::ParseError(error.to_string()))?;
        // 缓存本次生成的 auth.json：后续切换配置可离线复用，不发网络请求刷新 token
        {
            let mut accounts = self.accounts.write().await;
            if let Some(account) = accounts.get_mut(account_id) {
                if account.auth_json.as_deref() != Some(text.as_str()) {
                    account.auth_json = Some(text.clone());
                    if let Err(error) = self.save_account(account) {
                        tauri_plugin_log::log::warn!(
                            "[auth.live_sync] outcome=failure failure_kind=io_error error={error:?} msg=\"缓存 auth.json 失败\""
                        );
                    }
                }
            }
        }
        Ok(text)
    }

    /// 强制刷新账号的 access_token 后生成 auth.json，用于认证失败后的重试。
    pub async fn refresh_codex_auth_json(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _guard = refresh_lock.lock().await;
        self.access_tokens.write().await.remove(account_id);
        self.codex_auth_json_locked(account_id).await
    }

    /// 读取缓存的 auth.json（离线切换配置用，不触发 token 刷新）。
    pub async fn cached_auth_json(&self, account_id: &str) -> Option<String> {
        let accounts = self.accounts.read().await;
        accounts
            .get(account_id)
            .and_then(|account| account.auth_json.clone())
    }

    /// 把 Codex 运行中刷新过的同账号 auth.json 同步回 OAuth 账号。
    /// 只接受已在 Agofra 管理中的账号，避免把桌面登录误导入为新账号。
    /// 归属判定用 (workspace, 用户 sub) 双匹配：同 workspace 多账号时绝不串号。
    /// 新旧判定：live access_token 的 JWT iat 必须比账号的 authenticated_at
    /// （最后一次登录/凭证落库时刻）更新才回写——否则给激活中的账号重新授权时，
    /// workspace 归属相同的旧 live 残留会把刚登录的新凭证整个覆盖回去。
    pub async fn sync_external_auth_json(&self, text: &str) -> Result<bool, CodexOAuthError> {
        let Some(auth) = parse_external_auth_json(text) else {
            return Ok(false);
        };
        let Some(refresh_token) = auth.refresh_token.clone() else {
            return Ok(false);
        };
        // 纯跟随登录（没有任何托管账号）时归属不到是常态，不留日志；只有
        // 「有托管账号但归属失败」才是「切完掉登录」的前兆。
        if self.accounts.read().await.is_empty() {
            return Ok(false);
        }
        let Some(row_id) = self.resolve_external_auth_owner(&auth).await else {
            // 归属失败分两态：workspace 对得上托管账号但身份或邮箱对不上（真换了人，
            // 或旧行缺身份记录、email 兜底也没过），是「串号/切完掉登录」的前兆，
            // Warn 留痕但同 workspace 去重——状态持续时每次轮询都会命中，不去重会刷屏；
            // workspace 完全陌生（纯桌面端跟随用户）是常态，降 Debug 同样去重。
            let workspace = auth.account_id.as_str();
            let same_workspace_accounts: Vec<String> = self
                .accounts
                .read()
                .await
                .values()
                .filter(|account| account.workspace_id().as_deref() == Some(workspace))
                .map(|account| account.account_id.clone())
                .collect();
            if !same_workspace_accounts.is_empty() {
                let should_log = self
                    .last_mismatched_live_sync_workspace
                    .lock()
                    .map(|mut last| should_log_unmanaged_live_sync(&mut last, workspace))
                    .unwrap_or(false);
                if should_log {
                    tauri_plugin_log::log::warn!(
                        "[auth.live_sync] workspace={workspace:?} account_id={} outcome=failure failure_kind=auth_error msg=\"live auth.json 已被外部刷新且 workspace 匹配托管账号，但身份或邮箱对不上、无法归属，跳过同步\"",
                        same_workspace_accounts.join(",")
                    );
                }
            } else {
                let should_log = self
                    .last_unmanaged_live_sync_workspace
                    .lock()
                    .map(|mut last| should_log_unmanaged_live_sync(&mut last, workspace))
                    .unwrap_or(false);
                if should_log {
                    tauri_plugin_log::log::debug!(
                        "[auth.live_sync.skip] workspace={workspace:?} source=live_auth_json outcome=skipped reason=unowned msg=\"实时凭证不属于托管账号，跳过同步\""
                    );
                }
            }
            return Ok(false);
        };
        let refresh_lock = self.get_refresh_lock(&row_id).await;
        let _guard = refresh_lock.lock().await;
        let live_issued_at = parse_jwt_claims(&auth.access_token).and_then(|claims| claims.iat);
        let updated = {
            let mut accounts = self.accounts.write().await;
            let Some(account) = accounts.get_mut(&row_id) else {
                return Ok(false);
            };
            // 新旧闸门：live 不比账号凭证新（含 access_token 不可解析的异常文件）一律拒绝回写
            let is_newer =
                matches!(live_issued_at, Some(live_iat) if live_iat > account.authenticated_at);
            if !is_newer {
                let should_log = self
                    .last_stale_live_sync_workspace
                    .lock()
                    .map(|mut last| {
                        should_log_unmanaged_live_sync(&mut last, auth.account_id.as_str())
                    })
                    .unwrap_or(false);
                if should_log {
                    tauri_plugin_log::log::debug!(
                        "[auth.live_sync.skip] {} source=live_auth_json outcome=skipped reason=stale_skip msg=\"实时凭证无更新，跳过回写\"",
                        account_subject(&row_id, account.email.as_deref())
                    );
                }
                return Ok(false);
            }
            let mut changed = false;
            if account.refresh_token != refresh_token {
                account.refresh_token = refresh_token;
                changed = true;
            }
            if let Some(id_token) = auth.id_token.clone() {
                if account.id_token.as_deref() != Some(id_token.as_str()) {
                    account.id_token = Some(id_token);
                    changed = true;
                }
            }
            if let Some(email) = auth.email.clone() {
                if account.email.as_deref() != Some(email.as_str()) {
                    account.email = Some(email);
                    changed = true;
                }
            }
            // 无身份记录的旧行吸收 live 凭证证明的用户身份（自愈）
            if account.user_identity.is_none() {
                account.user_identity = auth.user_identity.clone();
            }
            // 套餐跟随 live 凭证刷新（桌面端可能升级/降级了订阅）
            if let Some(plan) = auth.plan_type.as_deref() {
                if account.plan_type.as_deref() != Some(plan) {
                    account.plan_type = Some(plan.to_string());
                    changed = true;
                }
            }
            if account.auth_json.as_deref() != Some(text) {
                account.auth_json = Some(text.to_string());
                changed = true;
            }
            changed.then(|| account.clone())
        };
        let Some(account) = updated else {
            return Ok(false);
        };
        // 外部 auth.json 没有可靠的 expires_in，不能把它伪装成内存有效 token；
        // 下一次生成 auth.json 时必须按数据库里的 refresh_token 重新验证。
        self.access_tokens.write().await.remove(&row_id);
        let subject = account_subject(&row_id, account.email.as_deref());
        self.save_account(&account).map_err(|error| {
            tauri_plugin_log::log::warn!(
                "[auth.live_sync] {subject} outcome=failure failure_kind=io_error error={error:?} msg=\"同步外部 auth.json 落库失败\""
            );
            error
        })?;
        tauri_plugin_log::log::info!(
            "[auth.live_sync] {subject} outcome=success msg=\"已把外部刷新的 live auth.json 同步回账号\""
        );
        Ok(true)
    }

    /// 判定 live auth.json 属于哪一行托管账号：
    /// 精确匹配 (workspace, sub)；旧行缺身份时以 email 辅助认领；缺 sub 的外部认证
    /// 只在 workspace 唯一归属时认领（多行宁可放弃，避免把 A 的凭证写进 B 的行）。
    async fn resolve_external_auth_owner(&self, auth: &ExternalCodexAuth) -> Option<String> {
        let accounts = self.accounts.read().await;
        let same_workspace: Vec<&CodexAccountData> = accounts
            .values()
            .filter(|account| account.workspace_id().as_deref() == Some(auth.account_id.as_str()))
            .collect();
        let target: Option<&CodexAccountData> = match auth.user_identity.as_deref() {
            Some(identity) => same_workspace
                .iter()
                .copied()
                .find(|account| account.user_identity.as_deref() == Some(identity))
                .or_else(|| {
                    // 存量行没有 id_token 无法回填身份：workspace + email 一致视为同一用户
                    same_workspace.iter().copied().find(|account| {
                        account.user_identity.is_none()
                            && account.email.is_some()
                            && account.email.as_deref() == auth.email.as_deref()
                    })
                }),
            None => (same_workspace.len() == 1).then(|| same_workspace[0]),
        };
        target.map(|account| account.account_id.clone())
    }

    #[cfg(test)]
    pub async fn seed_access_token_for_test(&self, account_id: &str, token: &str) {
        self.access_tokens.write().await.insert(
            account_id.to_string(),
            CachedAccessToken {
                token: token.to_string(),
                expires_at_ms: now_ms() + 3_600_000,
            },
        );
    }

    // ==================== 账号管理 ====================

    pub async fn get_status(&self) -> AuthStatus {
        let accounts = self.accounts.read().await.clone();
        let default_id = self.resolve_default_account_id().await;
        AuthStatus {
            authenticated: !accounts.is_empty(),
            default_account_id: default_id.clone(),
            accounts: sorted_accounts(&accounts, default_id.as_deref()),
            external: Vec::new(),
        }
    }

    pub async fn remove_account(&self, account_id: &str) -> Result<(), CodexOAuthError> {
        {
            let mut accounts = self.accounts.write().await;
            accounts
                .remove(account_id)
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
        }
        self.database
            .delete_account(account_id)
            .map_err(|error| CodexOAuthError::IoError(error.to_string()))?;
        self.access_tokens.write().await.remove(account_id);
        self.refresh_locks.write().await.remove(account_id);
        {
            let accounts = self.accounts.read().await;
            let mut default = self.default_account_id.write().await;
            if default.as_deref() == Some(account_id) {
                *default = fallback_default_account_id(&accounts);
            }
        }
        let default = self.default_account_id.read().await.clone();
        self.save_default_account(default.as_deref())?;
        // 删除是用户触发的里程碑，且清掉的是长期凭据，必须留痕
        let subject = format!("account_id={account_id}");
        tauri_plugin_log::log::info!(
            "[auth.account.remove] {subject} outcome=success msg=\"已移除托管账号\""
        );
        Ok(())
    }

    /// 日志定位字段：account_id 恒输出，email 仅 debug 构建附加（供连接层标注 OAuth 账号行为）。
    pub async fn account_subject_for(&self, row_id: &str) -> String {
        let email = self
            .accounts
            .read()
            .await
            .get(row_id)
            .and_then(|account| account.email.clone());
        account_subject(row_id, email.as_deref())
    }

    /// 行 id 对应的 ChatGPT workspace ID（出站请求头用）。
    /// 行 id 与 workspace 解耦后两者不同；查不到时回退行 id 本身（旧行为兼容）。
    pub async fn workspace_of(&self, row_id: &str) -> String {
        let accounts = self.accounts.read().await;
        accounts
            .get(row_id)
            .and_then(|account| account.workspace_id())
            .unwrap_or_else(|| row_id.to_string())
    }

    // ==================== 内部方法 ====================

    /// 登录完成的落库入口（find-or-create）：
    /// - 同 workspace 同用户（sub）→ 幂等更新：原地刷新凭证并保留原 id，供应商绑定不失效；
    /// - 同 workspace 不同用户 → 新增一行（本地主键与 workspace 解耦），
    ///   并发下由唯一索引 accounts_ws_user_uq 兜底，后到者转为更新。
    async fn add_account_internal(
        &self,
        credentials: LoginCredentials,
    ) -> Result<ManagedAccount, CodexOAuthError> {
        for _ in 0..2 {
            if let Some(row_id) = self
                .find_login_target(
                    &credentials.chatgpt_account_id,
                    credentials.user_identity.as_deref(),
                    credentials.email.as_deref(),
                )
                .await
            {
                return self.reauthenticate_account(&row_id, credentials).await;
            }
            let id = format!(
                "acc-{}-{}",
                now_ms(),
                ACCOUNT_ID_SEQ.fetch_add(1, Ordering::Relaxed)
            );
            let data = CodexAccountData {
                account_id: id,
                email: credentials.email.clone(),
                id_token: credentials.id_token.clone(),
                refresh_token: credentials.refresh_token.clone(),
                auth_json: None,
                authenticated_at: now_secs(),
                chatgpt_account_id: Some(credentials.chatgpt_account_id.clone()),
                user_identity: credentials.user_identity.clone(),
                plan_type: credentials.plan_type.clone(),
            };
            match self
                .database
                .insert_account_if_absent(&(&data).into())
                .map_err(|error| CodexOAuthError::IoError(error.to_string()))?
            {
                true => {
                    let id = data.account_id.clone();
                    {
                        let mut accounts = self.accounts.write().await;
                        accounts.insert(id.clone(), data);
                    }
                    let should_set_default = {
                        let mut default = self.default_account_id.write().await;
                        let replace = default.is_none();
                        if replace {
                            *default = Some(id.clone());
                        }
                        replace
                    };
                    if should_set_default {
                        self.save_default_account(Some(id.as_str()))?;
                    }
                    return self.managed_account_summary(&id).await;
                }
                // 并发登录抢先落库：重查后转为原地更新
                false => continue,
            }
        }
        Err(CodexOAuthError::RequestFailed(
            "并发登录处理失败，请重试".to_string(),
        ))
    }

    /// 为本次登录寻找归属行（见 add_account_internal 注释）。
    async fn find_login_target(
        &self,
        workspace: &str,
        user_identity: Option<&str>,
        email: Option<&str>,
    ) -> Option<String> {
        let accounts = self.accounts.read().await;
        let same_workspace = accounts
            .values()
            .filter(|account| account.workspace_id().as_deref() == Some(workspace));
        if let Some(identity) = user_identity {
            if let Some(row) = same_workspace
                .clone()
                .find(|account| account.user_identity.as_deref() == Some(identity))
            {
                return Some(row.account_id.clone());
            }
            // 存量行缺 id_token 无法回填身份：workspace + email 一致视为同一用户
            return email.and_then(|email| {
                same_workspace
                    .clone()
                    .find(|account| {
                        account.user_identity.is_none() && account.email.as_deref() == Some(email)
                    })
                    .map(|row| row.account_id.clone())
            });
        }
        // 登录响应缺 sub（异常 token）：退化为旧行为，更新同 workspace 最新一行
        same_workspace
            .max_by_key(|account| account.authenticated_at)
            .map(|row| row.account_id.clone())
    }

    /// 幂等更新：原地刷新凭证、保留行 id；旧凭证衍生的缓存一并失效。
    async fn reauthenticate_account(
        &self,
        row_id: &str,
        credentials: LoginCredentials,
    ) -> Result<ManagedAccount, CodexOAuthError> {
        let LoginCredentials {
            chatgpt_account_id,
            refresh_token,
            email,
            id_token,
            user_identity,
            plan_type,
        } = credentials;
        let updated = {
            let mut accounts = self.accounts.write().await;
            let Some(account) = accounts.get_mut(row_id) else {
                return Err(CodexOAuthError::AccountNotFound(row_id.to_string()));
            };
            account.refresh_token = refresh_token;
            if id_token.is_some() {
                account.id_token = id_token;
            }
            if email.is_some() {
                account.email = email;
            }
            account.chatgpt_account_id = Some(chatgpt_account_id);
            if user_identity.is_some() {
                account.user_identity = user_identity;
            }
            if plan_type.is_some() {
                account.plan_type = plan_type;
            }
            account.auth_json = None;
            account.authenticated_at = now_secs();
            account.clone()
        };
        self.save_account(&updated)?;
        self.access_tokens.write().await.remove(row_id);
        self.managed_account_summary(row_id).await
    }

    async fn managed_account_summary(
        &self,
        row_id: &str,
    ) -> Result<ManagedAccount, CodexOAuthError> {
        let data = {
            let accounts = self.accounts.read().await;
            accounts
                .get(row_id)
                .cloned()
                .ok_or_else(|| CodexOAuthError::AccountNotFound(row_id.to_string()))?
        };
        let is_default = self.default_account_id.read().await.as_deref() == Some(row_id);
        Ok(ManagedAccount {
            id: data.account_id,
            login: display_login(row_id, data.email),
            authenticated_at: data.authenticated_at,
            is_default,
            plan_type: data.plan_type,
            subscription_active_until: data
                .id_token
                .as_deref()
                .and_then(extract_subscription_active_until),
        })
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;
        if let Some(id) = stored {
            if accounts.contains_key(&id) {
                return Some(id);
            }
        }
        fallback_default_account_id(&accounts)
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        if let Some(lock) = self.refresh_locks.read().await.get(account_id) {
            return Arc::clone(lock);
        }
        Arc::clone(
            self.refresh_locks
                .write()
                .await
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn save_account(&self, account: &CodexAccountData) -> Result<(), CodexOAuthError> {
        self.database
            .upsert_account(&account.into())
            .map_err(|error| CodexOAuthError::IoError(error.to_string()))
    }

    fn save_default_account(&self, id: Option<&str>) -> Result<(), CodexOAuthError> {
        self.database
            .set_default_account(id)
            .map_err(|error| CodexOAuthError::IoError(error.to_string()))
    }

    fn load_accounts(&self) -> AppResult<()> {
        let stored = self.database.accounts()?;
        let accounts: HashMap<String, CodexAccountData> = stored
            .into_iter()
            .map(|account| (account.id.clone(), account.into()))
            .collect();
        tauri_plugin_log::log::debug!(
            "[auth.account.load] count={} outcome=success msg=\"已加载托管账号\"",
            accounts.len()
        );
        let default = self
            .database
            .app_state()?
            .1
            .or_else(|| fallback_default_account_id(&accounts));
        if let Ok(mut slot) = self.accounts.try_write() {
            *slot = accounts;
        }
        if let Ok(mut slot) = self.default_account_id.try_write() {
            *slot = default;
        }
        Ok(())
    }
}

/// Tauri 托管状态。管理器自身已按账号与数据类型细粒度加锁，外层不再持有跨网络的读写锁。
pub struct CodexOAuthState(pub Arc<CodexOAuthManager>);

fn sorted_accounts(
    accounts: &HashMap<String, CodexAccountData>,
    default_account_id: Option<&str>,
) -> Vec<ManagedAccount> {
    let mut list: Vec<ManagedAccount> = accounts
        .iter()
        .map(|(id, data)| ManagedAccount {
            id: id.clone(),
            login: display_login(id, data.email.clone()),
            authenticated_at: data.authenticated_at,
            is_default: default_account_id == Some(id.as_str()),
            plan_type: data.plan_type.clone(),
            subscription_active_until: data
                .id_token
                .as_deref()
                .and_then(extract_subscription_active_until),
        })
        .collect();
    list.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| b.authenticated_at.cmp(&a.authenticated_at))
            .then_with(|| a.login.cmp(&b.login))
    });
    list
}

fn display_login(account_id: &str, email: Option<String>) -> String {
    email.unwrap_or_else(|| format!("ChatGPT ({account_id})"))
}

fn fallback_default_account_id(accounts: &HashMap<String, CodexAccountData>) -> Option<String> {
    accounts
        .iter()
        .max_by(|(id_a, a), (id_b, b)| {
            a.authenticated_at
                .cmp(&b.authenticated_at)
                .then_with(|| id_b.cmp(id_a))
        })
        .map(|(id, _)| id.clone())
}

fn compute_expires_at_ms(expires_in: Option<i64>) -> i64 {
    now_ms() + expires_in.unwrap_or(3600) * 1000
}

fn parse_jwt_claims(token: &str) -> Option<IdTokenClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn reusable_auth_snapshot_access_token(account: &CodexAccountData) -> Option<CachedAccessToken> {
    let auth = parse_external_auth_json(account.auth_json.as_deref()?)?;
    if account.workspace_id().as_deref() != Some(auth.account_id.as_str()) {
        return None;
    }
    if matches!(
        (account.user_identity.as_deref(), auth.user_identity.as_deref()),
        (Some(expected), Some(actual)) if expected != actual
    ) {
        return None;
    }

    let expires_at_ms = parse_jwt_claims(&auth.access_token)?
        .exp?
        .saturating_mul(1_000);
    let cached = CachedAccessToken {
        token: auth.access_token,
        expires_at_ms,
    };
    (!cached.is_expiring_soon()).then_some(cached)
}

/// 从 JWT 提取稳定用户身份（sub）：跨刷新不变，判重与 live auth 所有权校验的依据。
/// 迁移回填（database.rs）与登录流程共用。
pub(crate) fn extract_user_identity(token: &str) -> Option<String> {
    parse_jwt_claims(token)?.sub
}

/// 从 JWT 提取订阅套餐（`https://api.openai.com/auth.chatgpt_plan_type`）。
/// 迁移回填（database.rs）与登录/刷新/外部同步共用；小写归一，官方原词。
pub(crate) fn extract_plan_type(token: &str) -> Option<String> {
    parse_jwt_claims(token)?
        .openai_auth?
        .chatgpt_plan_type
        .map(|plan| plan.trim().to_lowercase())
        .filter(|plan| !plan.is_empty())
}

/// 从同一份官方 id_token 取套餐到期时间，兼容 CLIProxyAPI 已保留的数字与 RFC 3339 两种形态。
pub(crate) fn extract_subscription_active_until(token: &str) -> Option<i64> {
    let value = parse_jwt_claims(token)?
        .openai_auth?
        .chatgpt_subscription_active_until?;
    subscription_timestamp_ms(&value)
}

fn subscription_timestamp_ms(value: &serde_json::Value) -> Option<i64> {
    let timestamp = match value {
        serde_json::Value::Number(number) => number.as_i64()?,
        serde_json::Value::String(text) => text.parse::<i64>().ok().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(text)
                .ok()
                .map(|time| time.timestamp_millis())
        })?,
        _ => return None,
    };
    (timestamp > 0).then_some(if timestamp > 1_000_000_000_000 {
        timestamp
    } else {
        timestamp.saturating_mul(1_000)
    })
}

fn extract_identity_from_tokens(tokens: &OAuthTokenResponse) -> (Option<String>, Option<String>) {
    let mut account_id: Option<String> = None;
    let mut email: Option<String> = None;

    if let Some(id_token) = tokens.id_token.as_deref() {
        (account_id, email) = identity_from_jwt(id_token);
    }

    if account_id.is_none() {
        let (fallback_id, fallback_email) = identity_from_jwt(&tokens.access_token);
        account_id = fallback_id;
        if email.is_none() {
            email = fallback_email;
        }
    }

    (account_id, email)
}

fn identity_from_jwt(token: &str) -> (Option<String>, Option<String>) {
    let Some(claims) = parse_jwt_claims(token) else {
        return (None, None);
    };
    (
        claims
            .chatgpt_account_id
            .clone()
            .or_else(|| {
                claims
                    .openai_auth
                    .as_ref()
                    .and_then(|auth| auth.chatgpt_account_id.clone())
            })
            .or_else(|| claims.organizations.first().and_then(|org| org.id.clone())),
        claims.email.clone(),
    )
}

/// Codex CLI 官方 auth.json 中识别出的 ChatGPT 订阅认证（只读、不导入数据库）。
#[derive(Debug, Clone)]
pub struct ExternalCodexAuth {
    /// ChatGPT workspace ID（tokens.account_id 或 id_token claims）
    pub account_id: String,
    pub email: Option<String>,
    pub access_token: String,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    /// id_token 的 sub：同 workspace 多账号时的所有权判据
    pub user_identity: Option<String>,
    /// 订阅套餐（free/plus/pro/team…），UI 徽标展示用
    pub plan_type: Option<String>,
    /// ChatGPT 订阅到期时间（Unix 毫秒）。
    pub subscription_active_until: Option<i64>,
}

/// 解析 Codex CLI 官方生成的 auth.json。
/// 校验为 ChatGPT 订阅认证（auth_mode=chatgpt 且 access_token 非空）；
/// id_token 缺失或解不出时 account_id 用占位 id。
pub fn parse_external_auth_json(text: &str) -> Option<ExternalCodexAuth> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value["auth_mode"].as_str() != Some("chatgpt") {
        return None;
    }
    let tokens = value.get("tokens")?;
    let access_token = tokens
        .get("access_token")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())?;
    let id_token = tokens
        .get("id_token")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    let refresh_token = tokens
        .get("refresh_token")
        .and_then(|token| token.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    let (jwt_account_id, email) = match id_token.as_deref() {
        Some(id_token) => identity_from_jwt(id_token),
        None => (None, None),
    };
    let user_identity = id_token.as_deref().and_then(extract_user_identity);
    let plan_type = id_token.as_deref().and_then(extract_plan_type);
    let subscription_active_until = id_token
        .as_deref()
        .and_then(extract_subscription_active_until);
    let account_id = jwt_account_id.or_else(|| {
        tokens
            .get("account_id")
            .and_then(|account_id| account_id.as_str())
            .filter(|account_id| !account_id.is_empty())
            .map(str::to_string)
    });
    Some(ExternalCodexAuth {
        account_id: account_id.unwrap_or_else(|| "codex-external".to_string()),
        email,
        access_token: access_token.to_string(),
        id_token,
        refresh_token,
        user_identity,
        plan_type,
        subscription_active_until,
    })
}

/// 浏览器授权完成后的本地成功页（双语，避免后端感知界面语言）。
/// 标签页由系统 shell 拉起、无脚本 opener，window.close 必被拦截，故只做静态提示、不做关闭逻辑。
const BROWSER_SUCCESS_HTML: &str = r#"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><title>Agofra</title>
<style>
body{font-family:system-ui,-apple-system,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0;background:#f5f5f4;color:#292524}
main{text-align:center;padding:2rem}
.success-icon{display:grid;place-items:center;width:56px;height:56px;margin:0 auto;border-radius:50%;background:#16a34a;color:#fff}
.success-icon svg{width:32px;height:32px;fill:none;stroke:currentColor;stroke-width:3;stroke-linecap:round;stroke-linejoin:round}
</style></head><body><main>
<div class="success-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M5 12l4 4L19 6"/></svg></div>
<h1>登录成功</h1>
<p>登录已完成，此标签页可以关闭了。 / Signed in. You can close this tab now.</p>
</main></body></html>"#;

/// 授权被取消/拒绝后的本地失败页
const BROWSER_DENIED_HTML: &str = "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>Agofra</title>\
<style>body{font-family:system-ui,-apple-system,sans-serif;display:grid;place-items:center;min-height:100vh;margin:0;background:#f5f5f4;color:#292524}\
main{text-align:center;padding:2rem}</style>\
</head><body><main><h1>授权未完成</h1><p>Authorization was not completed. You can close this page and try again in Agofra.</p></main></body></html>";

/// 浏览器回调监听线程：阻塞 accept 直到拿到合法回调、授权被拒、超时或取消。
/// 只回环监听、只读请求行，不解析请求体；state 不匹配的回调按过期标签页 400 掉后继续等。
fn serve_browser_callback(
    listener: std::net::TcpListener,
    expected_state: String,
    code_tx: tokio::sync::oneshot::Sender<BrowserCallback>,
) {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(BROWSER_LOGIN_TIMEOUT_SECS);
    let _ = listener.set_nonblocking(true);
    loop {
        // 登录被取消（receiver 已丢弃）时立即退出释放端口；
        // send 只在回调/超时才发生，不能靠它感知取消
        if code_tx.is_closed() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            tauri_plugin_log::log::warn!(
                "[auth.login.browser] stage=awaiting_callback outcome=failure failure_kind=timeout reason=no_valid_callback msg=\"OAuth 等待超时，未收到有效授权回调\""
            );
            let _ = code_tx.send(BrowserCallback::Timeout);
            return;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let Some(target) = read_request_target(&stream) else {
                    tauri_plugin_log::log::warn!(
                        "[auth.login.browser] stage=callback outcome=skipped reason=request_unreadable msg=\"OAuth 本地回调请求读取失败，继续等待\""
                    );
                    continue;
                };
                match parse_callback_query(&target) {
                    Some((code, state)) if state == expected_state => {
                        // 先应答浏览器再退出；接收端已取消时 send 失败，直接收摊释放端口
                        if code_tx.send(BrowserCallback::Code(code)).is_err() {
                            return;
                        }
                        let _ = write_http_response(&stream, 200, "OK", BROWSER_SUCCESS_HTML);
                        return;
                    }
                    // state 不匹配/缺参：可能是残留的旧标签页，回 400 继续等真正的回调
                    Some(_) => {
                        tauri_plugin_log::log::warn!(
                            "[auth.login.browser] stage=callback outcome=skipped reason=state_mismatch msg=\"OAuth 回调不属于本次登录，继续等待\""
                        );
                        let _ = write_http_response(&stream, 400, "Bad Request", "");
                    }
                    None => {
                        // 授权被拒/中断的回调带 error 参数：终止等待，把原因传回 UI
                        if let Some(message) = parse_callback_error(&target) {
                            let _ = code_tx.send(BrowserCallback::Denied(message));
                            let _ = write_http_response(
                                &stream,
                                400,
                                "Bad Request",
                                BROWSER_DENIED_HTML,
                            );
                            return;
                        }
                        if target.split('?').next() == Some("/auth/callback") {
                            tauri_plugin_log::log::warn!(
                                "[auth.login.browser] stage=callback outcome=skipped reason=missing_parameters msg=\"OAuth 回调缺少必要参数，继续等待\""
                            );
                        }
                        let _ = write_http_response(&stream, 404, "Not Found", "");
                    }
                }
            }
            // 无连接时轮询休眠，保证超时期能及时检查 deadline 与取消
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(150)),
        }
    }
}

/// 从连接读出请求行 `GET /path?query HTTP/1.1`，取中间的 path?query（不读请求体）
fn read_request_target(stream: &std::net::TcpStream) -> Option<String> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::BufReader::new(stream).read_line(&mut line).ok()?;
    line.split_whitespace().nth(1).map(str::to_string)
}

fn write_http_response(
    mut stream: &std::net::TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> std::io::Result<()> {
    use std::io::Write;
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// 解析回调目标 `GET /auth/callback?code=...&state=... HTTP/1.1` 的 path?query，
/// 返回 (code, state)；路径不符或缺任一参数返回 None。
fn parse_callback_query(target: &str) -> Option<(String, String)> {
    let (path, query) = target.split_once('?')?;
    if path != "/auth/callback" {
        return None;
    }
    let mut code = None;
    let mut state = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            _ => {}
        }
    }
    Some((code?, state?))
}

/// 解析授权失败的回调（用户取消/拒绝时 OpenAI 回调 `?error=...&error_description=...`）。
/// 路径不符或没有 error 参数返回 None；优先取 error_description，退回 error 原文。
fn parse_callback_error(target: &str) -> Option<String> {
    let (path, query) = target.split_once('?')?;
    if path != "/auth/callback" {
        return None;
    }
    let mut error = None;
    let mut description = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }
    let error = error?;
    Some(description.unwrap_or(error))
}

/// 拼授权 URL：参数与官方 Codex CLI 保持一致（prompt=login 强制弹登录/选号，便于连登多个账号）
fn build_authorize_url(state: &str, code_challenge: &str) -> String {
    let pairs = [
        ("client_id", CODEX_CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", BROWSER_REDIRECT_URI),
        ("scope", OAUTH_SCOPE_BROWSER),
        ("state", state),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("prompt", "login"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
    ];
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    format!("{BROWSER_AUTHORIZE_URL}?{query}")
}

/// 32 字节系统熵源 → base64url（43 字符，满足 PKCE verifier 与 OAuth state 的强度要求）
fn random_urlsafe_token() -> String {
    let mut bytes = [0u8; 32];
    // 熵源不可用属于无法恢复的系统故障，必须立即失败而不是降级弱随机
    getrandom::fill(&mut bytes).expect("系统熵源不可用");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// PKCE S256 challenge：base64url(sha256(verifier))
fn pkce_code_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// token 数学要 i64；"当前时刻"唯一定义在 paths，这里只做类型适配。
fn now_ms() -> i64 {
    crate::paths::now_ms() as i64
}

fn now_secs() -> i64 {
    crate::paths::now_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(dir: &std::path::Path) -> Arc<Database> {
        let paths = crate::paths::from_home(dir).unwrap();
        Arc::new(Database::open(&paths).unwrap())
    }

    #[test]
    fn unmanaged_live_sync_logs_once_per_workspace() {
        let mut last = None;
        assert!(should_log_unmanaged_live_sync(&mut last, "workspace-a"));
        assert!(!should_log_unmanaged_live_sync(&mut last, "workspace-a"));
        assert!(should_log_unmanaged_live_sync(&mut last, "workspace-b"));
    }

    #[test]
    fn parse_jwt_claims_extracts_account_and_email() {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD
            .encode(b"{\"chatgpt_account_id\":\"acc-123\",\"email\":\"test@example.com\"}");
        let claims = parse_jwt_claims(&format!("{header}.{payload}.")).unwrap();
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("acc-123"));
        assert_eq!(claims.email.as_deref(), Some("test@example.com"));
    }

    #[test]
    fn parse_jwt_claims_rejects_malformed() {
        assert!(parse_jwt_claims("not-a-jwt").is_none());
        assert!(parse_jwt_claims("only.two").is_none());
    }

    /// plan_type 在 `https://api.openai.com/auth` 嵌套 claim 里（同官方 token 形态）
    fn plan_token(plan: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "https://api.openai.com/auth": { "chatgpt_plan_type": plan }
            })
            .to_string(),
        );
        format!("{header}.{payload}.")
    }

    #[test]
    fn extract_plan_type_reads_nested_claim_and_normalizes() {
        assert_eq!(
            extract_plan_type(&plan_token("Plus")).as_deref(),
            Some("plus")
        );
        // 空串/缺 claim：视为未知套餐
        assert_eq!(extract_plan_type(&plan_token("  ")), None);
        assert_eq!(extract_plan_type("not-a-jwt"), None);
    }

    #[test]
    fn extract_subscription_active_until_normalizes_timestamp_and_rfc3339() {
        let token = |value: serde_json::Value| {
            let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
            let payload = URL_SAFE_NO_PAD.encode(
                serde_json::json!({
                    "https://api.openai.com/auth": {
                        "chatgpt_subscription_active_until": value,
                    }
                })
                .to_string(),
            );
            format!("{header}.{payload}.")
        };
        assert_eq!(
            extract_subscription_active_until(&token(serde_json::json!(1_789_000_000))),
            Some(1_789_000_000_000)
        );
        assert_eq!(
            extract_subscription_active_until(&token(serde_json::json!("2026-09-18T01:29:00Z"))),
            Some(1_789_694_940_000)
        );
        assert_eq!(extract_subscription_active_until("not-a-jwt"), None);
    }

    #[test]
    fn parse_external_auth_json_recognizes_chatgpt_shape() {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let payload = URL_SAFE_NO_PAD
            .encode(b"{\"chatgpt_account_id\":\"acc-123\",\"email\":\"test@example.com\"}");
        let id_token = format!("{header}.{payload}.");
        let json = format!(
            r#"{{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{{"access_token":"at-1","id_token":"{id_token}","refresh_token":"rt-1","account_id":"acc-123"}},"last_refresh":"2026-08-17T09:15:01Z"}}"#
        );
        let auth = parse_external_auth_json(&json).unwrap();
        assert_eq!(auth.account_id, "acc-123");
        assert_eq!(auth.email.as_deref(), Some("test@example.com"));
        assert_eq!(auth.access_token, "at-1");
        assert_eq!(auth.id_token.as_deref(), Some(id_token.as_str()));
        assert_eq!(auth.refresh_token.as_deref(), Some("rt-1"));
    }

    #[test]
    fn parse_external_auth_json_rejects_non_chatgpt_or_invalid() {
        assert!(parse_external_auth_json(r#"{"auth_mode":"api"}"#).is_none());
        assert!(parse_external_auth_json(r#"{"auth_mode":"chatgpt"}"#).is_none());
        assert!(parse_external_auth_json(
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":""}}"#
        )
        .is_none());
        assert!(parse_external_auth_json("not-json").is_none());
    }

    #[test]
    fn cached_token_expiry_window() {
        let now = now_ms();
        assert!(CachedAccessToken {
            token: "t".into(),
            expires_at_ms: now + 30_000,
        }
        .is_expiring_soon());
        assert!(!CachedAccessToken {
            token: "t".into(),
            expires_at_ms: now + 3_600_000,
        }
        .is_expiring_soon());
    }

    #[test]
    fn auth_snapshot_access_token_requires_matching_identity_and_valid_expiry() {
        let access_token = test_access_token(
            "workspace-a",
            chrono::Utc::now().timestamp().saturating_add(3_600),
        );
        let id_token = test_id_token("subject-a", "workspace-a");
        let auth_json = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token,
                "id_token": id_token,
                "refresh_token": "snapshot-refresh",
                "account_id": "workspace-a",
            },
        })
        .to_string();
        let account = CodexAccountData {
            account_id: "row-a".to_string(),
            email: None,
            id_token: Some(id_token),
            refresh_token: "rejected-refresh".to_string(),
            auth_json: Some(auth_json),
            authenticated_at: 0,
            chatgpt_account_id: Some("workspace-a".to_string()),
            user_identity: Some("subject-a".to_string()),
            plan_type: None,
        };

        let cached = reusable_auth_snapshot_access_token(&account).unwrap();
        assert_eq!(cached.token, access_token);
        assert!(!cached.is_expiring_soon());

        let mut wrong_workspace = account.clone();
        wrong_workspace.chatgpt_account_id = Some("workspace-b".to_string());
        assert!(reusable_auth_snapshot_access_token(&wrong_workspace).is_none());

        let mut wrong_user = account.clone();
        wrong_user.user_identity = Some("subject-b".to_string());
        assert!(reusable_auth_snapshot_access_token(&wrong_user).is_none());

        let expired_access_token = test_access_token(
            "workspace-a",
            chrono::Utc::now().timestamp().saturating_add(30),
        );
        let mut expiring = account;
        expiring.auth_json = Some(
            serde_json::json!({
                "auth_mode": "chatgpt",
                "tokens": {
                    "access_token": expired_access_token,
                    "id_token": test_id_token("subject-a", "workspace-a"),
                    "refresh_token": "snapshot-refresh",
                    "account_id": "workspace-a",
                },
            })
            .to_string(),
        );
        assert!(reusable_auth_snapshot_access_token(&expiring).is_none());
    }

    #[tokio::test]
    async fn valid_auth_snapshot_token_is_used_before_refreshing_account() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let manager = CodexOAuthManager::new(database);
        let account = manager
            .add_account_internal(test_credentials(
                "workspace-a",
                "rejected-refresh",
                Some("a@example.com"),
                Some(test_id_token("subject-a", "workspace-a")),
            ))
            .await
            .unwrap();
        let access_token = test_access_token(
            "workspace-a",
            chrono::Utc::now().timestamp().saturating_add(3_600),
        );
        let auth_json = serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token,
                "id_token": test_id_token("subject-a", "workspace-a"),
                "refresh_token": "rejected-refresh",
                "account_id": "workspace-a",
            },
        })
        .to_string();
        let account_for_save = {
            let mut accounts = manager.accounts.write().await;
            let account_data = accounts.get_mut(&account.id).unwrap();
            account_data.auth_json = Some(auth_json);
            account_data.clone()
        };
        manager.save_account(&account_for_save).unwrap();

        assert_eq!(
            manager
                .get_valid_token_for_account(&account.id)
                .await
                .unwrap(),
            access_token
        );
    }

    fn test_access_token(chatgpt_account_id: &str, exp: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({ "chatgpt_account_id": chatgpt_account_id, "exp": exp }).to_string(),
        );
        format!("{header}.{payload}.")
    }

    #[tokio::test]
    async fn manager_persists_accounts_to_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let added_id = {
            let manager = CodexOAuthManager::new(database.clone());
            manager
                .add_account_internal(test_credentials(
                    "acc-123",
                    "rt-secret",
                    Some("user@example.com"),
                    Some("id-jwt".to_string()),
                ))
                .await
                .unwrap()
                .id
        };
        let manager = CodexOAuthManager::new(database.clone());
        let status = manager.get_status().await;
        assert_eq!(status.accounts.len(), 1);
        // 本地主键与 workspace 解耦：行 id 是新生成的，但重启后保持稳定
        assert_eq!(status.accounts[0].id, added_id);
        assert_ne!(added_id, "acc-123");
        assert_eq!(status.accounts[0].login, "user@example.com");
        assert!(status.accounts[0].is_default);
        let stored = database.accounts().unwrap().pop().unwrap();
        assert_eq!(stored.chatgpt_account_id.as_deref(), Some("acc-123"));
    }

    #[tokio::test]
    async fn manager_remove_account_updates_default() {
        let dir = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(setup(dir.path()));
        let first = manager
            .add_account_internal(test_credentials(
                "acc-123",
                "rt",
                Some("a@example.com"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            manager.resolve_default_account_id().await.as_deref(),
            Some(first.id.as_str())
        );
        let second = manager
            .add_account_internal(test_credentials(
                "acc-456",
                "rt2",
                Some("b@example.com"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            manager.resolve_default_account_id().await.as_deref(),
            Some(first.id.as_str())
        );

        manager.remove_account(&first.id).await.unwrap();
        let accounts = manager.get_status().await.accounts;
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, second.id);
    }

    #[tokio::test]
    async fn sync_external_auth_updates_only_existing_account() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let manager = CodexOAuthManager::new(database.clone());
        manager
            .add_account_internal(test_credentials(
                "acc-1",
                "old-refresh",
                Some("old@example.com"),
                Some("old-id".to_string()),
            ))
            .await
            .unwrap();
        let auth = format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"new-id","access_token":"{}","refresh_token":"new-refresh","account_id":"acc-1"}}}}"#,
            test_access_token_with_iat(now_secs() + 60)
        );
        let auth = auth.as_str();

        assert!(manager.sync_external_auth_json(auth).await.unwrap());
        assert!(!manager.sync_external_auth_json(auth).await.unwrap());
        let stored = database.accounts().unwrap().pop().unwrap();
        assert_eq!(stored.refresh_token, "new-refresh");
        assert_eq!(stored.id_token.as_deref(), Some("new-id"));
        assert_eq!(stored.auth_json.as_deref(), Some(auth));

        let unknown = auth.replace("acc-1", "unknown");
        assert!(!manager.sync_external_auth_json(&unknown).await.unwrap());
        assert_eq!(manager.get_status().await.accounts.len(), 1);
    }

    #[tokio::test]
    async fn sync_external_auth_rejects_live_older_than_relogin() {
        // 给激活中的账号重新授权后，live auth.json 还是上一会话的旧残留：
        // workspace 归属匹配也必须拒绝回写，否则旧凭证会覆盖刚登录的新凭证
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let manager = CodexOAuthManager::new(database.clone());
        manager
            .add_account_internal(test_credentials(
                "acc-1",
                "fresh-refresh",
                Some("a@example.com"),
                Some(test_id_token("user-a", "acc-1")),
            ))
            .await
            .unwrap();

        // access_token 不可解析（损坏/异常文件）：无法证明比账号新，拒绝
        let garbage = r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x","access_token":"corrupted-access","refresh_token":"corrupted-refresh","account_id":"acc-1"}}"#;
        assert!(!manager.sync_external_auth_json(garbage).await.unwrap());

        // iat 早于重新登录时刻的旧会话文件：拒绝
        let older = format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{}","access_token":"{}","refresh_token":"old-session-refresh","account_id":"acc-1"}}}}"#,
            test_id_token("user-a", "acc-1"),
            test_access_token_with_iat(now_secs() - 3_600)
        );
        assert!(!manager.sync_external_auth_json(&older).await.unwrap());

        let stored = database.accounts().unwrap().pop().unwrap();
        assert_eq!(stored.refresh_token, "fresh-refresh");
        assert_eq!(stored.auth_json, None);

        // iat 更新的外部刷新仍然被吸收（保护既有能力）
        let newer = format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{}","access_token":"{}","refresh_token":"newer-refresh","account_id":"acc-1"}}}}"#,
            test_id_token("user-a", "acc-1"),
            test_access_token_with_iat(now_secs() + 60)
        );
        assert!(manager.sync_external_auth_json(&newer).await.unwrap());
        let stored = database.accounts().unwrap().pop().unwrap();
        assert_eq!(stored.refresh_token, "newer-refresh");
    }

    #[tokio::test]
    async fn codex_auth_json_matches_official_chatgpt_shape() {
        let dir = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(setup(dir.path()));
        let row_id = manager
            .add_account_internal(test_credentials(
                "acc-1",
                "rt-1",
                Some("a@example.com"),
                Some("id-jwt".to_string()),
            ))
            .await
            .unwrap()
            .id;
        manager.access_tokens.write().await.insert(
            row_id.clone(),
            CachedAccessToken {
                token: "at-1".to_string(),
                expires_at_ms: now_ms() + 3_600_000,
            },
        );

        let json = manager.codex_auth_json(&row_id).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["auth_mode"], "chatgpt");
        assert!(value["OPENAI_API_KEY"].is_null());
        assert_eq!(value["tokens"]["id_token"], "id-jwt");
        assert_eq!(value["tokens"]["access_token"], "at-1");
        assert_eq!(value["tokens"]["refresh_token"], "rt-1");
        assert_eq!(value["tokens"]["account_id"], "acc-1");
        assert!(value["last_refresh"].is_string());
    }

    /// 构造带 sub 与 workspace 的 id_token（同官方 token 的 claim 形态）
    fn test_id_token(sub: &str, chatgpt_account_id: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({ "sub": sub, "chatgpt_account_id": chatgpt_account_id }).to_string(),
        );
        format!("{header}.{payload}.")
    }

    /// 构造带 iat 的 access_token JWT（验证 live 同步的新旧闸门）
    fn test_access_token_with_iat(iat: i64) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::json!({ "iat": iat }).to_string());
        format!("{header}.{payload}.")
    }

    /// 测试用登录凭证：id_token 的 sub/plan 提取逻辑与生产路径一致
    fn test_credentials(
        chatgpt_account_id: &str,
        refresh_token: &str,
        email: Option<&str>,
        id_token: Option<String>,
    ) -> LoginCredentials {
        let user_identity = id_token.as_deref().and_then(extract_user_identity);
        let plan_type = id_token.as_deref().and_then(extract_plan_type);
        LoginCredentials {
            chatgpt_account_id: chatgpt_account_id.to_string(),
            refresh_token: refresh_token.to_string(),
            email: email.map(str::to_string),
            id_token,
            user_identity,
            plan_type,
        }
    }

    #[tokio::test]
    async fn login_same_workspace_and_user_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let manager = CodexOAuthManager::new(database.clone());
        let id_token = test_id_token("user-a", "ws-1");
        let first = manager
            .add_account_internal(test_credentials(
                "ws-1",
                "rt-1",
                Some("a@example.com"),
                Some(id_token.clone()),
            ))
            .await
            .unwrap();
        let second = manager
            .add_account_internal(test_credentials(
                "ws-1",
                "rt-2",
                Some("a@example.com"),
                Some(id_token),
            ))
            .await
            .unwrap();

        // 幂等更新：不产生新行、行 id 不变、refresh_token 已刷新
        assert_eq!(first.id, second.id);
        assert_eq!(manager.get_status().await.accounts.len(), 1);
        let stored = database.accounts().unwrap().pop().unwrap();
        assert_eq!(stored.refresh_token, "rt-2");
        assert_eq!(stored.user_identity.as_deref(), Some("user-a"));
        assert_eq!(stored.chatgpt_account_id.as_deref(), Some("ws-1"));
    }

    #[tokio::test]
    async fn login_same_workspace_different_users_coexist() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let manager = CodexOAuthManager::new(database.clone());
        let first = manager
            .add_account_internal(test_credentials(
                "ws-shared",
                "rt-a",
                Some("a@example.com"),
                Some(test_id_token("user-a", "ws-shared")),
            ))
            .await
            .unwrap();
        let second = manager
            .add_account_internal(test_credentials(
                "ws-shared",
                "rt-b",
                Some("b@example.com"),
                Some(test_id_token("user-b", "ws-shared")),
            ))
            .await
            .unwrap();

        // 同 workspace 两个用户各自成行，互不覆盖
        assert_ne!(first.id, second.id);
        assert_eq!(manager.get_status().await.accounts.len(), 2);
        let stored = database.accounts().unwrap();
        assert_eq!(stored.len(), 2);
        assert!(stored
            .iter()
            .all(|account| account.chatgpt_account_id.as_deref() == Some("ws-shared")));

        // 两行的 auth.json 出站 account_id 都是同一个 workspace ID
        manager.seed_access_token_for_test(&first.id, "at-a").await;
        manager.seed_access_token_for_test(&second.id, "at-b").await;
        for (row_id, token) in [(&first.id, "at-a"), (&second.id, "at-b")] {
            let value: serde_json::Value =
                serde_json::from_str(&manager.codex_auth_json(row_id).await.unwrap()).unwrap();
            assert_eq!(value["tokens"]["account_id"], "ws-shared");
            assert_eq!(value["tokens"]["access_token"], token);
        }
    }

    #[tokio::test]
    async fn sync_external_auth_never_crosses_users_in_same_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let manager = CodexOAuthManager::new(database.clone());
        manager
            .add_account_internal(test_credentials(
                "ws-shared",
                "rt-a",
                Some("a@example.com"),
                Some(test_id_token("user-a", "ws-shared")),
            ))
            .await
            .unwrap();
        manager
            .add_account_internal(test_credentials(
                "ws-shared",
                "rt-b",
                Some("b@example.com"),
                Some(test_id_token("user-b", "ws-shared")),
            ))
            .await
            .unwrap();

        // Codex CLI 用 user-b 的凭证刷新了 live auth.json：只允许落进 user-b 的行
        let live = format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{}","access_token":"{}","refresh_token":"rt-b2","account_id":"ws-shared"}}}}"#,
            test_id_token("user-b", "ws-shared"),
            test_access_token_with_iat(now_secs() + 60)
        );
        assert!(manager.sync_external_auth_json(&live).await.unwrap());

        let stored = database.accounts().unwrap();
        let row_a = stored
            .iter()
            .find(|account| account.user_identity.as_deref() == Some("user-a"))
            .unwrap();
        let row_b = stored
            .iter()
            .find(|account| account.user_identity.as_deref() == Some("user-b"))
            .unwrap();
        assert_eq!(row_a.refresh_token, "rt-a");
        assert_eq!(row_b.refresh_token, "rt-b2");
        assert_eq!(row_a.auth_json, None);
        assert!(row_b.auth_json.is_some());
        assert_eq!(manager.get_status().await.accounts.len(), 2);
    }

    #[tokio::test]
    async fn insert_account_if_absent_rejects_duplicate_identity() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        let make = |id: &str| StoredAccount {
            id: id.to_string(),
            email: None,
            id_token: None,
            refresh_token: "rt".to_string(),
            auth_json: None,
            authenticated_at: 1,
            chatgpt_account_id: Some("ws-1".to_string()),
            user_identity: Some("user-a".to_string()),
            plan_type: None,
        };
        assert!(database.insert_account_if_absent(&make("row-1")).unwrap());
        // 并发登录抢到同一 (workspace, sub)：唯一索引兜底，调用方转更新
        assert!(!database.insert_account_if_absent(&make("row-2")).unwrap());
        assert_eq!(database.accounts().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn legacy_row_without_id_token_claims_via_email_match() {
        let dir = tempfile::tempdir().unwrap();
        let database = setup(dir.path());
        // 模拟迁移后无 id_token 的存量行：身份两列拿不到 sub（须先于 manager 构造存在）
        database
            .upsert_account(&StoredAccount {
                id: "ws-legacy".to_string(),
                email: Some("a@example.com".to_string()),
                id_token: None,
                refresh_token: "rt-legacy".to_string(),
                auth_json: None,
                authenticated_at: 1,
                chatgpt_account_id: Some("ws-legacy".to_string()),
                user_identity: None,
                plan_type: None,
            })
            .unwrap();
        let manager = CodexOAuthManager::new(database.clone());

        let relogin = manager
            .add_account_internal(test_credentials(
                "ws-legacy",
                "rt-new",
                Some("a@example.com"),
                Some(test_id_token("user-a", "ws-legacy")),
            ))
            .await
            .unwrap();

        // workspace + email 命中存量行：原地更新并补齐身份，不产生新行
        assert_eq!(relogin.id, "ws-legacy");
        assert_eq!(manager.get_status().await.accounts.len(), 1);
        let stored = database.accounts().unwrap().pop().unwrap();
        assert_eq!(stored.refresh_token, "rt-new");
        assert_eq!(stored.user_identity.as_deref(), Some("user-a"));
    }

    #[test]
    fn browser_authorize_url_carries_required_params() {
        let url = build_authorize_url("st-ate-123", "challenge-xyz");
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?client_id="));
        // redirect_uri 是官方 client_id 的注册白名单项，编码后必须逐字匹配
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        // form_urlencoded 空格编码为 `+`，与官方 Codex CLI 的 URLSearchParams 一致
        assert!(url.contains("scope=openid+email+profile+offline_access"));
        assert!(url.contains("state=st-ate-123"));
        assert!(url.contains("code_challenge=challenge-xyz"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("prompt=login"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
    }

    #[test]
    fn browser_callback_ignores_invalid_requests_and_accepts_current_login() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            serve_browser_callback(listener, "current".into(), tx);
        });
        for (target, status) in [
            ("/auth/callback?code=secret&state=old", "400"),
            ("/auth/callback?state=current", "404"),
            ("/auth/callback?code=secret&state=current", "200"),
        ] {
            let mut stream = std::net::TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            write!(stream, "GET {target} HTTP/1.1\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with(&format!("HTTP/1.1 {status}")));
            assert!(!response.contains("secret"));
        }
        assert!(
            matches!(rx.blocking_recv().unwrap(), BrowserCallback::Code(code) if code == "secret")
        );
        server.join().unwrap();
    }

    #[test]
    fn browser_success_page_is_static_without_close_logic() {
        // 标签页无脚本 opener，window.close 必被拦截：成功页只做静态提示
        assert!(BROWSER_SUCCESS_HTML.contains("M5 12l4 4L19 6"));
        assert!(BROWSER_SUCCESS_HTML.contains("此标签页可以关闭了"));
        assert!(!BROWSER_SUCCESS_HTML.contains("window.close"));
        assert!(!BROWSER_SUCCESS_HTML.contains("<button"));
        assert!(!BROWSER_SUCCESS_HTML.contains("<script"));
    }

    #[test]
    fn parse_callback_query_extracts_code_and_state() {
        let (code, state) =
            parse_callback_query("/auth/callback?code=abc.123_def&state=st%2Date").unwrap();
        assert_eq!(code, "abc.123_def");
        assert_eq!(state, "st-ate");
    }

    #[test]
    fn parse_callback_query_rejects_wrong_path_or_missing_params() {
        assert!(parse_callback_query("/auth/callback").is_none());
        assert!(parse_callback_query("/success?code=a&state=b").is_none());
        assert!(parse_callback_query("/auth/callback?code=a").is_none());
        assert!(parse_callback_query("/auth/callback?state=b").is_none());
    }

    #[test]
    fn parse_callback_error_prefers_description_over_code() {
        // 有 error_description 时优先展示
        assert_eq!(
            parse_callback_error(
                "/auth/callback?error=access_denied&error_description=User%20cancelled%20sign-in"
            )
            .as_deref(),
            Some("User cancelled sign-in")
        );
        // 只有 error 时退回原文
        assert_eq!(
            parse_callback_error("/auth/callback?error=access_denied").as_deref(),
            Some("access_denied")
        );
        // 无 error 或路径不符：不是授权失败回调
        assert_eq!(parse_callback_error("/auth/callback?code=a&state=b"), None);
        assert_eq!(parse_callback_error("/success?error=oops"), None);
    }

    #[test]
    fn parse_callback_query_decodes_percent_and_plus() {
        let (code, state) =
            parse_callback_query("/auth/callback?code=a%20b+c&state=%2Fauth%3F").unwrap();
        assert_eq!(code, "a b c");
        assert_eq!(state, "/auth?");
        // 残缺的百分号序列按字面量保留
        let (code, _) = parse_callback_query("/auth/callback?code=100%&state=x").unwrap();
        assert_eq!(code, "100%");
    }

    #[test]
    fn pkce_challenge_is_base64_of_sha256_verifier() {
        use sha2::{Digest, Sha256};
        let verifier = "test-verifier";
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(pkce_code_challenge(verifier), expected);
        // RFC 7636：32 字节熵源的 base64url 无填充正好 43 字符
        assert_eq!(random_urlsafe_token().len(), 43);
    }
}
