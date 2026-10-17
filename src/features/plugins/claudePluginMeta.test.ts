import { describe, expect, it } from "vitest";
import { claudeInstalledIdSet, claudePluginParts, groupClaudePluginsByMarketplace } from "./claudePluginMeta";
import type { ClaudeAvailablePlugin, ClaudePluginEntry } from "../../types";

describe("claudePluginParts", () => {
  it("按第一个 @ 拆出插件名与市场名", () => {
    expect(claudePluginParts("ponytail@ponytail")).toEqual({ name: "ponytail", marketplace: "ponytail" });
    expect(claudePluginParts("code-review@claude-plugins-official")).toEqual({
      name: "code-review",
      marketplace: "claude-plugins-official",
    });
  });

  it("无市场后缀与空后缀都能安全兜底", () => {
    expect(claudePluginParts("bare")).toEqual({ name: "bare", marketplace: null });
    expect(claudePluginParts("x@")).toEqual({ name: "x", marketplace: null });
  });
});

describe("claudeInstalledIdSet", () => {
  it("跨 scope 收集 id 集合（同插件双行只留一个 id）", () => {
    const installed = [
      { id: "a@m", scope: "user" },
      { id: "a@m", scope: "project" },
      { id: "b@m", scope: "user" },
    ] as ClaudePluginEntry[];
    expect(claudeInstalledIdSet(installed)).toEqual(new Set(["a@m", "b@m"]));
  });
});

describe("groupClaudePluginsByMarketplace", () => {
  const available = [
    { pluginId: "z@beta", name: "z", description: null, marketplaceName: "beta" },
    { pluginId: "a@beta", name: "a", description: null, marketplaceName: "beta" },
    { pluginId: "m@alpha", name: "m", description: null, marketplaceName: "alpha" },
  ] as ClaudeAvailablePlugin[];

  it("按市场名分组：市场排序、组内按插件名排序", () => {
    const groups = groupClaudePluginsByMarketplace(available, []);
    expect(groups.map((group) => group.marketplace)).toEqual(["alpha", "beta"]);
    expect(groups[1].plugins.map((plugin) => plugin.name)).toEqual(["a", "z"]);
  });

  it("空目录不产生分组", () => {
    expect(groupClaudePluginsByMarketplace([], [])).toEqual([]);
  });

  it("补回已启用插件，并对跨 scope 和目录中已有的插件去重", () => {
    const installed = [
      { id: "only-installed@installed-market", scope: "user", enabled: true },
      { id: "only-installed@installed-market", scope: "project", enabled: true },
      { id: "a@beta", scope: "user", enabled: false },
    ] as ClaudePluginEntry[];
    const groups = groupClaudePluginsByMarketplace(available, installed);
    expect(groups.find((group) => group.marketplace === "installed-market")?.plugins).toEqual([
      { pluginId: "only-installed@installed-market", name: "only-installed", description: null, marketplaceName: "installed-market" },
    ]);
    expect(groups.find((group) => group.marketplace === "beta")?.plugins).toEqual([available[1], available[0]]);
  });
});
