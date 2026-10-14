import { balanceQueryProviders, codexBuiltinHasCatalog, codexPresetByKind, type ClientPreset } from "../presets";
import { buildSettingsText, readAdvancedSettings, splitEnvExtras } from "../features/claude/profileEnvText";
import { patchModelValue, readCatalogOptions, readModelValue } from "../features/codex/profileEditText";
import type {
  AppState,
  DatabaseBackupInfo,
  McpDiffEntryAction,
  McpProbeResult,
  McpServerSpec,
  MarketplacePlugin,
  PluginMarketplace,
  PluginPreview,
  PluginSkill,
  PluginSummary,
  ClaudeProfileDetail,
  SkillSummary,
  SkillCandidate,
  ProfileBalanceInfo,
  CodexProfileConnectionResult,
  CodexProfileDetail,
  CodexProfileSummary,
  Settings,
  CliFailure,
  CliStatus,
  CliUpdate,
} from "../types";

const webProfiles: CodexProfileSummary[] = [
  {
    id: "profile-zai-glm-high",
    name: "ZAI GLM 高推理",
    kind: "third_party",
    account_id: null,
    plan_type: null,
    model: "glm-5.3",
    provider: "ZAI",
    reasoning_effort: "high",
    fast_mode: false,
    has_base_url: true,
    has_key: true,
    admin_url: "https://open.bigmodel.cn/console",
    show_balance: true,
    icon: "zhipu",
    created_at: "2026-08-15 10:00:00",
    updated_at: "2026-08-15 10:00:00",
  },
  {
    id: "profile-official",
    name: "官方默认",
    kind: "official",
    account_id: null,
    auth_source: "desktop",
    auth_account_id: "web-codex-account",
    plan_type: "plus",
    model: "gpt-5.6",
    provider: null,
    reasoning_effort: "medium",
    fast_mode: false,
    has_base_url: false,
    has_key: false,
    admin_url: "https://chatgpt.com",
    show_balance: true,
    icon: "openai-chatgpt",
    created_at: "2026-08-15 10:02:00",
    updated_at: "2026-08-15 10:02:00",
  },
];

const webDescriptions = new Map<string, string>();

// label 是 i18n key，由前端 t() 翻译展示；与 Rust path_info（services/settings.rs）保持一致
const webPaths = [
  { label: "about.paths.appData", path: "C:\\Users\\<user>\\.agmux" },
  { label: "about.paths.backups", path: "C:\\Users\\<user>\\.agmux\\backups\\database" },
  { label: "about.paths.logs", path: "C:\\Users\\<user>\\.agmux\\logs" },
  { label: "about.paths.codexConfig", path: "C:\\Users\\<user>\\.codex\\config.toml" },
];

function patchContextOverrideForWeb(text: string, enabled: boolean, compactTokenLimit: number): string {
  if (enabled && (!Number.isInteger(compactTokenLimit) || compactTokenLimit < 1 || compactTokenLimit > 1_000_000)) {
    throw new Error("压缩阈值必须在 1 到 1000000 Token 之间");
  }
  const newline = text.includes("\r\n") ? "\r\n" : "\n";
  const lines = text
    .split(/\r?\n/)
    .filter(
      (line) =>
        !/^\s*model_context_window\s*=/.test(line) &&
        !/^\s*model_auto_compact_token_limit\s*=/.test(line),
    );
  if (enabled) {
    const sectionIndex = lines.findIndex((line) => /^\s*\[/.test(line));
    lines.splice(sectionIndex < 0 ? lines.length : sectionIndex, 0, "model_context_window = 1000000", `model_auto_compact_token_limit = ${compactTokenLimit}`);
  }
  return lines.join(newline);
}

// 与后端一致：开启时确保 [features] 表存在并写入键（无表则追加到文末），关闭时移除该键
function patchSystemProxyForWeb(text: string, enabled: boolean): string {
  const newline = text.includes("\r\n") ? "\r\n" : "\n";
  const lines = text
    .split(/\r?\n/)
    .filter((line) => !/^\s*respect_system_proxy\s*=/.test(line));
  if (!enabled) return lines.join(newline);
  const featuresIndex = lines.findIndex((line) => /^\s*\[features\]/.test(line));
  if (featuresIndex >= 0) {
    lines.splice(featuresIndex + 1, 0, "respect_system_proxy = true");
    return lines.join(newline);
  }
  lines.push("[features]", "respect_system_proxy = true");
  return lines.join(newline);
}

// 与后端一致：开启时写入 [features.context_management] 段（experimental_mode = true），关闭时移除整个段
function patchContextManagementForWeb(text: string, enabled: boolean): string {
  const newline = text.includes("\r\n") ? "\r\n" : "\n";
  let inContextManagement = false;
  const lines = text.split(/\r?\n/).filter((line) => {
    if (/^\s*\[/.test(line)) inContextManagement = /^\s*\[features\.context_management\]/.test(line);
    return !inContextManagement;
  });
  if (!enabled) return lines.join(newline);
  const featuresIndex = lines.findIndex((line) => /^\s*\[features\]/.test(line));
  let at = lines.length;
  if (featuresIndex >= 0) {
    for (let i = featuresIndex + 1; i < lines.length; i++) {
      if (/^\s*\[/.test(lines[i])) {
        at = i;
        break;
      }
    }
  }
  lines.splice(at, 0, "[features.context_management]", "experimental_mode = true");
  return lines.join(newline);
}

// 与后端一致：安装/卸载驱动 codex plugin CLI；列表读 codex plugin list、
// 插件列表与 Codex Skill 注册表分开；插件 origin 只描述插件来源
let webPlugins: PluginSummary[] = [
  {
    name: "memory-bank",
    version: "1.2.0",
    display_name: "Memory Bank",
    description: "跨会话的记忆管理插件（浏览器调试 fixture）",
    category: "memory",
    capabilities: ["read", "write"],
    contains: ["skills", "mcp"],
    enabled: true,
    origin: "codex",
    marketplace: "example-plugins",
    store_path: "C:\\Users\\<user>\\.codex\\plugins\\cache\\example-plugins\\memory-bank\\1.2.0",
    source_url: "https://github.com/example/plugins",
  },
  {
    name: "code-reviewer",
    version: "0.4.1",
    display_name: null,
    description: "代码审查工作流（已禁用示例）",
    category: null,
    capabilities: [],
    contains: ["skills"],
    enabled: false,
    origin: "codex",
    marketplace: "example-plugins",
    store_path: "C:\\Users\\<user>\\.codex\\plugins\\cache\\example-plugins\\code-reviewer\\0.4.1",
    source_url: null,
  },
  {
    name: "documents",
    version: "26.819.11345",
    display_name: "Documents",
    description: "官方市场里的已装插件（只读示例）",
    category: null,
    capabilities: [],
    contains: ["skills"],
    enabled: true,
    origin: "official",
    marketplace: "openai-primary-runtime",
    store_path: "C:\\Users\\<user>\\.cache\\codex-runtimes\\plugins\\documents",
    source_url: null,
  },
  {
    name: "ponytail",
    version: "4.9.0",
    display_name: "Ponytail",
    description: "用户自装的第三方市场插件（经 codex CLI 卸载）",
    category: null,
    capabilities: [],
    contains: ["skills"],
    enabled: true,
    origin: "codex",
    marketplace: "ponytail",
    store_path: "C:\\Users\\<user>\\.codex\\plugins\\cache\\ponytail\\ponytail\\4.9.0",
    source_url: "https://github.com/DietrichGebert/ponytail.git",
  },
];

const webPluginSkills: Record<string, PluginSkill[]> = {
  "memory-bank": [{ name: "session-summary", path: "skills/session-summary/SKILL.md", description: "整理跨会话摘要" }],
  ponytail: [{ name: "ponytail", path: "skills/ponytail/SKILL.md", description: "精简实现工作流" }],
};

const webSkills: SkillSummary[] = [
  {
    name: "lark-base",
    description: "Skill 注册表里的已装技能（只读示例）",
    source_url: "https://github.com/larksuite/cli.git",
    store_path: "C:\\Users\\<user>\\.agents\\skills\\lark-base",
    source_path: null,
    update_available: false,
    enabled: true,
    claude_enabled: false,
  },
];
const webLocalSkillPath = "C:\\Users\\<user>\\.agents\\skills\\local-skill";

// Claude Code 供应商 fixture：与后端 claude_profiles 表同构，save/apply/delete 有状态
const webClaudeProfiles: ClaudeProfileDetail[] = [
  {
    id: "cla-demo-1",
    name: "BigModel 中转",
    base_url: "https://open.bigmodel.cn/api/anthropic",
    auth_token: "demo-token",
    model: "glm-5.3[1m]",
    description: "演示配置",
    fetched_models: ["glm-5.3", "glm-5.3-air", "glm-5.3[1m]"],
    kind: "zhipu",
    admin_url: "https://open.bigmodel.cn",
    extra_env: null,
    raw_settings: JSON.stringify({ model: "sonnet", env: {
      ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic",
      ANTHROPIC_AUTH_TOKEN: "demo-token",
      ANTHROPIC_MODEL: "glm-5.3[1m]",
    } }, null, 2),
    icon: "zhipu",
    show_balance: false,
    sort_order: 0,
    updated_at: "1700000000000",
  },
];
let webActiveClaudeProfileId: string | null = "cla-demo-1";
let webClaudeCommonSettings: string | null = null;

let webMarketplaces: PluginMarketplace[] = [
  {
    name: "openai-bundled",
    root: "C:\\Users\\<user>\\.codex\\.tmp\\bundled-marketplaces\\openai-bundled",
    kind: "official",
    source_url: null,
    display_name: "OpenAI Bundled",
    description: "Codex 官方捆绑插件市场",
  },
  {
    name: "ponytail",
    root: "C:\\Users\\<user>\\.codex\\.tmp\\marketplaces\\ponytail",
    kind: "third-party",
    source_url: "https://github.com/DietrichGebert/ponytail.git",
    display_name: "Ponytail",
    description: "第三方插件市场，提供精简实现、YAGNI 和标准库优先的开发工作流",
  },
  {
    name: "youmind",
    root: "C:\\Users\\<user>\\.codex\\.tmp\\marketplaces\\youmind",
    kind: "third-party",
    source_url: "https://github.com/YouMind-OpenLab/plugin-marketplace.git",
    display_name: "YouMind",
    description: "YouMind 第三方插件市场，收录创作、设计和内容工作流插件",
  },
];

let webMarketplacePlugins: Record<string, MarketplacePlugin[]> = {
  ponytail: [
    {
      plugin_id: "ponytail@ponytail",
      name: "ponytail",
      version: "4.9.0",
      installed: true,
      auth_policy: "ON_INSTALL",
      source: "https://github.com/DietrichGebert/ponytail.git",
      display_name: "Ponytail",
      description: "Prefer YAGNI, the standard library, native platform features, and the smallest correct implementation",
      category: "Productivity",
      capabilities: ["Instructions", "Lifecycle hooks"],
      contains: ["skills", "hooks"],
    },
  ],
};

const webPluginUpdates = [{ name: "ponytail", marketplace: "ponytail", version: "5.0.0" }];

const webRecommendedMarketplacePlugins: Record<string, MarketplacePlugin[]> = {
  xiaolai: [
    {
      plugin_id: "grill@xiaolai",
      name: "grill",
      version: "1.3.0",
      installed: false,
      auth_policy: "ON_USE",
      source: "https://github.com/xiaolai/grill-for-claude.git",
      display_name: "Grill",
      description: "用于代码工作流与开发辅助的第三方插件",
      category: "Development",
      capabilities: ["Instructions"],
      contains: ["skills"],
    },
    {
      plugin_id: "xros@xiaolai",
      name: "xros",
      version: "0.3.0",
      installed: false,
      auth_policy: "ON_USE",
      source: "https://github.com/xiaolai/xros.git",
      display_name: "XROS",
      description: "面向终端工作流的第三方插件",
      category: "Productivity",
      capabilities: ["Instructions"],
      contains: ["skills"],
    },
  ],
  youmind: [
    {
      plugin_id: "hyperframes@youmind",
      name: "hyperframes",
      version: null,
      installed: false,
      auth_policy: "ON_INSTALL",
      source: "./plugins/hyperframes",
      display_name: "HyperFrames by HeyGen",
      description: "Write HTML, render video, and create interactive motion graphics with HeyGen's HyperFrames",
      category: "Design",
      capabilities: ["Read", "Write"],
      contains: ["app"],
    },
  ],
};

const webPluginPreview: PluginPreview = {
  repo: "openai/plugins",
  reference: "main",
  default_branch: "main",
  candidates: [
    {
      sub_path: "plugins/memory-bank",
      name: "memory-bank",
      version: "1.2.0",
      display_name: "Memory Bank",
      description: "浏览器调试用的候选插件 fixture",
      capabilities: ["read", "write"],
      contains: ["skills", "mcp"],
      files: [
        "plugins/memory-bank/.codex-plugin/plugin.json",
        "plugins/memory-bank/skills/session-summary/SKILL.md",
        "plugins/memory-bank/.mcp.json",
      ],
    },
  ],
};

// 从 2xx 的 JSON 响应体里识别供应商级错误（OpenAI 风格 error 或智谱风格 code/success）。
function connectionErrorFromBody(value: unknown): string | null {
  if (!value || typeof value !== "object") return null;
  const json = value as Record<string, unknown>;
  if (json.error !== undefined) {
    const error = json.error;
    const message =
      error && typeof error === "object"
        ? (error as Record<string, unknown>).message
        : error;
    return typeof message === "string" && message ? message : "requestFailed";
  }
  if (json.success === false) {
    const message = typeof json.msg === "string" ? json.msg : json.message;
    return typeof message === "string" && message ? message : "requestFailed";
  }
  const code = json.code;
  const codeNumber =
    typeof code === "number" ? code : typeof code === "string" ? Number(code) : NaN;
  if (Number.isFinite(codeNumber) && codeNumber >= 400) {
    const message = typeof json.msg === "string" ? json.msg : json.message;
    return typeof message === "string" && message ? message : "requestFailed";
  }
  return null;
}

function providerEndpointFormatError(baseUrl: string): CodexProfileConnectionResult | null {
  let protocol: string | null = null;
  try {
    protocol = new URL(baseUrl).protocol;
  } catch {
    // 非法 URL 视作格式无效
  }
  if (protocol === "http:" || protocol === "https:") return null;
  return { ok: false, latency_ms: null, status: null, error: "invalidEndpoint" };
}

function isOpenCodeGoBaseUrl(baseUrl: string): boolean {
  return baseUrl.replace(/\/+$/, "").toLowerCase() === "https://opencode.ai/zen/go/v1";
}

async function testProviderConnection(
  baseUrl: string,
  apiKey: string,
): Promise<CodexProfileConnectionResult> {
  const start = Date.now();
  const emptyProbe = isOpenCodeGoBaseUrl(baseUrl);
  try {
    const res = await fetch(`${baseUrl.replace(/\/+$/, "")}/${emptyProbe ? "responses" : "models"}`, {
      method: emptyProbe ? "POST" : "GET",
      headers: {
        Authorization: `Bearer ${apiKey}`,
        ...(emptyProbe ? { "Content-Type": "application/json" } : {}),
      },
    });
    const body = await res.text();
    const latency_ms = Date.now() - start;
    const text = body.toLowerCase();
    const probeValidationRejection =
      emptyProbe && [400, 422].includes(res.status) &&
      ["model", "messages", "input", "body", "json"].some((field) => text.includes(field)) &&
      ["required", "missing", "empty", "expect", "缺少", "必填", "为空"].some((word) => text.includes(word));
    let error: string | null = null;
    let detail = "";
    try {
      const json = JSON.parse(body);
      error = connectionErrorFromBody(json);
      detail = error ?? (typeof json?.message === "string" ? json.message : typeof json?.msg === "string" ? json.msg : "");
    } catch {
      if (!emptyProbe && res.ok) error = "requestFailed";
    }
    const ok = (res.ok && !error) || probeValidationRejection || (emptyProbe && res.status === 415);
    return {
      ok,
      latency_ms,
      status: res.status,
      error: ok ? null : (detail || error || "").split(apiKey).join("[REDACTED]").slice(0, 400),
    };
  } catch {
    return {
      ok: false,
      latency_ms: null,
      status: null,
      error: "连接失败：浏览器跨域限制无法真实请求，请用桌面版验证",
    };
  }
}

interface WebDetail {
  base_url: string | null;
  api_key: string | null;
  model_values: Record<string, string>;
  config_fragment: string;
  raw_config?: string | null;
  raw_catalog?: string | null;
  raw_auth?: string | null;
  desktop_login?: string | null;
  fetched_models?: string[];
  fetched_model_efforts?: Record<string, string[]>;
  fetched_model_defaults?: Record<string, string>;
}

const webDetails: Record<string, WebDetail> = {
  "profile-zai-glm-high": {
    base_url: "https://open.bigmodel.cn/api/v1",
    api_key: "sk-demo",
    model_values: {
      model: '"glm-5.3"',
      model_reasoning_effort: '"high"',
      model_catalog_json: '"zai.json"',
    },
    config_fragment:
      'model = "glm-5.3"\nmodel_reasoning_effort = "high"\nmodel_catalog_json = "zai.json"\n\n[model_providers.ZAI]\nname = "ZAI"\nbase_url = "https://open.bigmodel.cn/api/v1"\nwire_api = "responses"\nexperimental_bearer_token = "••••••••"',
    fetched_models: ["glm-5.3", "glm-5.2", "glm-4.7"],
  },
  "profile-official": {
    base_url: null,
    api_key: null,
    model_values: {
      model: '"gpt-5.6"',
      model_reasoning_effort: '"medium"',
    },
    config_fragment: 'model = "gpt-5.6"\nmodel_reasoning_effort = "medium"',
  },
};

function webProfileDetail(id: string): CodexProfileDetail {
  const profile = webProfiles.find((item) => item.id === id);
  if (!profile) throw new Error("供应商配置不存在");
  const detail = webDetails[id];
  return {
    id: profile.id,
    name: profile.name,
    description: webDescriptions.get(id) ?? null,
    account_id: profile.account_id,
    auth_source: profile.auth_source ?? (profile.account_id ? "oauth" : profile.kind === "official" ? "desktop" : null),
    desktop_login: detail?.desktop_login ?? null,
    icon: profile.icon,
    provider: profile.provider,
    base_url: detail?.base_url ?? null,
    api_key: detail?.api_key ?? null,
    model_values: detail?.model_values ?? {},
    config_fragment: detail?.config_fragment ?? "",
    raw_config: detail?.raw_config ?? null,
    catalog_content: detail?.model_values.model_catalog_json
      ? String.raw`{
    "models": [
        {
            "slug": "glm-5.3",
            "input_modalities": [
			"text",
  "image"
            ],
            "truncation_policy": {
                "mode": "bytes"
            }
        }
    ]
}`
      : null,
    raw_catalog: detail?.raw_catalog ?? null,
    raw_auth: detail?.raw_auth ?? null,
    admin_url: profile.admin_url,
    show_balance: profile.show_balance,
    fetched_models: detail?.fetched_models ?? [],
    fetched_model_efforts: detail?.fetched_model_efforts ?? {},
    fetched_model_defaults: detail?.fetched_model_defaults ?? {},
    updated_at: profile.updated_at,
  };
}

/** Web 调试模式的「已更新到 vX」一次性标记（真实环境存后端文件） */
let webUpdateMarker: string | null = null;

let webSettings: Settings = {
  theme: "system",
  language: "system",
  auto_restart: false,
  autostart_enabled: false,
  silent_start: false,
  minimize_to_tray: false,
  tray_click_action: "show_window",
  auto_check_update: true,
  auto_backup_interval_hours: 0,
  database_backup_keep_count: 5,
  proxy_mode: "auto",
  proxy_url: "",
  startup_view: "codexProfiles",
  macos_terminal: "terminal",
  claude_terminal_dirs: [],
};

const webClaudeCliStatus: CliStatus = {
  installation: "native",
  source: "web-mock",
  version: "1.0.0",
  path: "C:\\Users\\<user>\\.local\\bin\\claude.exe",
  other_paths: [],
  platform: "windows",
  network: "direct",
  proxy: null,
  busy: false,
};
const webClaudeCliUpdate: CliUpdate = {
  status: webClaudeCliStatus,
  latest_version: "99.0.0",
  channel: "latest",
  available: true,
};

let webBackups: DatabaseBackupInfo[] = [];
// MCP 页 web 调试样例：stdio + http 各一条，字段对齐 Codex 官方配置格式
let webMcpServers: McpServerSpec[] = [
  {
    name: "github",
    enabled: null,
    startup_timeout_sec: null,
    tool_timeout_sec: null,
    command: "github-mcp-server",
    args: ["stdio"],
    env: { GITHUB_PERSONAL_ACCESS_TOKEN: "ghp_demo" },
    url: null,
    bearer_token_env_var: null,
    http_headers: {},
    env_http_headers: {},
  },
  {
    name: "tavily",
    enabled: null,
    startup_timeout_sec: null,
    tool_timeout_sec: null,
    command: null,
    args: [],
    env: {},
    url: "https://mcp.tavily.com/mcp",
    bearer_token_env_var: "TAVILY_API_KEY",
    http_headers: {},
    env_http_headers: {},
  },
];
let webClaudeMcpServers = structuredClone(webMcpServers);

function saveWebMcpServer(original: string | null, spec: McpServerSpec, tool: "codex" | "claude") {
  const name = original ?? spec.name;
  const current = tool === "codex" ? webMcpServers : webClaudeMcpServers;
  if (name !== spec.name || original === null) {
    if (current.some((server) => server.name === spec.name)) throw new Error("已存在同名 MCP 服务器");
  }
  const next = current.filter((server) => server.name !== name);
  next.push({ ...structuredClone(spec), enabled: null });
  if (tool === "codex") webMcpServers = next;
  else webClaudeMcpServers = next;
}

// 与后端一致：激活状态只由“使用”显式建立，添加/捕获供应商不激活
let webActiveProfileId: string | null = null;
const webBalanceCache: Record<string, ProfileBalanceInfo> = {};
const webChatgptQuota: ProfileBalanceInfo = {
  currency: "",
  total_balance: "",
  usage_percent: 18,
  usage_reset: "3h12m",
  usage_reset_at: Date.now() + 3 * 60 * 60 * 1000 + 12 * 60 * 1000,
  usage_label: "5小时",
  weekly_usage_percent: 42,
  weekly_reset: "4d8h",
  weekly_reset_at: Date.now() + 4 * 86_400_000 + 8 * 3_600_000,
  weekly_label: "7天",
  reset_credits_available: 2,
  reset_credits: [
    { id: "reset-credit-1", reset_type: "full", expires_at: Date.now() + 16 * 86_400_000 },
    { id: "reset-credit-2", reset_type: "full", expires_at: Date.now() + 17 * 86_400_000 },
  ],
};
const webAuthStatus = {
  authenticated: true,
  default_account_id: null,
  external: [
    {
      id: "web-codex-account",
      login: "codex@example.com",
      authenticated_at: 0,
      is_default: false,
      plan_type: "plus",
      subscription_active_until: Date.now() + 14 * 86_400_000,
    },
  ],
  accounts: ["alpha", "beta"].map((name, index) => ({
    id: "web-" + name,
    login: name + "@example.com",
    authenticated_at: 0,
    is_default: false,
    plan_type: index % 2 ? "free" : "plus",
    subscription_active_until: index % 2 ? null : Date.now() + (index + 3) * 86_400_000,
  })),
};

function databaseBackupName(date = new Date()): string {
  const pad = (value: number, length = 2) => String(value).padStart(length, "0");
  return `cg-backup-manual-${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}-${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}-${pad(date.getMilliseconds(), 3)}.db`;
}

function webState(): AppState {
  return {
    codex_profiles: webProfiles.map((profile) => {
      const cached = profile.kind === "official" && !profile.provider ? webDetails[profile.id] : undefined;
      const text = cached?.raw_config ?? cached?.config_fragment ?? "";
      const hasLocal = !!readModelValue(text, "model_catalog_json");
      let local = readCatalogOptions(null);
      if (hasLocal) {
        try { local = readCatalogOptions(cached?.raw_catalog ?? null); } catch { /* 与桌面版一样，损坏目录不回退到远程缓存。 */ }
      }
      const model = profile.model || (hasLocal ? local.models[0] : cached?.fetched_models?.[0]) || null;
      return { ...profile, model, reasoning_effort: profile.reasoning_effort || (hasLocal ? local.defaults[model ?? ""] : cached?.fetched_model_defaults?.[model ?? ""]) || null };
    }),
    active_codex_profile_id: webActiveProfileId,
    active_claude_profile_id: webActiveClaudeProfileId,
    codex: {
      running: true,
      display_path: "OpenAI.Codex_2p2nqsd0c76g0!App",
      source: "packaged-app",
    },
    settings: { ...webSettings },
    paths: webPaths,
    auth_status: webAuthStatus,
    balance_cache: { ...webBalanceCache },
  };
}

// MCP 单服务器片段的 web 调试渲染（仅建模字段；真实渲染在 Rust 侧 toml_edit）
function renderMcpFragmentWeb(spec: McpServerSpec): string {
  const lines = [`[mcp_servers.${spec.name}]`];
  if (spec.command) lines.push(`command = "${spec.command}"`);
  if (spec.args.length) {
    lines.push(`args = [${spec.args.map((arg) => `"${arg}"`).join(", ")}]`);
  }
  if (spec.url) lines.push(`url = "${spec.url}"`);
  if (spec.bearer_token_env_var) {
    lines.push(`bearer_token_env_var = "${spec.bearer_token_env_var}"`);
  }
  if (spec.enabled !== null) lines.push(`enabled = ${spec.enabled}`);
  const env = Object.entries(spec.env);
  if (env.length) {
    lines.push(`[mcp_servers.${spec.name}.env]`);
    for (const [key, value] of env) lines.push(`${key} = "${value}"`);
  }
  return lines.join("\n") + "\n";
}

function renderClaudeMcpEntryWeb(spec: McpServerSpec): string {
  const entry: Record<string, unknown> = spec.command
    ? { type: "stdio", command: spec.command }
    : { type: "http", url: spec.url ?? "" };
  if (spec.args.length) entry.args = spec.args;
  if (Object.keys(spec.env).length) entry.env = spec.env;
  const headers = { ...spec.http_headers } as Record<string, string>;
  for (const [key, variable] of Object.entries(spec.env_http_headers)) headers[key] = `\${${variable}}`;
  if (spec.bearer_token_env_var && !headers.Authorization) headers.Authorization = `Bearer \${${spec.bearer_token_env_var}}`;
  if (Object.keys(headers).length) entry.headers = headers;
  return JSON.stringify(entry, null, 2);
}

function parseClaudeMcpEntryWeb(name: string, text: string): McpServerSpec {
  const entry = JSON.parse(text) as Record<string, unknown>;
  const command = typeof entry.command === "string" ? entry.command : null;
  const url = typeof entry.url === "string" ? entry.url : null;
  if ((command ? 1 : 0) + (url ? 1 : 0) !== 1) throw new Error("Claude MCP 必须包含 command 或 url");
  const headers = entry.headers && typeof entry.headers === "object" ? entry.headers as Record<string, unknown> : {};
  const http_headers: Record<string, string> = {};
  const env_http_headers: Record<string, string> = {};
  let bearer_token_env_var: string | null = null;
  for (const [key, raw] of Object.entries(headers)) {
    const value = String(raw);
    const bearer = /^Bearer \$\{(.+)\}$/.exec(value);
    const variable = /^\$\{(.+)\}$/.exec(value);
    if (bearer && key.toLowerCase() === "authorization") bearer_token_env_var = bearer[1];
    else if (variable) env_http_headers[key] = variable[1];
    else http_headers[key] = value;
  }
  return {
    name,
    enabled: null,
    startup_timeout_sec: null,
    tool_timeout_sec: null,
    command,
    args: Array.isArray(entry.args) ? entry.args.map(String) : [],
    env: entry.env && typeof entry.env === "object" ? Object.fromEntries(Object.entries(entry.env as Record<string, unknown>).map(([key, value]) => [key, String(value)])) : {},
    url,
    bearer_token_env_var,
    http_headers,
    env_http_headers,
  };
}

// 浏览器调试模式不查后端命令，用预设的展示元数据合成最小 config 模板
// （正式运行时由后端 builtin 模板给出完整原文）
function mockBuiltinFragment(preset: ClientPreset, apiKey: string): string {
  const modelLine = preset.model ? `model = "${preset.model}"\n` : "";
  return preset.provider
    ? `${modelLine}model_provider = "${preset.provider}"\nmodel_reasoning_effort = "high"\n\n[model_providers.${preset.provider}]\nname = "${preset.provider}"\nbase_url = "${preset.base_url}"\nwire_api = "responses"\nexperimental_bearer_token = "${apiKey || "<YOUR_API_KEY>"}"`
    : `${modelLine}model_reasoning_effort = "high"`;
}

// 与后端 profile_copy_plan 一致，两端 mock 共用复制命名和插入位置。
function profileCopyPlan(profiles: { id: string; name: string }[], source: { id: string; name: string }) {
  const base = [...source.name.trim()].slice(0, 45).join("");
  let name = `${base} copy`;
  let counter = 2;
  while (profiles.some((profile) => profile.name.toLowerCase() === name.toLowerCase())) {
    name = `${base} copy ${counter++}`;
  }
  return { name, index: profiles.indexOf(source) + 1 };
}

export async function webInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  await new Promise((resolve) => setTimeout(resolve, 120));
  switch (command) {    case "get_state":
      return webState() as T;
    case "get_settings":
      return { ...webSettings } as T;
    case "get_proxy_status":
      // 浏览器无法读取系统代理或控制网络路由，不能伪造已生效的代理。
      throw new Error("浏览器预览不支持读取应用代理状态");
    case "set_update_marker":
      webUpdateMarker = (args?.version as string) ?? null;
      return null as T;
    case "take_update_marker": {
      // args?.rollback 仅后端日志分级用，mock 无日志，无需区分
      const marker = webUpdateMarker;
      webUpdateMarker = null;
      return marker as T;
    }
    case "check_app_update":
      throw new Error("应用更新需要在桌面版执行，浏览器无法模拟原生更新资源");
    case "log_update_event":
      return undefined as T;
    case "report_cli_update_tick":
      // 纯日志回传，浏览器无后端日志管道，直接吞掉
      return undefined as T;
    case "get_codex_status":
      return webState().codex as T;
    case "codex_capture_profile": {
      const now = new Date().toISOString();
      const profile: CodexProfileSummary = {
        id: `profile-${Date.now()}`,
        name: String(args?.name ?? "新供应商"),
        kind: "third_party",
        account_id: null,
    plan_type: null,
        model: "glm-5.3",
        provider: "ZAI",
        reasoning_effort: "high",
        fast_mode: false,
        has_base_url: true,
        has_key: true,
        admin_url: null,
        show_balance: false,
        icon: null,
        created_at: now,
        updated_at: now,
      };
      webProfiles.push(profile);
      return profile as T;
    }
    case "codex_add_builtin_profile": {
      const preset = codexPresetByKind(String(args?.kind ?? ""));
      if (!preset) throw new Error("未知的内置供应商类型");
      const rawKey = String(args?.apiKey ?? "");
      const apiKey = preset.provider ? rawKey : null;
      const rawBaseUrl = String(args?.baseUrl ?? "").trim();
      const baseUrl = preset.provider ? rawBaseUrl || preset.base_url : null;
      const rawAdminUrl = String(args?.adminUrl ?? "");
      const adminUrl = rawAdminUrl || preset.admin_url;
      const syntheticFragment = mockBuiltinFragment(preset, rawKey);
      const syntheticModelValues: Record<string, string> = { model: JSON.stringify(preset.model), model_reasoning_effort: '"high"' };
      const now = new Date().toISOString();
      const profile: CodexProfileSummary = {
        id: `profile-${Date.now()}`,
        name: preset.name,
        kind: preset.provider ? "third_party" : "official",
        account_id: preset.provider ? null : (typeof args?.accountId === "string" ? args.accountId : null),
        plan_type: preset.provider ? null : (typeof args?.accountId === "string" && args.accountId ? "plus" : "free"),
        auth_source: preset.provider ? null : (typeof args?.accountId === "string" && args.accountId ? "oauth" : "desktop"),
        model: preset.model,
        provider: preset.provider,
        reasoning_effort: "high",
        fast_mode: false,
        has_base_url: Boolean(baseUrl),
        has_key: preset.provider ? Boolean(rawKey.trim()) : false,
        admin_url: adminUrl,
        show_balance: false,
        icon: preset.icon,
        created_at: now,
        updated_at: now,
      };
      webProfiles.push(profile);
      if (typeof args?.description === "string" && args.description.trim()) webDescriptions.set(profile.id, args.description.trim());
      webDetails[profile.id] = {
        base_url: baseUrl,
        api_key: apiKey,
        model_values: syntheticModelValues,
        config_fragment: syntheticFragment,
      };
      return profile as T;
    }
    case "codex_add_custom_profile": {
      const now = new Date().toISOString();
      const configText = String(args?.configText ?? "");
      const provider = /^\s*model_provider\s*=\s*["']([^"']+)["']/m.exec(configText)?.[1] ?? null;
      const baseUrl = typeof args?.baseUrl === "string" ? args.baseUrl.trim() : "";
      const apiKey = typeof args?.apiKey === "string" ? args.apiKey.trim() : "";
      const profile: CodexProfileSummary = {
        id: `profile-${Date.now()}`,
        name: String(args?.name ?? "自定义供应商"),
        kind: "third_party",
        account_id: null,
    plan_type: null,
        model: null,
        provider,
        reasoning_effort: null,
        fast_mode: readModelValue(configText, "service_tier") === "fast",
        has_base_url: Boolean(baseUrl),
        has_key: Boolean(apiKey),
        admin_url:
          typeof args?.adminUrl === "string" && args.adminUrl ? args.adminUrl : null,
        show_balance: false,
        icon: "custom",
        created_at: now,
        updated_at: now,
      };
      webProfiles.push(profile);
      if (typeof args?.description === "string" && args.description.trim()) webDescriptions.set(profile.id, args.description.trim());
      webDetails[profile.id] = {
        base_url: baseUrl || null,
        api_key: apiKey || null,
        model_values: {},
        config_fragment: configText,
        raw_config: configText,
        raw_catalog:
          typeof args?.catalogText === "string" && args.catalogText ? args.catalogText : null,
        raw_auth: typeof args?.authText === "string" && args.authText ? args.authText : null,
      };
      return profile as T;
    }
    case "codex_get_builtin_catalog": {
      const kind = String(args?.kind ?? "");
      if (!codexBuiltinHasCatalog(kind)) return null as T;
      return '{\n  "models": [\n    { "id": "preview", "name": "模型目录预览" }\n  ]\n}' as T;
    }
    case "codex_get_builtin_config": {
      const preset = codexPresetByKind(String(args?.kind ?? ""));
      if (!preset) throw new Error("未知的内置供应商类型");
      return mockBuiltinFragment(preset, String(args?.apiKey ?? "")) as T;
    }
    // 浏览器跨域请求无法可靠模拟供应商模型接口，保持明确失败。
    case "codex_fetch_provider_models":
      throw new Error("请在桌面版获取供应商模型列表");
    case "codex_fetch_chatgpt_models":
      // 浏览器没有桌面账号缓存；只读返回空，联网仍明确失败。
      if (args?.refresh === false) return [] as T;
      throw new Error("请在桌面版获取 ChatGPT 模型列表");
    case "codex_test_provider_connection": {
      const apiKey = String(args?.apiKey ?? "");
      const baseUrl = String(args?.baseUrl ?? "");
      if (!apiKey.trim()) throw new Error("请填写 API Key");
      if (!baseUrl.trim()) throw new Error("请填写 API 端点");
      const endpointFormatError = providerEndpointFormatError(baseUrl.trim());
      if (endpointFormatError) return endpointFormatError as T;
      return (await testProviderConnection(baseUrl.trim(), apiKey.trim())) as T;
    }
    case "codex_test_profile_connection": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      if (!profile.provider) {
        // 官方订阅：网页调试模式直接模拟认证连通正常
        return { ok: true, latency_ms: 12, status: 200, error: null } as T;
      }
      const apiKey = args?.apiKey !== undefined ? String(args.apiKey) : "saved-key";
      if (!apiKey.trim()) throw new Error("请填写 API Key");
      const baseUrl = args?.baseUrl !== undefined ? String(args.baseUrl) : "https://api.example.com";
      if (!baseUrl.trim()) throw new Error("请填写 API 端点");
      const endpointFormatError = providerEndpointFormatError(baseUrl.trim());
      if (endpointFormatError) return endpointFormatError as T;
      return (await testProviderConnection(baseUrl.trim(), apiKey.trim())) as T;
    }
    case "codex_get_profile_balance": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      // 与后端一致：官方 ChatGPT 配置按其固定登录来源查询用量。
      if (profile.kind === "official") {
        return { is_available: true, balance_infos: [webChatgptQuota], latency_ms: 210 } as T;
      }
      if (!balanceQueryProviders.has(profile.provider ?? "")) {
        throw new Error("该供应商不支持余额/用量查询");
      }
      await new Promise((resolve) => setTimeout(resolve, 400));
      if (profile.provider === "minimax" || profile.provider === "ZAI") {
        return {
          is_available: true,
          balance_infos: [
            {
              currency: "",
              total_balance: "",
              usage_percent: 15,
              usage_reset: "2h23m",
              usage_label: "5小时",
              weekly_usage_percent: 4,
              weekly_reset: "5d21h",
              weekly_label: "7天",
            },
          ],
          latency_ms: 210,
        } as T;
      }
      return {
        is_available: true,
        balance_infos: [
          {
            currency: "CNY",
            total_balance: "110.00",
            usage_percent: null,
            usage_reset: null,
            weekly_usage_percent: null,
            weekly_reset: null,
          },
        ],
        latency_ms: 210,
      } as T;
    }
    case "claude_get_profile_balance": {
      const profile = webClaudeProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("Claude 供应商配置不存在");
      if (profile.kind !== "deepseek" && profile.kind !== "minimax" && profile.kind !== "zhipu") throw new Error("该 Claude 供应商不支持用量查询");
      await new Promise((resolve) => setTimeout(resolve, 400));
      return profile.kind !== "deepseek"
        ? { is_available: true, balance_infos: [{ currency: "", total_balance: "", usage_percent: 15, usage_reset: "2h23m", usage_label: "5小时", weekly_usage_percent: 4, weekly_reset: "5d21h", weekly_label: "7天" }], latency_ms: 210 } as T
        : { is_available: true, balance_infos: [{ currency: "CNY", total_balance: "110.00", usage_percent: null, usage_reset: null, weekly_usage_percent: null, weekly_reset: null }], latency_ms: 210 } as T;
    }
    case "export_database": {
      const name = databaseBackupName();
      webBackups.unshift({ name, size_bytes: 20480, created_at: Math.floor(Date.now() / 1000) });
      webBackups = webBackups.slice(0, webSettings.database_backup_keep_count);
      return `C:\\Users\\<user>\\.agmux\\backups\\database\\${name}` as T;
    }
    case "export_database_to": {
      const name = databaseBackupName();
      const directory = String(args?.directory ?? "C:\\Users\\<user>\\Downloads");
      return `${directory}\\${name}` as T;
    }
    case "import_database":
      return undefined as T;
    case "list_database_backups":
      return [...webBackups] as T;
    case "restore_database":
      return undefined as T;
    case "delete_database_backup": {
      webBackups = webBackups.filter((backup) => backup.name !== args?.name);
      return undefined as T;
    }
    case "rename_database_backup": {
      const backup = webBackups.find((item) => item.name === args?.oldName);
      if (backup) {
        let stem = String(args?.title ?? "").trim();
        if (stem.startsWith("cg-backup-")) stem = stem.slice("cg-backup-".length);
        if (stem.startsWith("agmux-export-")) stem = stem.slice("agmux-export-".length);
        if (stem.endsWith(".db")) stem = stem.slice(0, -3);
        stem = stem.replace(/[<>:"/\\|?*]/g, "").trim();
        if (stem) backup.name = `cg-backup-${stem}.db`;
      }
      return undefined as T;
    }
    case "rename_profile": {
      const profiles = args?.tool === "claude" ? webClaudeProfiles : webProfiles;
      const profile = profiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      const name = String(args?.name ?? "").trim();
      if (!name || new TextEncoder().encode(name).length > 50) throw new Error("供应商名称长度必须在 1 到 50 个字符之间");
      profile.name = name;
      profile.updated_at = String(Date.now());
      return undefined as T;
    }
    case "codex_set_profile_icon": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (profile) profile.icon = (args?.icon as string | null) ?? null;
      return undefined as T;
    }
    case "codex_set_profile_show_balance": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (profile) profile.show_balance = Boolean(args?.enabled);
      return undefined as T;
    }
    case "codex_set_profile_fetched_models": {
      const detail = webDetails[String(args?.id)];
      if (detail) {
        detail.fetched_models = Array.isArray(args?.models) ? args.models.filter((model): model is string => typeof model === "string") : [];
        detail.fetched_model_efforts = (args?.efforts as Record<string, string[]> | undefined) ?? {};
        detail.fetched_model_defaults = (args?.defaults as Record<string, string> | undefined) ?? {};
      }
      return undefined as T;
    }
    case "set_profile_balance": {
      if (typeof args?.id === "string" && args?.info) {
        webBalanceCache[args.id] = args.info as ProfileBalanceInfo;
      }
      return undefined as T;
    }
    case "auth_get_quota":
      // 与后端一致：Settings 的 Codex/OAuth 账号均返回官方用量窗口。
      return { is_available: true, balance_infos: [webChatgptQuota], latency_ms: 210 } as T;
    case "auth_warmup":
      // 浏览器 mock 不持有真实账号凭证，不能模拟消耗订阅额度成功。
      throw new Error("ChatGPT warm-up requires the desktop app");
    case "codex_duplicate_profile": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      const now = new Date().toISOString();
      const { name, index } = profileCopyPlan(webProfiles, profile);
      const copy: CodexProfileSummary = {
        ...profile,
        id: `profile-${Date.now()}`,
        name,
        created_at: now,
        updated_at: now,
      };
      webProfiles.splice(index, 0, copy);
      const description = webDescriptions.get(profile.id);
      if (description) webDescriptions.set(copy.id, description);
      if (webDetails[profile.id]) webDetails[copy.id] = { ...webDetails[profile.id] };
      return copy as T;
    }
    case "codex_get_profile":
      return webProfileDetail(String(args?.id)) as T;
    case "codex_update_profile": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      profile.name = String(args?.name ?? profile.name);
      if (typeof args?.description === "string") {
        const description = args.description.trim();
        if (description) webDescriptions.set(profile.id, description);
        else webDescriptions.delete(profile.id);
      }
      const detail = webDetails[profile.id];
      if (detail) {
        if (typeof args?.baseUrl === "string") detail.base_url = args.baseUrl.trim() || null;
        if (typeof args?.apiKey === "string") detail.api_key = args.apiKey.trim() || null;
        profile.has_base_url = Boolean(detail.base_url?.trim());
        profile.has_key = Boolean(detail.api_key?.trim());
      }
      if (typeof args?.adminUrl === "string") profile.admin_url = args.adminUrl || null;
      return { ...profile } as T;
    }
    case "codex_set_profile_model": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      if (typeof args?.fast === "boolean" && (profile.kind !== "official" || profile.provider !== null)) {
        throw new Error("快速模式只支持官方 ChatGPT 订阅配置");
      }
      const detail = webDetails[profile.id];
      if (!detail) throw new Error("浏览器 mock 缺少配置详情");
      let text = detail.raw_config ?? detail.config_fragment;
      // 浏览器只有行级演示 parser；含多行字符串的配置交给桌面端 TOML parser。
      if (text.includes('"""') || text.includes("'''")) throw new Error("多行 TOML 配置需要在桌面版修改");
      for (const [arg, key] of [["model", "model"], ["effort", "model_reasoning_effort"]] as const) {
        if (typeof args?.[arg] !== "string") continue;
        const value = (args[arg] as string).trim();
        text = patchModelValue(text, value, key);
        if (value) detail.model_values[key] = JSON.stringify(value);
        else delete detail.model_values[key];
        if (arg === "model") profile.model = value || null;
        else profile.reasoning_effort = value || null;
      }
      if (typeof args?.fast === "boolean") {
        profile.fast_mode = args.fast;
        const value = args.fast ? "fast" : "";
        text = patchModelValue(text, value, "service_tier");
        if (value) detail.model_values.service_tier = JSON.stringify(value);
        else delete detail.model_values.service_tier;
      }
      detail.raw_config = text;
      detail.config_fragment = text;
      profile.updated_at = new Date().toISOString();
      return webProfileDetail(profile.id) as T;
    }
    case "codex_update_profile_config": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      const source = profile.auth_source ?? (profile.account_id ? "oauth" : profile.kind === "official" ? "desktop" : null);
      if (profile.kind === "official" && source === "oauth" && typeof args?.authText === "string") {
        throw new Error("OAuth 配置的认证由账号选择管理，不能编辑 Desktop auth.json");
      }
      const detail = webDetails[profile.id];
      if (detail) {
        if (source === "desktop" && typeof args?.authText === "string" && (args.authText.trim() || null) !== (detail.raw_auth ?? null)) {
          // 浏览器 mock 无法验证 Desktop JWT 身份，不能假装更新认证和账号模型缓存。
          throw new Error("请在桌面版更新 Desktop 认证和模型缓存");
        }
        if (typeof args?.configText === "string") {
          detail.raw_config = args.configText;
          profile.fast_mode = readModelValue(args.configText, "service_tier") === "fast";
        }
        if (typeof args?.catalogText === "string") detail.raw_catalog = args.catalogText.trim() ? args.catalogText : null;
        if (typeof args?.authText === "string") detail.raw_auth = args.authText.trim() ? args.authText : null;
      }
      return webProfileDetail(profile.id) as T;
    }
    case "codex_patch_chatgpt_context_config":
      return patchContextOverrideForWeb(
        String(args?.configText ?? ""),
        Boolean(args?.enabled),
        Number(args?.compactTokenLimit ?? 900_000),
      ) as T;
    case "codex_patch_system_proxy_config":
      return patchSystemProxyForWeb(
        String(args?.configText ?? ""),
        Boolean(args?.enabled),
      ) as T;
    case "codex_patch_context_management_config":
      return patchContextManagementForWeb(
        String(args?.configText ?? ""),
        Boolean(args?.enabled),
      ) as T;
    case "list_plugins":
      return webPlugins.map((plugin) => ({ ...plugin })) as T;
    case "list_skills":
      return webSkills.map((skill) => ({ ...skill })) as T;
    case "get_skill_content":
      return `---\ndescription: 浏览器调试用 Skill\n---\n\n# ${String(args?.name ?? "Skill")}` as T;
    case "get_import_skill_content":
      return `---\ndescription: 浏览器调试用导入 Skill\n---\n\n# ${String(args?.sourcePath ?? "Skill")}` as T;
    case "scan_unmanaged_skills":
      return (webSkills.some((skill) => skill.name === "local-skill") ? [] : [{ name: "local-skill", description: "本机已存在、尚未由 Agmux 管理", store_path: webLocalSkillPath, source: "Agent", has_content_conflict: false, is_update: false, modified_at: 0 } satisfies SkillCandidate]) as T;
    case "import_skill":
      if (args?.sourcePath === webLocalSkillPath && !webSkills.some((skill) => skill.name === "local-skill")) {
        webSkills.push({
          name: "local-skill", description: "本机已存在、尚未由 Agmux 管理",
          source_url: null, store_path: webLocalSkillPath, source_path: webLocalSkillPath,
          update_available: false, enabled: false, claude_enabled: false,
        });
      }
      return 1 as T;
    case "enable_skill":
    case "disable_skill": {
      // 与后端一致：按 tool 只翻动对应端的分发状态，两端互不影响
      const field = args?.tool === "claude" ? "claude_enabled" : "enabled";
      const skill = webSkills.find((item) => item.name === args?.name);
      if (skill) skill[field] = command === "enable_skill";
      return undefined as T;
    }
    case "delete_skill":
      return undefined as T;
    case "claude_get_cli_status":
      return { ...webClaudeCliStatus } as T;
    case "claude_check_cli_update":
      return { ...webClaudeCliUpdate, status: { ...webClaudeCliStatus } } as T;
    case "claude_install_cli":
    case "claude_update_cli":
    case "codex_get_cli_status":
    case "codex_check_cli_update":
    case "codex_install_cli":
    case "codex_update_cli":
      throw {
        stage: "desktop",
        kind: "validation_error",
        message: "CLI 检测、安装与升级需要在桌面版执行",
      } satisfies CliFailure;
    case "claude_get_common_settings":
      return webClaudeCommonSettings as T;
    case "claude_save_common_settings": {
      const text = args?.text as string | null;
      if (text !== null && splitEnvExtras(text) === undefined) throw new Error("settings.json 无效");
      webClaudeCommonSettings = text;
      return undefined as T;
    }
    // 浏览器无法拉起新终端窗口，也不该假装覆盖文件已写入。
    case "claude_open_terminal":
      throw new Error("启动终端需要在桌面版执行，浏览器无法模拟");
    case "claude_list_profiles":
      return webClaudeProfiles.map((profile) => ({
        id: profile.id,
        name: profile.name,
        base_url: profile.base_url,
        has_token: Boolean(profile.auth_token),
        model: profile.model,
        reasoning_effort: readAdvancedSettings(buildSettingsText(profile)).effortLevel || null,
        description: profile.description,
        icon: profile.icon,
        admin_url: profile.admin_url,
        kind: profile.kind,
        show_balance: profile.show_balance,
        updated_at: profile.updated_at,
      })) as T;
    case "claude_get_profile": {
      // 与后端一致：详情含 token 明文供编辑回显
      const profile = webClaudeProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("Claude 供应商配置不存在");
      return { ...profile } as T;
    }
    case "claude_capture_profile": {
      const name = String(args?.name ?? "").trim();
      if (!name) throw new Error("配置名称不能为空");
      const timestamp = String(Date.now());
      const captured: ClaudeProfileDetail = {
        id: `cla-web-${timestamp}`,
        name,
        base_url: "https://relay.example/v1",
        auth_token: "demo-token",
        model: "example-model",
        description: null,
        fetched_models: [],
        kind: null,
        admin_url: null,
        extra_env: null,
        raw_settings: JSON.stringify({ model: "sonnet", env: {
          ANTHROPIC_BASE_URL: "https://relay.example/v1",
          ANTHROPIC_AUTH_TOKEN: "demo-token",
          ANTHROPIC_MODEL: "example-model",
        } }, null, 2),
        icon: null,
        show_balance: false,
        sort_order: webClaudeProfiles.length,
        updated_at: timestamp,
      };
      webClaudeProfiles.push(captured);
      return { ...captured } as T;
    }
    case "claude_save_profile": {
      const timestamp = String(Date.now());
      const base = { name: String(args?.name ?? "").trim(), base_url: (args?.baseUrl as string | null) ?? null, auth_token: (args?.authToken as string | null) ?? null, model: (args?.model as string | null) ?? null, description: (args?.description as string | null) ?? null, fetched_models: (args?.fetchedModels as string[] | null) ?? [], kind: (args?.kind as string | null) ?? null, admin_url: (args?.adminUrl as string | null) ?? null, extra_env: (args?.extraEnv as string | null) ?? null, raw_settings: (args?.rawSettings as string | null) ?? null, icon: (args?.icon as string | null) ?? null, show_balance: Boolean(args?.showBalance) };
      if (!base.name) throw new Error("配置名称不能为空");
      if (base.raw_settings !== null) {
        const settings = JSON.parse(base.raw_settings) as Record<string, unknown>;
        if (!settings || typeof settings !== "object" || Array.isArray(settings)) throw new Error("settings.json 顶层必须是 JSON 对象");
        const env = (settings.env ?? {}) as Record<string, unknown>;
        if (!env || typeof env !== "object" || Array.isArray(env) || Object.values(env).some((value) => typeof value !== "string")) throw new Error("settings.json 的 env 必须是字符串键值对象");
        base.base_url = (env.ANTHROPIC_BASE_URL as string | undefined) ?? null;
        base.auth_token = (env.ANTHROPIC_AUTH_TOKEN as string | undefined) ?? (env.ANTHROPIC_API_KEY as string | undefined) ?? null;
        base.model = (env.ANTHROPIC_MODEL as string | undefined) ?? null;
        const extras = Object.fromEntries(Object.entries(env).filter(([key]) => !["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_MODEL"].includes(key)));
        base.extra_env = Object.keys(extras).length ? JSON.stringify(extras) : null;
      }
      if (typeof args?.id === "string") {
        const profile = webClaudeProfiles.find((item) => item.id === args?.id);
        if (!profile) throw new Error("Claude 供应商配置不存在");
        Object.assign(profile, base, { updated_at: timestamp });
        return { ...profile } as T;
      }
      const maxSort = webClaudeProfiles.reduce((max, item) => Math.max(max, item.sort_order), -1);
      const created: ClaudeProfileDetail = { id: `cla-web-${timestamp}`, ...base, sort_order: maxSort + 1, updated_at: timestamp };
      webClaudeProfiles.push(created);
      return { ...created } as T;
    }
    case "claude_set_profile_icon": {
      const profile = webClaudeProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("Claude 供应商配置不存在");
      profile.icon = (args?.icon as string | null) ?? null;
      return undefined as T;
    }
    case "claude_set_profile_show_balance": {
      const profile = webClaudeProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("Claude 供应商配置不存在");
      profile.show_balance = Boolean(args?.enabled);
      profile.updated_at = String(Date.now());
      return undefined as T;
    }
    case "claude_reorder_profiles": {
      const ids = (args?.ids as string[] | undefined) ?? [];
      ids.forEach((id, index) => {
        const profile = webClaudeProfiles.find((item) => item.id === id);
        if (profile) profile.sort_order = index;
      });
      webClaudeProfiles.sort((a, b) => a.sort_order - b.sort_order);
      return undefined as T;
    }
    case "claude_duplicate_profile": {
      const source = webClaudeProfiles.find((item) => item.id === args?.id);
      if (!source) throw new Error("Claude 供应商配置不存在");
      const timestamp = String(Date.now());
      const { name, index } = profileCopyPlan(webClaudeProfiles, source);
      const copy: ClaudeProfileDetail = { ...source, id: `cla-web-${timestamp}`, name, sort_order: index, updated_at: timestamp };
      webClaudeProfiles.splice(index, 0, copy);
      webClaudeProfiles.forEach((profile, sortOrder) => { profile.sort_order = sortOrder; });
      return { ...copy } as T;
    }
    case "claude_test_profile": {
      const profile = webClaudeProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("Claude 供应商配置不存在");
      if (!profile.base_url?.trim()) throw new Error("请先填写 API 地址");
      if (!profile.auth_token?.trim()) throw new Error("请先填写 API Token");
      throw new Error("请在桌面版验证 Claude API 端点和密钥，浏览器调试不模拟连通成功");
    }
    case "claude_delete_profile": {
      // 使用中的配置不可删除：与后端守卫一致
      if (webActiveClaudeProfileId === String(args?.id)) throw new Error("无法删除使用中的 Claude 供应商配置，请先切换到其他配置");
      const index = webClaudeProfiles.findIndex((item) => item.id === args?.id);
      if (index < 0) throw new Error("Claude 供应商配置不存在");
      webClaudeProfiles.splice(index, 1);
      return undefined as T;
    }
    case "claude_apply_profile": {
      if (!webClaudeProfiles.some((item) => item.id === args?.id)) throw new Error("Claude 供应商配置不存在");
      webActiveClaudeProfileId = String(args?.id);
      return undefined as T;
    }
    case "claude_fetch_models": {
      // 与后端一致：Anthropic 兼容 /v1/models，无 Token 直接拒绝
      if (!String(args?.authToken ?? "").trim()) throw new Error("请先填写 API Token 再获取模型列表");
      return ["glm-5.3", "glm-5.3-air", "glm-5.3[1m]"] as T;
    }
    case "claude_test_connection": {
      // 浏览器侧不发真实 HTTP，不能据必填字段模拟连通成功。
      if (!String(args?.baseUrl ?? "").trim()) throw new Error("请先填写 API 地址");
      if (!String(args?.authToken ?? "").trim()) throw new Error("请先填写 API Token");
      throw new Error("请在桌面版验证 Claude API 端点和密钥，浏览器调试不模拟连通成功");
    }
    case "list_plugin_skills": {
      // 与后端一致：storePath 仅用于 Tauri 端跳过重复的插件列表解析，Web mock 直接读 fixture。
      const name = String(args?.name ?? "");
      return (webPluginSkills[name] ?? []).map((skill) => ({ ...skill })) as T;
    }
    case "list_plugin_marketplaces":
      return [...webMarketplaces]
        .sort((left, right) => Number(left.kind !== "official") - Number(right.kind !== "official") || left.name.localeCompare(right.name))
        .map((marketplace) => ({ ...marketplace })) as T;
    case "list_marketplace_plugins":
      return (webMarketplacePlugins[String(args?.marketplace ?? "")] ?? []).map((plugin) => ({ ...plugin })) as T;
    case "add_plugin_marketplace": {
      const source = String(args?.url ?? "");
      const name = source.includes("ponytail") ? "ponytail" : source.includes("xiaolai") ? "xiaolai" : "third-party-marketplace";
      const marketplace: PluginMarketplace = {
        name,
        root: `C:\\Users\\<user>\\.codex\\.tmp\\marketplaces\\${name}`,
        kind: "third-party",
        source_url: source,
        display_name: name === "xiaolai" ? "xiaolai (Codex)" : name,
        description: name === "xiaolai"
          ? "第三方 Codex 插件市场，收录 xiaolai 维护的 Claude/Codex 插件"
          : "第三方 Codex 插件市场",
      };
      webMarketplaces = [...webMarketplaces.filter((item) => item.name !== name), marketplace];
      webMarketplacePlugins[name] ??= (webRecommendedMarketplacePlugins[name] ?? []).map((plugin) => ({ ...plugin }));
      return marketplace as T;
    }
    case "remove_plugin_marketplace": {
      const name = String(args?.name ?? "");
      if (name.startsWith("openai")) throw new Error("官方市场不能移除");
      webMarketplaces = webMarketplaces.filter((marketplace) => marketplace.name !== name);
      return undefined as T;
    }
    case "install_marketplace_plugin": {
      const marketplace = String(args?.marketplace ?? "");
      const name = String(args?.name ?? "");
      const catalog = webMarketplacePlugins[marketplace] ?? [];
      const item = catalog.find((plugin) => plugin.name === name);
      if (!item) throw new Error("没有找到可安装的插件");
      item.installed = true;
      const summary: PluginSummary = {
        name: item.name,
        version: item.version,
        display_name: item.display_name,
        description: item.description ?? `来自 ${marketplace} 市场的插件`,
        category: item.category,
        capabilities: item.capabilities,
        contains: item.contains,
        enabled: true,
        origin: "codex",
        marketplace,
        store_path: `C:\\Users\\<user>\\.codex\\plugins\\cache\\${marketplace}\\${item.name}\\${item.version ?? "latest"}`,
        source_url: item.source,
      };
      webPlugins = [...webPlugins.filter((plugin) => plugin.name !== name), summary].sort((a, b) => a.name.localeCompare(b.name));
      return summary as T;
    }
    case "check_plugin_updates":
      return webPluginUpdates.filter((update) => {
        const marketplace = webMarketplaces.find((item) => item.name === update.marketplace);
        const plugin = webPlugins.find((item) => item.name === update.name && item.marketplace === update.marketplace);
        return marketplace?.kind === "third-party" && plugin?.version !== update.version;
      }) as T;
    case "upgrade_marketplace_plugin": {
      const marketplace = String(args?.marketplace ?? "");
      const name = String(args?.name ?? "");
      const update = webPluginUpdates.find((item) => item.name === name && item.marketplace === marketplace);
      const target = webPlugins.find((item) => item.name === name && item.marketplace === marketplace);
      if (!update || !target || webMarketplaces.find((item) => item.name === marketplace)?.kind !== "third-party") {
        throw new Error("只能升级第三方插件市场中的插件");
      }
      target.version = update.version;
      (webMarketplacePlugins[marketplace] ?? []).forEach((plugin) => {
        if (plugin.name === name) plugin.version = update.version;
      });
      return undefined as T;
    }
    case "preview_plugin": {
      // 与后端一致：预览需要真实 GitHub 网络；浏览器环境仅对示例地址返回 fixture，其余抛错
      const url = String(args?.url ?? "");
      if (url.includes("openai/plugins")) return webPluginPreview as T;
      throw new Error("浏览器调试无法访问 GitHub，插件预览请在 Tauri 窗口测试");
    }
    case "install_plugin": {
      const candidate = webPluginPreview.candidates[0];
      const summary: PluginSummary = {
        name: candidate.name,
        version: candidate.version,
        display_name: candidate.display_name,
        description: candidate.description,
        category: null,
        capabilities: candidate.capabilities,
        contains: candidate.contains,
        enabled: true,
        origin: "codex",
        marketplace: "openai-plugins",
        store_path: `C:\\Users\\<user>\\.codex\\plugins\\cache\\openai-plugins\\${candidate.name}\\${candidate.version ?? "latest"}`,
        source_url: "https://github.com/openai/plugins",
      };
      webPlugins = [...webPlugins.filter((plugin) => plugin.name !== summary.name), summary].sort((a, b) => a.name.localeCompare(b.name));
      return summary as T;
    }
    case "uninstall_plugin": {
      // 与后端一致：官方市场拒绝卸载；Skill 在独立页面展示
      const target = webPlugins.find((plugin) => plugin.name === args?.name);
      if (webSkills.some((skill) => skill.name === args?.name)) {
        throw new Error("该 Skill 属于 Codex Skill 注册表，请在 Codex 内管理它");
      }
      if (target?.origin === "official") {
        throw new Error("该插件属于 Codex 官方市场，请在 Codex 内管理它");
      }
      webPlugins = webPlugins.filter((plugin) => plugin.name !== args?.name);
      Object.values(webMarketplacePlugins).forEach((plugins) => {
        plugins.forEach((plugin) => {
          if (plugin.name === args?.name) {
            plugin.installed = false;
          }
        });
      });
      return undefined as T;
    }
    case "codex_delete_profile": {
      // 使用中的配置不可删除：与后端守卫一致
      if (webActiveProfileId === String(args?.id)) throw new Error("无法删除使用中的供应商配置，请先切换到其他配置");
      const index = webProfiles.findIndex((item) => item.id === args?.id);
      if (index >= 0) webProfiles.splice(index, 1);
      webDescriptions.delete(String(args?.id));
      return undefined as T;
    }
    case "codex_reorder_profiles": {
      const ids = Array.isArray(args?.ids) ? (args.ids as string[]) : [];
      webProfiles.sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id));
      return undefined as T;
    }
    case "codex_apply_profile":
      // 与后端一致：官方配置的认证来源在创建时固定；Web mock 不操作本机 auth.json。
      webActiveProfileId = typeof args?.id === "string" ? args.id : null;
      await new Promise((resolve) => setTimeout(resolve, 500));
      return undefined as T;
    case "restart_codex":
      await new Promise((resolve) => setTimeout(resolve, 500));
      return undefined as T;
    case "set_window_theme":
      return undefined as T;
    // 浏览器调试无系统托盘，仅接受调用不产生副作用
    case "set_tray_menu":
      return undefined as T;
    case "auth_get_status":
      return webAuthStatus as T;
    case "auth_preview": {
      if (typeof args?.accountId !== "string" || !args.accountId) {
        throw new Error("OAuth 账号不能为空");
      }
      const accountId = args.accountId;
      return JSON.stringify({
        auth_mode: "chatgpt",
        OPENAI_API_KEY: null,
        tokens: { account_id: accountId, access_token: "<preview-only>" },
      }, null, 2) as T;
    }
    case "codex_set_profile_account": {
      const profile = webProfiles.find((item) => item.id === args?.id);
      if (!profile) throw new Error("供应商配置不存在");
      const source = profile.auth_source ?? (profile.account_id ? "oauth" : profile.kind === "official" ? "desktop" : null);
      if (source !== "oauth") throw new Error("Desktop 配置不能切换为 OAuth，请新建 ChatGPT 配置");
      if (typeof args?.accountId !== "string" || !args.accountId) throw new Error("OAuth 配置必须绑定一个订阅账号");
      if (profile.account_id !== args.accountId) {
        const detail = webDetails[profile.id];
        if (detail) { detail.fetched_models = []; detail.fetched_model_efforts = {}; detail.fetched_model_defaults = {}; }
      }
      profile.account_id = args.accountId;
      return undefined as T;
    }
    case "open_url":
      return undefined as T;
    case "open_path":
      return undefined as T;
    // 启动里程碑只落桌面端日志；web 调试无日志管道，直接吞掉
    case "report_startup_mark":
      return undefined as T;
    case "save_settings":
      webSettings = { ...(args?.settings as Settings) };
      webBackups = webBackups.slice(0, webSettings.database_backup_keep_count);
      return { ...webSettings } as T;
    // Web 调试模式无 Rust 侧解析器，TOML 校验一律视为通过（与既有 mock 桩风格一致）
    case "validate_toml":
      return [] as T;
    // Web 调试模式无 Rust 侧格式化器，保持输入透传，供浏览器走查按钮流程
    case "format_toml":
      return String(args?.text ?? "") as T;
    case "codex_list_mcp_servers":
      return [...webMcpServers] as T;
    case "claude_list_mcp_servers":
      return [...webClaudeMcpServers] as T;
    case "claude_get_mcp_server_json": {
      const server = webClaudeMcpServers.find((item) => item.name === args?.name);
      return (server ? renderClaudeMcpEntryWeb(server) : null) as unknown as T;
    }
    case "claude_save_mcp_server": {
      const name = String(args?.name ?? "").trim();
      const original = typeof args?.originalName === "string" ? args.originalName : null;
      const spec = parseClaudeMcpEntryWeb(name, String(args?.json ?? "{}"));
      saveWebMcpServer(original, spec, "claude");
      return undefined as T;
    }
    case "claude_delete_mcp_server":
      webClaudeMcpServers = webClaudeMcpServers.filter((server) => server.name !== args?.name);
      return undefined as T;
    case "probe_mcp_server": {
      const name = String(args?.name ?? "MCP");
      const includeTools = Boolean(args?.includeTools);
      // args?.manual 仅后端日志分级用，mock 恒为成功，无需区分
      const result: McpProbeResult = {
        ok: true,
        latency_ms: 18,
        status: name === "tavily" ? 200 : null,
        protocol_version: "2025-11-25",
        server_info: { name: `${name} demo`, version: "1.0.0" },
        tools: includeTools ? [
          {
            name: "search",
            title: "Search",
            description: "Search available sources",
            input_schema: { type: "object", properties: { query: { type: "string" } }, required: ["query"] },
          },
          {
            name: "fetch",
            title: "Fetch",
            description: "Fetch a source by URL",
            input_schema: { type: "object", properties: { url: { type: "string" } }, required: ["url"] },
          },
        ] : [],
        tools_truncated: false,
        error: null,
        tools_error: null,
      };
      return result as T;
    }
    case "codex_get_mcp_section_toml": {
      // 创建表单预填用：把 mock 列表渲染成 config.toml 片段
      return webMcpServers.map((server) => renderMcpFragmentWeb({ ...server, enabled: null })).join("\n") as T;
    }
    case "restore_mcp_from_database":
      return webMcpServers.length as T;
    case "codex_mcp_sync_preview": {
      // web 调试样例：一条“内容不同”+ 一条“仅配置文件”，便于在 pnpm dev 里走查差异页
      const first = webMcpServers[0];
      const changed = first
        ? {
            name: first.name,
            kind: "changed" as const,
            live_spec: first,
            db_spec: { ...first, url: first.url ? first.url + "-old" : first.url },
            live_toml: `[mcp_servers.${first.name}]\nurl = "${first.url ?? ""}"`,
            db_toml: `[mcp_servers.${first.name}]\nurl = "${first.url ? first.url + "-old" : ""}"`,
          }
        : null;
      const liveOnly = {
        name: "web-demo-live-only",
        kind: "live_only" as const,
        live_spec: {
          name: "web-demo-live-only",
          enabled: null,
          startup_timeout_sec: null,
          tool_timeout_sec: null,
          command: "npx",
          args: ["-y", "web-demo"],
          env: {},
          url: null,
          bearer_token_env_var: null,
          http_headers: {},
          env_http_headers: {},
        },
        db_spec: null,
        live_toml: '[mcp_servers.web-demo-live-only]\ncommand = "npx"\nargs = ["-y", "web-demo"]',
        db_toml: null,
      };
      const entries = [changed, liveOnly].filter((entry) => entry !== null);
      return {
        entries,
        live_count: webMcpServers.length + 1,
        db_count: webMcpServers.length,
      } as T;
    }
    case "codex_save_mcp_server": {
      const spec = args?.spec as McpServerSpec;
      const original = typeof args?.originalName === "string" ? args.originalName : null;
      saveWebMcpServer(original, spec, "codex");
      return undefined as T;
    }
    // —— MCP 编辑页双向同步的 web 调试桩（精度有限：仅渲染/解析建模字段，
    //    未建模键与注释不保留；真实保真逻辑在 Rust 侧 toml_edit）——
    case "codex_get_mcp_server_toml": {
      const server = webMcpServers.find((item) => item.name === args?.name);
      return (server ? renderMcpFragmentWeb({ ...server, enabled: null }) : null) as unknown as T;
    }
    case "patch_mcp_fragment": {
      const source = await webInvoke<McpServerSpec>("parse_mcp_fragment", { toml: args?.toml });
      return renderMcpFragmentWeb({ ...args?.spec as McpServerSpec, enabled: source.enabled }) as unknown as T;
    }
    case "parse_mcp_fragment": {
      const text = String(args?.toml ?? "");
      const spec: McpServerSpec = {
        name: "",
        enabled: null,
        startup_timeout_sec: null,
        tool_timeout_sec: null,
        command: null,
        args: [],
        env: {},
        url: null,
        bearer_token_env_var: null,
        http_headers: {},
        env_http_headers: {},
      };
      let inEnv = false;
      for (const raw of text.split("\n")) {
        const line = raw.trim();
        const envHeader = /^\[mcp_servers\.([A-Za-z0-9_-]+)\.env\]$/.exec(line);
        if (envHeader) {
          spec.name = envHeader[1];
          inEnv = true;
          continue;
        }
        const header = /^\[mcp_servers\.([A-Za-z0-9_-]+)\]$/.exec(line);
        if (header) {
          spec.name = header[1];
          inEnv = false;
          continue;
        }
        if (/^\[.*\]$/.test(line)) {
          inEnv = false;
          continue;
        }
        if (inEnv) {
          const envMatch = /^(\w+)\s*=\s*"(.*)"$/.exec(line);
          if (envMatch) spec.env[envMatch[1]] = envMatch[2];
          continue;
        }
        const enabled = /^enabled\s*=\s*(true|false)(?:\s*#.*)?$/.exec(line);
        if (enabled) { spec.enabled = enabled[1] === "true"; continue; }
        const match = /^(command|url|bearer_token_env_var|args)\s*=\s*(.+)$/.exec(line);
        if (!match) continue;
        const value = match[2].trim();
        if (match[1] === "args") {
          try {
            spec.args = (JSON.parse(value.replace(/'/g, '"')) as unknown[]).map(String);
          } catch {
            // 输入进行中的中间态，忽略
          }
        } else {
          const str = value.replace(/^"|"$/g, "");
          if (match[1] === "command") spec.command = str;
          else if (match[1] === "url") spec.url = str;
          else spec.bearer_token_env_var = str;
        }
      }
      if (!spec.name) throw new Error("Web 调试模式：片段中没有服务器");
      return spec as T;
    }
    case "codex_delete_mcp_server":
      webMcpServers = webMcpServers.filter((server) => server.name !== args?.name);
      return undefined as T;
    case "set_mcp_server_enabled": {
      const name = String(args?.name ?? "");
      const enabled = Boolean(args?.enabled);
      const servers = args?.tool === "claude" ? webClaudeMcpServers : webMcpServers;
      const next = servers.map((server) => server.name === name
        ? { ...server, enabled: enabled ? null : false }
        : server);
      if (args?.tool === "claude") webClaudeMcpServers = next;
      else webMcpServers = next;
      return undefined as T;
    }
    case "set_mcp_mirror":
    case "revert_mcp_live":
    case "set_mcp_mirror_entries":
    case "revert_mcp_live_entries": {
      // web 调试沙箱只有一份服务器列表、没有独立的镜像侧：单条与批量都当同名 upsert 处理，
      // fragment 为空表示删除该条目；fragment 不做 TOML 解析
      const batched = Array.isArray(args?.actions);
      const actions = batched
        ? (args?.actions as McpDiffEntryAction[])
        : [{ name: args?.name as string, fragment: (args?.fragment ?? null) as string | null }];
      for (const action of actions) {
        webMcpServers = webMcpServers.filter((server) => server.name !== action.name);
        if (action.fragment) webMcpServers.push({ name: action.name } as McpServerSpec);
      }
      return (batched ? actions.length : undefined) as T;
    }
    // 浏览器没有独立 Claude 实时 JSON 和数据库镜像，不能可靠模拟差异裁决。
    case "claude_mcp_sync_preview":
    case "claude_resolve_mcp_entries":
      throw new Error("Web 调试模式不支持 Claude MCP 实时配置与数据库差异处理");
    // 认证登录流程没有可靠的浏览器 mock；保持默认错误，避免伪造 OAuth 状态
    default:
      throw new Error(`Web 调试模式不支持命令：${command}`);
  }
}
