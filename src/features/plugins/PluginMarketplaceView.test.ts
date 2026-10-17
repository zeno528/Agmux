// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { setupI18n } from "../../i18n";
import { MarketplaceRow } from "./components/MarketplaceLayout";

const source = readFileSync(new URL("./PluginMarketplaceView.tsx", import.meta.url), "utf8");

describe("插件市场卡片", () => {
  it("显示每个已添加市场的插件总数", () => {
    expect(source).toContain("marketplacePluginCounts");
    expect(source).toContain("refreshMarketplacePlugins(marketplace.name, marketplace.root)");
    setupI18n("zh-CN");
    expect(renderToStaticMarkup(createElement(MarketplaceRow, { name: "fixture", source: "", count: 3, actions: null }))).toContain("3 个插件");
    expect(renderToStaticMarkup(createElement(MarketplaceRow, { name: "fixture", source: "", count: null, actions: null }))).toContain("插件数量暂不可用");
  });

  it("市场列表首帧从缓存同步直出，与其他管理页同款", () => {
    // 首帧就是内容：缓存命中时 useState 初始化直接吃缓存，不画一帧转圈
    expect(source).toContain("const cachedMarketplaces = getCachedPluginMarketplaces();");
    expect(source).toContain("useState<PluginMarketplace[]>(cachedMarketplaces ?? [])");
    expect(source).toContain("useState(cachedMarketplaces !== null)");
  });

  it("页面分官方/第三方两卡：分组直出、未配置推荐渲染安装行，推荐折叠区不再存在", () => {
    expect(source).toContain("splitMarketplaceCards(marketplaces)");
    expect(source).toContain('t("action.install")');
    expect(source).not.toContain("AppDisclosure");
    expect(source).not.toContain("recommendedTitle");
  });
});
