import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppUpdateProvider } from "./AppUpdateProvider";
import type { AppUpdate } from "./appUpdate";
import type { Settings } from "../../types";

const hooks = vi.hoisted(() => ({ cells: [] as unknown[], index: 0, effects: [] as (() => void | (() => void))[], check: vi.fn() }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useCallback: <T,>(callback: T) => callback,
  useEffect: (effect: () => void | (() => void)) => { hooks.effects.push(effect); },
  useRef: (initial: unknown) => {
    const index = hooks.index++;
    hooks.cells[index] ??= { current: initial };
    return hooks.cells[index];
  },
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.cells)) hooks.cells[index] = initial;
    return [hooks.cells[index], (value: unknown) => { hooks.cells[index] = value; }];
  },
}));
vi.mock("./appUpdate", () => ({ checkForAppUpdate: hooks.check, UPDATED_VERSION_KEY: "fixture" }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ error: vi.fn(), success: vi.fn() }) }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

function render(proxyMode: Settings["proxy_mode"], proxyUrl = "", enabled = false) {
  hooks.index = 0;
  hooks.effects.length = 0;
  return AppUpdateProvider({ enabled, ready: false, children: null, proxyMode, proxyUrl }).props.value;
}
beforeEach(() => { hooks.cells.length = 0; hooks.index = 0; hooks.check.mockReset(); });
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it("自动检查延迟 5 秒，effect 清理后仍可重新安排", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("window", globalThis);
  hooks.check.mockResolvedValue(null);
  render("auto", "", true);
  const effect = hooks.effects[1];
  const cleanup = effect();
  cleanup?.();
  expect(vi.getTimerCount()).toBe(0);
  effect();
  expect(hooks.check).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(5000);
  expect(hooks.check).toHaveBeenCalledOnce();
});

it("代理模式或地址改变后，旧检查结果不得下载；新检查才能升级", async () => {
  const update: AppUpdate = { version: "1.2.3", notes: null, install: vi.fn(async () => undefined) };
  hooks.check.mockResolvedValue(update);
  await render("auto").check();
  const stale = render("auto");
  render("off");
  await stale.install();
  expect(update.install).not.toHaveBeenCalled();
  await render("off").check();
  await render("off").install();
  expect(update.install).toHaveBeenCalledOnce();

  await render("custom", "http://proxy.invalid:8080").check();
  const firstProxy = render("custom", "http://proxy.invalid:8080");
  render("custom", "http://proxy.invalid:9090");
  await firstProxy.install();
  expect(update.install).toHaveBeenCalledOnce();
});

it("检查尚未返回时改变模式，迟到的结果不能重新覆盖新网络状态", async () => {
  let finish: (update: AppUpdate) => void = () => undefined;
  hooks.check.mockReturnValue(new Promise<AppUpdate>((resolve) => { finish = resolve; }));
  const pending = render("auto").check();
  render("off");
  const update: AppUpdate = { version: "1.2.3", notes: null, install: vi.fn(async () => undefined) };
  finish(update);
  expect(await pending).toBeNull();
  expect(render("off").update).toBeNull();
  expect(update.install).not.toHaveBeenCalled();
});
