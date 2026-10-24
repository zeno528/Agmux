import { beforeEach, expect, it, vi } from "vitest";
import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import ClaudePluginsView from "./ClaudePluginsView";
import { PluginCard } from "./components/PluginCard";
import PluginFilterChips from "./components/PluginFilterChips";
import type { ClaudePluginInventory } from "../../types";

const hooks = vi.hoisted(() => ({
  cells: [] as unknown[], index: 0, inventory: { installed: [], available: [] } as ClaudePluginInventory,
  check: vi.fn(), update: vi.fn(), load: vi.fn(), error: vi.fn(),
}));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useEffect: () => undefined,
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
vi.mock("../../api", () => ({ api: { claudeCheckPluginUpdates: hooks.check, claudeUpdatePlugin: hooks.update } }));
vi.mock("../../app/managementDataCache", () => ({
  getCachedClaudePlugins: () => hooks.inventory, loadClaudePlugins: hooks.load, setClaudePluginsCache: vi.fn(),
}));

function elements(node: ReactNode): ReactElement<Record<string, unknown>>[] {
  return Children.toArray(node).flatMap((child) => isValidElement<{ children?: ReactNode }>(child)
    ? [child as ReactElement<Record<string, unknown>>, ...elements(child.props.children)] : []);
}
function render() {
  hooks.index = 0;
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
  hooks.inventory = { installed: [{
    id: "sample@fixture", scope: "user", projectPath: null, enabled: true, version: "1.0.0",
    installPath: null, installedAt: null, lastUpdated: null,
  }], available: [] };
  hooks.check.mockResolvedValue([{ id: "sample@fixture", version: "stale-catalog-version" }]);
  hooks.load.mockImplementation(async () => hooks.inventory);
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

it("可安装首批优先显示第三方插件，保留组内顺序且不修改缓存目录", () => {
  const plugin = (name: string, marketplaceName: string) => ({
    name, marketplaceName, pluginId: `${name}@${marketplaceName}`, description: null,
  });
  const official = Array.from({ length: 45 }, (_, index) => plugin(`official-${index}`, "claude-plugins-official"));
  hooks.inventory.available = [...official, plugin("ponytail", "ponytail"), plugin("custom", "another-market")];
  (render().filters.props.onChange as (filter: string) => void)("available");
  const names = render().cards.map((card) => card.props.name);
  expect(names).toHaveLength(40);
  expect(names.slice(0, 4)).toEqual(["ponytail", "custom", "official-0", "official-1"]);
  expect(hooks.inventory.available.slice(0, 45)).toEqual(official);
});
