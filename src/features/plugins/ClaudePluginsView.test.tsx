import { beforeEach, expect, it, vi } from "vitest";
import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import ClaudePluginsView from "./ClaudePluginsView";
import { PluginCard } from "./components/PluginCard";
import { pluginCatalogRange } from "./components/PluginCatalogPagination";
import PluginFilterChips from "./components/PluginFilterChips";
import type { ClaudePluginInventory } from "../../types";

const hooks = vi.hoisted(() => ({
  cells: [] as unknown[], index: 0, inventory: { installed: [], available: [] } as ClaudePluginInventory,
  effects: [] as (() => void)[], cold: false,
  check: vi.fn(), update: vi.fn(), toggle: vi.fn(), load: vi.fn(), error: vi.fn(),
}));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useEffect: (effect: () => void) => { hooks.effects.push(effect); },
  useLayoutEffect: () => undefined,
  useMemo: (compute: () => unknown) => compute(),
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.cells)) hooks.cells[index] = typeof initial === "function" ? initial() : initial;
    return [hooks.cells[index], (value: unknown) => {
      hooks.cells[index] = typeof value === "function" ? value(hooks.cells[index]) : value;
    }];
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ success: vi.fn(), info: vi.fn(), error: hooks.error }) }));
vi.mock("../../api", () => ({ api: { claudeCheckPluginUpdates: hooks.check, claudeUpdatePlugin: hooks.update, claudeSetPluginEnabled: hooks.toggle } }));
vi.mock("../../app/managementDataCache", () => ({
  getCachedClaudePlugins: () => hooks.cold ? null : hooks.inventory, loadClaudePlugins: hooks.load, setClaudePluginsCache: vi.fn(),
}));

function elements(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  return Children.toArray(node).flatMap((child) => isValidElement<{ children?: ReactNode }>(child)
    ? [child as ReactElement<Record<string, unknown>>, ...elements(child.props.children)] : []);
}
function render() {
  hooks.index = 0;
  hooks.effects.length = 0;
  const tree = ClaudePluginsView({ popEpoch: 0, query: "", onHeaderChange: () => undefined });
  const all = elements(tree);
  const cards = all.filter((element) => element.type === PluginCard);
  return {
    cards,
    filters: all.find((element) => element.type === PluginFilterChips)!,
    check: all.find((element) => element.type === "button" && element.props.children
      && Children.toArray(element.props.children as ReactNode).includes("action.checkUpdates"))!,
    upgrades: cards.flatMap((card) => elements(card.props.foot as ReactNode))
      .filter((element) => element.props["aria-label"] === "claudePlugin.action.upgradeTo"),
  };
}
function click(button: ReactElement<Record<string, unknown>>) {
  (button.props.onClick as () => void)();
}
beforeEach(() => {
  vi.clearAllMocks();
  hooks.cells.length = 0;
  hooks.cold = false;
  hooks.inventory = { installed: [{
    id: "sample@fixture", scope: "user", projectPath: null, enabled: true, version: "1.0.0",
    installPath: null, installedAt: null, lastUpdated: null,
  }], available: [] };
  hooks.check.mockResolvedValue([{ id: "sample@fixture", version: "stale-catalog-version" }]);
  hooks.load.mockImplementation(async () => hooks.inventory);
});

it("首次进入默认全部，先显示已安装插件再读取完整目录", async () => {
  hooks.cold = true;
  let finish!: (value: ClaudePluginInventory) => void;
  const installed = new Promise<ClaudePluginInventory>((resolve) => { finish = resolve; });
  hooks.load.mockReturnValueOnce(installed).mockReturnValueOnce(installed);

  expect(render().filters.props.value).toBe("all");
  hooks.effects.slice(0, 2).forEach((effect) => effect());
  expect(hooks.load.mock.calls).toEqual([[false, false], [true, false]]);

  finish(hooks.inventory);
  await installed;
  expect(render().cards.map((card) => card.props.name)).toEqual(["sample"]);
  hooks.effects[1]();
  expect(hooks.load).toHaveBeenLastCalledWith(true, true);
});

it("开关写入期间保留检查更新和卸载按钮，更新检查等待写入完成", async () => {
  let finish!: () => void;
  hooks.toggle.mockReturnValueOnce(new Promise<void>((resolve) => { finish = resolve; }));
  const action = render().cards[0].props.action as ReactElement<{ onCheckedChange: (next: boolean) => Promise<void> }>;
  const pending = action.props.onCheckedChange(false);
  const { check, cards } = render();
  expect(check.props.disabled).toBe(false);
  expect(check.props["aria-disabled"]).toBe(true);
  expect(elements(cards[0].props.foot as ReactNode).some((node) => node.props["aria-label"] === "claudePlugin.action.uninstallAria")).toBe(true);
  click(check);
  expect(hooks.check).not.toHaveBeenCalled();
  finish();
  await pending;
  expect(render().check.props["aria-disabled"]).toBeUndefined();
});

it.each(["updated", "up_to_date"])("CLI 返回 %s 后清除该行升级图标，其他 scope 仍可升级", async (updateOutcome) => {
  hooks.inventory.installed.push({ ...hooks.inventory.installed[0], scope: "project", projectPath: "/fixture" });
  hooks.update.mockImplementation(async () => {
    hooks.inventory = { ...hooks.inventory, installed: hooks.inventory.installed.map((entry) => entry.scope === "user" ? { ...entry, version: "2.0.0" } : entry) };
    return { updateOutcome, oldVersion: "1.0.0", newVersion: "2.0.0" };
  });
  click(render().check);
  await vi.waitFor(() => expect(render().upgrades).toHaveLength(2));
  click(render().upgrades[0]);
  expect(render().check.props.disabled).toBe(true);
  await vi.waitFor(() => expect(render().check.props.disabled).toBe(false));
  expect(hooks.update).toHaveBeenCalledExactlyOnceWith("sample@fixture", "user", null);
  expect(hooks.load).toHaveBeenCalled();
  expect(render().upgrades).toHaveLength(1);
  expect(hooks.error).not.toHaveBeenCalled();
});

it("升级失败保留更新入口供重试", async () => {
  hooks.update.mockRejectedValue(new Error("fixture failure"));
  click(render().check);
  await vi.waitFor(() => expect(render().upgrades).toHaveLength(1));
  click(render().upgrades[0]);
  await vi.waitFor(() => expect(hooks.error).toHaveBeenCalledWith("Error: fixture failure"));
  expect(render().upgrades).toHaveLength(1);
  expect(hooks.load).not.toHaveBeenCalled();
});

it("目录分页随列数填满整行，缩放时保留原位置附近的卡片", () => {
  expect(pluginCatalogRange(85, 3, 0)).toEqual({ start: 0, end: 24, pageSize: 24 });
  expect(pluginCatalogRange(85, 5, 24)).toEqual({ start: 20, end: 40, pageSize: 20 });
  expect(pluginCatalogRange(85, 4, 24)).toEqual({ start: 24, end: 48, pageSize: 24 });
  expect(pluginCatalogRange(85, 3, 80)).toEqual({ start: 72, end: 85, pageSize: 24 });
  expect(pluginCatalogRange(12, 3, 80)).toEqual({ start: 0, end: 12, pageSize: 24 });
});

it("可安装首批优先显示第三方插件，保留组内顺序且不修改缓存目录", () => {
  const plugin = (name: string, marketplaceName: string) => ({
    name, marketplaceName, pluginId: `${name}@${marketplaceName}`, description: null,
  });
  const official = Array.from({ length: 45 }, (_, index) => plugin(`official-${index}`, "claude-plugins-official"));
  hooks.inventory.available = [...official, plugin("ponytail", "ponytail"), plugin("custom", "another-market")];
  (render().filters.props.onChange as (filter: string) => void)("available");
  const names = render().cards.map((card) => card.props.name);
  expect(names).toHaveLength(24);
  expect(names.slice(0, 4)).toEqual(["ponytail", "custom", "official-0", "official-1"]);
  expect(hooks.inventory.available.slice(0, 45)).toEqual(official);
});
