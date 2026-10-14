import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getState: vi.fn(),
  stateUpdates: [] as unknown[],
  errorUpdates: [] as string[],
  stateHookCalls: 0,
}));

vi.mock("../api", () => ({
  api: { getState: mocks.getState },
  isTauri: false,
}));

vi.mock("react", () => ({
  useCallback: <T,>(callback: T) => callback,
  useEffect: () => undefined,
  useRef: (current: unknown) => ({ current }),
  useState: (initial: unknown) => {
    const hookIndex = mocks.stateHookCalls++;
    const setValue = (value: unknown) => {
      if (hookIndex === 0) mocks.stateUpdates.push(value);
      if (hookIndex === 2) mocks.errorUpdates.push(String(value));
    };
    return [initial, setValue];
  },
}));

describe("asAppView 坏值回落", () => {
  it("合法 AppView 原样通过，非法值与空值回落默认页", async () => {
    // 模块顶层消费预发 get_state：先给 mock 一个实现再加载
    mocks.getState.mockResolvedValue({ settings: {} });
    const { asAppView } = await import("./appShellHooks");
    expect(asAppView("claudeProfiles")).toBe("claudeProfiles");
    expect(asAppView("bogus")).toBe("codexProfiles");
    expect(asAppView(null)).toBe("codexProfiles");
  });
});

describe("useAppState 启动状态读取", () => {
  beforeEach(() => {
    vi.resetModules();
    mocks.getState.mockReset();
    mocks.stateUpdates.length = 0;
    mocks.errorUpdates.length = 0;
    mocks.stateHookCalls = 0;
  });

  it("预发请求失败后，下一次 refresh 重新请求并恢复状态", async () => {
    const startupError = new Error("startup failed");
    const recoveredState = { settings: { language: "zh-CN" } };
    let rejectStartup: (reason?: unknown) => void = () => undefined;
    const startupRequest = new Promise<never>((_, reject) => {
      rejectStartup = reject;
    });
    mocks.getState.mockReturnValueOnce(startupRequest);

    const { useAppState } = await import("./appShellHooks");
    const appState = useAppState();

    rejectStartup(startupError);
    await appState.refresh();
    expect(mocks.errorUpdates).toEqual([String(startupError)]);
    expect(mocks.getState).toHaveBeenCalledTimes(1);

    mocks.getState.mockResolvedValueOnce(recoveredState);
    await appState.refresh();

    expect(mocks.getState).toHaveBeenCalledTimes(2);
    expect(mocks.stateUpdates).toEqual([recoveredState]);
    expect(mocks.errorUpdates[mocks.errorUpdates.length - 1]).toBe("");
  });

  it("成功启动时首个 refresh 只消费预发请求，后续 refresh 才发新请求", async () => {
    const startupState = { settings: { language: "zh-CN" } };
    const refreshedState = { settings: { language: "en-US" } };
    mocks.getState.mockResolvedValueOnce(startupState);

    const { useAppState } = await import("./appShellHooks");
    const appState = useAppState();

    await appState.refresh();
    expect(mocks.getState).toHaveBeenCalledTimes(1);

    mocks.getState.mockResolvedValueOnce(refreshedState);
    await appState.refresh();

    expect(mocks.getState).toHaveBeenCalledTimes(2);
    expect(mocks.stateUpdates).toEqual([startupState, refreshedState]);
  });
});
