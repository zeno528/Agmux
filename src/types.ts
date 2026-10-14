export interface ChatgptModel {
  slug: string;
  display_name: string;
  supported_reasoning_levels: { effort: string }[];
  default_reasoning_level: string | null;
}

export interface CodexProfileSummary {
  id: string;
  name: string;
  kind: "official" | "third_party";
  /** 官方配置绑定的订阅账号 id；第三方为 null。 */
  account_id: string | null;
  /** 官方配置创建时固定的认证来源；旧数据缺失时由 account_id 推断。 */
  auth_source?: "desktop" | "oauth" | null;
  /** 账号页额度查询所用身份：OAuth 为本地账号 id，Desktop 为 workspace id。 */
  auth_account_id?: string | null;
  /** 绑定账号的订阅套餐（free/plus/pro/team…官方原词）；第三方或未知为 null。 */
  plan_type: string | null;
  model: string | null;
  provider: string | null;
  reasoning_effort: string | null;
  fast_mode: boolean;
  /** 供应商是否已配置非空 API 端点。 */
  has_base_url: boolean;
  has_key: boolean;
  admin_url: string | null;
  /** 供应商级开关：是否在卡片显示并自动刷新余额/用量。 */
  show_balance: boolean;
  icon: string | null;
  created_at: string;
  updated_at: string;
}

export interface CodexProfileDetail {
  id: string;
  name: string;
  description: string | null;
  /** 官方配置绑定的订阅账号 id；第三方为 null。 */
  account_id: string | null;
  /** 官方配置创建时固定的认证来源。 */
  auth_source?: "desktop" | "oauth" | null;
  /** Desktop 配置自身 auth.json 解析出的登录账号。 */
  desktop_login: string | null;
  icon: string | null;
  provider: string | null;
  base_url: string | null;
  api_key: string | null;
  model_values: Record<string, string>;
  config_fragment: string;
  /** 供应商自己保存的完整 config 原文（内置供应商可全量编辑；普通供应商为片段）。 */
  raw_config: string | null;
  catalog_content: string | null;
  /** 供应商自己保存的 models.json 原文。 */
  raw_catalog: string | null;
  /** 供应商自己保存的 auth.json 原文。 */
  raw_auth: string | null;
  admin_url: string | null;
  show_balance: boolean;
  /** 最近一次成功获取的供应商模型列表。 */
  fetched_models: string[];
  fetched_model_efforts?: Record<string, string[]>;
  fetched_model_defaults?: Record<string, string>;
  updated_at: string;
}

export interface CodexProfileConnectionResult {
  ok: boolean;
  latency_ms: number | null;
  status: number | null;
  error: string | null;
}

/** ~/.codex/config.toml [mcp_servers.*] 的一条服务器配置（建模字段子集；未建模键由后端原样保留）。 */
export interface McpServerSpec {
  name: string;
  /** null = 未写入该键（Codex 默认启用）；false 显式停用。 */
  enabled: boolean | null;
  startup_timeout_sec: number | null;
  tool_timeout_sec: number | null;
  /** STDIO 传输：有 command 无 url。 */
  command: string | null;
  args: string[];
  env: Record<string, string>;
  /** Streamable HTTP 传输：有 url 无 command。 */
  url: string | null;
  bearer_token_env_var: string | null;
  http_headers: Record<string, string>;
  env_http_headers: Record<string, string>;
}

export interface McpServerInfo {
  name: string | null;
  version: string | null;
}

export interface McpTool {
  name: string;
  title: string | null;
  description: string | null;
  input_schema: Record<string, unknown>;
}

export interface McpProbeResult {
  ok: boolean;
  latency_ms: number | null;
  status: number | null;
  protocol_version: string | null;
  server_info: McpServerInfo | null;
  tools: McpTool[];
  tools_truncated: boolean;
  error: string | null;
  tools_error: string | null;
}

/** MCP 同步预览的一条差异（live = config.toml，db = 数据库镜像）。 */
export type McpSyncEntryKind = "live_only" | "db_only" | "changed";

export interface McpSyncDiffEntry {
  name: string;
  kind: McpSyncEntryKind;
  live_spec: McpServerSpec | null;
  db_spec: McpServerSpec | null;
  live_toml: string | null;
  db_toml: string | null;
}

/** MCP 差异批量处理的一条动作：fragment 为 null 表示删除该侧条目。 */
export interface McpDiffEntryAction {
  name: string;
  fragment: string | null;
}

export interface McpSyncPreview {
  entries: McpSyncDiffEntry[];
  live_count: number;
  db_count: number;
}

export interface TomlDiagnostic {
  from: number;
  to: number;
  message: string;
}

export interface EditorDiagnosticSummary {
  count: number;
  firstLine: number | null;
}

export interface ProfileBalanceInfo {
  currency: string;
  total_balance: string;
  /** 用量型供应商（如 MiniMax Token Plan）的已用百分比；余额型供应商为 null。 */
  usage_percent: number | null;
  /** 主用量窗口重置倒计时（如 "2h23m"）。 */
  usage_reset: string | null;
  /** 主用量窗口重置时间（Unix 毫秒）。 */
  usage_reset_at?: number | null;
  /** 主用量窗口名称；由接口返回的窗口长度推断。 */
  usage_label?: string | null;
  /** 次用量窗口已用百分比；仅用量型供应商返回。 */
  weekly_usage_percent: number | null;
  /** 次用量窗口重置倒计时（如 "5d21h"）。 */
  weekly_reset: string | null;
  /** 次用量窗口重置时间（Unix 毫秒）。 */
  weekly_reset_at?: number | null;
  /** 次用量窗口名称；免费方案可为 30 天。 */
  weekly_label?: string | null;
  /** Codex 主动重置卡可用次数；仅 ChatGPT 订阅账号返回。 */
  reset_credits_available?: number | null;
  /** 可用主动重置卡的到期时间。 */
  reset_credits?: ChatgptResetCredit[] | null;
}

export interface ChatgptResetCredit {
  id: string;
  reset_type?: string | null;
  expires_at?: number | null;
}

export interface ProfileBalance {
  is_available: boolean;
  balance_infos: ProfileBalanceInfo[];
  latency_ms: number | null;
}

export interface DatabaseBackupInfo {
  name: string;
  size_bytes: number;
  created_at: number;
}

export interface BrowserLoginStart {
  authorize_url: string;
  expires_in: number;
}

export interface ManagedAccount {
  id: string;
  login: string;
  authenticated_at: number;
  is_default: boolean;
  /** 订阅套餐（free/plus/pro/team…官方原词），未知为 null */
  plan_type: string | null;
  /** ChatGPT 订阅到期时间（Unix 毫秒），未知时省略。 */
  subscription_active_until?: number | null;
}

export interface AuthStatus {
  authenticated: boolean;
  default_account_id: string | null;
  accounts: ManagedAccount[];
  /** Desktop 来源账号：由数据库认证快照派生，不随配置切换漂移。 */
  external: ManagedAccount[];
}

/** 界面语言设置；"system" 表示跟随系统语言。 */
export type LanguageSetting = "system" | "zh-CN" | "en-US";

/** 侧栏顶层视图；startup_view 用它做冷启动落页。 */
export type AppView = "codexProfiles" | "mcp" | "plugins" | "skills" | "accounts" | "settings" | "agentTools" | "claudeProfiles";

export interface Settings {
  theme: "system" | "light" | "dark";
  language: LanguageSetting;
  auto_restart: boolean;
  autostart_enabled: boolean;
  silent_start: boolean;
  minimize_to_tray: boolean;
  tray_click_action: "show_window" | "show_menu";
  /** 冷启动时默认打开的页面；托盘/后台恢复不拽页。 */
  startup_view: AppView;
  /** macOS 供应商终端的宿主终端；Windows 分支不读取此字段。 */
  macos_terminal: "terminal" | "ghostty";
  auto_check_update: boolean;
  auto_backup_interval_hours: number;
  database_backup_keep_count: number;
  proxy_mode: "auto" | "off" | "custom";
  proxy_url: string;
  /** 供应商终端的最近工作目录（最近使用在前）。卡片下拉框的新建/删除直接改这里。 */
  claude_terminal_dirs: string[];
}

export interface PluginSkill {
  name: string;
  path: string;
  description: string | null;
}

export interface PluginMarketplace {
  name: string;
  root: string;
  kind: "official" | "third-party";
  source_url: string | null;
  display_name: string | null;
  description: string | null;
}

export interface MarketplacePlugin {
  plugin_id: string;
  name: string;
  version: string | null;
  installed: boolean;
  auth_policy: string;
  source: string | null;
  display_name: string | null;
  description: string | null;
  category: string | null;
  capabilities: string[];
  contains: string[];
}

export interface PluginUpdate {
  name: string;
  marketplace: string;
  version: string;
}

/** Skill 启停的目标工具：同一份正本分发到各工具自己的 skills 目录。 */
export type SkillTool = "codex" | "claude";

export interface ClaudeProfileSummary {
  id: string;
  name: string;
  base_url: string | null;
  /** token 是否已设置；明文只在详情里回显（卡片测试连通按钮的门控）。 */
  has_token: boolean;
  model: string | null;
  reasoning_effort: string | null;
  description: string | null;
  icon: string | null;
  admin_url: string | null;
  kind: string | null;
  show_balance: boolean;
  updated_at: string;
}

export interface ClaudeProfileDetail {
  id: string;
  name: string;
  base_url: string | null;
  auth_token: string | null;
  model: string | null;
  description: string | null;
  fetched_models: string[];
  /** 创建时选择的预设 kind：编辑页反查端点档用。 */
  kind: string | null;
  admin_url: string | null;
  /** 旧配置的附加 env 键值；新配置由 raw_settings 派生。 */
  extra_env: string | null;
  /** 完整 settings.json 原文；旧配置可能为空。 */
  raw_settings: string | null;
  /** 图标 id（icons.ts 收集的 provider 图标；NULL 显示名称首字）。 */
  icon: string | null;
  show_balance: boolean;
  updated_at: string;
  /** 卡片排序（拖拽持久化）。 */
  sort_order: number;
}

export interface SkillSummary {
  name: string;
  description: string | null;
  source_url: string | null;
  store_path: string;
  source_path: string | null;
  update_available: boolean;
  enabled: boolean;
  /** Claude Code 侧的分发状态（~/.claude/skills/<name> 是否存在）。 */
  claude_enabled: boolean;
}

export interface SkillCandidate {
  name: string;
  description: string | null;
  store_path: string;
  source: string;
  has_content_conflict: boolean;
  is_update: boolean;
  modified_at: number;
}

export interface PluginSummary {
  name: string;
  version: string | null;
  display_name: string | null;
  description: string | null;
  category: string | null;
  capabilities: string[];
  contains: string[];
  enabled: boolean;
  /** official=官方市场；codex=第三方市场。 */
  origin: "official" | "codex";
  marketplace: string | null;
  store_path: string;
  source_url: string | null;
}

export interface PluginCandidate {
  sub_path: string;
  name: string;
  version: string | null;
  display_name: string | null;
  description: string | null;
  capabilities: string[];
  contains: string[];
  files: string[];
}

export interface PluginPreview {
  repo: string;
  reference: string;
  default_branch: string;
  candidates: PluginCandidate[];
}

export interface CodexAppStatus {
  running: boolean;
  display_path: string;
  source: string;
}

export interface PathInfo {
  /** i18n key（settings 命名空间 about.paths.*），展示时经 t() 翻译 */
  label: string;
  path: string;
}

export interface AppState {
  codex_profiles: CodexProfileSummary[];
  active_codex_profile_id: string | null;
  /** Claude Code 侧当前激活的供应商（claude_profiles 表）。 */
  active_claude_profile_id: string | null;
  codex: CodexAppStatus;
  settings: Settings;
  paths: PathInfo[];
  auth_status: AuthStatus;
  /** 供应商级余额/用量缓存（上次成功查询结果），保证卡片静默显示不闪烁。 */
  balance_cache: Record<string, ProfileBalanceInfo>;
}
/** CLI 拒绝结果：阶段和错误类别用于界面翻译，message 保留诊断信息。 */
export interface CliFailure {
  stage: string;
  kind: string;
  message: string;
}

export interface CliStatus {
  installation: "missing" | "native" | "other" | "conflict" | "broken";
  source: string | null;
  version: string | null;
  path: string | null;
  other_paths: string[];
  embedded_paths?: string[];
  platform: string;
  network: "direct" | "proxy";
  proxy: string | null;
  busy: boolean;
}

export interface CliUpdate {
  status: CliStatus;
  latest_version: string;
  channel: string;
  available: boolean;
}
