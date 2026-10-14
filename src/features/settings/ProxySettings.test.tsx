import { Children, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
// @ts-expect-error 测试运行于 Node，浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { beforeEach, expect, it, vi } from "vitest";
import type { Settings } from "../../types";
import { ProxySettings } from "./ProxySettings";

// Node 中只模拟 React 调度与反馈边界，按钮和输入事件运行组件的真实处理函数。
const hooks = vi.hoisted(() => ({ cells: [] as unknown[], index: 0, error: vi.fn(), effects: [] as (() => unknown)[], layoutEffects: [] as (() => unknown)[], tauri: false, proxyStatus: vi.fn() }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useEffect: (effect: () => unknown) => { hooks.effects.push(effect); },
  useLayoutEffect: (effect: () => unknown) => { hooks.layoutEffects.push(effect); },
  useRef: (initial: unknown) => {
    const index = hooks.index++;
    hooks.cells[index] ??= { current: initial };
    return hooks.cells[index];
  },
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.cells)) hooks.cells[index] = typeof initial === "function" ? initial() : initial;
    return [hooks.cells[index], (value: unknown) => { hooks.cells[index] = value; }];
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, options?: { address?: string }) => `${key}${options?.address ?? ""}` }) }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ error: hooks.error }) }));
vi.mock("../../api", () => ({ api: { getProxyStatus: hooks.proxyStatus }, get isTauri() { return hooks.tauri; } }));

const settings: Settings = {
  theme: "system", language: "system", auto_restart: false, autostart_enabled: false,
  silent_start: false, minimize_to_tray: false, tray_click_action: "show_window", startup_view: "codexProfiles", macos_terminal: "terminal",
  auto_check_update: true, auto_backup_interval_hours: 0, database_backup_keep_count: 5,
  proxy_mode: "auto", proxy_url: "", claude_terminal_dirs: [],
};

interface Control {
  children?: ReactNode;
  className?: string;
  disabled?: boolean;
  value?: string;
  "aria-pressed"?: boolean;
  ref?: { current: { focus: (options?: FocusOptions) => void } | null };
  "data-proxy-mode"?: Settings["proxy_mode"];
  onClick?: () => void;
  onChange?: (event: { target: { value: string } }) => void;
  onKeyDown?: (event: { key: string; preventDefault: () => void }) => void;
  onBlur?: (event: { relatedTarget: unknown }) => void;
}
function controls(node: ReactNode): Control[] {
  const result: Control[] = [];
  Children.forEach(node, (child) => {
    if (!isValidElement<Control>(child)) return;
    if (child.type === "button" || child.type === "input") result.push(child.props);
    else result.push(...controls(child.props.children));
  });
  return result;
}
function render(onSave: (patch: Partial<Settings>) => Promise<boolean>, current = settings, saving = false) {
  hooks.index = 0;
  return ProxySettings({ settings: current, saving, onSave });
}
const saveButton = (node: ReactNode) => controls(node).find((control) => control.children === "proxy.save" || control.children === "proxy.saved")!;
beforeEach(() => { hooks.cells.length = 0; hooks.index = 0; hooks.error.mockClear(); hooks.effects.length = 0; hooks.layoutEffects.length = 0; hooks.tauri = false; hooks.proxyStatus.mockReset(); });

it("浏览器预览始终显示固定提示，不请求系统代理或闪过读取失败", () => {
  const onSave = vi.fn(async () => true);
  render(onSave);
  hooks.effects.splice(0).forEach((effect) => effect());
  const html = renderToStaticMarkup(render(onSave));
  expect(html).toContain("proxy.previewDescription");
  expect(html).not.toContain("proxy.statusUnavailable");
  expect(hooks.proxyStatus).not.toHaveBeenCalled();
});

it("桌面版重进常规设置或重新激活窗口，刷新完成前仍显示上次代理结果", async () => {
  hooks.tauri = true;
  const cache = await import("../../app/managementDataCache");
  hooks.proxyStatus.mockResolvedValueOnce("http://proxy.invalid:8080");
  await cache.loadProxyStatus(true);
  let finish: (proxy: string) => void = () => undefined;
  hooks.proxyStatus.mockReturnValueOnce(new Promise<string>((resolve) => { finish = resolve; }));
  const listeners: (() => void)[] = [];
  vi.stubGlobal("window", { addEventListener: (_type: string, listener: () => void) => listeners.push(listener), removeEventListener: vi.fn() });
  try {
    const onSave = vi.fn(async () => true);
    render(onSave);
    hooks.effects.splice(0).forEach((effect) => effect());
    listeners.forEach((listener) => listener());
    expect(hooks.proxyStatus).toHaveBeenCalledTimes(2);
    expect(renderToStaticMarkup(render(onSave))).toContain("proxy.autoProxyhttp://proxy.invalid:8080");
    const pending = cache.loadProxyStatus(true);
    finish("http://proxy.invalid:9090");
    await pending;
    expect(renderToStaticMarkup(render(onSave))).toContain("proxy.autoProxyhttp://proxy.invalid:9090");
    hooks.cells.length = 0;
    expect(renderToStaticMarkup(render(onSave))).toContain("proxy.autoProxyhttp://proxy.invalid:9090");
  } finally { vi.unstubAllGlobals(); }
});

it("地址独立位于模式行下方，复用既有动画并在折叠时不可交互", () => {
  const onSave = vi.fn(async () => true);
  const custom: Settings = { ...settings, proxy_mode: "custom", proxy_url: "http://proxy.invalid:8080" };
  const card = Children.toArray(render(onSave, custom).props.children)[0];
  if (!isValidElement<Control>(card)) throw new Error("代理卡片缺失");
  const [header, address] = Children.toArray(card.props.children);
  expect(controls(header).filter((control) => control["data-proxy-mode"])).toHaveLength(3);
  expect(controls(header).some((control) => control.onChange)).toBe(false);
  expect(controls(address).filter((control) => control.onChange)).toHaveLength(1);
  if (!isValidElement<Control>(address)) throw new Error("地址展开区缺失");
  expect(address.props.className).toContain("apple-disclosure--open");
  const customHtml = renderToStaticMarkup(render(onSave, custom));
  expect(customHtml).toContain('for="settings-proxy-address"');
  expect(customHtml).toContain('class="app-segmented-control h-9 w-52 shrink-0"');
  expect(customHtml).toContain('style="--segment-count:3;--segment-index:2"');
  expect(customHtml.indexOf('<input id="settings-proxy-address"')).toBeLessThan(customHtml.indexOf('>proxy.saved</button>'));
  expect(customHtml).toContain('class="flex items-center gap-3"');
  hooks.cells.length = 0;
  const closed = render(onSave);
  expect(controls(closed).find((control) => control.onChange)?.disabled).toBe(true);
  const closedHtml = renderToStaticMarkup(closed);
  expect(closedHtml).toContain('class="apple-disclosure__content" aria-hidden="true" inert=""');
  expect(closedHtml).toContain('class="apple-disclosure__body"');
  expect(closedHtml).not.toContain("apple-disclosure--open");
});

it("展开时说明保持单行，动画容器不瞬间切换纵向间距，聚焦不触发页面滚动", async () => {
  const onSave = vi.fn(async () => true);
  const initial = render(onSave);
  const focus = vi.fn();
  controls(initial).find((control) => control.onChange)!.ref!.current = { focus };
  hooks.effects.length = 0;
  controls(initial).find((control) => control["data-proxy-mode"] === "custom")!.onClick!();
  const opened = render(onSave);
  const html = renderToStaticMarkup(opened);
  expect(html).toContain('class="setting-description mt-0.5 truncate" role="status" title="proxy.customHint"');
  expect(html).toContain("proxy-address-disclosure apple-disclosure--open");
  const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");
  const geometry = styles.match(/\.proxy-address-disclosure \.apple-disclosure__content,\s*\.proxy-address-disclosure \.apple-disclosure__body\s*\{([^}]+)\}/)?.[1] ?? "";
  for (const property of ["margin-bottom", "padding-top", "padding-bottom"]) expect(geometry).toContain(`${property}: 0;`);
  vi.stubGlobal("window", { addEventListener: vi.fn(), removeEventListener: vi.fn() });
  try {
    hooks.effects.splice(0).forEach((effect) => effect());
    expect(focus).toHaveBeenCalledExactlyOnceWith({ preventScroll: true });
    await Promise.resolve();
  } finally { vi.unstubAllGlobals(); }
});

it("自动和关闭即时保存，切换模式保留之前的自定义地址", async () => {
  const onSave = vi.fn(async () => true);
  controls(render(onSave)).find((control) => control["data-proxy-mode"] === "off")!.onClick!();
  await Promise.resolve();
  expect(onSave).toHaveBeenCalledExactlyOnceWith({ proxy_mode: "off" });
  const off: Settings = { ...settings, proxy_mode: "off", proxy_url: "http://proxy.invalid:1234" };
  controls(render(onSave, off)).find((control) => control["data-proxy-mode"] === "auto")!.onClick!();
  expect(onSave).toHaveBeenLastCalledWith({ proxy_mode: "auto" });
});

it("自定义只由保存按钮提交，重复点击只保存一次；保存失败保留草稿且不声称已生效", async () => {
  let finish: (saved: boolean) => void = () => undefined;
  const onSave = vi.fn(() => new Promise<boolean>((resolve) => { finish = resolve; }));
  controls(render(onSave)).find((control) => control["data-proxy-mode"] === "custom")!.onClick!();
  expect(onSave).not.toHaveBeenCalled();
  controls(render(onSave)).find((control) => control.onChange)!.onChange!({ target: { value: "  http://proxy.invalid:8080  " } });
  const opened = render(onSave);
  const input = controls(opened).find((control) => control.onChange)!;
  expect(input.onKeyDown).toBeUndefined();
  expect(input.onBlur).toBeUndefined();
  expect(onSave).not.toHaveBeenCalled();
  const button = saveButton(opened);
  button.onClick!();
  button.onClick!();
  expect(onSave).toHaveBeenCalledExactlyOnceWith({ proxy_mode: "custom", proxy_url: "http://proxy.invalid:8080" });
  finish(false);
  await Promise.resolve();
  const html = renderToStaticMarkup(render(onSave));
  expect(html).toContain("proxy.customHint");
  expect(html).not.toContain("proxy.customDescription");
  expect(saveButton(render(onSave)).children).toBe("proxy.save");
  expect(saveButton(render(onSave)).disabled).toBe(false);
  expect(controls(render(onSave)).find((control) => control.onChange)?.value).toBe("  http://proxy.invalid:8080  ");
});

it("无效地址和 SOCKS 地址都不能提交；保存中所有代理控件不可操作", () => {
  const onSave = vi.fn(async () => true);
  const custom: Settings = { ...settings, proxy_mode: "custom", proxy_url: "http://proxy.invalid:8080" };
  for (const value of ["invalid proxy", "http://", "socks5://proxy.invalid:1080"]) {
    controls(render(onSave, custom)).find((control) => control.onChange)!.onChange!({ target: { value } });
    expect(saveButton(render(onSave, custom)).disabled).toBe(false);
    saveButton(render(onSave, custom)).onClick!();
  }
  expect(hooks.error).toHaveBeenCalledTimes(3);
  expect(hooks.error).toHaveBeenCalledWith("proxy.invalid");
  expect(onSave).not.toHaveBeenCalled();
  const html = renderToStaticMarkup(render(onSave, custom, true));
  expect(html.match(/disabled=""/g)).toHaveLength(5);
});

it.each(["auto", "off", "custom"] as const)("从 %s 状态编辑自定义但未保存，保活页恢复时回到已生效的模式和地址", (mode) => {
  const onSave = vi.fn(async () => true);
  const saved: Settings = { ...settings, proxy_mode: mode, proxy_url: "http://proxy.invalid:8080" };
  controls(render(onSave, saved)).find((control) => control["data-proxy-mode"] === "custom")!.onClick!();
  expect(onSave).not.toHaveBeenCalled();
  controls(render(onSave, saved)).find((control) => control.onChange)!.onChange!({ target: { value: "http://proxy.invalid:9090" } });
  expect(renderToStaticMarkup(render(onSave, saved))).toContain('role="status" title="proxy.customHint"');
  // Activity 隐藏后重新显示会重建 effects，状态单元仍保留；不清空 cells 模拟这条生命周期边界。
  hooks.layoutEffects[hooks.layoutEffects.length - 1]();
  const restored = render(onSave, saved);
  expect(controls(restored).find((control) => control["data-proxy-mode"] === mode)?.["aria-pressed"]).toBe(true);
  expect(controls(restored).find((control) => control.onChange)?.value).toBe(saved.proxy_url);
  expect(onSave).not.toHaveBeenCalled();
});

it("保存成功后以新的持久化配置恢复；已保存地址不重复提交", async () => {
  const onSave = vi.fn(async () => true);
  controls(render(onSave)).find((control) => control["data-proxy-mode"] === "custom")!.onClick!();
  controls(render(onSave)).find((control) => control.onChange)!.onChange!({ target: { value: "http://proxy.invalid:9090" } });
  saveButton(render(onSave)).onClick!();
  await Promise.resolve();
  expect(onSave).toHaveBeenCalledExactlyOnceWith({ proxy_mode: "custom", proxy_url: "http://proxy.invalid:9090" });
  const saved: Settings = { ...settings, proxy_mode: "custom", proxy_url: "http://proxy.invalid:9090" };
  render(onSave, saved);
  hooks.layoutEffects[hooks.layoutEffects.length - 1]();
  const restored = render(onSave, saved);
  expect(renderToStaticMarkup(restored)).toContain("proxy.customDescription");
  expect(saveButton(restored).children).toBe("proxy.saved");
  expect(saveButton(restored).disabled).toBe(true);
  saveButton(restored).onClick!();
  expect(onSave).toHaveBeenCalledTimes(1);
  controls(restored).find((control) => control.onChange)!.onChange!({ target: { value: "http://proxy.invalid:8080" } });
  const edited = render(onSave, saved);
  expect(saveButton(edited).children).toBe("proxy.save");
  expect(saveButton(edited).disabled).toBe(false);
  expect(onSave).toHaveBeenCalledTimes(1);
  controls(edited).find((control) => control.onChange)!.onChange!({ target: { value: saved.proxy_url } });
  expect(saveButton(render(onSave, saved)).children).toBe("proxy.saved");
  expect(saveButton(render(onSave, saved)).disabled).toBe(true);
});
