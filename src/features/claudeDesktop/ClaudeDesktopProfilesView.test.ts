import { describe, expect, it } from "vitest";
import { selectedAggregateMembers } from "./ClaudeDesktopProfilesView";

describe("remembered aggregate selection", () => {
  it("preserves an explicitly empty list instead of turning the active provider back on", () => {
    expect(selectedAggregateMembers({ members: ["a", "b"], selected_members: [] }, "a")).toEqual([]);
    expect(selectedAggregateMembers({ members: ["a"], selected_members: ["b"] }, "a")).toEqual(["b"]);
  });

  it("uses the previously applied selection when upgrading older saved state", () => {
    expect(selectedAggregateMembers({ members: ["a", "b", "b"], selected_members: null }, "a")).toEqual(["a", "b"]);
    expect(selectedAggregateMembers({ members: [], selected_members: null }, null)).toEqual([]);
  });
});
