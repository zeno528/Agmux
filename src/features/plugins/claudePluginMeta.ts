import type { ClaudeAvailablePlugin, ClaudePluginEntry } from "../../types";

/// Claude 插件 id（name@marketplace）拆分：@ 后整体是市场名，插件名内不允许 @（CLI 命名约束）。
export function claudePluginParts(id: string): { name: string; marketplace: string | null } {
  const at = id.indexOf("@");
  if (at < 0) return { name: id, marketplace: null };
  return { name: id.slice(0, at), marketplace: id.slice(at + 1) || null };
}

/// 已装 id 集合：目录条目据此显示「已安装」。
export function claudeInstalledIdSet(installed: ClaudePluginEntry[]): Set<string> {
  return new Set(installed.map((entry) => entry.id));
}

/// CLI 的 available 会排除已启用插件，合并已安装项后再按市场分组。
export function groupClaudePluginsByMarketplace(
  available: ClaudeAvailablePlugin[],
  installed: ClaudePluginEntry[],
): { marketplace: string; plugins: ClaudeAvailablePlugin[] }[] {
  const catalog = new Map(available.map((plugin) => [plugin.pluginId, plugin]));
  for (const entry of installed) {
    if (catalog.has(entry.id)) continue;
    const { name, marketplace } = claudePluginParts(entry.id);
    if (marketplace) catalog.set(entry.id, { pluginId: entry.id, name, description: null, marketplaceName: marketplace });
  }
  const groups = new Map<string, ClaudeAvailablePlugin[]>();
  for (const plugin of catalog.values()) {
    const list = groups.get(plugin.marketplaceName) ?? [];
    list.push(plugin);
    groups.set(plugin.marketplaceName, list);
  }
  return [...groups.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([marketplace, plugins]) => ({
      marketplace,
      plugins: [...plugins].sort((left, right) => left.name.localeCompare(right.name)),
    }));
}
