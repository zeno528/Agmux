import { describe, expect, it } from "vitest";
import { slotsFromModels } from "./ClaudeDesktopProfileEdit";
import { providerPresets } from "../../presets";
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
    expect(slots.opus).toEqual({ name: "claude-opus-5-5", label: "DeepSeek V4 Pro", supports1m: true, upstreamModel: "" });
    expect(slots.sonnet).toEqual({ name: "", label: "", supports1m: false, upstreamModel: "" });
    expect(slots.haiku).toEqual({ name: "", label: "", supports1m: false, upstreamModel: "" });
  });

  it("DeepSeek 预设的三档默认映射覆盖全部槽位且模型名为 claude-* 形态", () => {
    const preset = providerPresets.find((item) => item.kind === "deepseek")!.desktop!;
    const slots = slotsFromModels(
      (preset.default_models ?? []).map((model) => ({
        slot: model.slot,
        name: model.name,
        label_override: model.labelOverride ?? null,
        supports_1m: false,
      })),
    );
    for (const key of ["sonnet", "opus", "haiku", "fable"] as const) {
      expect(slots[key].name, `${key} 槽位必须有默认模型`).toMatch(/^claude-/);
      expect(slots[key].label).toBeTruthy();
    }
  });
});
