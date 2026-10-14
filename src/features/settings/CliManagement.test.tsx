import { renderToStaticMarkup } from "react-dom/server";
import { Children, isValidElement, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createInstance, type TFunction } from "i18next";
import enSettings from "../../i18n/locales/en-US/settings";
import zhSettings from "../../i18n/locales/zh-CN/settings";
import type { CliFailure, CliStatus } from "../../types";
import { CliCard, useCliManagement } from "./CliManagement";
import { cliFailureMessage } from "../../components/CliUpgradePill";

const hooks = vi.hoisted(() => ({
  cells: [] as unknown[], effects: [] as (() => void | (() => void))[], index: 0, t: null as TFunction<"settings"> | null,
  error: vi.fn(), info: vi.fn(), success: vi.fn(), command: vi.fn(),
  cached: vi.fn((): import("../../types").CliStatus | null => null), cacheSet: vi.fn(),
  cachedUpdate: vi.fn((): { latest_version: string; channel: string; available: boolean } | null => null),
  serviceCheck: vi.fn(), touch: vi.fn(), cacheUpdate: vi.fn(),
}));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
    const index = hooks.index++;
    const previous = hooks.cells[index] as unknown[] | undefined;
    hooks.cells[index] = deps;
    hooks.effects.push(!previous || deps.some((dep, i) => !Object.is(dep, previous[i])) ? effect : () => undefined);
  },
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
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: hooks.t ?? ((key: string, options?: { version?: string }) => `${key}${options?.version ?? ""}`) }) }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ error: hooks.error, info: hooks.info, success: hooks.success }) }));
vi.mock("../../app/managementDataCache", () => ({ getCachedCliStatus: hooks.cached, getCachedCliUpdate: hooks.cachedUpdate, subscribeCliUpdate: () => () => undefined, setCliStatusCache: hooks.cacheSet, runCliUpdateCheck: hooks.serviceCheck, touchCliUpdateCheckedAt: hooks.touch, clearCachedCliUpdate: hooks.cacheUpdate }));
vi.mock("../../api", () => ({ api: {
  codexGetCliStatus: hooks.command, codexCheckCliUpdate: hooks.command, codexInstallCli: hooks.command, codexUpdateCli: hooks.command,
  claudeGetCliStatus: hooks.command, claudeCheckCliUpdate: hooks.command, claudeInstallCli: hooks.command, claudeUpdateCli: hooks.command,
} }));
beforeEach(() => {
  hooks.cells.length = 0;
  hooks.effects.length = 0;
  hooks.index = 0;
  hooks.t = null;
  hooks.command.mockReset();
  hooks.error.mockClear();
  hooks.info.mockClear();
  hooks.success.mockClear();
  hooks.cached.mockClear().mockReturnValue(null);
  hooks.cachedUpdate.mockClear().mockReturnValue(null);
  hooks.cacheSet.mockClear();
  hooks.serviceCheck.mockReset();
  hooks.touch.mockClear();
  hooks.cacheUpdate.mockClear();
});
afterEach(() => vi.useRealTimers());

const status: CliStatus = {
  installation: "native", source: null, version: "1.2.3", path: "/fixture/bin/cli", other_paths: [],
  platform: "darwin-arm64", network: "direct", proxy: null, busy: false,
};
const management = {
  status, busy: false, operation: null, refresh: vi.fn(), check: vi.fn(), run: vi.fn(),
};
// 清空全部微任务：effect 里检测 → 静默更新检查是链式异步，单次微任务等不完。
const flush = () => new Promise<void>((resolve) => { setTimeout(resolve, 0); });

it.each(["codex", "claude"] as const)("%s 首页任务结束后恢复 Agent 页状态", async (client) => {
  vi.useFakeTimers();
  hooks.cached.mockReturnValue(status);
  hooks.command.mockResolvedValueOnce({ ...status, busy: true }).mockResolvedValueOnce(status);
  const render = () => { hooks.index = 0; hooks.effects.length = 0; return useCliManagement(client, true); };
  render();
  hooks.effects[hooks.effects.length - 1](); // 进页检测读到首页任务仍在执行。
  await vi.advanceTimersByTimeAsync(0);
  const working = render();
  expect(working.status?.busy).toBe(true);
  expect(hooks.serviceCheck).not.toHaveBeenCalled();
  const stop = hooks.effects[0](); // 跟踪外部任务。
  await vi.advanceTimersByTimeAsync(2000);
  const settled = render();
  expect(settled.status).toEqual(status);
  expect(buttons(CliCard({ client, management: settled }))[0].disabled).toBe(false);
  if (stop) stop();
  expect(hooks.effects[0]()).toBeUndefined(); // 任务结束后不再轮询。
});

it.each(["codex", "claude"] as const)("%s 静默和主动检查共用按钮检查中状态，结束后恢复", async (client) => {
  const checked = { status, latest_version: "1.2.3", channel: "latest", available: false };
  hooks.cached.mockReturnValue(status);
  hooks.command.mockResolvedValueOnce(status);
  let finishQuiet!: (value: typeof checked) => void;
  hooks.serviceCheck.mockReturnValueOnce(new Promise<typeof checked>((resolve) => { finishQuiet = resolve; }));
  const render = () => { hooks.index = 0; return useCliManagement(client, true); };
  render();
  hooks.effects[hooks.effects.length - 1]();
  await flush();
  const assertChecking = () => {
    const current = render();
    expect(current.busy).toBe(true);
    expect(current.operation).toBe("check");
    const card = CliCard({ client, management: current });
    const button = buttons(card)[0];
    expect(button.disabled).toBe(true);
    expect(renderToStaticMarkup(<>{button.children}</>)).toContain("cli.checkingUpdate");
    expect(renderToStaticMarkup(card).match(/cli.checkingUpdate/g)).toHaveLength(1);
  };
  assertChecking();
  await render().check();
  expect(hooks.serviceCheck).toHaveBeenCalledOnce();
  finishQuiet(checked);
  await flush();
  expect(render().busy).toBe(false);
  expect(buttons(CliCard({ client, management: render() }))[0].disabled).toBe(false);
  expect(hooks.success).not.toHaveBeenCalled();
  let finishManual!: (value: typeof checked) => void;
  hooks.serviceCheck.mockReturnValueOnce(new Promise<typeof checked>((resolve) => { finishManual = resolve; }));
  const manual = render().check();
  assertChecking();
  finishManual(checked);
  await manual;
  expect(render().busy).toBe(false);
  expect(render().operation).toBeNull();
  expect(hooks.success).toHaveBeenCalledWith("cli.noUpdate1.2.3");
});

it.each(["codex", "claude"] as const)("%s 本地检测期间安装按钮禁用，检测结束后恢复", async (client) => {
  const missing = { ...status, installation: "missing" as const, version: null, path: null };
  hooks.cached.mockReturnValue(missing);
  let finishDetection!: (value: CliStatus) => void;
  hooks.command.mockReturnValueOnce(new Promise<CliStatus>((resolve) => { finishDetection = resolve; }));
  const render = () => { hooks.index = 0; return useCliManagement(client, true); };
  render();
  hooks.effects[hooks.effects.length - 1]();
  expect(buttons(CliCard({ client, management: render() }))[0].disabled).toBe(true);
  expect(renderToStaticMarkup(CliCard({ client, management: render() }))).toContain('style="opacity:1"');
  finishDetection(missing);
  await flush();
  expect(buttons(CliCard({ client, management: render() }))[0].disabled).toBe(false);
  expect(hooks.serviceCheck).not.toHaveBeenCalled();
});

it("首次检测、安装和不可管理状态仍保留禁用外观", () => {
  for (const current of [
    { ...management, status: null, busy: true, operation: "refresh" as const },
    { ...management, busy: true, operation: "install" as const },
    { ...management, status: { ...status, busy: true }, busy: true, operation: "refresh" as const },
    { ...management, status: { ...status, installation: "conflict" as const }, busy: true, operation: "refresh" as const },
  ]) {
    const card = CliCard({ client: "codex", management: current });
    expect(buttons(card)[0].disabled).toBe(true);
    expect(renderToStaticMarkup(card)).not.toContain('style="opacity:1"');
  }
});

it.each(["codex", "claude"] as const)("%s Activity 隐藏后重新进入会检测升级后的版本", async (client) => {
  const render = (active: boolean) => { hooks.index = 0; const current = useCliManagement(client, active); hooks.effects[hooks.effects.length - 1](); return current; };
  render(false);
  expect(hooks.command).not.toHaveBeenCalled();
  const checked = { status, latest_version: "1.2.4", channel: "latest", available: true };
  hooks.command.mockResolvedValueOnce(status); // 本地检测
  hooks.serviceCheck.mockResolvedValueOnce(checked); // 静默更新检查走共享服务
  render(true);
  const reconnect = hooks.effects[hooks.effects.length - 1];
  expect(render(true).busy).toBe(true);
  expect(render(true).operation).toBe("refresh");
  await flush();
  const restored = render(true);
  expect(restored.status).toEqual(status);
  expect(hooks.serviceCheck).toHaveBeenCalledOnce(); // 静默检查结果写入共享缓存 → 升级胶囊自行订阅
  const refreshed = { ...status, version: "1.2.4", path: "/fixture/bin/cli-v2" };
  const checkedAgain = { status: refreshed, latest_version: "1.2.5", channel: "latest", available: false };
  hooks.command.mockResolvedValueOnce(refreshed);
  hooks.serviceCheck.mockResolvedValueOnce(checkedAgain);
  reconnect(); // Activity 重新显示时重跑 effect，不依赖 active=false 分支清理标记。
  await flush();
  const reentered = render(true);
  expect(hooks.command).toHaveBeenCalledTimes(2);
  expect(hooks.serviceCheck).toHaveBeenCalledTimes(2);
  expect(reentered.status).toEqual(refreshed);
  const html = renderToStaticMarkup(<CliCard client={client} management={reentered} />);
  expect(html).toContain(refreshed.path!);
  hooks.serviceCheck.mockResolvedValueOnce(checkedAgain);
  hooks.info.mockClear();
  hooks.success.mockClear();
  await reentered.check(); // 主动检查更新仍可用（走共享服务）
  expect(hooks.info).not.toHaveBeenCalled();
  expect(hooks.success).toHaveBeenCalledWith("cli.noUpdate1.2.4");
  expect(hooks.command).toHaveBeenCalledTimes(2);
});

it("进入分区首帧直出缓存状态不闪骨架，静默刷新补齐最新值并写穿缓存", async () => {
  const cached = { ...status, version: "1.2.0" };
  const checked = { status, latest_version: "1.2.4", channel: "latest", available: false };
  hooks.cached.mockReturnValueOnce(cached);
  hooks.command.mockResolvedValueOnce(status);
  hooks.serviceCheck.mockResolvedValueOnce(checked);
  const render = (active: boolean) => { hooks.index = 0; const current = useCliManagement("codex", active); hooks.effects[hooks.effects.length - 1](); return current; };
  // 首帧断言：不等任何异步，缓存值必须已经在状态里（闪骨架 = 这里拿到 null）
  const first = render(true);
  expect(first.status).toEqual(cached);
  await flush();
  const settled = render(true);
  expect(settled.status).toEqual(status);
  expect(hooks.command).toHaveBeenCalledOnce();
  expect(hooks.cacheSet).toHaveBeenCalledWith("codex", status);
  hooks.command.mockResolvedValueOnce(status);
  hooks.serviceCheck.mockResolvedValueOnce(checked);
  render(false);
  const reentered = render(true);
  expect(reentered.status).toEqual(status);
  await flush();
  expect(hooks.command).toHaveBeenCalledTimes(2);
});

it("静默更新检查失败不打扰界面，主动检查失败才透出", async () => {
  const render = (active: boolean) => { hooks.index = 0; const current = useCliManagement("codex", active); hooks.effects[hooks.effects.length - 1](); return current; };
  const failure = { stage: "fetch_version", kind: "timeout", message: "fixture" };
  hooks.command.mockResolvedValueOnce(status);
  hooks.serviceCheck.mockRejectedValueOnce(failure); // 静默失败由共用 catch 处理
  render(true);
  await flush();
  const settled = render(true);
  expect(settled.status).toEqual(status);
  expect(hooks.error).not.toHaveBeenCalled(); // 静默失败只留后端日志，不进界面
  expect(settled.busy).toBe(false);
  expect(hooks.touch).toHaveBeenCalledOnce();
  hooks.serviceCheck.mockRejectedValueOnce(failure);
  await settled.check(); // 主动检查失败照常透出
  expect(hooks.error).toHaveBeenCalledOnce();
  expect(hooks.touch).toHaveBeenCalledWith("codex"); // 两种检查失败都推进冷却闸
  expect(hooks.touch).toHaveBeenCalledTimes(2);
});

it("首次检测失败后再次进入仍可重试，未安装结果也会保留", async () => {
  const render = (active: boolean) => { hooks.index = 0; const current = useCliManagement("codex", active); hooks.effects[hooks.effects.length - 1](); return current; };
  hooks.command.mockRejectedValueOnce({ stage: "detect", kind: "io_error", message: "fixture" });
  render(true);
  await Promise.resolve();
  expect(render(false).status).toBeNull();
  const missing = { ...status, installation: "missing" as const, version: null, path: null };
  hooks.command.mockResolvedValueOnce(missing);
  render(true);
  await Promise.resolve();
  expect(render(true).status).toEqual(missing);
  expect(hooks.command).toHaveBeenCalledTimes(2);
});

it("两端卡片共用行为，运行中的 Codex 不会禁用 Claude 按钮", () => {
  const codex = renderToStaticMarkup(<CliCard client="codex" management={{
    ...management, busy: true,
    operation: "install" as const,
  }} />);
  const claude = renderToStaticMarkup(<CliCard client="claude" management={management} />);
  expect(codex.match(/disabled=""/g)).toHaveLength(1);
  expect(codex).toContain("animate-spin");
  expect(codex).toContain("cli.installing");
  expect(codex).toContain('role="status"');
  expect(codex).toContain('aria-live="polite"');
  expect(claude).not.toContain('disabled=""');
  expect(claude).not.toContain("animate-spin");
  expect(claude).toContain("cli.checkUpdate");
  expect(claude).not.toContain("cli.updateTo");
});

it("进行中反馈按动作显示对应文案", () => {
  const render = (operation: "refresh" | "check" | "install") => renderToStaticMarkup(<CliCard client="codex" management={{
    ...management, busy: true, operation,
  }} />);
  expect(render("refresh")).not.toContain("cli.refreshing");
  expect(render("refresh")).not.toContain('role="status"');
  expect(render("check")).toContain("cli.checkingUpdate");
  expect(render("install")).toContain("cli.installing");
});

it("只显示可管理的安装来源", () => {
  const claude = renderToStaticMarkup(<CliCard client="claude" management={{
    ...management, status: { ...status, source: "native" },
  }} />);
  const codex = renderToStaticMarkup(<CliCard client="codex" management={{
    ...management, status: { ...status, installation: "missing", source: "embedded", version: null },
  }} />);
  expect(claude).toContain("cli.sources.claude.native");
  expect(claude).toContain('class="plan-badge">cli.sources.claude.native</span>');
  expect(claude).not.toContain("apple-chip--success");
  expect(codex).not.toContain("cli.sources.codex.embedded");
  expect(codex).not.toContain("cli.missing");
});

it.each(["codex", "claude"] as const)("%s 保留安装路径，移除代理信息且无路径时不显示空底栏", (client) => {
  const html = renderToStaticMarkup(<CliCard client={client} management={{
    ...management, status: { ...status, other_paths: ["/fixture/other/cli"], network: "proxy", proxy: "http://proxy.invalid:9001" },
  }} />);
  expect(html).toContain("cli.pathLabel");
  expect(html).toContain(status.path!);
  expect(html).toContain("/fixture/other/cli");
  expect(html).not.toContain("cli.networkLabel");
  expect(html).not.toContain("proxy.invalid");
  for (const missing of [{ ...status, installation: "missing" as const, path: null }, null]) {
    const empty = renderToStaticMarkup(<CliCard client={client} management={{ ...management, status: missing }} />);
    expect(empty).not.toContain("cli-facts");
    expect(empty).not.toContain("cli.pathLabel");
  }
});

it("版本查询错误随界面语言翻译，不显示后端中文或内部阶段标识", async () => {
  const i18n = createInstance();
  await i18n.init({ lng: "en-US", defaultNS: "settings", resources: {
    "en-US": { settings: enSettings }, "zh-CN": { settings: zhSettings },
  } });
  const failure: CliFailure = { stage: "fetch_version", kind: "timeout", message: "版本查询超时，请检查网络或代理" };
  hooks.t = i18n.getFixedT("en-US", "settings");
  for (const client of ["codex", "claude"] as const) {
    const html = renderToStaticMarkup(<CliCard client={client} management={management} />);
    expect(html).not.toContain('role="alert"');
    for (const kind of ["network_error", "parse_error", "unknown"]) {
      const message = cliFailureMessage({ ...failure, kind }, client, hooks.t);
      expect(message).not.toMatch(/版本查询|cli\./);
    }
  }
  expect(cliFailureMessage(failure, "claude", hooks.t)).toBe("Claude Code: The version query timed out. Check your network or proxy and retry");
  hooks.t = i18n.getFixedT("zh-CN", "settings");
  expect(cliFailureMessage(failure, "claude", hooks.t)).toContain("版本查询超时");
});

it("刷新失败静默，主动检查和安装失败透出，均恢复状态且不写回结果", async () => {
  const failure: CliFailure = { stage: "fetch_version", kind: "timeout", message: "诊断信息" };
  for (const client of ["codex", "claude"] as const) {
    hooks.cells.length = 0;
    hooks.success.mockClear();
    const render = () => { hooks.index = 0; return useCliManagement(client, false); };
    for (const action of ["refresh", "check", "install"] as const) {
      if (action === "refresh") hooks.command.mockRejectedValueOnce(failure);
      else if (action === "check") hooks.serviceCheck.mockRejectedValueOnce(failure);
      else hooks.command.mockRejectedValueOnce(failure);
      const current = render();
      const result = await (action === "refresh" ? current.refresh() : action === "check" ? current.check() : current.run());
      if (action === "refresh") expect(result).toBeNull();
      expect(render().busy).toBe(false);
      expect(render().operation).toBeNull();
      expect(render().status).toBeNull();
      expect(hooks.error).toHaveBeenCalledTimes(action === "refresh" ? 0 : 1);
      expect(hooks.success).not.toHaveBeenCalled();
      hooks.error.mockClear();
    }
    hooks.command.mockResolvedValueOnce(status);
    await render().run();
    expect(render().status).toEqual(status);
    expect(hooks.success).toHaveBeenCalledOnce();
    expect(hooks.success).toHaveBeenCalledWith("cli.installComplete1.2.3");
    expect(hooks.error).not.toHaveBeenCalled();
  }
});

function buttons(node: ReactNode): { children?: ReactNode; onClick: () => void; disabled?: boolean }[] {
  const result: { children?: ReactNode; onClick: () => void; disabled?: boolean }[] = [];
  Children.forEach(node, (child) => {
    if (!isValidElement<{ children?: ReactNode; onClick: () => void; disabled?: boolean }>(child)) return;
    if (child.type === "button") result.push(child.props);
    else result.push(...buttons(child.props.children));
  });
  return result;
}

it("检查只查询，发现更新后仍需单独点击升级；没有更新时不提供升级", () => {
  hooks.cached.mockReturnValue(status);
  for (const client of ["codex", "claude"] as const) {
    const actions = { ...management, check: vi.fn(), run: vi.fn() };
    const unchecked = CliCard({ client, management: actions });
    buttons(unchecked).find((button) => button.children === "cli.checkUpdate")!.onClick();
    expect(actions.check).toHaveBeenCalledOnce();
    expect(actions.run).not.toHaveBeenCalled();
    for (const available of [false, true]) {
      hooks.cachedUpdate.mockReturnValue(available ? { latest_version: "1.2.4", channel: "latest", available: true } : null);
      const checked = CliCard({ client, management: actions });
      const html = renderToStaticMarkup(checked);
      expect(html.includes('class="plan-badge gap-1"')).toBe(available);
      expect(html).not.toContain("cli.noUpdate");
      if (available) expect(html).toContain("cliUpdate.upgradeTitle1.2.4");
      expect(actions.run).not.toHaveBeenCalled();
    }
  }
});

it("版本号与升级动作复用同一徽标样式", () => {
  hooks.cached.mockReturnValue(status);
  hooks.cachedUpdate.mockReturnValue({ latest_version: "1.2.4", channel: "latest", available: true });
  const html = renderToStaticMarkup(<CliCard client="claude" management={management} />);
  expect(html).toContain('class="plan-badge">1.2.3</span>');
  expect(html).toContain('<button type="button" class="plan-badge gap-1"');
});

it("混装与其他任务阻止安装，桌面内置副本可以共存", () => {
  for (const installation of ["other", "conflict"] as const) {
    const card = renderToStaticMarkup(<CliCard client="codex" management={{
      ...management, status: { ...status, installation },
    }} />);
    expect(card.match(/disabled=""/g)).toHaveLength(1);
  }
  const card = renderToStaticMarkup(<CliCard client="codex" management={{
    ...management, status: { ...status, installation: "missing", version: null, embedded_paths: ["/fixture/desktop"] },
  }} />);
  expect(card).not.toContain('disabled=""');
  expect(card).toContain("cli.install");
  expect(card).toContain("app-button--primary");
  expect(card).not.toContain("/fixture/desktop");
  const remoteTask = renderToStaticMarkup(<CliCard client="claude" management={{
    ...management, status: { ...status, busy: true },
  }} />);
  expect(remoteTask.match(/disabled=""/g)).toHaveLength(1);
});
