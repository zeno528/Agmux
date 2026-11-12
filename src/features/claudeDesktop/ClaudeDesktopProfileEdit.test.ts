import { describe, expect, it } from "vitest";
import { slotsFromModels } from "./ClaudeDesktopProfileEdit";
import type { ClaudeDesktopModelSlot } from "../../types";

const slot = (partial: Partial<ClaudeDesktopModelSlot>): ClaudeDesktopModelSlot => ({
  slot: "sonnet",
  name: "claude-sonnet-5-5",
  label_override: null,
  supports_1m: false,
  ...partial,
});

describe("slotsFromModels", () => {
  it("按槽位名落位，未知槽位忽略，缺槽留空", () => {
    const slots = slotsFromModels([
      slot({ slot: "opus", name: "claude-opus-5-5", label_override: "DeepSeek V4 Pro", supports_1m: true }),
      slot({ slot: "unknown", name: "claude-x" }),
    ]);
    expect(slots.opus).toEqual({ name: "claude-opus-5-5", label: "DeepSeek V4 Pro", supports1m: true });
    expect(slots.sonnet).toEqual({ name: "", label: "", supports1m: false });
    expect(slots.haiku).toEqual({ name: "", label: "", supports1m: false });
  });

  it("新建时四个角色的请求模型和显示名全部为空，不隐式启用 1M", () => {
    const slots = slotsFromModels([]);
    for (const value of Object.values(slots)) {
      expect(value).toEqual({ name: "", label: "", supports1m: false });
    }
  });
});
