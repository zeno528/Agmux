import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../api";
import type { AppState } from "../types";
import { switchProfileFromTray } from "./traySwitch";

vi.mock("../api", () => ({
  api: { codexApplyProfile: vi.fn(), restartCodex: vi.fn() },
}));

beforeEach(() => vi.clearAllMocks());

it("applies, refreshes, then follows the auto restart setting", async () => {
  const calls: string[] = [];
  vi.mocked(api.codexApplyProfile).mockImplementation(async () => { calls.push("apply"); });
  vi.mocked(api.restartCodex).mockImplementation(async () => { calls.push("restart"); });
  const refresh = async () => { calls.push("refresh"); };
  const state = { settings: { auto_restart: true }, codex: { running: true } } as AppState;

  await switchProfileFromTray("provider-1", state, refresh);
  expect(calls).toEqual(["apply", "refresh", "restart", "refresh"]);

  calls.length = 0;
  state.settings.auto_restart = false;
  await switchProfileFromTray("provider-2", state, refresh);
  expect(calls).toEqual(["apply", "refresh"]);

  calls.length = 0;
  state.settings.auto_restart = true;
  state.codex.running = false;
  await switchProfileFromTray("provider-3", state, refresh);
  expect(calls).toEqual(["apply", "refresh", "restart", "refresh"]);
});
