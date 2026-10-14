import type { TFunction } from "i18next";

/**
 * 后端下发的用量窗口标签 → 本语言展示文案的 i18n 键。
 *
 * 后端在 usage_label / weekly_label 里直接给中文展示值（"5小时"、"7天"…），
 * 前端只做换词：中文取值映射回原值（中文界面逐字不变），英文给出对应写法。
 * 表中没有的取值（如 "12天" 这类动态窗口）原样返回，与改动前行为一致。
 * 彻底解法是后端改下发窗口代码、文案全归前端，见 docs/i18n-glossary.md 的 P2。
 */
const WINDOW_LABEL_KEYS: Record<string, "balance.window5h" | "balance.window7d" | "balance.window30d" | "card.usage" | "balance.period"> = {
  "5小时": "balance.window5h", // i18n-exempt: 与后端下发值比对，不是界面文案
  "7天": "balance.window7d", // i18n-exempt: 与后端下发值比对
  "30天": "balance.window30d", // i18n-exempt: 与后端下发值比对
  "额度": "card.usage", // i18n-exempt: 兼容后端旧值并统一为用量
  "用量": "card.usage", // i18n-exempt: 与后端下发值比对
  "周期": "balance.period", // i18n-exempt: 与后端下发值比对
};

/**
 * 后端窗口标签 → 当前语言的展示文案；取值缺失返回 null，未知取值原样返回。
 * t 必须是 profiles 命名空间的（键都在那里）。
 */
export function localizeBalanceLabel(raw: string | null | undefined, t: TFunction<"profiles">): string | null {
  if (!raw) return null;
  const key = WINDOW_LABEL_KEYS[raw];
  return key ? t(key) : raw;
}

/** 是否为每周窗口（决定标题用「每周使用限额」还是「{{label}}使用限额」）。 */
export function isWeeklyWindowLabel(raw: string | null | undefined): boolean {
  return raw === "7天"; // i18n-exempt: 与后端下发值比对
}
