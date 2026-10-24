import type { ClaudePluginEntry } from "../../types";

/// 官方市场名：排序置后第三方、市场列表换官方图标都认它（与后端 claude.rs 的字面量对应）。
export const CLAUDE_OFFICIAL_MARKETPLACE = "claude-plugins-official";

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
