import { describe, expect, it } from "vitest";
import { claudeInstalledIdSet, claudePluginParts } from "./claudePluginMeta";
import type { ClaudePluginEntry } from "../../types";

describe("claudePluginParts", () => {
  it("按第一个 @ 拆出插件名与市场名", () => {
    expect(claudePluginParts("ponytail@ponytail")).toEqual({ name: "ponytail", marketplace: "ponytail" });
    expect(claudePluginParts("code-review@claude-plugins-official")).toEqual({
      name: "code-review",
      marketplace: "claude-plugins-official",
    });
  });

  it("无市场后缀与空后缀都能安全兜底", () => {
    expect(claudePluginParts("bare")).toEqual({ name: "bare", marketplace: null });
    expect(claudePluginParts("x@")).toEqual({ name: "x", marketplace: null });
  });
});

describe("claudeInstalledIdSet", () => {
  it("跨 scope 收集 id 集合（同插件双行只留一个 id）", () => {
    const installed = [
      { id: "a@m", scope: "user" },
      { id: "a@m", scope: "project" },
      { id: "b@m", scope: "user" },
    ] as ClaudePluginEntry[];
    expect(claudeInstalledIdSet(installed)).toEqual(new Set(["a@m", "b@m"]));
  });
});
