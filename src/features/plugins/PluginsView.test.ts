// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { Activity, Children, createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { setupI18n } from "../../i18n";
import { webInvoke } from "../../api/web-mock";
import { FeedbackProvider } from "../../app/Feedback";
import { loadClaudePluginMarketplaces, setClaudePluginsCache } from "../../app/managementDataCache";
import { ClientTargetSwitch } from "../../components/ClientTargetSwitch";
import PluginsView from "./PluginsView";
import ClaudePluginsView from "./ClaudePluginsView";
import ClaudePluginMarketplaceView from "./ClaudePluginMarketplaceView";
import type { AppState, ClaudePluginInventory } from "../../types";

// PluginsView 是双引擎壳，Codex 看板本体在 CodexPluginsView
const source = readFileSync(new URL("./CodexPluginsView.tsx", import.meta.url), "utf8");
const claudeSource = readFileSync(new URL("./ClaudePluginsView.tsx", import.meta.url), "utf8");
const shell = readFileSync(new URL("./PluginsView.tsx", import.meta.url), "utf8");

describe("插件搜索", () => {
  it("Claude 插件看板统一搜索已安装与可安装，已安装项不重复提供安装", async () => {
    setupI18n("zh-CN");
    const fixture = await webInvoke<ClaudePluginInventory>("claude_list_plugins", { includeAvailable: true });
    setClaudePluginsCache({
      installed: ["needle", "other"].map((name) => ({ ...fixture.installed[0], id: `${name}@fixture` })),
      available: [
        { ...fixture.available[0], name: "needle", pluginId: "needle@fixture", marketplaceName: "fixture" },
        { ...fixture.available[0], name: "needle-catalog", pluginId: "needle-catalog@fixture", marketplaceName: "fixture" },
        { ...fixture.available[0], name: "other-catalog", pluginId: "other-catalog@other", marketplaceName: "other" },
      ],
    });
    const html = renderToStaticMarkup(createElement(FeedbackProvider, null,
      createElement(ClaudePluginsView, { popEpoch: 0, query: "  NEEDLE ", onHeaderChange: () => undefined }),
    ));
    expect(html).toContain(">needle</span>");
    expect(html).toContain(">needle-catalog</span>");
    expect(html).not.toContain(">other</span>");
    expect(html).not.toContain(">other-catalog</span>");
    expect(html.match(/aria-label="安装"/g)).toHaveLength(1);
  });

  it("可安装目录分批进 DOM（目录可达数千条，不能一次铺满），搜索仍覆盖未展示条目", async () => {
    const fixture = await webInvoke<ClaudePluginInventory>("claude_list_plugins", { includeAvailable: true });
    setClaudePluginsCache({
      installed: [],
      available: Array.from({ length: 85 }, (_, index) => ({
        ...fixture.available[0], pluginId: `plugin-${index}@fixture`, name: `plugin-${index}`,
      })),
    });
    const props = { popEpoch: 0, query: "", onHeaderChange: () => undefined };
    const render = (query: string) => renderToStaticMarkup(createElement(FeedbackProvider, null,
      createElement(ClaudePluginsView, { ...props, query }),
    ));
    expect(render("")).toContain(">plugin-39</span>");
    expect(render("")).not.toContain(">plugin-40</span>");
    expect(render("")).toContain("显示更多");
    expect(render("plugin-84")).toContain(">plugin-84</span>");
  });

  it("市场源弹层只列已配置来源，不把内置可安装目录混入市场", async () => {
    const markets = await loadClaudePluginMarketplaces(true);
    setClaudePluginsCache({
      installed: [],
      available: [{ pluginId: "catalog@anthropic-plugin-directory", name: "catalog", description: null, marketplaceName: "anthropic-plugin-directory" }],
    });
    const html = renderToStaticMarkup(createElement(FeedbackProvider, null,
      createElement(ClaudePluginMarketplaceView, { onChanged: async () => undefined }),
    ));
    for (const marketplace of markets) expect(html).toContain(marketplace.name);
    expect(html).not.toContain("anthropic-plugin-directory");
    expect(html).not.toContain("浏览插件");
    expect(html).toContain("<form");
  });

  it("通栏搜索留在父层壳里、先于客户端看板渲染，客户端视图按名称吃 query", () => {
    expect(shell.split("<PluginSearchInput").length - 1).toBe(1);
    expect(shell.indexOf("<PluginSearchInput")).toBeLessThan(shell.indexOf("<CodexPluginsView"));
    expect(source).toContain("matchesQuery(plugin, query)");
  });

  it("Claude 推荐 Ponytail 市场，添加后不再显示推荐入口，也不推荐 Codex 专属市场", async () => {
    setupI18n("zh-CN");
    await webInvoke("claude_remove_plugin_marketplace", { name: "ponytail" });
    await loadClaudePluginMarketplaces(true);
    const render = () => renderToStaticMarkup(createElement(FeedbackProvider, null,
      createElement(ClaudePluginMarketplaceView, { onChanged: async () => undefined }),
    ));
    expect(render()).toContain(">Ponytail</div>");
    expect(render()).toContain("DietrichGebert/ponytail");
    expect(render()).not.toContain(">OpenAI Plugins</div>");
    try {
      await webInvoke("claude_add_plugin_marketplace", { source: "DietrichGebert/ponytail" });
      await loadClaudePluginMarketplaces(true);
      expect(render()).not.toContain(">Ponytail</div>");
      expect(render()).toContain(">ponytail</span>");
    } finally {
      await webInvoke("claude_remove_plugin_marketplace", { name: "ponytail" });
      await loadClaudePluginMarketplaces(true);
    }
  });
});

it("切换插件引擎保留父层页头和滑块，并将正文滚动归零", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const frames: ReactElement<Parameters<typeof ClientTargetSwitch>[0]>[] = [];
  const panels: ReactElement<{ mode: string; children: ReactNode }>[][] = [];
  function Capture() {
    const tree = PluginsView({ state, activationEpoch: 0, popEpoch: 0 });
    const header = Children.toArray(tree.props.children)[0] as ReactElement<{ children: ReactNode }>;
    const headerChildren = Children.toArray(header.props.children);
    const toolbar = headerChildren[headerChildren.length - 1] as ReactElement<{ children: ReactNode }>;
    const control = Children.toArray(toolbar.props.children).find((child) => isValidElement(child) && child.type === ClientTargetSwitch) as typeof frames[number];
    frames.push(control);
    const content = Children.toArray(tree.props.children)[1] as ReactElement<{ children: ReactNode }>;
    panels.push(Children.toArray(content.props.children).filter((child) => isValidElement(child) && child.type === Activity) as typeof panels[number]);
    if (control.props.value === "codex") {
      const content = Children.toArray(tree.props.children)[1] as ReactElement<{ ref: { current: { scrollTop: number } | null } }>;
      const scrollContent = { scrollTop: 240 };
      content.props.ref.current = scrollContent;
      control.props.onChange("codex");
      expect(scrollContent.scrollTop).toBe(240);
      control.props.onChange("claude");
      expect(scrollContent.scrollTop).toBe(0);
    }
    return null;
  }
  renderToStaticMarkup(createElement(Capture));
  expect(frames.map((control) => control.props.value)).toEqual(["codex", "claude"]);
  expect(frames[1].type).toBe(frames[0].type);
  expect(frames[1].key).toBe(frames[0].key);
  // 保活结构契约：切客户端不能卸载仍在检查的看板；未访问客户端不挂载。
  expect(panels[0].map((panel) => panel.props.mode)).toEqual(["visible"]);
  expect(panels[1].map((panel) => panel.props.mode)).toEqual(["hidden", "visible"]);
  expect(panels[1][0].key).toBe(panels[0][0].key);
});

// 一级列表（MCP / Skill）共用一套行卡定义。页面各自复制结构、
// 或各起一个行类，都算破坏这条契约——定义只住在 style.css 里一份。
// 插件页已改为看板卡片网格（.plugin-board），不在这条契约里。
describe("一级列表共用定义", () => {
  const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");
  const pages = [
    readFileSync(new URL("../mcp/McpView.tsx", import.meta.url), "utf8"),
    readFileSync(new URL("../skills/SkillsView.tsx", import.meta.url), "utf8"),
  ];

  it("各页面都只写同一个 class 名，不各自造行类", () => {
    for (const page of pages) expect(page).toContain('className="apple-group apple-list-card"');
    expect(styles).not.toContain("plugin-list-row");
    expect(styles).not.toContain("skill-list-row");
  });

  it("插件、MCP、Skill 一级页头复用统一标题计数组件", () => {
    for (const page of [...pages, source, claudeSource]) expect(page).toContain("ManagementPageTitle");
    expect(styles).toContain(".management-page-title {");
    expect(styles).toContain(".management-page-title__count {");
  });

  it("插件与 MCP 页复用同一个客户端切换器", () => {
    for (const path of ["./PluginsView.tsx", "../mcp/McpView.tsx"]) {
      expect(readFileSync(new URL(path, import.meta.url), "utf8")).toContain("<ClientTargetSwitch");
    }
  });

  it("卡片的定义只有一处：整卡内边距、分割线、行的去卡片化都在 style.css", () => {
    const selectors = [".apple-list-card {", ".apple-group.apple-list-card {", ".apple-list-card > * + * {", ".apple-list-card .apple-list-row {"];
    for (const selector of selectors) expect(styles).toContain(selector);
    // 只钉语义，不钉数值：密度和配色会反复调，画面一动就红的测试没人愿意留
    expect(styles).toMatch(/\.apple-list-card \.apple-list-row \{[^}]*background: transparent/);
    expect(styles).toMatch(/\.apple-list-card \.apple-list-row \{[^}]*box-shadow: none/);
  });
});

// 看板契约：市场子页已被拼平的可安装区 + 市场源弹层取代。
describe("Codex 看板", () => {
  it("市场源管理共用弹层表单，不再提供仓库安装子页", () => {
    expect(source).toContain("<AppDialog");
    expect(source).toContain("<PluginMarketplaceSources");
    expect(source).not.toContain("AddPluginView");
  });

  it("更新检查共用页头一个按钮，检查结果按当前搜索过滤进「有更新」区", () => {
    expect(source).toContain("checkPluginUpdates");
    expect(source).toContain("visibleUpdates");
  });

  it("Claude 看板没有批量更新检查，不出现「有更新」维度", () => {
    expect(claudeSource).not.toContain("checkPluginUpdates");
    expect(claudeSource).not.toContain("updates:");
  });
});
