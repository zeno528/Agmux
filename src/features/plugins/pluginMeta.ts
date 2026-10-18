import type { CodexAppSummary, MarketplacePlugin, PluginSummary } from "../../types";

/** 同一服务优先复用官方插件图标，避免应用目录的图片地址失效或缺省。 */
export function resolveAppIcon(
  app: Pick<CodexAppSummary, "name" | "icon_path">,
  plugins: readonly Pick<PluginSummary, "name" | "display_name" | "origin" | "icon_path">[],
): string | null {
  const name = app.name.trim().toLowerCase();
  return plugins.find((plugin) => plugin.origin === "official" && plugin.icon_path
    && [plugin.name, plugin.display_name].some((label) => label?.trim().toLowerCase() === name))?.icon_path
    ?? app.icon_path;
}

/** 插件来源数据值（official / codex）→ 展示标签的 i18n 键。 */
export const originLabels: Partial<Record<PluginSummary["origin"], "origin.official" | "origin.thirdParty">> = {
  official: "origin.official",
  codex: "origin.thirdParty",
};

export const recommendedMarketplaces = [
  {
    name: "openai-curated",
    clients: ["codex"],
    kind: "official",
    // Codex 桌面端在不同渠道下会把官方精选市场物化为 openai-api-curated（~/.codex/.tmp/plugins）
    aliases: ["openai-api-curated"],
    displayName: "OpenAI Plugins",
    source: "openai/plugins",
    descriptionKey: "recommended.openaiCurated",
  },
  {
    name: "ponytail",
    clients: ["codex", "claude"],
    kind: "third-party",
    aliases: [],
    displayName: "Ponytail",
    source: "DietrichGebert/ponytail",
    descriptionKey: "recommended.ponytail",
  },
] as const;

/// 插件排序分层：第三方市场（用户添加）→ 内置 runtime 市场 → 官方 curated 市场。组内按名称。
function pluginTier(plugin: PluginSummary): number {
  if (plugin.origin !== "official") return 0;
  return plugin.marketplace?.includes("curated") ? 2 : 1;
}

export function comparePlugins(left: PluginSummary, right: PluginSummary): number {
  return pluginTier(left) - pluginTier(right) || left.name.localeCompare(right.name);
}

/// 推荐市场是否已配置：按市场名或别名匹配（官方市场在不同渠道下名称不同）。
export function findConfiguredMarketplace<T extends { name: string }>(
  recommended: { name: string; aliases: readonly string[] },
  marketplaces: T[],
): T | undefined {
  return marketplaces.find(
    (marketplace) => marketplace.name === recommended.name || recommended.aliases.includes(marketplace.name),
  );
}

/// 别名市场纠正：Codex 给同一目录挂镜像/远程双身份，安装记录只落在其中一个；
/// 保留远程目录的安装标记，并补齐本地安装名单；同名但不同市场的插件不合并。
export function resolveAliasInstalled(
  plugins: MarketplacePlugin[],
  installedIds: ReadonlySet<string>,
): MarketplacePlugin[] {
  return plugins.map((plugin) => {
    const installed = plugin.installed || installedIds.has(plugin.plugin_id) || installedIds.has(`${plugin.plugin_id}-remote`);
    return plugin.installed === installed ? plugin : { ...plugin, installed };
  });
}

/// 市场明细排序：已安装在前，组内按名称。
export function compareMarketplacePlugins(left: MarketplacePlugin, right: MarketplacePlugin): number {
  return Number(right.installed) - Number(left.installed) || left.name.localeCompare(right.name);
}

/// 插件搜索：仅匹配插件名（忽略大小写子串），不搜显示名与描述。
export function matchesQuery(plugin: { name: string }, query: string): boolean {
  return plugin.name.toLowerCase().includes(query.trim().toLowerCase());
}
