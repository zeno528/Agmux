import { afterEach, describe, expect, it, vi } from "vitest";
import { relaunch } from "@tauri-apps/plugin-process";
import { checkForAppUpdate, toAppUpdate } from "./appUpdate";

const { check, sdkInvoke, logUpdateEvent, setUpdateMarker, takeUpdateMarker } = vi.hoisted(() => ({
  check: vi.fn(),
  sdkInvoke: vi.fn(),
  logUpdateEvent: vi.fn(async (_event: string, _version?: string) => undefined),
  setUpdateMarker: vi.fn(async (_version: string) => undefined),
  takeUpdateMarker: vi.fn(async () => null as string | null),
}));

vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));
vi.mock("../../api", () => ({
  api: { checkAppUpdate: check, logUpdateEvent, setUpdateMarker, takeUpdateMarker },
  isTauri: true,
}));

afterEach(() => vi.unstubAllGlobals());

describe("toAppUpdate", () => {
  it("透传更新日志：body 映射为 notes，缺失归一为 null", () => {
    const noop = vi.fn(async () => {});
    expect(toAppUpdate({ version: "0.16.0", body: "### 新增\n- 弹窗展示更新日志", download: noop, install: noop }).notes)
      .toBe("### 新增\n- 弹窗展示更新日志");
    expect(toAppUpdate({ version: "0.16.0", download: noop, install: noop }).notes).toBeNull();
  });

  it("记录检查结果：有更新、无更新和检查失败分别可辨识", async () => {
    logUpdateEvent.mockClear();
    check.mockResolvedValueOnce(null);
    expect(await checkForAppUpdate()).toBeNull();
    expect(logUpdateEvent).toHaveBeenLastCalledWith("check_latest", undefined);

    check.mockResolvedValueOnce({ rid: 7, currentVersion: "0.10.4", version: "0.10.5", rawJson: {} });
    expect((await checkForAppUpdate())?.version).toBe("0.10.5");
    expect(logUpdateEvent).toHaveBeenLastCalledWith("check_available", "0.10.5");

    check.mockRejectedValueOnce(new Error("检查失败"));
    await expect(checkForAppUpdate()).rejects.toThrow("检查失败");
    expect(logUpdateEvent).toHaveBeenLastCalledWith("check_failure", undefined);
  });

  it("共享网络检查返回的原生资源继续交给官方插件下载和安装", async () => {
    vi.stubGlobal("window", {
      __TAURI_INTERNALS__: { transformCallback: () => 1, invoke: sdkInvoke },
    });
    sdkInvoke.mockReset();
    sdkInvoke.mockImplementation(async (command: string) => {
      if (command === "plugin:updater|download") return 18;
      if (command === "plugin:updater|install") return undefined;
      throw new Error(`意外调用：${command}`);
    });
    check.mockResolvedValueOnce({
      rid: 17, currentVersion: "0.10.4", version: "0.10.5",
      body: "fixture notes", date: "2026-01-01T00:00:00Z", rawJson: { fixture: true },
    });
    const update = await checkForAppUpdate();
    expect(update?.notes).toBe("fixture notes");
    await update!.install();
    expect(sdkInvoke).toHaveBeenNthCalledWith(1, "plugin:updater|download", expect.objectContaining({ rid: 17 }), undefined);
    expect(sdkInvoke).toHaveBeenNthCalledWith(2, "plugin:updater|install", { updateRid: 17, bytesRid: 18 }, undefined);
  });

  it("安装成功：下载后先把版本标记原子落盘，再启动安装器", async () => {
    const download = vi.fn(async () => {});
    const install = vi.fn(async () => {});
    vi.mocked(relaunch).mockClear();
    logUpdateEvent.mockClear();
    setUpdateMarker.mockClear();
    takeUpdateMarker.mockClear();
    const update = toAppUpdate({ version: "0.10.5", download, install });

    expect(update.version).toBe("0.10.5");
    await update.install();
    expect(download).toHaveBeenCalledOnce();
    expect(install).toHaveBeenCalledOnce();
    // 顺序必须是 download → 标记落盘 → install：Windows 安装器启动即杀进程，
    // localStorage 异步提交来不及写盘会让升级后通知丢失
    const [downloadAt, markAt, installAt] = [download, setUpdateMarker, install].map(
      (mock) => mock.mock.invocationCallOrder[0],
    );
    expect(downloadAt).toBeLessThan(markAt);
    expect(markAt).toBeLessThan(installAt);
    expect(setUpdateMarker).toHaveBeenCalledWith("0.10.5");
    expect(logUpdateEvent.mock.calls).toEqual([
      ["download_start", "0.10.5"],
      ["download_complete", "0.10.5"],
      ["install_complete", "0.10.5"],
    ]);
    expect(takeUpdateMarker).not.toHaveBeenCalled();
    expect(relaunch).toHaveBeenCalledOnce();
  });

  it("安装失败：清除标记并向上抛错", async () => {
    logUpdateEvent.mockClear();
    setUpdateMarker.mockClear();
    takeUpdateMarker.mockClear();
    const failure = vi.fn(async () => { throw new Error("安装器启动失败"); });
    const update = toAppUpdate({ version: "0.10.5", download: vi.fn(async () => {}), install: failure });

    await expect(update.install()).rejects.toThrow("安装器启动失败");
    expect(setUpdateMarker).toHaveBeenCalledWith("0.10.5");
    expect(logUpdateEvent.mock.calls).toEqual([
      ["download_start", "0.10.5"],
      ["download_complete", "0.10.5"],
      ["install_failure", "0.10.5"],
    ]);
    expect(takeUpdateMarker).toHaveBeenCalledOnce();
  });
});
