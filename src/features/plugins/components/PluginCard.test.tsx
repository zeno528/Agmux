import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { PluginCard } from "./PluginCard";

const { pluginIcon } = vi.hoisted(() => ({ pluginIcon: vi.fn() }));
vi.mock("../../../api", () => ({ api: { pluginIcon } }));

it("没有可用 Logo 时显示默认插件图标，不显示名称首字母", () => {
  const html = renderToStaticMarkup(<PluginCard name="Fixture" />);
  expect(html).toContain("<svg");
  expect(html).not.toContain(">F</span>");
  expect(html).not.toContain("<img");
});

it("图标请求去重，重挂载首帧复用图片；缓存只在会话内，刷新后重新请求且失败允许重试", async () => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  const image = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=";
  const icon = pluginIcon.mockResolvedValue(image);
  try {
    const cache = await import("../../../app/managementDataCache");
    const request = cache.loadPluginIcon("/fixture/icon.png");
    expect(cache.loadPluginIcon("/fixture/icon.png")).toBe(request);
    await request;
    for (let mount = 0; mount < 2; mount += 1) {
      const html = renderToStaticMarkup(<PluginCard name="Fixture" iconPath="/fixture/icon.png" />);
      expect(html).toContain("<img");
      expect(html).toContain(`src="${image}"`);
    }
    expect(icon).toHaveBeenCalledTimes(1);

    // 持久层是后端磁盘缓存：刷新后会话缓存清空、不落 localStorage，重新请求一次。
    vi.resetModules();
    const refreshed = await import("../../../app/managementDataCache");
    expect(refreshed.getCachedPluginIcon("/fixture/icon.png")).toBeNull();
    await expect(refreshed.loadPluginIcon("/fixture/icon.png")).resolves.toBe(image);
    expect(icon).toHaveBeenCalledTimes(2);
    expect(storage.size).toBe(0);

    icon.mockRejectedValueOnce(new Error("temporarily unavailable"));
    await expect(refreshed.loadPluginIcon("/fixture/retry.png")).rejects.toThrow("temporarily unavailable");
    expect(refreshed.getCachedPluginIcon("/fixture/retry.png")).toBeNull();
    await expect(refreshed.loadPluginIcon("/fixture/retry.png")).resolves.toBe(image);
    expect(icon).toHaveBeenCalledTimes(4);
  } finally {
    icon.mockReset();
    vi.unstubAllGlobals();
  }
});
