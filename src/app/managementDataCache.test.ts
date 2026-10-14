import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CliStatus } from "../types";

const { codexListMcpServers, claudeListMcpServers, claudeListProfiles, listMarketplacePlugins, listPlugins, listDatabaseBackups, getProxyStatus, codexGetCliStatus, claudeGetCliStatus, codexCheckCliUpdate, claudeCheckCliUpdate } = vi.hoisted(() => ({
  codexListMcpServers: vi.fn(),
  claudeListMcpServers: vi.fn(),
  claudeListProfiles: vi.fn(),
  listMarketplacePlugins: vi.fn(),
  listPlugins: vi.fn(),
  listDatabaseBackups: vi.fn(),
  getProxyStatus: vi.fn(),
  codexGetCliStatus: vi.fn(),
  claudeGetCliStatus: vi.fn(),
  codexCheckCliUpdate: vi.fn(),
  claudeCheckCliUpdate: vi.fn(),
}));
const persistedStorage = new Map<string, string>();
const localStorageMock = {
  getItem: (key: string) => persistedStorage.get(key) ?? null,
  setItem: (key: string, value: string) => persistedStorage.set(key, value),
};

vi.mock("../api", () => ({ api: { codexListMcpServers, claudeListMcpServers, claudeListProfiles, listMarketplacePlugins, listPlugins, listDatabaseBackups, getProxyStatus, codexGetCliStatus, claudeGetCliStatus, codexCheckCliUpdate, claudeCheckCliUpdate } }));

describe("managementDataCache", () => {
  beforeEach(() => {
    vi.resetModules();
    codexListMcpServers.mockReset();
    claudeListMcpServers.mockReset();
    claudeListProfiles.mockReset();
    listDatabaseBackups.mockReset();
    getProxyStatus.mockReset();
    codexGetCliStatus.mockReset();
    claudeGetCliStatus.mockReset();
    codexCheckCliUpdate.mockReset();
    claudeCheckCliUpdate.mockReset();
    persistedStorage.clear();
    vi.stubGlobal("localStorage", localStorageMock);
  });

  afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

  it("两端 MCP 差异角标独立更新和恢复，不互相覆盖", async () => {
    const cache = await import("./managementDataCache");
    cache.setMcpDiffBadge({ count: 7, error: false });
    cache.setMcpDiffBadge({ count: 8, error: false }, "claude");
    expect(cache.getMcpDiffBadge()).toEqual({ count: 7, error: false });
    expect(cache.getMcpDiffBadge("claude")).toEqual({ count: 8, error: false });
    cache.setMcpDiffBadge({ count: 0, error: false });
    vi.resetModules();
    const restored = await import("./managementDataCache");
    expect(restored.getMcpDiffBadge()).toEqual({ count: 0, error: false });
    expect(restored.getMcpDiffBadge("claude")).toEqual({ count: 8, error: false });
  });

  it("代理状态刷新时保留缓存并合并请求，失败缓存能在下一次刷新自愈", async () => {
    const cache = await import("./managementDataCache");
    getProxyStatus.mockResolvedValueOnce("http://proxy.invalid:8080/");
    await cache.loadProxyStatus();
    expect(cache.getCachedProxyStatus()).toEqual({ proxy: "http://proxy.invalid:8080/", error: false });
    let fail: (error: Error) => void = () => undefined;
    getProxyStatus.mockReturnValueOnce(new Promise<never>((_, reject) => { fail = reject; }));
    const refresh = cache.loadProxyStatus(true);
    expect(cache.loadProxyStatus(true)).toBe(refresh);
    expect(cache.getCachedProxyStatus()).toEqual({ proxy: "http://proxy.invalid:8080/", error: false });
    fail(new Error("fixture failure"));
    expect(await refresh).toEqual({ proxy: null, error: true });
    expect(cache.getCachedProxyStatus()).toEqual({ proxy: null, error: true });
    expect(getProxyStatus).toHaveBeenCalledTimes(2);
    getProxyStatus.mockResolvedValueOnce(null);
    await cache.loadProxyStatus(true);
    expect(cache.getCachedProxyStatus()).toEqual({ proxy: null, error: false });
  });

  it("CLI 状态缓存跨重启持久化，恢复时 busy 归零，形状损坏视为不存在", async () => {
    const status: CliStatus = { installation: "native", source: null, version: "1.2.3", path: "/fixture/bin/cli", other_paths: [], platform: "windows-x86_64", network: "direct", proxy: null, busy: true };
    const cache = await import("./managementDataCache");
    expect(cache.getCachedCliStatus("codex")).toBeNull();
    cache.setCliStatusCache("codex", status);
    expect(persistedStorage.get("agmux.codex-cli-status-v1")).toContain("native");

    // 模拟重启：新模块实例从 localStorage 恢复，进行中任务不跨会话，busy 归零
    vi.resetModules();
    const restored = await import("./managementDataCache");
    expect(restored.getCachedCliStatus("codex")).toEqual({ ...status, busy: false });

    // 损坏 JSON / 缺字段：只影响首开直出，进页静默刷新会纠正
    persistedStorage.set("agmux.claude-cli-status-v1", "{broken");
    vi.resetModules();
    const broken = await import("./managementDataCache");
    expect(broken.getCachedCliStatus("claude")).toBeNull();
    persistedStorage.set("agmux.claude-cli-status-v1", JSON.stringify({ installation: 5 }));
    vi.resetModules();
    const invalid = await import("./managementDataCache");
    expect(invalid.getCachedCliStatus("claude")).toBeNull();
  });

  it("代理状态跨页面刷新持久化，形状损坏视为不存在", async () => {
    getProxyStatus.mockResolvedValueOnce("http://proxy.invalid:8080/");
    const cache = await import("./managementDataCache");
    await cache.loadProxyStatus();
    expect(cache.getCachedProxyStatus()).toEqual({ proxy: "http://proxy.invalid:8080/", error: false });

    // 模拟 F5：新模块实例从 localStorage 恢复，首帧直出上次检测结果不闪兜底文案
    vi.resetModules();
    const restored = await import("./managementDataCache");
    expect(restored.getCachedProxyStatus()).toEqual({ proxy: "http://proxy.invalid:8080/", error: false });

    // 缺 error 字段：整份弃用，进页静默刷新会纠正
    persistedStorage.set("agmux.proxy-status-v1", JSON.stringify({ proxy: "http://x.invalid" }));
    vi.resetModules();
    const invalid = await import("./managementDataCache");
    expect(invalid.getCachedProxyStatus()).toBeNull();
  });

  it("CLI 更新提示跨重启保留，冷却只在本次运行生效，静默失败保留旧结果", async () => {
    claudeCheckCliUpdate.mockResolvedValueOnce({ status: {}, latest_version: "2.0.0", channel: "latest", available: true });
    const cache = await import("./managementDataCache");
    await cache.runCliUpdateCheckQuietly("claude");
    expect(cache.getCachedCliUpdate("claude")).toEqual({ available: true, latest_version: "2.0.0", channel: "latest" });
    expect(cache.cliUpdateCheckStale("claude")).toBe(false);

    // 模拟重启：新模块实例从 localStorage 恢复，胶囊不用等新一轮检查
    vi.resetModules();
    const restored = await import("./managementDataCache");
    expect(restored.getCachedCliUpdate("claude")).toEqual({ available: true, latest_version: "2.0.0", channel: "latest" });
    expect(restored.cliUpdateCheckStale("claude")).toBe(true);

    // 静默失败：冷却推进、旧结果保留、不抛错
    claudeCheckCliUpdate.mockRejectedValueOnce(new Error("fixture"));
    await restored.runCliUpdateCheckQuietly("claude");
    expect(restored.getCachedCliUpdate("claude")).toEqual({ available: true, latest_version: "2.0.0", channel: "latest" });
    expect(restored.cliUpdateCheckStale("claude")).toBe(false);

    // 冷却过期后才再次放行
    vi.useFakeTimers();
    vi.advanceTimersByTime(6 * 60 * 60 * 1000 + 1);
    expect(restored.cliUpdateCheckStale("claude")).toBe(true);
  });

  it("无更新结果只推进冷却，不跨页面或重启恢复结果", async () => {
    claudeCheckCliUpdate.mockResolvedValueOnce({ status: {}, latest_version: "2.0.0", channel: "latest", available: false });
    const cache = await import("./managementDataCache");
    await cache.runCliUpdateCheckQuietly("claude");
    expect(cache.getCachedCliUpdate("claude")).toBeNull();
    expect(cache.cliUpdateCheckStale("claude")).toBe(false);

    vi.resetModules();
    const restored = await import("./managementDataCache");
    expect(restored.getCachedCliUpdate("claude")).toBeNull();
    expect(restored.cliUpdateCheckStale("claude")).toBe(true);
  });

  it("恢复前发生的静默失败也保留持久化的旧升级结果", async () => {
    // 重启后未读过缓存就遇静默失败（离线）：冷却推进，旧结果不能被清成 null
    persistedStorage.set("agmux.codex-cli-update-v1", JSON.stringify({
      checked_at: Date.now() - 7 * 60 * 60 * 1000,
      update: { available: true, latest_version: "2.0.0", channel: "latest" },
    }));
    codexCheckCliUpdate.mockRejectedValueOnce(new Error("fixture"));
    const cache = await import("./managementDataCache");
    await cache.runCliUpdateCheckQuietly("codex");
    expect(cache.getCachedCliUpdate("codex")).toEqual({ available: true, latest_version: "2.0.0", channel: "latest" });
    expect(cache.cliUpdateCheckStale("codex")).toBe(false);
  });

  it("returns the cached MCP list when the management page remounts", async () => {
    const servers = [{ name: "github", command: "github-mcp-server", args: [], env: {}, enabled: null }];
    codexListMcpServers.mockResolvedValue(servers);
    const cache = await import("./managementDataCache");

    expect(cache.getCachedMcpServers()).toBeNull();
    await cache.loadMcpServers();
    await cache.loadMcpServers();

    expect(cache.getCachedMcpServers()).toEqual(servers);
    expect(codexListMcpServers).toHaveBeenCalledTimes(1);
  });

  it("returns the cached Claude MCP list when the management page remounts", async () => {
    const servers = [{ name: "github", command: "github-mcp-server", args: [], env: {}, enabled: null }];
    claudeListMcpServers.mockResolvedValue(servers);
    const cache = await import("./managementDataCache");

    expect(cache.getCachedClaudeMcpServers()).toBeNull();
    await cache.loadClaudeMcpServers();
    await cache.loadClaudeMcpServers();

    expect(cache.getCachedClaudeMcpServers()).toEqual(servers);
    expect(claudeListMcpServers).toHaveBeenCalledTimes(1);
  });

  it("returns the cached Claude profile list when the management page remounts", async () => {
    const profiles = [{ id: "claude-1", name: "Provider", base_url: "https://api.example", has_token: true, model: null, description: null, icon: null, admin_url: null, kind: null, show_balance: false, updated_at: "0" }];
    claudeListProfiles.mockResolvedValue(profiles);
    const cache = await import("./managementDataCache");

    expect(cache.getCachedClaudeProfiles()).toBeNull();
    await cache.loadClaudeProfiles();
    await cache.loadClaudeProfiles();

    expect(cache.getCachedClaudeProfiles()).toEqual(profiles);
    expect(claudeListProfiles).toHaveBeenCalledTimes(1);
  });

  it("returns the cached backup list when the settings page remounts", async () => {
    const backups = [{ name: "cg-backup-20260923-120000-000.db", size_bytes: 400, created_at: 0 }];
    listDatabaseBackups.mockResolvedValue(backups);
    const cache = await import("./managementDataCache");

    expect(cache.getCachedDatabaseBackups()).toBeNull();
    await cache.loadDatabaseBackups();
    await cache.loadDatabaseBackups();

    expect(cache.getCachedDatabaseBackups()).toEqual(backups);
    expect(listDatabaseBackups).toHaveBeenCalledTimes(1);
  });

  it("shares MCP probe and tool results only for the same configuration", async () => {
    const cache = await import("./managementDataCache");
    const result = {
      ok: true,
      latency_ms: 1,
      status: 200,
      protocol_version: "2025-03-26",
      server_info: { name: "github", version: "1.0.0" },
      tools: [{ name: "search", title: null, description: null, input_schema: {} }],
      tools_truncated: false,
      error: null,
      tools_error: null,
    };

    cache.setCachedMcpProbe("github", { fingerprint: "v1", result, toolsLoaded: true });

    expect(cache.getCachedMcpProbe("github", "v1")?.toolsLoaded).toBe(true);
    expect(cache.getCachedMcpProbe("github", "v2")).toBeNull();
  });

  it("keeps Codex and Claude probe results separate for same-named servers", async () => {
    const cache = await import("./managementDataCache");
    const result = {
      ok: true,
      latency_ms: 1,
      status: 200,
      protocol_version: "2025-03-26",
      server_info: { name: "github", version: "1.0.0" },
      tools: [],
      tools_truncated: false,
      error: null,
      tools_error: null,
    };

    cache.setCachedMcpProbe("github", { fingerprint: "codex", result, toolsLoaded: false });
    cache.setCachedMcpProbe("github", { fingerprint: "claude", result, toolsLoaded: false }, "claude");

    expect(cache.getCachedMcpProbe("github", "codex")?.fingerprint).toBe("codex");
    expect(cache.getCachedMcpProbe("github", "claude", "claude")?.fingerprint).toBe("claude");
  });

  it("persists only MCP tool names and restores them after a reload", async () => {
    const cache = await import("./managementDataCache");
    const result = {
      ok: true,
      latency_ms: 1,
      status: 200,
      protocol_version: "2025-03-26",
      server_info: { name: "github", version: "1.0.0" },
      tools: [{ name: "search", title: "Search", description: "large detail", input_schema: { query: "string" } }],
      tools_truncated: false,
      error: null,
      tools_error: null,
    };

    cache.setCachedMcpProbe("github", { fingerprint: "v1", result, toolsLoaded: true });
    expect(JSON.parse(persistedStorage.get("agmux.mcp-probe-cache") ?? "{}").github.result.tools).toEqual(["search"]);

    vi.resetModules();
    const reloaded = await import("./managementDataCache");
    expect(reloaded.getCachedMcpProbe("github", "v1")?.result.tools.map((tool) => tool.name)).toEqual(["search"]);
  });
});

describe("marketplace plugin catalog cache", () => {
  beforeEach(() => {
    vi.resetModules();
    listMarketplacePlugins.mockReset();
    persistedStorage.clear();
    vi.stubGlobal("localStorage", localStorageMock);
  });

  afterEach(() => vi.unstubAllGlobals());

  const plugin = (name: string) => ({
    plugin_id: `${name}@fixture`,
    name,
    version: "1.0.0",
    installed: false,
    auth_policy: "ON_USE",
    source: null,
    display_name: null,
    description: null,
    category: null,
    capabilities: [],
    contains: [],
  });

  it("caches the catalog after a refresh and serves later reads from cache", async () => {
    const catalog = [plugin("a")];
    listMarketplacePlugins.mockResolvedValue(catalog);
    const cache = await import("./managementDataCache");

    expect(cache.getCachedMarketplacePlugins("market-a")).toBeNull();
    const items = await cache.refreshMarketplacePlugins("market-a");
    expect(cache.getCachedMarketplacePlugins("market-a")).toBe(items);
    expect(listMarketplacePlugins).toHaveBeenCalledWith("market-a", undefined);
  });

  it("persists marketplace catalogs and restores them after a reload", async () => {
    listMarketplacePlugins.mockResolvedValue([plugin("a")]);
    const cache = await import("./managementDataCache");
    await cache.refreshMarketplacePlugins("market-persist");
    expect(persistedStorage.get("agmux.marketplace-plugins-cache-v1")).toContain("market-persist");

    vi.resetModules();
    const reloaded = await import("./managementDataCache");
    expect(reloaded.getCachedMarketplacePlugins("market-persist")).toEqual([plugin("a")]);
    expect(listMarketplacePlugins).toHaveBeenCalledTimes(1); // 重载后命中持久化缓存，不再发请求
  });

  it("deduplicates concurrent refreshes for the same marketplace", async () => {
    listMarketplacePlugins.mockReturnValue(new Promise(() => {}));
    const cache = await import("./managementDataCache");

    const first = cache.refreshMarketplacePlugins("market-dup");
    const second = cache.refreshMarketplacePlugins("market-dup");
    expect(first).toBe(second);
    expect(listMarketplacePlugins).toHaveBeenCalledTimes(1);
  });

  it("clears the in-flight request on failure so the next entry can retry", async () => {
    listMarketplacePlugins.mockRejectedValueOnce(new Error("cli failed"));
    const cache = await import("./managementDataCache");

    await expect(cache.refreshMarketplacePlugins("market-err")).rejects.toThrow("cli failed");
    expect(cache.getCachedMarketplacePlugins("market-err")).toBeNull();

    listMarketplacePlugins.mockResolvedValueOnce([]);
    await expect(cache.refreshMarketplacePlugins("market-err")).resolves.toEqual([]);
  });
});

describe("list cache persistence", () => {
  beforeEach(() => {
    vi.resetModules();
    listPlugins.mockReset();
    persistedStorage.clear();
    vi.stubGlobal("localStorage", localStorageMock);
  });

  afterEach(() => vi.unstubAllGlobals());

  const plugin = { name: "ponytail", version: "1.0.0", enabled: true };

  it("persists the plugins cache and restores it after a module reload", async () => {
    listPlugins.mockResolvedValue([plugin]);
    const cache = await import("./managementDataCache");
    await cache.loadPlugins();
    expect(persistedStorage.get("agmux.plugins-cache-v1")).toContain("ponytail");

    vi.resetModules();
    const reloaded = await import("./managementDataCache");
    await expect(reloaded.loadPlugins()).resolves.toEqual([plugin]);
    expect(listPlugins).toHaveBeenCalledTimes(1); // 重载后命中持久化缓存，不再发请求
  });

  it("ignores corrupt persisted caches and falls back to a fresh load", async () => {
    persistedStorage.set("agmux.plugins-cache-v1", "{not-json");
    const cache = await import("./managementDataCache");
    listPlugins.mockResolvedValue([plugin]);

    await expect(cache.loadPlugins()).resolves.toEqual([plugin]);
    expect(listPlugins).toHaveBeenCalledTimes(1);
  });

  it("rejects persisted snapshots whose entries lack a string name", async () => {
    persistedStorage.set("agmux.plugins-cache-v1", JSON.stringify([{ version: "1.0.0" }]));
    const cache = await import("./managementDataCache");
    listPlugins.mockResolvedValue([plugin]);

    await expect(cache.loadPlugins()).resolves.toEqual([plugin]);
    expect(listPlugins).toHaveBeenCalledTimes(1);
  });

  it("keeps stale cache readable while a forced refresh is in flight", async () => {
    listPlugins.mockResolvedValueOnce([plugin]);
    const cache = await import("./managementDataCache");
    await cache.loadPlugins();

    // 强刷挂起（模拟慢扫盘）：期间普通读取必须直出旧缓存，不能转圈等请求
    listPlugins.mockReturnValueOnce(new Promise(() => {}));
    void cache.loadPlugins(true);
    await expect(cache.loadPlugins()).resolves.toEqual([plugin]);
    expect(cache.getCachedPlugins()).toEqual([plugin]);
  });

  it("shares the in-flight request between a cold load and a forced refresh", async () => {
    listPlugins.mockReturnValueOnce(new Promise(() => {}));
    const cache = await import("./managementDataCache");

    const cold = cache.loadPlugins();
    expect(cache.loadPlugins(true)).toBe(cold);
    expect(listPlugins).toHaveBeenCalledTimes(1);
  });
});
