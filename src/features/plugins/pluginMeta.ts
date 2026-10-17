import type { MarketplacePlugin, PluginMarketplace, PluginSummary } from "../../types";

type ContainsLabelKey = "contains.skills" | "contains.mcp" | "contains.app" | "contains.hooks" | "contains.agents" | "contains.commands";

/** 插件清单 contains / capabilities 的数据键 → 展示标签的 i18n 键（数据键本身不是界面文案）。 */
export const containsLabels: Partial<Record<string, ContainsLabelKey>> = {
  skills: "contains.skills",
  mcp: "contains.mcp",
  app: "contains.app",
  hooks: "contains.hooks",
  agents: "contains.agents",
  commands: "contains.commands",
};

/** 插件来源数据值（official / codex）→ 展示标签的 i18n 键。 */
export const originLabels: Partial<Record<PluginSummary["origin"], "origin.official" | "origin.thirdParty">> = {
  official: "origin.official",
  codex: "origin.thirdParty",
};

/** 可卸载的来源（官方市场与 Skill 注册表除外） */
export const removableOrigins: readonly PluginSummary["origin"][] = ["codex"];

export const marketplaceKindLabels: Record<PluginMarketplace["kind"], "origin.official" | "origin.thirdParty"> = {
  official: "origin.official",
  "third-party": "origin.thirdParty",
};

export const recommendedMarketplaces = [
  {
    name: "openai-curated",
    kind: "official",
    // Codex 桌面端在不同渠道下会把官方精选市场物化为 openai-api-curated（~/.codex/.tmp/plugins）
    aliases: ["openai-api-curated"],
    displayName: "OpenAI Plugins",
    source: "openai/plugins",
    descriptionKey: "recommended.openaiCurated",
  },
  {
    name: "ponytail",
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
export function findConfiguredMarketplace(
  recommended: { name: string; aliases: readonly string[] },
  marketplaces: PluginMarketplace[],
): PluginMarketplace | undefined {
  return marketplaces.find(
    (marketplace) => marketplace.name === recommended.name || recommended.aliases.includes(marketplace.name),
  );
}

/// 别名市场纠正：Codex 给同一目录挂镜像/远程双身份，安装记录只落在其中一个；
/// 条目标记未安装但同名插件已在任意市场安装时视为已装（名单来自已安装列表，无需额外 CLI 查询）。
export function resolveAliasInstalled(
  plugins: MarketplacePlugin[],
  installedNames: ReadonlySet<string>,
): MarketplacePlugin[] {
  return plugins.map((plugin) =>
    !plugin.installed && installedNames.has(plugin.name) ? { ...plugin, installed: true } : plugin,
  );
}

/// 市场明细排序：已安装在前，组内按名称。
export function compareMarketplacePlugins(left: MarketplacePlugin, right: MarketplacePlugin): number {
  return Number(right.installed) - Number(left.installed) || left.name.localeCompare(right.name);
}

/// 插件搜索：仅匹配插件名（忽略大小写子串），不搜显示名与描述。
export function matchesQuery(plugin: { name: string }, query: string): boolean {
  return plugin.name.toLowerCase().includes(query.trim().toLowerCase());
}
