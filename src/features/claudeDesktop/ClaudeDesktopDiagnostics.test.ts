import { describe, expect, it } from "vitest";
import type { ClaudeDesktopModelUsage } from "../../types";
import { summarizeModelUsage } from "./ClaudeDesktopDiagnostics";

const model = (input: number | null, output: number | null): ClaudeDesktopModelUsage => ({
  route: { desktop_model: "claude-sonnet-test", upstream_model: "actual-model", label: "Model", host: "example.com" },
  requests: 3, completed: 1, failed: 1, cancelled: 1, usage_reported: input !== null && output !== null ? 1 : 0,
  tokens: { input_tokens: input, output_tokens: output, cache_read_input_tokens: null, cache_creation_input_tokens: null },
});

describe("diagnostic session totals", () => {
  it("keeps unknown usage distinct from a reported zero", () => {
    expect(summarizeModelUsage([])).toMatchObject({ requests: 0, input: null, output: null });
    expect(summarizeModelUsage([model(null, null)])).toMatchObject({ input: null, output: null, reported: 0 });
    expect(summarizeModelUsage([model(0, 0)])).toMatchObject({ input: 0, output: 0, reported: 1 });
  });

  it("sums only reported usage and reconciles request outcomes", () => {
    const totals = summarizeModelUsage([model(12, 5), model(null, 7), model(8, null)]);
    expect(totals).toEqual({ requests: 9, completed: 3, failed: 3, cancelled: 3, reported: 1, input: 20, output: 12 });
    expect(totals.requests - totals.completed - totals.failed - totals.cancelled).toBe(0);
  });
});
