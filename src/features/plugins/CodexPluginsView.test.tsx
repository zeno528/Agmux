import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import { webInvoke } from "../../api/web-mock";
import { getCachedPluginCatalog, getCachedPlugins, loadPluginCatalog, loadPlugins, setCachedPluginCatalog, setCachedPlugins } from "../../app/managementDataCache";
import { setupI18n } from "../../i18n";
import { AppSwitch } from "../../components/AppSwitch";
import { PluginCard } from "./components/PluginCard";
import type { AppState, MarketplacePlugin, PluginSummary } from "../../types";
import CodexPluginsView from "./CodexPluginsView";

const ui = vi.hoisted(() => ({ filter: "installed" }));
// 当前渲染树中只有筛选器以 "installed" 初始化状态。
vi.mock("react", async (original) => {
  const actual = await original<typeof import("react")>();
  return { ...actual, useState: (initial: unknown) => actual.useState(initial === "installed" ? ui.filter : initial) };
});
beforeEach(() => { ui.filter = "installed"; });

const feedback = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), confirm: vi.fn() }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => feedback }));

vi.mock("../../app/managementDataCache", async (original) => ({
  ...await original<typeof import("../../app/managementDataCache")>(),
  getCachedPluginMarketplaces: () => [{ name: "fixture", root: "/fixture", kind: "third-party" }],
  getCachedPluginCatalog: vi.fn().mockReturnValue([]),
  loadPluginCatalog: vi.fn().mockResolvedValue([]),
  setCachedPluginCatalog: vi.fn(),
  getCachedCodexApps: () => [{ id: "app-one", name: "App one", enabled: false, description: null, icon_path: null }],
  loadPlugins: vi.fn().mockResolvedValue([]),
}));

it("Codex 首次读取可安装目录时显示加载反馈", async () => {
  ui.filter = "available";
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  setCachedPlugins(await webInvoke<PluginSummary[]>("list_plugins"));
  vi.mocked(getCachedPluginCatalog).mockReturnValue(null);
  try {
    const html = renderToStaticMarkup(<CodexPluginsView state={state} activationEpoch={0} popEpoch={0} query="" onHeaderChange={() => undefined} />);
    expect(html).toContain('role="status"');
    expect(html).toContain("正在加载…");
    expect(html).not.toContain("没有匹配的插件");
  } finally { vi.mocked(getCachedPluginCatalog).mockReturnValue([]); }
});

it("插件与应用分别计数，运行时不冒充已安装或可安装插件，禁用插件仍在名单中", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const installed = await webInvoke<PluginSummary[]>("list_plugins");
  const plugin = { ...installed[0], name: "sample", display_name: "Sample", version: "1.2.3", enabled: false };
  const runtime = { ...installed[0], name: "runtime", display_name: "Runtime", desktop_visible: false };
  setCachedPlugins([plugin, runtime]);
  vi.mocked(getCachedPluginCatalog).mockReturnValueOnce([{
    plugin_id: `${runtime.name}@${runtime.marketplace}`, name: runtime.name, installed: true,
    version: null, auth_policy: "ON_USE", source: null, display_name: null, description: null,
  }]);
  function Capture() {
    const tree = CodexPluginsView({ state, activationEpoch: 0, popEpoch: 0, query: "", onHeaderChange: () => undefined });
    expect(pluginCards(tree).map((card) => card.props.name)).toEqual(["Sample"]);
    const html = renderToStaticMarkup(tree);
    expect(html).toContain('aria-pressed="true">已安装');
    expect(html).not.toContain("animate-spin");
    const text = html.replace(/<[^>]*>/g, "");
    expect(text).toContain("插件1");
    expect(text).toContain("应用1");
    expect(text).toContain("v1.2.3");
    expect(html).not.toContain("Runtime");
    return null;
  }
  try { renderToStaticMarkup(<Capture />); }
  finally { setCachedPlugins(installed); }
});

it("刷新市场成功后重新读取已安装插件，失败时不报告成功", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const refresh = vi.spyOn(api, "refreshPluginMarketplace").mockResolvedValue(undefined);
  let click!: () => void;
  function Capture() {
    const tree = CodexPluginsView({ state, activationEpoch: 0, popEpoch: 0, query: "", onHeaderChange: () => undefined });
    function visit(node: ReactNode) {
      Children.forEach(node, (child) => {
        if (!isValidElement(child)) return;
        const element = child as ReactElement<{ children?: ReactNode; "aria-label"?: string; onClick?: () => void }>;
        if (element.props["aria-label"] === "刷新插件市场 fixture") click = element.props.onClick!;
        visit(element.props.children);
      });
    }
    visit(tree);
    return null;
  }
  try {
    renderToStaticMarkup(<Capture />);
    click();
    await vi.waitFor(() => expect(loadPlugins).toHaveBeenCalledExactlyOnceWith(true));
    expect(feedback.success).toHaveBeenCalledOnce();
    feedback.success.mockClear();
    vi.mocked(loadPlugins).mockClear();
    refresh.mockRejectedValueOnce(new Error("refresh failed"));
    click();
    await vi.waitFor(() => expect(feedback.error).toHaveBeenCalledWith("Error: refresh failed"));
    expect(loadPlugins).not.toHaveBeenCalled();
    expect(feedback.success).not.toHaveBeenCalled();
  } finally {
    refresh.mockRestore();
  }
});

function pluginCards(node: ReactNode): ReactElement<Parameters<typeof PluginCard>[0]>[] {
  const cards: ReactElement<Parameters<typeof PluginCard>[0]>[] = [];
  Children.forEach(node, (child) => {
    if (!isValidElement(child)) return;
    if (child.type === PluginCard) cards.push(child as ReactElement<Parameters<typeof PluginCard>[0]>);
    else cards.push(...pluginCards((child.props as { children?: ReactNode }).children));
  });
  return cards;
}

it("远程目录分批展示、按显示名搜索全部条目，并使用远程市场身份安装", async () => {
  ui.filter = "all";
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const installed = await webInvoke<PluginSummary[]>("list_plugins");
  const catalog: MarketplacePlugin[] = Array.from({ length: 85 }, (_, index) => ({
    plugin_id: `sample-${index}@openai-curated-remote`, name: `sample-${index}`,
    display_name: `Library ${index}`, description: "Fixture", version: null,
    installed: false, auth_policy: "ON_USE", source: null,
  }));
  catalog.push({ ...catalog[0], plugin_id: `${installed[0].name}@${installed[0].marketplace}`, name: installed[0].name });
  catalog.push({ ...catalog[0], plugin_id: "installed@remote", name: "installed", installed: true });
  vi.mocked(getCachedPluginCatalog).mockReturnValue(catalog);
  setCachedPlugins([installed[0]]);
  const install = vi.spyOn(api, "installMarketplacePlugin").mockResolvedValue(installed);
  vi.mocked(loadPluginCatalog).mockResolvedValueOnce(catalog);
  let query = "";
  let cards: ReturnType<typeof pluginCards> = [];
  function Capture() {
    cards = pluginCards(CodexPluginsView({ state, activationEpoch: 0, popEpoch: 0, query, onHeaderChange: () => undefined }));
    return null;
  }
  try {
    renderToStaticMarkup(<Capture />);
    expect(cards).toHaveLength(41); // 已安装 1 项，加首批可安装 40 项。
    query = "Library 84";
    renderToStaticMarkup(<Capture />);
    expect(cards.map((card) => card.props.name)).toEqual(["Library 84"]);
    const action = cards[0].props.action as ReactElement<{ onClick: () => void }>;
    action.props.onClick();
    await vi.waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith("openai-curated-remote", "sample-84"));
    await vi.waitFor(() => expect(setCachedPluginCatalog).toHaveBeenLastCalledWith(expect.arrayContaining([
      expect.objectContaining({ plugin_id: "sample-84@openai-curated-remote", installed: true }),
    ])));
  } finally {
    vi.mocked(getCachedPluginCatalog).mockReturnValue([]);
    setCachedPlugins(installed);
    install.mockRestore();
  }
});

it("Codex 开关等待真实结果后更新卡片和缓存，失败时保留原状态", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const installed = await webInvoke<PluginSummary[]>("list_plugins");
  const plugin = installed.find((item) => item.name === "memory-bank")!;
  const disabled = { ...plugin, enabled: false };
  setCachedPlugins([plugin]);
  let complete!: (items: PluginSummary[]) => void;
  const toggle = vi.spyOn(api, "setPluginEnabled").mockImplementationOnce(() =>
    new Promise((resolve) => { complete = resolve; }));
  let control!: ReactElement<Parameters<typeof AppSwitch>[0]>;
  function Capture() {
    const tree = CodexPluginsView({ state, activationEpoch: 0, popEpoch: 0, query: "", onHeaderChange: () => undefined });
    const action = pluginCards(tree).find((card) => card.props.name === "Memory Bank")!.props.action as ReactElement<{ children: ReactNode }>;
    control = Children.toArray(action.props.children).find((child) => isValidElement(child) && child.type === AppSwitch) as typeof control;
    const html = renderToStaticMarkup(action);
    expect(html.indexOf('role="img"')).toBeLessThan(html.indexOf('role="switch"'));
    return null;
  }
  try {
    renderToStaticMarkup(<Capture />);
    expect(control.props.checked).toBe(true);
    control.props.onCheckedChange(false);
    expect(toggle).toHaveBeenCalledExactlyOnceWith("memory-bank@example-plugins", false);
    expect(getCachedPlugins()![0].enabled).toBe(true);
    complete([disabled]);
    await vi.waitFor(() => expect(getCachedPlugins()![0].enabled).toBe(false));
    renderToStaticMarkup(<Capture />);
    expect(control.props.checked).toBe(false);
    toggle.mockRejectedValueOnce(new Error("write failed"));
    vi.mocked(loadPlugins).mockResolvedValueOnce([disabled]);
    control.props.onCheckedChange(true);
    await vi.waitFor(() => expect(feedback.error).toHaveBeenCalledWith("Error: write failed"));
    expect(getCachedPlugins()![0].enabled).toBe(false);
  } finally {
    toggle.mockRestore();
    setCachedPlugins(installed);
  }
});

it("远程插件只显示真实启停状态，可卸载性由能力决定", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const installed = await webInvoke<PluginSummary[]>("list_plugins");
  const remote: PluginSummary = { ...installed[0], name: "remote", display_name: "Remote", origin: "official",
    marketplace: "openai-curated-remote", can_toggle: false, can_uninstall: true, enabled: false };
  const managed = { ...remote, name: "managed", display_name: "Managed", can_uninstall: false };
  setCachedPlugins([remote, managed]);
  const uninstall = vi.spyOn(api, "uninstallPlugin").mockResolvedValue([managed]);
  vi.mocked(loadPluginCatalog).mockResolvedValueOnce([{
    plugin_id: "remote@openai-curated-remote", name: "remote", installed: true,
    version: null, display_name: "Remote", description: null, auth_policy: "ON_USE", source: null,
  }]);
  feedback.confirm.mockResolvedValue(true);
  let remove!: () => void;
  function Capture() {
    const tree = CodexPluginsView({ state, activationEpoch: 0, popEpoch: 0, query: "", onHeaderChange: () => undefined });
    const cards = pluginCards(tree);
    const card = cards.find((item) => item.props.name === "Remote")!;
    expect((card.props.action as ReactElement).type).not.toBe(AppSwitch);
    expect(renderToStaticMarkup(card)).toContain("已禁用");
    expect(renderToStaticMarkup(card)).not.toContain(">已禁用<");
    const actionHtml = renderToStaticMarkup(card.props.action);
    expect(actionHtml.indexOf('role="img"')).toBeLessThan(actionHtml.indexOf("由 Codex 管理"));
    expect(renderToStaticMarkup(card)).toContain("由 Codex 管理");
    remove = (card.props.foot as ReactElement<{ onClick: () => void }>).props.onClick;
    expect(cards.find((item) => item.props.name === "Managed")!.props.foot).toBeNull();
    return null;
  }
  try {
    renderToStaticMarkup(<Capture />);
    remove();
    await vi.waitFor(() => expect(uninstall).toHaveBeenCalledExactlyOnceWith("remote", "openai-curated-remote"));
    await vi.waitFor(() => expect(getCachedPlugins()).toEqual([managed]));
    await vi.waitFor(() => expect(setCachedPluginCatalog).toHaveBeenLastCalledWith([
      expect.objectContaining({ plugin_id: "remote@openai-curated-remote", installed: false }),
    ]));
  } finally {
    uninstall.mockRestore();
    setCachedPlugins(installed);
  }
});
