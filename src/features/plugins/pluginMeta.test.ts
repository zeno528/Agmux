import { describe, expect, it } from "vitest";
import { compareMarketplacePlugins, comparePlugins, matchesQuery, resolveAliasInstalled, resolveAppIcon } from "./pluginMeta";

it("应用优先复用同一服务的官方插件图标，不借用无关插件的图标", () => {
  const plugins = [
    { name: "sample", display_name: "Sample", origin: "codex", icon_path: "/third-party.png" },
    { name: "app-sample", display_name: "Sample", origin: "official", icon_path: "/official.png" },
    { name: "other", display_name: "Other", origin: "official", icon_path: null },
  ] as const;
  expect(resolveAppIcon({ name: " SAMPLE ", icon_path: "https://example.test/unavailable.png" }, plugins)).toBe("/official.png");
  expect(resolveAppIcon({ name: "App-sample", icon_path: null }, plugins)).toBe("/official.png");
  expect(resolveAppIcon({ name: "Other", icon_path: "https://example.test/other.png" }, plugins)).toBe("https://example.test/other.png");
  expect(resolveAppIcon({ name: "Unknown", icon_path: null }, plugins)).toBeNull();
  expect(resolveAppIcon({ name: "Sample", icon_path: null }, plugins.slice(0, 1))).toBeNull();
});

describe("插件搜索", () => {
  it("仅按插件名忽略大小写匹配，不搜显示名与描述", () => {
    const plugin = { name: "app-69ea", display_name: "Exa", description: "Web search for AI" };
    expect(matchesQuery(plugin, "")).toBe(true);
    expect(matchesQuery(plugin, "app")).toBe(true);
    expect(matchesQuery(plugin, "  APP-69 ")).toBe(true);
    expect(matchesQuery(plugin, "exa")).toBe(false); // 显示名不参与
    expect(matchesQuery(plugin, "search for ai")).toBe(false); // 描述不参与
    expect(matchesQuery(plugin, "figma")).toBe(false);
  });
});

describe("插件列表排序", () => {
  it("第三方市场 → 内置 runtime 市场 → 官方 curated 市场，组内按名称", () => {
    const plugin = (name: string, origin: string, marketplace: string) =>
      ({ name, origin, marketplace }) as Parameters<typeof comparePlugins>[0];
    const ordered = [
      plugin("app-69ea", "official", "openai-curated-remote"),
      plugin("browser", "official", "openai-bundled"),
      plugin("ponytail", "codex", "ponytail"),
      plugin("documents", "official", "openai-primary-runtime"),
      plugin("gmail", "official", "openai-curated-remote"),
    ].sort(comparePlugins);
    expect(ordered.map((item) => item.name)).toEqual(["ponytail", "browser", "documents", "app-69ea", "gmail"]);
  });
});

describe("别名安装纠正", () => {
  it("保留目录的安装标记，补齐远程别名，不合并其他市场的同名插件", () => {
    const plugin = (name: string, installed: boolean) =>
      ({ plugin_id: `${name}@m`, name, installed }) as Parameters<typeof resolveAliasInstalled>[0][number];
    const input = [plugin("canva", false), plugin("grill", false), plugin("github", true)];
    const resolved = resolveAliasInstalled(input, new Set(["canva@m-remote", "notion@m", "grill@another-market"]));
    expect(resolved.map((item) => item.installed)).toEqual([true, false, true]);
    expect(input.map((item) => item.installed)).toEqual([false, false, true]); // 原数组不被就地修改
  });
});

describe("市场明细排序", () => {
  it("已安装在前，组内按名称", () => {
    const plugin = (name: string, installed: boolean) =>
      ({ name, installed }) as Parameters<typeof compareMarketplacePlugins>[0];
    const ordered = [
      plugin("zeta", false),
      plugin("canva", true),
      plugin("beta", false),
      plugin("app-69ea", true),
    ].sort(compareMarketplacePlugins);
    expect(ordered.map((item) => item.name)).toEqual(["app-69ea", "canva", "beta", "zeta"]);
  });
});
