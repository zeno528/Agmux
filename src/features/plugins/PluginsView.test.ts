// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Children, createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { setupI18n } from "../../i18n";
import { webInvoke } from "../../api/web-mock";
import { FeedbackProvider } from "../../app/Feedback";
import { setClaudePluginsCache } from "../../app/managementDataCache";
import { ClientTargetSwitch } from "../../components/ClientTargetSwitch";
import PluginsView from "./PluginsView";
import ClaudePluginsView from "./ClaudePluginsView";
import ClaudePluginMarketplaceView from "./ClaudePluginMarketplaceView";
import { MarketplaceRow } from "./components/MarketplaceLayout";
import type { AppState, ClaudePluginInventory } from "../../types";

// PluginsView 现在是双引擎壳，Codex 列表本体在 CodexPluginsView
const source = readFileSync(new URL("./CodexPluginsView.tsx", import.meta.url), "utf8");

describe("插件搜索", () => {
  it("Claude 一级页只搜索已安装项，可安装插件只出现在市场页", async () => {
    setupI18n("zh-CN");
    const fixture = await webInvoke<ClaudePluginInventory>("claude_list_plugins", { includeAvailable: true });
    const inventory = {
      installed: ["needle", "other"].map((name) => ({ ...fixture.installed[0], id: `${name}@fixture` })),
      available: [
        { ...fixture.available[0], name: "needle-catalog", pluginId: "needle-catalog@fixture", marketplaceName: "fixture" },
        { ...fixture.available[0], name: "other-catalog", pluginId: "other-catalog@other", marketplaceName: "other" },
      ],
    };
    setClaudePluginsCache(inventory);
    setClaudePluginsCache(inventory, true);
    const html = renderToStaticMarkup(createElement(FeedbackProvider, null,
      createElement(ClaudePluginsView, { popEpoch: 0, query: "  NEEDLE ", onHeaderChange: () => undefined }),
    ));
    expect(html).toContain(">needle</span>");
    expect(html).not.toContain(">needle-catalog</span>");
    expect(html).not.toContain(">other</span>");
    expect(html).not.toContain(">other-catalog</span>");
    const market = renderToStaticMarkup(createElement(FeedbackProvider, null,
      createElement(ClaudePluginMarketplaceView, { onBack: () => undefined, onInstalled: async () => undefined }),
    ));
    expect(market).toContain(">fixture</span>");
    expect(market).toContain("3 个插件");
    expect(market).not.toContain(">needle-catalog</span>");
    function Browse() {
      const tree = ClaudePluginMarketplaceView({ onBack: () => undefined, onInstalled: async () => undefined });
      const visit = (node: ReactNode) => Children.forEach(node, (child) => {
        if (!isValidElement<{ children?: ReactNode; name?: string; actions?: ReactElement<{ children: ReactNode }> }>(child)) return;
        if (child.type === MarketplaceRow && child.props.name === "fixture") {
          const button = Children.toArray(child.props.actions!.props.children).find((item) => isValidElement(item) && item.type === "button") as ReactElement<{ onClick: () => void }>;
          button.props.onClick();
        } else visit(child.props.children);
      });
      visit(tree);
      return tree;
    }
    const detail = renderToStaticMarkup(createElement(FeedbackProvider, null, createElement(Browse)));
    expect(detail).toContain(">needle-catalog</span>");
    expect(detail).toContain(">needle</span>");
    expect(detail).not.toContain(">other-catalog</span>");
  });

  it("已安装列表接入搜索框并按名称过滤", () => {
    expect(source.split("matchesQuery(plugin, query)").length - 1).toBe(1);
    const shell = readFileSync(new URL("./PluginsView.tsx", import.meta.url), "utf8");
    expect(shell.split("<PluginSearchInput").length - 1).toBe(1);
    expect(shell.indexOf("<PluginSearchInput")).toBeLessThan(shell.indexOf("<ClientTargetSwitch"));
  });
});

it("切换插件引擎保留父层页头和同一个滑块位置", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const frames: ReactElement<Parameters<typeof ClientTargetSwitch>[0]>[] = [];
  function Capture() {
    const tree = PluginsView({ state, popEpoch: 0 });
    const header = Children.toArray(tree.props.children)[0] as ReactElement<{ children: ReactNode }>;
    const headerChildren = Children.toArray(header.props.children);
    const toolbar = headerChildren[headerChildren.length - 1] as ReactElement<{ children: ReactNode }>;
    const control = Children.toArray(toolbar.props.children).find((child) => isValidElement(child) && child.type === ClientTargetSwitch) as typeof frames[number];
    frames.push(control);
    if (control.props.value === "codex") control.props.onChange("claude");
    return null;
  }
  renderToStaticMarkup(createElement(Capture));
  expect(frames.map((control) => control.props.value)).toEqual(["codex", "claude"]);
  expect(frames[1].type).toBe(frames[0].type);
  expect(frames[1].key).toBe(frames[0].key);
});

// 一级列表（MCP / 插件 / Skill）共用一套大卡片定义。页面各自复制结构、
// 或各起一个行类，都算破坏这条契约——定义只住在 style.css 里一份。
describe("一级列表共用定义", () => {
  const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");
  const marketplaceSource = readFileSync(new URL("./PluginMarketplaceView.tsx", import.meta.url), "utf8");
  const pages = [
    source,
    readFileSync(new URL("./ClaudePluginsView.tsx", import.meta.url), "utf8"),
    readFileSync(new URL("../mcp/McpView.tsx", import.meta.url), "utf8"),
    readFileSync(new URL("../skills/SkillsView.tsx", import.meta.url), "utf8"),
  ];

  it("各页面都只写同一个 class 名，不各自造行类", () => {
    for (const page of pages) expect(page).toContain('className="apple-group apple-list-card"');
    expect(styles).not.toContain("plugin-list-row");
    expect(styles).not.toContain("skill-list-row");
  });

  it("插件、MCP、Skill 一级页头复用统一标题计数组件", () => {
    for (const page of pages) expect(page).toContain("ManagementPageTitle");
    expect(styles).toContain(".management-page-title {");
    expect(styles).toContain(".management-page-title__count {");
  });

  it("插件与 MCP 页复用同一个客户端切换器", () => {
    for (const path of ["./PluginsView.tsx", "../mcp/McpView.tsx"]) {
      expect(readFileSync(new URL(path, import.meta.url), "utf8")).toContain("<ClientTargetSwitch");
    }
  });

  it("插件市场详情页也复用统一标题计数样式", () => {
    const marketplaceDetailSource = readFileSync(new URL("./MarketplaceDetailView.tsx", import.meta.url), "utf8");
    expect(marketplaceDetailSource).toContain("ManagementPageTitle");
    expect(marketplaceDetailSource).toContain('count={loaded ? installedPluginCount : undefined}');
  });

  it("插件市场两卡与一级列表同构：行直接进 apple-group apple-list-card，不再套面板外壳", () => {
    // 官方/第三方两卡都由 renderMarketplaceCard 渲染，容器类只剩一处定义；
    // 不再用 apple-panel-section 包一层（面板内边距叠上行内边距，首尾留白不等）
    const shared = readFileSync(new URL("./components/MarketplaceLayout.tsx", import.meta.url), "utf8");
    expect(shared.split('className="apple-group apple-list-card"').length - 1).toBe(1);
    for (const path of ["./PluginMarketplaceView.tsx", "./ClaudePluginMarketplaceView.tsx"]) {
      const page = readFileSync(new URL(path, import.meta.url), "utf8");
      expect(page).toContain("<MarketplacePage");
      expect(page).toContain("<MarketplaceGroup");
    }
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
