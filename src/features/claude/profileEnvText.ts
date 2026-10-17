/** 完整 settings.json 编辑器 ↔ 表单托管键的双向同步。 */

/** Agmux 托管的 env 键：编辑器里可见、与表单双向绑定；使用时由后端写入 settings.json。 */
export const CLAUDE_MANAGED_ENV_KEYS = ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_MODEL"] as const;

export interface ClaudeEnvFields {
  baseUrl: string;
  authToken: string;
  model: string;
}

/** Claude Code 支持的模型入口；展示元数据仍由全文编辑器保留，不单独做表单项。 */
export const CLAUDE_MODEL_MAPPING_KEYS = [
  "ANTHROPIC_MODEL",
  // 表单不展示：v2.1.236 才新增，且填了 ANTHROPIC_MODEL 时永远轮不到它。仍留在托管列表里，
  // 已有配置里的这个键才会被原样读回、写回（删行不删数据，回滚只需把行加回表单）
  "ANTHROPIC_DEFAULT_MODEL",
  "ANTHROPIC_SMALL_FAST_MODEL",
  "ANTHROPIC_DEFAULT_FABLE_MODEL",
  "ANTHROPIC_DEFAULT_OPUS_MODEL",
  "ANTHROPIC_DEFAULT_SONNET_MODEL",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL",
  "CLAUDE_CODE_SUBAGENT_MODEL",
  "ANTHROPIC_CUSTOM_MODEL_OPTION",
] as const;

export type ClaudeModelMappingKey = typeof CLAUDE_MODEL_MAPPING_KEYS[number];
export type ClaudeModelMappings = Record<ClaudeModelMappingKey, string>;

/** 只展示会出现在 /model 菜单中的自定义名称；描述和能力仍留在全文编辑器。 */
export const CLAUDE_MODEL_DISPLAY_KEYS = [
  "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
  "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
  "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
  "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
  "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
] as const;

export type ClaudeModelDisplayKey = typeof CLAUDE_MODEL_DISPLAY_KEYS[number];
export type ClaudeModelDisplayNames = Record<ClaudeModelDisplayKey, string>;

// 通用模板只排除应用实际可填写的字段，不按前缀删除用户自定义 env。
const TEMPLATE_EXCLUDED_ENV_KEYS = new Set<string>([
  ...CLAUDE_MANAGED_ENV_KEYS,
  "ANTHROPIC_API_KEY",
  ...CLAUDE_MODEL_MAPPING_KEYS,
  ...CLAUDE_MODEL_DISPLAY_KEYS,
  "DISABLE_AUTO_COMPACT",
  "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
  "CLAUDE_CODE_ATTRIBUTION_HEADER",
  "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
  "CLAUDE_CODE_EFFORT_LEVEL",
  "CLAUDE_CODE_DISABLE_AUTO_MEMORY",
  "CLAUDE_CODE_BASH_EDIT_DIFF",
]);

const oneMillionSuffix = /\[1m\]\s*$/i;

export function hasOneMillionModelSuffix(value: string) {
  return oneMillionSuffix.test(value.trim());
}

export function setOneMillionModelSuffix(value: string, enabled: boolean) {
  const base = value.trim().replace(oneMillionSuffix, "");
  return enabled && base ? `${base}[1M]` : base;
}

/** 卡片选主会话模型时按本配置的既有声明决定 1M 后缀：任一映射槽（含主模型自身）为同 base
 * 模型声明过 [1M] 就带上；悬浮卡不展示该开关，声明是用户意图的唯一真源。 */
export function resolveOneMillionByDeclaration(text: string, next: string) {
  const base = setOneMillionModelSuffix(next, false);
  const declared = Object.values(readModelMappings(text))
    .some((value) => hasOneMillionModelSuffix(value) && setOneMillionModelSuffix(value, false) === base);
  return setOneMillionModelSuffix(next, declared);
}

const MANAGED_KEY_SET = new Set<string>(CLAUDE_MANAGED_ENV_KEYS);
const CLAUDE_API_KEY_KINDS = new Set(["anthropic", "kimi-code", "siliconflow"]);

function usesClaudeApiKey(kind?: string | null) {
  return kind != null && CLAUDE_API_KEY_KINDS.has(kind);
}

function parseSettingsObject(text: string): Record<string, unknown> | null {
  try {
    const value = JSON.parse(text) as unknown;
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    return value as Record<string, unknown>;
  } catch {
    return null;
  }
}

function parseSettingsForEdit(text: string) {
  return text.trim() ? parseSettingsObject(text) : {};
}

/** 本地 JSON 排版的结果：只判状态，提示文案与写回由调用方自己接（各界面标签不同）。 */
export type FormatJsonOutcome =
  | { status: "empty" }
  | { status: "unchanged" }
  | { status: "formatted"; text: string }
  | { status: "failed"; error: string };

/** 排版按钮的共同逻辑：空文本、无法解析、已排好、需要写回这四种结局。 */
export function formatJsonText(text: string): FormatJsonOutcome {
  if (!text.trim()) return { status: "empty" };
  try {
    const formatted = JSON.stringify(JSON.parse(text), null, 2);
    return formatted === text ? { status: "unchanged" } : { status: "formatted", text: formatted };
  } catch (error) {
    return { status: "failed", error: String(error) };
  }
}

/** 读取 Claude Code env 中的任意字符串字段，供高级开关复用。 */
export function readEnvValue(text: string, key: string) {
  const settings = parseSettingsObject(text);
  const env = settings?.env;
  if (!env || typeof env !== "object" || Array.isArray(env)) return "";
  const value = (env as Record<string, unknown>)[key];
  return typeof value === "string" ? value : "";
}

/** 只更新一个 Claude Code env 字段，保留 settings.json 其他内容。 */
export function patchEnvValue(text: string, key: string, value: string | null, fallbackSettingsKey?: string) {
  const settings = parseSettingsForEdit(text);
  if (!settings) return text;
  const currentEnv = settings.env === undefined ? {} : settings.env;
  if (!currentEnv || typeof currentEnv !== "object" || Array.isArray(currentEnv)) return text;
  const env = { ...currentEnv } as Record<string, unknown>;
  const nextValue = value?.trim() ?? "";
  if (nextValue) env[key] = nextValue;
  else delete env[key];
  // 用户编辑同一能力时清除顶层同义设置，避免恢复默认后旧值继续生效。
  const hadFallback = fallbackSettingsKey !== undefined && Object.prototype.hasOwnProperty.call(settings, fallbackSettingsKey);
  if (fallbackSettingsKey !== undefined) delete settings[fallbackSettingsKey];
  if (JSON.stringify(env) === JSON.stringify(currentEnv) && !hadFallback) return text;
  return JSON.stringify({ ...settings, env }, null, 2);
}

/** 环境变量优先；自动压缩只要任一配置关闭即关闭，auto 表示模型默认推理强度。 */
export function readAdvancedSettings(text: string) {
  const settings = parseSettingsObject(text);
  const effortLevel = readEnvValue(text, "CLAUDE_CODE_EFFORT_LEVEL")
    || (typeof settings?.effortLevel === "string" ? settings.effortLevel : "");
  const disableAutoMemory = readEnvValue(text, "CLAUDE_CODE_DISABLE_AUTO_MEMORY");
  const bashEditDiff = readEnvValue(text, "CLAUDE_CODE_BASH_EDIT_DIFF");
  const permissions = settings?.permissions;
  return {
    autoCompactDisabled: settings?.autoCompactEnabled === false || readEnvValue(text, "DISABLE_AUTO_COMPACT") === "1",
    autoCompactWindow: readEnvValue(text, "CLAUDE_CODE_AUTO_COMPACT_WINDOW")
      || (typeof settings?.autoCompactWindow === "number" ? String(settings.autoCompactWindow) : ""),
    effortLevel: effortLevel === "auto" ? "" : effortLevel,
    autoMemoryEnabled: disableAutoMemory === "0" || (disableAutoMemory !== "1" && settings?.autoMemoryEnabled !== false),
    bashEditDiffEnabled: bashEditDiff === "1" || (bashEditDiff !== "0" && settings?.bashEditDiffEnabled === true),
    bypassPermissionsEnabled: !!permissions && typeof permissions === "object" && !Array.isArray(permissions)
      && (permissions as Record<string, unknown>).defaultMode === "bypassPermissions",
  };
}

/** 只切换默认 bypass 模式，保留权限规则和组织限制；取消不改变其他模式。 */
export function patchBypassPermissions(text: string, enabled: boolean) {
  const settings = parseSettingsForEdit(text);
  if (!settings) return text;
  const current = settings.permissions === undefined ? {} : settings.permissions;
  if (!current || typeof current !== "object" || Array.isArray(current)) return text;
  const permissions = { ...current } as Record<string, unknown>;
  if (enabled) {
    if (permissions.defaultMode === "bypassPermissions") return text;
    permissions.defaultMode = "bypassPermissions";
  } else {
    if (permissions.defaultMode !== "bypassPermissions") return text;
    delete permissions.defaultMode;
  }
  if (Object.keys(permissions).length) settings.permissions = permissions;
  else delete settings.permissions;
  return JSON.stringify(settings, null, 2);
}

/** 读取 Claude Code 顶层 Git 归属开关；兼容已弃用的 includeCoAuthoredBy。 */
export function readGitAttributionDisabled(text: string) {
  const settings = parseSettingsObject(text);
  if (!settings) return false;
  const attribution = settings.attribution;
  if (attribution === false) return true;
  if (attribution && typeof attribution === "object" && !Array.isArray(attribution)) {
    const values = attribution as Record<string, unknown>;
    if (values.commit !== undefined || values.pr !== undefined) return values.commit === "" && values.pr === "";
  }
  return settings.includeCoAuthoredBy === false;
}

/** 用官方 attribution.commit/pr 关闭或恢复 Claude Code 的 Git 归属文本。 */
export function patchGitAttribution(text: string, disabled: boolean) {
  const settings = parseSettingsForEdit(text);
  if (!settings) return text;
  const current = settings.attribution;
  const attribution: Record<string, unknown> = current && typeof current === "object" && !Array.isArray(current)
    ? { ...(current as Record<string, unknown>) }
    : current === false && disabled ? { sessionUrl: false } : {};
  if (disabled) {
    attribution.commit = "";
    attribution.pr = "";
  } else {
    delete attribution.commit;
    delete attribution.pr;
    delete settings.includeCoAuthoredBy;
  }
  if (Object.keys(attribution).length) settings.attribution = attribution;
  else delete settings.attribution;
  return JSON.stringify(settings, null, 2);
}

export function emptyModelMappings(): ClaudeModelMappings {
  return Object.fromEntries(CLAUDE_MODEL_MAPPING_KEYS.map((key) => [key, ""])) as ClaudeModelMappings;
}

function emptyModelDisplayNames(): ClaudeModelDisplayNames {
  return Object.fromEntries(CLAUDE_MODEL_DISPLAY_KEYS.map((key) => [key, ""])) as ClaudeModelDisplayNames;
}

/** 从完整 settings.json 读取 Claude Code 的模型映射字段。 */
export function readModelMappings(text: string): ClaudeModelMappings {
  const settings = parseSettingsObject(text);
  const env = settings?.env;
  if (!env || typeof env !== "object" || Array.isArray(env)) return emptyModelMappings();
  const values = env as Record<string, unknown>;
  const mappings = emptyModelMappings();
  for (const key of CLAUDE_MODEL_MAPPING_KEYS) {
    if (typeof values[key] === "string") mappings[key] = values[key];
  }
  return mappings;
}

/** 只更新模型映射字段，保留 settings.json 其他顶层键和 env 键。 */
export function patchModelMappings(text: string, mappings: ClaudeModelMappings): string {
  const settings = parseSettingsForEdit(text);
  if (!settings) return text;
  const currentEnv = settings.env === undefined ? {} : settings.env;
  if (!currentEnv || typeof currentEnv !== "object" || Array.isArray(currentEnv)) return text;
  const env = { ...currentEnv } as Record<string, unknown>;
  for (const key of CLAUDE_MODEL_MAPPING_KEYS) {
    const value = mappings[key].trim();
    if (value) env[key] = value;
    else delete env[key];
  }
  if (JSON.stringify(env) === JSON.stringify(currentEnv)) return text;
  return JSON.stringify({ ...settings, env }, null, 2);
}

/** 从完整 settings.json 读取模型选择器的显示名称。 */
export function readModelDisplayNames(text: string): ClaudeModelDisplayNames {
  const settings = parseSettingsObject(text);
  const env = settings?.env;
  if (!env || typeof env !== "object" || Array.isArray(env)) return emptyModelDisplayNames();
  const values = env as Record<string, unknown>;
  const names = emptyModelDisplayNames();
  for (const key of CLAUDE_MODEL_DISPLAY_KEYS) {
    if (typeof values[key] === "string") names[key] = values[key];
  }
  return names;
}

/** 只更新模型选择器显示名称，保留其他 settings 内容。 */
export function patchModelDisplayNames(text: string, names: ClaudeModelDisplayNames): string {
  const settings = parseSettingsForEdit(text);
  if (!settings) return text;
  const currentEnv = settings.env === undefined ? {} : settings.env;
  if (!currentEnv || typeof currentEnv !== "object" || Array.isArray(currentEnv)) return text;
  const env = { ...currentEnv } as Record<string, unknown>;
  for (const key of CLAUDE_MODEL_DISPLAY_KEYS) {
    const value = names[key].trim();
    if (value) env[key] = value;
    else delete env[key];
  }
  if (JSON.stringify(env) === JSON.stringify(currentEnv)) return text;
  return JSON.stringify({ ...settings, env }, null, 2);
}

/** 旧配置没有全文时，只能从已保存的 env 字段构造兼容视图。 */
export function buildSettingsText(detail: { raw_settings: string | null; base_url: string | null; auth_token: string | null; model: string | null; extra_env: string | null; kind?: string | null }): string {
  if (detail.raw_settings !== null) return detail.raw_settings;
  const env: Record<string, unknown> = parseSettingsObject(detail.extra_env ?? "") ?? {};
  const managed: [string, string | null][] = [
    ["ANTHROPIC_BASE_URL", detail.base_url],
    [usesClaudeApiKey(detail.kind) ? "ANTHROPIC_API_KEY" : "ANTHROPIC_AUTH_TOKEN", detail.auth_token],
    ["ANTHROPIC_MODEL", detail.model],
  ];
  for (const [key, value] of managed) {
    if (value) env[key] = value;
    else delete env[key];
  }
  if (detail.kind === "openrouter") env.ANTHROPIC_API_KEY = "";
  return JSON.stringify({ env }, null, 2);
}

/** 从完整文件的 env 对象读出三个托管键。 */
export function readEnvFields(text: string, kind?: string | null): ClaudeEnvFields | null {
  if (!text.trim()) return { baseUrl: "", authToken: "", model: "" };
  const settings = parseSettingsObject(text);
  if (!settings) return null;
  const env = settings.env === undefined ? {} : settings.env;
  if (!env || typeof env !== "object" || Array.isArray(env)) return null;
  const fields = env as Record<string, unknown>;
  const value = (key: string) => (typeof fields[key] === "string" ? (fields[key] as string) : "");
  return {
    baseUrl: value("ANTHROPIC_BASE_URL"),
    authToken: usesClaudeApiKey(kind) ? value("ANTHROPIC_API_KEY") || value("ANTHROPIC_AUTH_TOKEN") : value("ANTHROPIC_AUTH_TOKEN"),
    model: value("ANTHROPIC_MODEL"),
  };
}

/** 表单只改完整文件的 env 托管键，其余顶层键原样保留。 */
export function patchEnvFields(text: string, fields: ClaudeEnvFields, kind?: string | null): string {
  const settings = parseSettingsForEdit(text);
  if (!settings) return text;
  const currentEnv = settings.env === undefined ? {} : settings.env;
  if (!currentEnv || typeof currentEnv !== "object" || Array.isArray(currentEnv)) return text;
  const env = { ...currentEnv } as Record<string, unknown>;
  const managed: [string, string][] = [
    ["ANTHROPIC_BASE_URL", fields.baseUrl.trim()],
    [usesClaudeApiKey(kind) ? "ANTHROPIC_API_KEY" : "ANTHROPIC_AUTH_TOKEN", fields.authToken.trim()],
    ["ANTHROPIC_MODEL", fields.model.trim()],
  ];
  for (const [key, value] of managed) {
    if (value) env[key] = value;
    else delete env[key];
  }
  if (usesClaudeApiKey(kind)) delete env.ANTHROPIC_AUTH_TOKEN;
  // OpenRouter 等网关要求保留空的 ANTHROPIC_API_KEY，避免回退官方鉴权；后端 apply 同步硬编码此规则。
  else if (kind === "openrouter") env.ANTHROPIC_API_KEY = "";
  else delete env.ANTHROPIC_API_KEY;
  const hadHelper = kind === "claude-account" && Object.prototype.hasOwnProperty.call(settings, "apiKeyHelper");
  if (kind === "claude-account") {
    for (const key of ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY", "ANTHROPIC_PROFILE", "ANTHROPIC_FEDERATION_RULE_ID", "ANTHROPIC_ORGANIZATION_ID"]) delete env[key];
    delete settings.apiKeyHelper;
  }
  if (JSON.stringify(env) === JSON.stringify(currentEnv) && !hadHelper) return text;
  return JSON.stringify({ ...settings, env }, null, 2);
}

/** Profile 与通用模板保存共用的文档校验。 */
function parseSettingsWithEnv(text: string): Record<string, unknown> | null {
  const settings = parseSettingsObject(text);
  if (!settings) return null;
  const env = settings.env === undefined ? {} : settings.env;
  if (!env || typeof env !== "object" || Array.isArray(env)) return null;
  if (Object.values(env).some((value) => typeof value !== "string")) return null;
  return settings;
}

/** 保存时校验完整文件并拆分 env 附加键。 */
export function splitEnvExtras(text: string): string | null | undefined {
  const settings = parseSettingsWithEnv(text);
  if (!settings) return undefined;
  const env = (settings.env ?? {}) as Record<string, string>;
  const extras = Object.fromEntries(Object.entries(env).filter(([key]) => !MANAGED_KEY_SET.has(key)));
  return Object.keys(extras).length ? JSON.stringify(extras, null, 2) : null;
}

/** 从全文提取通用字段；非法文档返回 null，调用方保留原草稿。 */
export function extractClaudeCommonSettings(text: string): string | null {
  const settings = parseSettingsWithEnv(text);
  if (!settings) return null;
  if (settings.env !== undefined) {
    const env = Object.fromEntries(Object.entries(settings.env as Record<string, string>)
      .filter(([key]) => !TEMPLATE_EXCLUDED_ENV_KEYS.has(key)));
    if (Object.keys(env).length) settings.env = env;
    else delete settings.env;
  }
  for (const key of ["autoCompactEnabled", "autoCompactWindow", "effortLevel", "includeCoAuthoredBy", "autoMemoryEnabled", "bashEditDiffEnabled"]) delete settings[key];
  const permissions = settings.permissions;
  if (permissions && typeof permissions === "object" && !Array.isArray(permissions)) {
    const fields = permissions as Record<string, unknown>;
    delete fields.defaultMode;
    if (!Object.keys(fields).length) delete settings.permissions;
  }
  const attribution = settings.attribution;
  if (attribution === false) delete settings.attribution;
  if (attribution && typeof attribution === "object" && !Array.isArray(attribution)) {
    const fields = attribution as Record<string, unknown>;
    delete fields.commit;
    delete fields.pr;
    if (!Object.keys(fields).length) delete settings.attribution;
  }
  return JSON.stringify(settings, null, 2);
}

function fillMissingSettings(current: Record<string, unknown>, template: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries([
    ...Object.entries(current).map(([key, value]) => {
      const fallback = Object.prototype.hasOwnProperty.call(template, key) ? template[key] : undefined;
      return [key, value && typeof value === "object" && !Array.isArray(value)
        && fallback && typeof fallback === "object" && !Array.isArray(fallback)
        ? fillMissingSettings(value as Record<string, unknown>, fallback as Record<string, unknown>)
        : value];
    }),
    ...Object.entries(template).filter(([key]) => !Object.prototype.hasOwnProperty.call(current, key)),
  ]);
}

/** 通用字段以模板为准，数组整体替换；再次过滤模板，保留当前表单托管字段。 */
export function fillClaudeCommonSettings(text: string, templateText: string): string | null {
  const settings = parseSettingsWithEnv(text);
  const common = extractClaudeCommonSettings(templateText);
  if (!settings || common === null) return null;
  const merged = fillMissingSettings(JSON.parse(common) as Record<string, unknown>, settings);
  return JSON.stringify(merged) === JSON.stringify(settings) ? text : JSON.stringify(merged, null, 2);
}
