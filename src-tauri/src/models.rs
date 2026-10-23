use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::auth::codex_oauth::AuthStatus;

/// 供应商类型：官方订阅（ChatGPT）或第三方供应商（Codex 协议）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexProfileKind {
    Official,
    ThirdParty,
}

impl CodexProfileKind {
    pub fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "official" => Some(Self::Official),
            "third_party" => Some(Self::ThirdParty),
            _ => None,
        }
    }

    pub fn as_db(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::ThirdParty => "third_party",
        }
    }
}

/// 官方 ChatGPT 配置的固定认证来源；OAuth 账号只能在 OAuth 来源内切换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Desktop,
    Oauth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CodexProfilePayload {
    /// 仅供 Agmux 卡片展示，不写入 Codex 配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub model_values: BTreeMap<String, String>,
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub provider_body: Option<String>,
    /// 内置官方供应商类型（deepseek/minimax/zhipu/chatgpt）；普通捕获的供应商为 None。
    #[serde(default)]
    pub builtin: Option<String>,
    /// 官方配置创建时确定的认证来源；旧数据缺失时由 account_id 兼容推断。
    #[serde(default)]
    pub auth_source: Option<AuthSource>,
    /// 供应商自己保存的完整 config 原文（内置供应商可全量编辑；普通供应商无该字段）。
    #[serde(default)]
    pub raw_config: Option<String>,
    /// 供应商自己保存的 models.json 原文（编辑后随供应商应用写入 ~/.codex）。
    #[serde(default)]
    pub raw_catalog: Option<String>,
    /// Desktop/第三方供应商保存的 auth.json 原文；OAuth 官方配置不吸收 live 快照。
    #[serde(default)]
    pub raw_auth: Option<String>,
    /// 历史快照标记；Some(false) 表示用户明确手动接管认证内容。
    #[serde(default)]
    pub auth_auto_sync: Option<bool>,
    /// 模型提供方的管理后台网址（卡片显示跳转按钮）。
    #[serde(default)]
    pub admin_url: Option<String>,
    /// 供应商级开关：是否在卡片显示并自动刷新余额/用量（默认关，用户自行开启）。
    #[serde(default = "default_false")]
    pub show_balance: bool,
    /// 最近一次从供应商接口获取的模型 ID，供编辑页离线复用。
    #[serde(default)]
    pub fetched_models: Vec<String>,
    /// 最近获取的模型推理档位；与模型 ID 一起缓存，不写入实时配置。
    #[serde(default)]
    pub fetched_model_efforts: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub fetched_model_defaults: std::collections::BTreeMap<String, String>,
}

impl CodexProfilePayload {
    /// 同账号刷新 token 不影响目录；换 workspace 或用户后不能复用旧目录。
    pub(crate) fn invalidate_models_for_auth_change(&mut self, previous_auth: Option<&str>) {
        let identity = |text: Option<&str>| {
            text.and_then(crate::auth::codex_oauth::parse_external_auth_json)
                .map(|auth| (auth.account_id, auth.user_identity))
        };
        if identity(previous_auth) != identity(self.raw_auth.as_deref()) {
            self.fetched_models.clear();
            self.fetched_model_efforts.clear();
            self.fetched_model_defaults.clear();
        }
    }

    pub fn effective_auth_source(
        &self,
        kind: CodexProfileKind,
        account_id: Option<&str>,
    ) -> Option<AuthSource> {
        if kind != CodexProfileKind::Official {
            return None;
        }
        Some(self.auth_source.unwrap_or(if account_id.is_some() {
            AuthSource::Oauth
        } else {
            AuthSource::Desktop
        }))
    }
}

fn default_false() -> bool {
    false
}

/// ~/.codex/config.toml [mcp_servers.*] 的一条服务器配置（建模字段子集；未建模键由 toml_edit 原样保留）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct McpServerSpec {
    pub name: String,
    /// None = 未写入该键（Codex 默认启用）；Some(false) 显式停用。
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub startup_timeout_sec: Option<i64>,
    #[serde(default)]
    pub tool_timeout_sec: Option<i64>,
    /// STDIO 传输：有 command 无 url。
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Streamable HTTP 传输：有 url 无 command。
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub bearer_token_env_var: Option<String>,
    #[serde(default)]
    pub http_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub env_http_headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpTool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpProbeResult {
    pub ok: bool,
    pub connection_state: Option<McpConnectionState>,
    pub auth_status: Option<String>,
    pub latency_ms: Option<u128>,
    pub status: Option<u16>,
    pub protocol_version: Option<String>,
    pub server_info: Option<McpServerInfo>,
    pub tools: Vec<McpTool>,
    pub tools_truncated: bool,
    pub error: Option<String>,
    pub tools_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConnectionState {
    Connected,
    AuthenticationRequired,
    Failed,
    Unknown,
}

/// MCP 同步预览的一条差异（live = ~/.codex/config.toml，db = 数据库镜像）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpSyncEntryKind {
    /// 仅配置文件有（导入数据库会新增；恢复到配置会被删除）。
    LiveOnly,
    /// 仅数据库有（恢复到配置会加回；导入数据库会被清除）。
    DbOnly,
    /// 两侧都有但内容不同。
    Changed,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpSyncDiffEntry {
    pub name: String,
    pub kind: McpSyncEntryKind,
    pub live_spec: Option<McpServerSpec>,
    pub db_spec: Option<McpServerSpec>,
    /// 两侧的原始 TOML 片段（单侧独有时另一侧为 None），展开明细时逐行对比展示。
    pub live_toml: Option<String>,
    pub db_toml: Option<String>,
}

/// 差异批量处理的一条动作：fragment=Some 写入该侧内容，fragment=None 表示删除该侧条目。
#[derive(Debug, Clone, Deserialize)]
pub struct McpDiffEntryAction {
    pub name: String,
    #[serde(default)]
    pub fragment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpSyncPreview {
    pub entries: Vec<McpSyncDiffEntry>,
    pub live_count: usize,
    pub db_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexProfileSummary {
    pub id: String,
    pub name: String,
    pub kind: CodexProfileKind,
    /// 官方配置绑定的订阅账号；第三方恒为 None。
    pub account_id: Option<String>,
    pub auth_source: Option<AuthSource>,
    /// 账号页额度查询使用的认证账号标识；OAuth 为本地账号主键，Desktop 为 workspace ID。
    pub auth_account_id: Option<String>,
    /// 绑定账号的订阅套餐；第三方或未知为 None（get_state 聚合时填充）。
    pub plan_type: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub reasoning_effort: Option<String>,
    pub fast_mode: bool,
    /// 供应商是否已配置非空 API 端点
    pub has_base_url: bool,
    /// 供应商是否已配置有效 API 密钥（占位符视为未配置）
    pub has_key: bool,
    pub admin_url: Option<String>,
    pub show_balance: bool,
    pub icon: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatgptResetCredit {
    pub id: String,
    /// 官方返回的重置范围（如 full）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_type: Option<String>,
    /// 主动重置卡的到期时间（Unix 毫秒）；无到期时间时为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileBalanceInfo {
    pub currency: String,
    pub total_balance: String,
    /// 用量型供应商（如 MiniMax Token Plan）的已用百分比；余额型供应商为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_percent: Option<u32>,
    /// 主用量窗口重置倒计时（如 "2h23m"）；余额型供应商为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_reset: Option<String>,
    /// 主用量窗口重置时间（Unix 毫秒）；余额型供应商为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_reset_at: Option<i64>,
    /// 主用量窗口名称；由接口返回的窗口长度推断。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_label: Option<String>,
    /// 次用量窗口已用百分比；仅用量型供应商返回。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_usage_percent: Option<u32>,
    /// 次用量窗口重置倒计时（如 "5d21h"）；仅用量型供应商返回。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_reset: Option<String>,
    /// 次用量窗口重置时间（Unix 毫秒）；仅用量型供应商返回。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_reset_at: Option<i64>,
    /// 次用量窗口名称；免费方案可为“30天”。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_label: Option<String>,
    /// Codex 主动重置卡可用次数；仅 ChatGPT 订阅账号返回。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits_available: Option<i64>,
    /// 可用主动重置卡的到期时间；详情接口不可用时为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<Vec<ChatgptResetCredit>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexProfileDetail {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// 官方配置绑定的订阅账号；第三方恒为 None。
    pub account_id: Option<String>,
    pub auth_source: Option<AuthSource>,
    /// Desktop 配置自身 auth.json 解析出的登录账号；OAuth 由 account_id 管理。
    pub desktop_login: Option<String>,
    pub icon: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_values: std::collections::BTreeMap<String, String>,
    pub config_fragment: String,
    pub raw_config: Option<String>,
    pub catalog_content: Option<String>,
    pub raw_catalog: Option<String>,
    pub raw_auth: Option<String>,
    pub admin_url: Option<String>,
    pub show_balance: bool,
    pub fetched_models: Vec<String>,
    pub updated_at: String,
    pub fetched_model_efforts: std::collections::BTreeMap<String, Vec<String>>,
    pub fetched_model_defaults: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrayClickAction {
    #[default]
    ShowWindow,
    ShowMenu,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Auto,
    Off,
    Custom,
}

/// macOS 供应商终端的宿主终端：Terminal.app 系统必带；Ghostty 走官方 CLI。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MacosTerminal {
    #[default]
    Terminal,
    Ghostty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    /// 界面语言；"system" 表示跟随系统语言。
    pub language: String,
    pub auto_restart: bool,
    pub autostart_enabled: bool,
    pub silent_start: bool,
    pub minimize_to_tray: bool,
    pub tray_click_action: TrayClickAction,
    /// 冷启动落页：前端 AppView 标识原样透传，不解析；非法值由前端回落默认页。
    pub startup_view: String,
    /// macOS 供应商终端的宿主终端；Windows 分支不读取此字段。
    pub macos_terminal: MacosTerminal,
    pub auto_check_update: bool,
    pub auto_backup_interval_hours: u64,
    pub database_backup_keep_count: u32,
    pub proxy_mode: ProxyMode,
    pub proxy_url: String,
    /// 供应商终端的最近工作目录（最近使用在前）。卡片下拉框的新建/删除直接改这里，
    /// 与 ~/.claude 无关，不参与应用与回写。
    pub claude_terminal_dirs: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            language: "system".into(),
            auto_restart: false,
            autostart_enabled: false,
            silent_start: false,
            minimize_to_tray: false,
            tray_click_action: TrayClickAction::ShowWindow,
            startup_view: "codexProfiles".into(),
            macos_terminal: MacosTerminal::Terminal,
            auto_check_update: true,
            auto_backup_interval_hours: 0,
            database_backup_keep_count: 5,
            proxy_mode: ProxyMode::Auto,
            proxy_url: String::new(),
            claude_terminal_dirs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::{ProxyMode, Settings, TrayClickAction};

    #[test]
    fn proxy_settings_default_to_auto_and_round_trip_custom_address() {
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy, Settings::default());
        assert_eq!(legacy.proxy_mode, ProxyMode::Auto);
        assert_eq!(legacy.startup_view, "codexProfiles");
        assert!(legacy.proxy_url.is_empty());
        let custom = Settings {
            proxy_mode: ProxyMode::Custom,
            proxy_url: "https://proxy.invalid:8443".into(),
            ..legacy
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&custom).unwrap()).unwrap();
        assert_eq!(custom, restored);
        assert!(serde_json::from_str::<Settings>(r#"{"proxy_mode":"invalid"}"#).is_err());
    }

    #[test]
    fn tray_click_action_defaults_for_existing_settings_and_accepts_only_two_modes() {
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.tray_click_action, TrayClickAction::ShowWindow);

        let menu: Settings = serde_json::from_str(r#"{"tray_click_action":"show_menu"}"#).unwrap();
        assert_eq!(menu.tray_click_action, TrayClickAction::ShowMenu);
        assert!(serde_json::from_str::<Settings>(r#"{"tray_click_action":"invalid"}"#).is_err());
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PathInfo {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexAppStatus {
    pub running: bool,
    pub display_path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppState {
    pub codex_profiles: Vec<CodexProfileSummary>,
    pub active_codex_profile_id: Option<String>,
    /// Claude Code 侧当前激活的供应商（claude_profiles 表）。
    pub active_claude_profile_id: Option<String>,
    pub codex: CodexAppStatus,
    pub settings: Settings,
    pub paths: Vec<PathInfo>,
    pub auth_status: AuthStatus,
    /// 供应商级余额缓存（上次成功查询结果），保证卡片静默显示、切换不闪烁。
    pub balance_cache: std::collections::BTreeMap<String, ProfileBalanceInfo>,
}

/// Claude Code 供应商列表摘要：token 只回是否已设置，明文只在详情里回显。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClaudeProfileSummary {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    /// token 是否已设置；明文只在详情里回显（卡片测试连通按钮的门控）。
    pub has_token: bool,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub admin_url: Option<String>,
    pub kind: Option<String>,
    /// Whether usage is shown and refreshed on the Claude provider card.
    pub show_balance: bool,
    pub updated_at: String,
}

/// Claude Code 供应商详情：编辑回显含 token 明文（与 Codex CodexProfileDetail.api_key 同语义）。
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClaudeProfileDetail {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub model: Option<String>,
    pub description: Option<String>,
    pub fetched_models: Vec<String>,
    /// 创建时选择的预设 kind：编辑页反查端点档用。
    pub kind: Option<String>,
    pub admin_url: Option<String>,
    /// 附加 env 键值（JSON 对象原文）：编辑页 settings.json 编辑器直接编辑这段文本。
    pub extra_env: Option<String>,
    /// settings.json 全文；旧配置可为空。
    pub raw_settings: Option<String>,
    /// 图标 id（icons.ts 收集的 provider 图标；NULL 显示名称首字）。
    pub icon: Option<String>,
    pub show_balance: bool,
    pub updated_at: String,
}

/// claude_save_profile 的载荷：字段即编辑页表单，一一对应。
#[derive(Debug, Clone, Default)]
pub struct ClaudeProfileInput {
    pub name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub model: Option<String>,
    pub description: Option<String>,
    pub fetched_models: Option<Vec<String>>,
    pub kind: Option<String>,
    pub admin_url: Option<String>,
    pub extra_env: Option<String>,
    pub raw_settings: Option<String>,
    pub icon: Option<String>,
    pub show_balance: bool,
}
