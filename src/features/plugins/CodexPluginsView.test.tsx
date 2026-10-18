import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { api } from "../../api";
import { webInvoke } from "../../api/web-mock";
import { loadPlugins } from "../../app/managementDataCache";
import { setupI18n } from "../../i18n";
import type { AppState } from "../../types";
import CodexPluginsView from "./CodexPluginsView";

const feedback = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => feedback }));

vi.mock("../../app/managementDataCache", async (original) => ({
  ...await original<typeof import("../../app/managementDataCache")>(),
  getCachedPluginMarketplaces: () => [{ name: "fixture", root: "/fixture", kind: "third-party" }],
  loadPlugins: vi.fn().mockResolvedValue([]),
  refreshMarketplacePlugins: vi.fn().mockResolvedValue([]),
}));

it("刷新市场成功后重新读取已安装插件，失败时不报告成功", async () => {
  setupI18n("zh-CN");
  const state = await webInvoke<AppState>("get_state");
  const refresh = vi.spyOn(api, "refreshPluginMarketplace").mockResolvedValue(undefined);
  let click!: () => void;
  function Capture() {
    const tree = CodexPluginsView({ state, popEpoch: 0, query: "", onHeaderChange: () => undefined });
    function visit(node: ReactNode) {
      Children.forEach(node, (child) => {
        if (!isValidElement(child)) return;
        const element = child as ReactElement<{ children?: ReactNode; "aria-label"?: string; onClick?: () => void }>;
        if (element.props["aria-label"] === "刷新插件市场 fixture") click = element.props.onClick!;
        visit(element.props.children);
      });
    }
    visit(tree);
    return null;
  }
  try {
    renderToStaticMarkup(<Capture />);
    click();
    await vi.waitFor(() => expect(loadPlugins).toHaveBeenCalledExactlyOnceWith(true));
    expect(feedback.success).toHaveBeenCalledOnce();
    feedback.success.mockClear();
    vi.mocked(loadPlugins).mockClear();
    refresh.mockRejectedValueOnce(new Error("refresh failed"));
    click();
    await vi.waitFor(() => expect(feedback.error).toHaveBeenCalledWith("Error: refresh failed"));
    expect(loadPlugins).not.toHaveBeenCalled();
    expect(feedback.success).not.toHaveBeenCalled();
  } finally {
    refresh.mockRestore();
  }
});
