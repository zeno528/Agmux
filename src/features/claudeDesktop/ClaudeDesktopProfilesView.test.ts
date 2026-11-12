import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { setupI18n } from "../../i18n";
import { FeedbackProvider } from "../../app/Feedback";
import { setClaudeDesktopProfilesCache } from "../../app/managementDataCache";
import ClaudeDesktopProfilesView, { selectedAggregateMembers } from "./ClaudeDesktopProfilesView";
import type { ClaudeDesktopAppStatus, ClaudeDesktopProfileSummary } from "../../types";

// 列表视图的拖拽预览与 Toast 都走 createPortal(document.body)；静态渲染不支持
// portal，本用例只断言页面文字与按钮门槛，portal 内容与测试无关。
vi.mock("react-dom", async (original) => {
  const actual = await original<typeof import("react-dom")>();
  return { ...actual, createPortal: () => null };
});

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

describe("aggregate banner", () => {
  // mock 掉 createPortal 后 document.body 仍会作为参数求值，给个桩
  (globalThis as { document?: unknown }).document = { body: { nodeType: 1 } };
  const summary = (id: string, name: string): ClaudeDesktopProfileSummary => ({
    id, name, base_url: "https://fixture.example", has_key: true, model: null, requires_routing: false,
    kind: null, admin_url: null, icon: null, show_balance: false, updated_at: "0",
  });
  const status = (overrides: Partial<ClaudeDesktopAppStatus>): ClaudeDesktopAppStatus => ({
    installed: true, running: false, display_path: "", source: "auto", deployment_mode: "3p",
    mode: "aggregate", members: [], selected_members: null, gateway_running: true, ...overrides,
  });
  const render = (activeId: string | null, state: ClaudeDesktopAppStatus) => renderToStaticMarkup(createElement(FeedbackProvider, null,
    createElement(ClaudeDesktopProfilesView, {
      activeId, status: state, onChanged: async () => undefined, activationEpoch: 0, coldStart: true,
      balanceCache: {}, popEpoch: 0, syncFailure: undefined,
    }),
  ));
  const applyButton = (html: string) => html.match(/<button[^>]*aria-pressed="(?:true|false)"[^>]*>[\s\S]*?<\/button>/)?.[0] ?? "";

  it("已应用时按钮禁用只显示计数，待应用或缺默认时给出对应提示", () => {
    setupI18n("zh-CN");
    setClaudeDesktopProfilesCache([summary("p1", "First"), summary("p2", "Second")]);

    // 已应用的干净态：默认徽章在激活卡上，应用按钮禁用，无待应用提示
    const applied = render("p1", status({ members: ["p1", "p2"], selected_members: ["p1", "p2"] }));
    expect(applied).toContain("默认供应商");
    expect(applied).toContain("已选 2 家");
    expect(applied).not.toContain("有更改未应用");
    expect(applyButton(applied)).toContain('disabled=""');
    expect(applied.indexOf("apple-group flex items-center justify-between gap-4 px-5 py-3.5")).toBeLessThan(applied.indexOf('data-profile-id="p1"'));
    expect(applied).toContain("brand-gradient-surface");
    expect(applyButton(applied)).toContain('aria-pressed="true"');

    // 成员选择比已应用的多一家：提示待应用，按钮可点
    const pending = render("p1", status({ members: ["p1"], selected_members: ["p1", "p2"] }));
    expect(pending).toContain("有更改未应用");
    expect(applyButton(pending)).not.toContain('disabled=""');

    // 没有默认供应商：按钮禁用并提示先选默认
    const empty = render(null, status({ members: [], selected_members: [], gateway_running: false }));
    expect(empty).toContain("请选择默认供应商");
    expect(applyButton(empty)).toContain('disabled=""');
  });
});
