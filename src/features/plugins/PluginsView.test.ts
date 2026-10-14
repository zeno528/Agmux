// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./PluginsView.tsx", import.meta.url), "utf8");

describe("插件搜索", () => {
  it("已安装列表接入搜索框并按名称过滤", () => {
    expect(source.split("matchesQuery(plugin, query)").length - 1).toBe(1);
    expect(source.split("<PluginSearchInput").length - 1).toBe(1);
  });
});

// 一级列表（MCP / 插件 / Skill）共用一套大卡片定义。页面各自复制结构、
// 或各起一个行类，都算破坏这条契约——定义只住在 style.css 里一份。
describe("一级列表共用定义", () => {
  const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");
  const marketplaceSource = readFileSync(new URL("./PluginMarketplaceView.tsx", import.meta.url), "utf8");
  const pages = [
    source,
    readFileSync(new URL("../mcp/McpView.tsx", import.meta.url), "utf8"),
    readFileSync(new URL("../skills/SkillsView.tsx", import.meta.url), "utf8"),
  ];

  it("三个页面都只写同一个 class 名，不各自造行类", () => {
    for (const page of pages) expect(page).toContain('className="apple-group apple-list-card"');
    expect(styles).not.toContain("plugin-list-row");
    expect(styles).not.toContain("skill-list-row");
  });

  it("插件、MCP、Skill 一级页头复用统一标题计数组件", () => {
    for (const page of pages) expect(page).toContain("ManagementPageTitle");
    expect(styles).toContain(".management-page-title {");
    expect(styles).toContain(".management-page-title__count {");
  });

  it("插件市场详情页也复用统一标题计数样式", () => {
    const marketplaceDetailSource = readFileSync(new URL("./MarketplaceDetailView.tsx", import.meta.url), "utf8");
    expect(marketplaceDetailSource).toContain("ManagementPageTitle");
    expect(marketplaceDetailSource).toContain('count={loaded ? installedPluginCount : undefined}');
  });

  it("插件市场两卡与一级列表同构：行直接进 apple-group apple-list-card，不再套面板外壳", () => {
    // 官方/第三方两卡都由 renderMarketplaceCard 渲染，容器类只剩一处定义；
    // 不再用 apple-panel-section 包一层（面板内边距叠上行内边距，首尾留白不等）
    expect(marketplaceSource.split('className="apple-group apple-list-card"').length - 1).toBe(1);
    expect(marketplaceSource).not.toContain("apple-panel-section");
    expect(marketplaceSource).not.toContain("shadow-[0_0_0_1px_var(--panel-ring)]");
  });

  it("卡片的定义只有一处：整卡内边距、分割线、行的去卡片化都在 style.css", () => {
    const selectors = [".apple-list-card {", ".apple-group.apple-list-card {", ".apple-list-card > * + * {", ".apple-list-card .apple-list-row {"];
    for (const selector of selectors) expect(styles).toContain(selector);
    // 只钉语义，不钉数值：密度和配色会反复调，画面一动就红的测试没人愿意留
    expect(styles).toMatch(/\.apple-list-card \.apple-list-row \{[^}]*background: transparent/);
    expect(styles).toMatch(/\.apple-list-card \.apple-list-row \{[^}]*box-shadow: none/);
  });
});
