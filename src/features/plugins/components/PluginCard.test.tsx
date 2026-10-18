import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { PluginCard } from "./PluginCard";

const { pluginIcon } = vi.hoisted(() => ({ pluginIcon: vi.fn() }));
vi.mock("../../../api", () => ({ api: { pluginIcon } }));

it("图标请求去重，重挂载首帧复用图片，刷新后同步恢复且失败允许重试", async () => {
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

    vi.resetModules();
    const refreshed = await import("../../../app/managementDataCache");
    expect(refreshed.getCachedPluginIcon("/fixture/icon.png")).toBe(image);
    await expect(refreshed.loadPluginIcon("/fixture/icon.png")).resolves.toBe(image);
    expect(icon).toHaveBeenCalledTimes(1);

    icon.mockRejectedValueOnce(new Error("temporarily unavailable"));
    await expect(refreshed.loadPluginIcon("/fixture/retry.png")).rejects.toThrow("temporarily unavailable");
    expect(refreshed.getCachedPluginIcon("/fixture/retry.png")).toBeNull();
    await expect(refreshed.loadPluginIcon("/fixture/retry.png")).resolves.toBe(image);
    expect(icon).toHaveBeenCalledTimes(3);

    await refreshed.loadPluginIcon("https://example.test/content?id=one&sig=old");
    expect(refreshed.getCachedPluginIcon("https://example.test/content?sig=new&id=one")).toBe(image);
    expect(refreshed.getCachedPluginIcon("https://example.test/content?id=two&sig=new")).toBeNull();
    expect(icon).toHaveBeenCalledTimes(4);
  } finally {
    icon.mockReset();
    vi.unstubAllGlobals();
  }
});
