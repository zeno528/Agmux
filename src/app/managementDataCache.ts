import { api } from "../api";
import type { ClaudeProfileSummary, CliStatus, DatabaseBackupInfo, MarketplacePlugin, McpProbeResult, McpServerSpec, PluginMarketplace, PluginSummary, SkillSummary } from "../types";

export type McpProbeCacheEntry = {
  fingerprint: string;
  result: McpProbeResult;
  toolsLoaded: boolean;
};

export type McpProbeScope = "codex" | "claude";

type PersistedMcpProbeCacheEntry = Omit<McpProbeCacheEntry, "result"> & {
  result: Omit<McpProbeResult, "tools"> & { tools: string[] };
};

const MCP_PROBE_CACHE_STORAGE_KEY = "agofra.mcp-probe-cache";

/// 列表缓存从 localStorage 恢复时的最小形状校验：数组且每项含字符串 name 字段。
function restoreNamedList<T extends { name: string }>(raw: unknown): T[] | null {
  if (!Array.isArray(raw)) return null;
  if (raw.some((item) => typeof item !== "object" || item === null || typeof (item as { name?: unknown }).name !== "string")) return null;
  return raw as T[];
}

/// localStorage JSON 读写统一管道：SSR/隐私模式降级为空，损坏缓存视为不存在。
function readJson(key: string): unknown {
  if (typeof localStorage === "undefined") return null;
  try {
    return JSON.parse(localStorage.getItem(key) ?? "null");
  } catch {
    return null; // 损坏或旧版本缓存只影响首开直出，进页静默刷新会纠正。
  }
}

function writeJson(key: string, value: unknown): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // 本地存储不可用时保留内存缓存，当前操作仍可继续。
  }
}

function createManagementCache<T>(loader: () => Promise<T>, persist?: { key: string; restore: (raw: unknown) => T | null }) {
  let cache: T | null = null;
  let request: Promise<T> | null = null;
  let restored = false;

  const restoreFromStorage = () => {
    if (restored || !persist) return;
    restored = true;
    const raw = readJson(persist.key);
    if (raw !== null) cache = persist.restore(raw);
  };

  const saveToStorage = (items: T) => {
    if (!persist) return;
    writeJson(persist.key, items);
  };

  return {
    load(force = false): Promise<T> {
      restoreFromStorage();
      // force 不抹缓存：绕过快路径、后台去重取新，取回前旧值照常直出。抹缓存会
      // 制造「强刷在途 + 内存为空」窗口，而 restored 一次性标记挡住 localStorage，
      // 窗口期内切页再回来只能对着在途请求转圈。
      if (cache !== null && !force) return Promise.resolve(cache);
      if (!request) {
        request = loader()
          .then((items) => {
            cache = items;
            saveToStorage(items);
            return items;
          })
          .finally(() => {
            request = null;
          });
      }
      return request;
    },
    get(): T | null {
      restoreFromStorage();
      return cache;
    },
    set(items: T): void {
      cache = items;
      saveToStorage(items);
    },
  };
}

const plugins = createManagementCache<PluginSummary[]>(api.listPlugins, {
  key: "agofra.plugins-cache-v1",
  restore: restoreNamedList<PluginSummary>,
});
const skills = createManagementCache<SkillSummary[]>(api.listSkills, {
  key: "agofra.skills-cache-v2", // v2：SkillSummary 增加 claude_enabled，旧缓存缺字段直接弃用
  restore: restoreNamedList<SkillSummary>,
});
const mcpServers = createManagementCache<McpServerSpec[]>(api.codexListMcpServers, {
  key: "agofra.mcp-servers-cache-v1",
  restore: restoreNamedList<McpServerSpec>,
});
const claudeMcpServers = createManagementCache<McpServerSpec[]>(api.claudeListMcpServers, {
  key: "agofra.claude-mcp-servers-cache-v1",
  restore: restoreNamedList<McpServerSpec>,
});
/// Claude 供应商列表可持久化：摘要里只有 has_token 布尔，明文 token 不进缓存。
const claudeProfiles = createManagementCache<ClaudeProfileSummary[]>(api.claudeListProfiles, {
  key: "agofra.claude-profiles-cache-v1",
  restore: restoreNamedList<ClaudeProfileSummary>,
});
const pluginMarketplaces = createManagementCache<PluginMarketplace[]>(api.listPluginMarketplaces, {
  key: "agofra.plugin-marketplaces-cache-v1",
  restore: restoreNamedList<PluginMarketplace>,
});
// 备份记录列表：只在内存缓存（不落 localStorage）——设置页切分页重挂载时直出，
// 避免先闪"还没有备份记录"空态；跨重启的首开由进页静默刷新立刻补齐。
const databaseBackups = createManagementCache<DatabaseBackupInfo[]>(api.listDatabaseBackups);
// CLI 检测摘要：进 CLI 管理页先直出上次状态再静默刷新，消除每次进页的骨架闪帧；
// CLI 在外部被安装/升级/卸载由进页检测纠正。只存摘要（安装方式/版本/路径）。
// 读写都走 set/get，不暴露 load：检测的发起时机与去重仍归 useCliManagement 管。
function restoreCliStatus(raw: unknown): CliStatus | null {
  if (typeof raw !== "object" || raw === null) return null;
  const candidate = raw as Partial<CliStatus>;
  if (typeof candidate.installation !== "string" || !Array.isArray(candidate.other_paths) || typeof candidate.busy !== "boolean") return null;
  // 进程任务不跨重启，恢复时 busy 一律归零，避免恢复出一张永远转圈的卡片。
  return { ...(raw as CliStatus), busy: false };
}
const cliStatusCaches = {
  codex: createManagementCache<CliStatus>(api.codexGetCliStatus, { key: "agofra.codex-cli-status-v1", restore: restoreCliStatus }),
  claude: createManagementCache<CliStatus>(api.claudeGetCliStatus, { key: "agofra.claude-cli-status-v1", restore: restoreCliStatus }),
};
export type ProxyStatus = { proxy: string | null; error: boolean };
/// 检测结果持久化：刷新页面/重启后首帧直出上次状态再静默刷新，消除自动模式
/// 的兜底文案闪变；只存脱敏后的展示地址，系统代理变化由进页刷新纠正。
function restoreProxyStatus(raw: unknown): ProxyStatus | null {
  if (typeof raw !== "object" || raw === null) return null;
  const candidate = raw as Partial<ProxyStatus>;
  if (typeof candidate.error !== "boolean") return null;
  return { proxy: typeof candidate.proxy === "string" ? candidate.proxy : null, error: candidate.error };
}
const proxyStatus = createManagementCache<ProxyStatus>(async () => {
  try {
    return { proxy: await api.getProxyStatus(), error: false };
  } catch {
    return { proxy: null, error: true };
  }
}, { key: "agofra.proxy-status-v1", restore: restoreProxyStatus });
const mcpProbes = new Map<string, McpProbeCacheEntry>();
let mcpProbeStorageLoaded = false;

function mcpProbeCacheKey(name: string, scope: McpProbeScope): string {
  return scope === "codex" ? name : `\u0000${scope}\u0000${name}`;
}

function restoreMcpProbeCacheKey(key: string): string {
  const prefix = "\u0000claude\u0000";
  return key.startsWith(prefix) ? key : mcpProbeCacheKey(key, "codex");
}

function compactMcpProbeResult(result: McpProbeResult): PersistedMcpProbeCacheEntry["result"] {
  const { tools, ...rest } = result;
  return { ...rest, tools: tools.map((tool) => tool.name) };
}

function restoreMcpProbeResult(result: PersistedMcpProbeCacheEntry["result"]): McpProbeResult {
  const { tools, ...rest } = result;
  return {
    ...rest,
    tools: tools
      .filter((name): name is string => typeof name === "string")
      .map((name) => ({ name, title: null, description: null, input_schema: {} })),
  };
}

function loadMcpProbeStorage(): void {
  if (mcpProbeStorageLoaded) return;
  mcpProbeStorageLoaded = true;
  const stored = readJson(MCP_PROBE_CACHE_STORAGE_KEY);
  if (stored === null || typeof stored !== "object") return;
  for (const [name, entry] of Object.entries(stored as Record<string, PersistedMcpProbeCacheEntry>)) {
    if (!entry || typeof entry.fingerprint !== "string" || !Array.isArray(entry.result?.tools)) continue;
    mcpProbes.set(restoreMcpProbeCacheKey(name), { ...entry, result: restoreMcpProbeResult(entry.result) });
  }
}

function persistMcpProbeStorage(): void {
  const stored = Object.fromEntries(
    [...mcpProbes].map(([name, entry]) => [name, { ...entry, result: compactMcpProbeResult(entry.result) }]),
  );
  writeJson(MCP_PROBE_CACHE_STORAGE_KEY, stored);
}

export function loadPlugins(force = false): Promise<PluginSummary[]> {
  return plugins.load(force);
}

export function getCachedPlugins(): PluginSummary[] | null {
  return plugins.get();
}

export function getCachedPluginMarketplaces(): PluginMarketplace[] | null {
  return pluginMarketplaces.get();
}

export function loadPluginMarketplaces(force = false): Promise<PluginMarketplace[]> {
  return pluginMarketplaces.load(force);
}

export function getCachedSkills(): SkillSummary[] | null {
  return skills.get();
}

export function loadSkills(force = false): Promise<SkillSummary[]> {
  return skills.load(force);
}

export function setSkillsCache(items: SkillSummary[]): void {
  skills.set(items);
}

export function loadMcpServers(force = false): Promise<McpServerSpec[]> {
  return mcpServers.load(force);
}

export function getCachedMcpServers(): McpServerSpec[] | null {
  return mcpServers.get();
}

export function setMcpServersCache(items: McpServerSpec[]): void {
  mcpServers.set(items);
}

export function setClaudeMcpServersCache(items: McpServerSpec[]): void {
  claudeMcpServers.set(items);
}

export function loadClaudeMcpServers(force = false): Promise<McpServerSpec[]> {
  return claudeMcpServers.load(force);
}

export function getCachedClaudeMcpServers(): McpServerSpec[] | null {
  return claudeMcpServers.get();
}

export function loadClaudeProfiles(force = false): Promise<ClaudeProfileSummary[]> {
  return claudeProfiles.load(force);
}

export function getCachedClaudeProfiles(): ClaudeProfileSummary[] | null {
  return claudeProfiles.get();
}

export function setClaudeProfilesCache(items: ClaudeProfileSummary[]): void {
  claudeProfiles.set(items);
}

export function loadProxyStatus(force = false): Promise<ProxyStatus> {
  return proxyStatus.load(force);
}

export function getCachedProxyStatus(): ProxyStatus | null {
  return proxyStatus.get();
}

export function loadDatabaseBackups(force = false): Promise<DatabaseBackupInfo[]> {
  return databaseBackups.load(force);
}

export function getCachedDatabaseBackups(): DatabaseBackupInfo[] | null {
  return databaseBackups.get();
}

export function getCachedCliStatus(client: "codex" | "claude"): CliStatus | null {
  return cliStatusCaches[client].get();
}

export function setCliStatusCache(client: "codex" | "claude", status: CliStatus): void {
  cliStatusCaches[client].set(status);
}

// ==================== CLI 更新检查（全局时机 + 供应商页胶囊共享） ====================

/// 检查结果跨页共享：Claude 供应商页胶囊、Agent 工具页静默检查、全局懒计时器
/// 三处消费同一份事实。checked_at 每次尝试都推进（失败也算），update 只在成功
/// 且有新版本时写入——"无更新"不留痕，卡片与胶囊都不显示。
export type CliUpdateInfo = { available: boolean; latest_version: string; channel: string };
type CliUpdateEntry = { checked_at: number; update: CliUpdateInfo | null };

/// 享受全局更新检查的客户端清单：走 app 原生安装链路的都进来，新增客户端 =
/// 这里加一个 id（并补 cliStatusCache / storage key / 各自页面的胶囊挂点）。
export const CLI_UPDATE_CLIENTS = ["codex", "claude"] as const;
export type CliClient = (typeof CLI_UPDATE_CLIENTS)[number];

const CLI_UPDATE_COOLDOWN_MS = 6 * 60 * 60 * 1000;
const cliUpdateStorageKeys: Record<CliClient, string> = {
  codex: "agofra.codex-cli-update-v1",
  claude: "agofra.claude-cli-update-v1",
};
const cliUpdateEntries: Record<CliClient, CliUpdateEntry | null> = { codex: null, claude: null };
const cliUpdateRestored: Record<CliClient, boolean> = { codex: false, claude: false };
const cliUpdateListeners = new Set<() => void>();

function restoreCliUpdateEntry(raw: unknown): CliUpdateEntry | null {
  if (typeof raw !== "object" || raw === null) return null;
  const candidate = raw as Partial<CliUpdateEntry>;
  if (typeof candidate.checked_at !== "number") return null;
  const update = candidate.update;
  if (update !== null && (typeof update !== "object" || typeof (update as CliUpdateInfo).latest_version !== "string" || typeof (update as CliUpdateInfo).channel !== "string" || typeof (update as CliUpdateInfo).available !== "boolean")) return null;
  // 只恢复更新提示，冷却不跨会话：冷启动必须重新检查官方版本。
  return { checked_at: 0, update: update && (update as CliUpdateInfo).available ? { ...(update as CliUpdateInfo) } : null };
}

function notifyCliUpdate(): void {
  for (const listener of cliUpdateListeners) listener();
}

export function getCachedCliUpdate(client: CliClient): CliUpdateInfo | null {
  if (!cliUpdateRestored[client]) {
    cliUpdateRestored[client] = true;
    cliUpdateEntries[client] = restoreCliUpdateEntry(readJson(cliUpdateStorageKeys[client]));
  }
  return cliUpdateEntries[client]?.update ?? null;
}

/// 升级/安装成功的权威落点：本机版本已追平官方（后端 verify_update 校验过），
/// 直接翻转缓存让所有页面的胶囊同步消失，不再发网络请求确认。
export function clearCachedCliUpdate(client: CliClient): void {
  setCliUpdateEntry(client, { checked_at: Date.now(), update: null });
}

function setCliUpdateEntry(client: CliClient, entry: CliUpdateEntry): void {
  cliUpdateRestored[client] = true;
  cliUpdateEntries[client] = entry;
  writeJson(cliUpdateStorageKeys[client], entry);
  notifyCliUpdate();
}

export function subscribeCliUpdate(listener: () => void): () => void {
  cliUpdateListeners.add(listener);
  return () => { cliUpdateListeners.delete(listener); };
}

/// 自动轮询的冷却闸：本次运行内检查后 6 小时不重复，冷启动重新放行。
export function cliUpdateCheckStale(client: CliClient): boolean {
  if (!cliUpdateRestored[client]) getCachedCliUpdate(client);
  const entry = cliUpdateEntries[client];
  return entry === null || Date.now() - entry.checked_at > CLI_UPDATE_COOLDOWN_MS;
}

/// 执行一次官方版本检查并落缓存；失败向上抛，由调用方决定静默还是透出。
export async function runCliUpdateCheck(client: CliClient) {
  const result = await (client === "codex" ? api.codexCheckCliUpdate : api.claudeCheckCliUpdate)();
  setCliUpdateEntry(client, {
    checked_at: Date.now(),
    update: result.available ? { available: true, latest_version: result.latest_version, channel: result.channel } : null,
  });
  return result;
}

/// 静默版：失败推进冷却时间戳但不抛错（离线场景不会每小时重试打官方接口）。
export async function runCliUpdateCheckQuietly(client: CliClient) {
  try {
    return await runCliUpdateCheck(client);
  } catch {
    touchCliUpdateCheckedAt(client);
    return null;
  }
}

/// 手动/静默检查失败的共同落点：推进冷却但保留旧结果。必须先确保持久化条目
/// 已恢复进内存，否则失败会把上一会话的升级结果清成 null（胶囊无声消失）。
export function touchCliUpdateCheckedAt(client: CliClient): void {
  if (!cliUpdateRestored[client]) getCachedCliUpdate(client);
  const entry = cliUpdateEntries[client];
  setCliUpdateEntry(client, { checked_at: Date.now(), update: entry?.update ?? null });
}

async function tickCliUpdateClient(client: CliClient): Promise<void> {
  // 原生安装才有升级链路；缓存缺失（从未进过 Agent 页）先做一次本地检测再判断。
  let status = getCachedCliStatus(client);
  if (!status) {
    try {
      status = await (client === "codex" ? api.codexGetCliStatus : api.claudeGetCliStatus)();
      setCliStatusCache(client, status);
    } catch {
      api.reportCliUpdateTick(client, "detect_failed");
      return;
    }
  }
  // 跳过决策必须留痕，否则"为什么没查"又是排除法谜题；真实检查的成败由后端落 Info/Warn。
  if (status.installation !== "native") {
    api.reportCliUpdateTick(client, "not_native");
    return;
  }
  if (!cliUpdateCheckStale(client)) {
    api.reportCliUpdateTick(client, "cooldown_skip");
    return;
  }
  await runCliUpdateCheckQuietly(client);
}

/// 全局懒计时器：首帧后 initialDelayMs 触发首轮，之后每小时醒一次——醒后先看
/// 冷却闸，没到期只是读个时间戳就继续睡。app 层一次挂载活整个会话，不绑页面。
export function armCliUpdateTicker(initialDelayMs: number): void {
  const tick = () => { for (const client of CLI_UPDATE_CLIENTS) void tickCliUpdateClient(client); };
  setTimeout(() => {
    tick();
    window.setInterval(tick, 60 * 60 * 1000);
  }, initialDelayMs);
}

// ==================== MCP 差异角标（侧栏 + MCP 页） ====================

/// 角标两态：`count` 项可逐条处理的差异，或 `error`（config.toml 解析不了，差异
/// 根本算不出来）。两态共用一条通道是刻意的——"配置坏了"跟"有差异"一样是用户
/// 不点进 MCP 页就看不见的状况，必须搭同一班车到侧栏。
/// MCP 页查完写回，侧栏首屏读缓存直出；侧栏自己不发起查询（差异查询实测 0.5ms
/// 级，但仍不进启动关键路径，见 AppShell 的延迟刷新）。
export interface McpDiffBadge {
  count: number;
  error: boolean;
}

const MCP_DIFF_COUNT_STORAGE_KEYS = { codex: "agofra.mcp-diff-count-v1", claude: "agofra.mcp-diff-count-v1-claude" };
const mcpDiffBadges: Partial<Record<McpProbeScope, McpDiffBadge | null>> = {};
const mcpDiffBadgeListeners = new Set<() => void>();

/// 返回的必须是稳定引用（存下来的那个对象），否则 useSyncExternalStore 会无限重渲染。
export function getMcpDiffBadge(scope: McpProbeScope = "codex"): McpDiffBadge | null {
  if (mcpDiffBadges[scope] === undefined) {
    const raw = readJson(MCP_DIFF_COUNT_STORAGE_KEYS[scope]);
    mcpDiffBadges[scope] = typeof raw === "number" && Number.isFinite(raw) && raw >= 0 ? { count: raw, error: false } : null;
  }
  return mcpDiffBadges[scope] ?? null;
}

/// 角标文本：有差异显示数量（>9 折成 9+），差异算不出来显示 `!`，都没有则不显示。
/// 客户端图标与更新入口共用这一条显示规则。
export function mcpDiffBadgeText(badge: McpDiffBadge | null): string | null {
  if (!badge) return null;
  if (badge.count > 0) return badge.count > 9 ? "9+" : String(badge.count);
  return badge.error ? "!" : null;
}

export function setMcpDiffBadge(next: McpDiffBadge, scope: McpProbeScope = "codex"): void {
  const mcpDiffBadge = getMcpDiffBadge(scope);
  if (mcpDiffBadge && mcpDiffBadge.count === next.count && mcpDiffBadge.error === next.error) return;
  mcpDiffBadges[scope] = next;
  writeJson(MCP_DIFF_COUNT_STORAGE_KEYS[scope], next.count);
  for (const listener of mcpDiffBadgeListeners) listener();
}

export function subscribeMcpDiffBadge(listener: () => void): () => void {
  mcpDiffBadgeListeners.add(listener);
  return () => {
    mcpDiffBadgeListeners.delete(listener);
  };
}

export function getCachedMcpProbe(name: string, fingerprint: string, scope: McpProbeScope = "codex"): McpProbeCacheEntry | null {
  loadMcpProbeStorage();
  const entry = mcpProbes.get(mcpProbeCacheKey(name, scope));
  return entry?.fingerprint === fingerprint ? entry : null;
}

export function setCachedMcpProbe(name: string, entry: McpProbeCacheEntry, scope: McpProbeScope = "codex"): void {
  loadMcpProbeStorage();
  mcpProbes.set(mcpProbeCacheKey(name, scope), entry);
  persistMcpProbeStorage();
}

export function deleteCachedMcpProbe(name: string, scope: McpProbeScope = "codex"): void {
  loadMcpProbeStorage();
  if (mcpProbes.delete(mcpProbeCacheKey(name, scope))) persistMcpProbeStorage();
}

// ==================== 市场插件目录缓存 ====================

/// 按市场名缓存目录快照：进入页面先显缓存，再静默刷新回填；请求按市场名去重。
/// 快照持久化到 localStorage（模式同 MCP 探测缓存：会话加载一次、写入即落盘），
/// 重启后首次进入市场页/目录详情才能直出，不用每个市场重新转圈；超配额时
/// writeJson 静默降级为纯内存缓存。
const MARKETPLACE_PLUGINS_STORAGE_KEY = "agofra.marketplace-plugins-cache-v1";
const marketplacePlugins = new Map<string, MarketplacePlugin[]>();
const marketplaceRequests = new Map<string, Promise<MarketplacePlugin[]>>();
let marketplacePluginsStorageLoaded = false;

function loadMarketplacePluginsStorage(): void {
  if (marketplacePluginsStorageLoaded) return;
  marketplacePluginsStorageLoaded = true;
  const stored = readJson(MARKETPLACE_PLUGINS_STORAGE_KEY);
  if (stored === null || typeof stored !== "object") return;
  for (const [name, items] of Object.entries(stored as Record<string, unknown>)) {
    const restored = restoreNamedList<MarketplacePlugin>(items);
    if (restored) marketplacePlugins.set(name, restored);
  }
}

function persistMarketplacePluginsStorage(): void {
  writeJson(MARKETPLACE_PLUGINS_STORAGE_KEY, Object.fromEntries(marketplacePlugins));
}

export function getCachedMarketplacePlugins(name: string): MarketplacePlugin[] | null {
  loadMarketplacePluginsStorage();
  return marketplacePlugins.get(name) ?? null;
}

export function setCachedMarketplacePlugins(name: string, items: MarketplacePlugin[]): void {
  loadMarketplacePluginsStorage();
  marketplacePlugins.set(name, items);
  persistMarketplacePluginsStorage();
}

export function refreshMarketplacePlugins(name: string, root?: string): Promise<MarketplacePlugin[]> {
  loadMarketplacePluginsStorage();
  const pending = marketplaceRequests.get(name);
  if (pending) return pending;
  const request = api
    .listMarketplacePlugins(name, root)
    .then((items) => {
      setCachedMarketplacePlugins(name, items);
      return items;
    })
    .finally(() => {
      marketplaceRequests.delete(name);
    });
  marketplaceRequests.set(name, request);
  return request;
}
