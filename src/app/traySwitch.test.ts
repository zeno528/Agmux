import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../api";
import type { AppState } from "../types";
import { switchProfileFromTray } from "./traySwitch";

vi.mock("../api", () => ({
  api: { codexApplyProfile: vi.fn(), claudeApplyProfile: vi.fn(), restartCodex: vi.fn() },
}));

beforeEach(() => vi.clearAllMocks());

it("applies, refreshes, then follows the auto restart setting", async () => {
  const calls: string[] = [];
  vi.mocked(api.codexApplyProfile).mockImplementation(async () => { calls.push("apply"); });
  vi.mocked(api.restartCodex).mockImplementation(async () => { calls.push("restart"); });
  const refresh = async () => { calls.push("refresh"); };
  const state = { settings: { auto_restart: true }, codex: { running: true } } as AppState;

  await switchProfileFromTray("codex", "provider-1", state, refresh);
  expect(calls).toEqual(["apply", "refresh", "restart", "refresh"]);

  calls.length = 0;
  state.settings.auto_restart = false;
  await switchProfileFromTray("codex", "provider-2", state, refresh);
  expect(calls).toEqual(["apply", "refresh"]);

  calls.length = 0;
  state.settings.auto_restart = true;
  state.codex.running = false;
  await switchProfileFromTray("codex", "provider-3", state, refresh);
  expect(calls).toEqual(["apply", "refresh", "restart", "refresh"]);
});

it("applies Claude Code without restarting Codex even when auto restart is enabled", async () => {
  const refresh = vi.fn();
  const state = { settings: { auto_restart: true } } as AppState;
  await switchProfileFromTray("claude", "claude-1", state, refresh);
  expect(api.claudeApplyProfile).toHaveBeenCalledWith("claude-1");
  expect(refresh).toHaveBeenCalledOnce();
  expect(api.codexApplyProfile).not.toHaveBeenCalled();
  expect(api.restartCodex).not.toHaveBeenCalled();
});

it("propagates a failed Claude Code switch without reporting a refresh or restart", async () => {
  vi.mocked(api.claudeApplyProfile).mockRejectedValueOnce(new Error("apply failed"));
  const refresh = vi.fn();
  await expect(switchProfileFromTray("claude", "claude-1", {} as AppState, refresh)).rejects.toThrow("apply failed");
  expect(refresh).not.toHaveBeenCalled();
  expect(api.restartCodex).not.toHaveBeenCalled();
});
