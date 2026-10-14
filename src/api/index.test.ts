// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { afterEach, expect, it, vi } from "vitest";
import { webInvoke } from "./web-mock";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

afterEach(() => vi.unstubAllGlobals());

// 四个命令注册测试共用同一份 lib.rs 注册表快照。
const handlerSource = readFileSync(new URL("../../src-tauri/src/lib.rs", import.meta.url), "utf8").match(/generate_handler!\[([\s\S]*?)\]/)?.[1] ?? "";
const registered = new Set([...handlerSource.matchAll(/commands::(\w+)/g)].map((match) => match[1]));

it("获取 Codex 模型列表使用实际注册的 Tauri 命令", async () => {
  const models = ["fixture-model"];
  mocks.invoke.mockImplementation(async (command: string) => {
    if (!registered.has(command)) throw new Error(`未注册 Tauri 命令：${command}`);
    return models;
  });
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { api } = await import("./index");

  expect(await api.codexFetchProviderModels("https://example.test", "fixture-key")).toEqual(models);
  expect(mocks.invoke).toHaveBeenCalledWith("codex_fetch_provider_models", {
    baseUrl: "https://example.test",
    apiKey: "fixture-key",
  });
});

it("浏览器 mock 明确说明模型列表需要在桌面版获取", async () => {
  await expect(webInvoke("codex_fetch_provider_models", {
    baseUrl: "https://example.test",
    apiKey: "fixture-key",
  })).rejects.toThrow("请在桌面版获取供应商模型列表");
  await expect(webInvoke("codex_fetch_chatgpt_models", { id: "fixture-profile", source: "desktop", accountId: null }))
    .rejects.toThrow("请在桌面版获取 ChatGPT 模型列表");
  expect(await webInvoke("codex_fetch_chatgpt_models", { source: "desktop", refresh: false })).toEqual([]);
});

it("ChatGPT 模型目录命令传递指定配置与账号身份", async () => {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (!registered.has(command)) throw new Error(`未注册 Tauri 命令：${command}`);
    return [];
  });
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { api } = await import("./index");
  expect(await api.codexFetchChatgptModels("fixture-profile", "oauth", "fixture-account")).toEqual([]);
  expect(mocks.invoke).toHaveBeenLastCalledWith("codex_fetch_chatgpt_models", {
    id: "fixture-profile", source: "oauth", accountId: "fixture-account", refresh: true,
  });
  await api.codexFetchChatgptModels(null, "oauth", "fixture-account", false);
  expect(mocks.invoke).toHaveBeenLastCalledWith("codex_fetch_chatgpt_models", {
    id: null, source: "oauth", accountId: "fixture-account", refresh: false,
  });
});

it("代理状态使用已注册的后端命令，浏览器预览不能假装已接管网络", async () => {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (!registered.has(command)) throw new Error(`未注册 Tauri 命令：${command}`);
    return "http://proxy.invalid:8080/";
  });
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { api } = await import("./index");
  expect(await api.getProxyStatus()).toBe("http://proxy.invalid:8080/");
  expect(mocks.invoke).toHaveBeenLastCalledWith("get_proxy_status", undefined);
  await expect(webInvoke("get_proxy_status")).rejects.toThrow("浏览器预览不支持读取应用代理状态");
});

it("应用更新经已注册的共享网络命令返回原生资源，浏览器禁止模拟", async () => {
  const metadata = { rid: 17, currentVersion: "1.0.0", version: "1.0.1", rawJson: {} };
  mocks.invoke.mockImplementation(async (command: string) => {
    if (!registered.has(command)) throw new Error(`未注册 Tauri 命令：${command}`);
    return metadata;
  });
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { api } = await import("./index");
  expect(await api.checkAppUpdate()).toEqual(metadata);
  expect(mocks.invoke).toHaveBeenLastCalledWith("check_app_update", undefined);
  await expect(webInvoke("check_app_update")).rejects.toThrow("应用更新需要在桌面版执行");
});

it("CLI 命令全部注册且浏览器禁止执行", async () => {
  mocks.invoke.mockImplementation(async (command: string) => {
    expect(registered.has(command)).toBe(true);
    return { installation: "native", version: "1.2.3" };
  });
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { api } = await import("./index");
  await api.claudeCheckCliUpdate();
  await api.codexCheckCliUpdate();
  expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["claude_check_cli_update", "codex_check_cli_update"]);
  expect(mocks.invoke.mock.calls.every(([, args]) => args === undefined)).toBe(true);
  await api.claudeGetCliStatus();
  await api.claudeInstallCli();
  await api.claudeUpdateCli();
  await api.codexGetCliStatus();
  await api.codexInstallCli();
  await api.codexUpdateCli();
  for (const command of ["claude_install_cli", "claude_update_cli", "codex_get_cli_status", "codex_check_cli_update", "codex_install_cli", "codex_update_cli"]) {
    await expect(webInvoke(command)).rejects.toMatchObject({ stage: "desktop", kind: "validation_error" });
  }
  // 状态查询与更新检查在浏览器 mock 可用（升级胶囊需要状态展示），主动安装/升级仍仅限桌面版
  await expect(webInvoke("claude_get_cli_status")).resolves.toMatchObject({ installation: "native" });
  await expect(webInvoke("claude_check_cli_update")).resolves.toMatchObject({ available: true });
});

it("两端 CLI 的 Tauri 拒绝保留可翻译的阶段和错误类别", async () => {
  const failure = { stage: "fetch_version", kind: "timeout", message: "版本查询超时，请检查网络或代理" };
  mocks.invoke.mockRejectedValue(failure);
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { api } = await import("./index");
  await expect(api.claudeCheckCliUpdate()).rejects.toEqual(failure);
  await expect(api.codexCheckCliUpdate()).rejects.toEqual(failure);
});
