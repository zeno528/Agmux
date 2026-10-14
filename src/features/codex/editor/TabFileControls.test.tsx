import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { FeedbackProvider } from "../../../app/Feedback";
import { setupI18n } from "../../../i18n";
import TabFileControls from "./TabFileControls";

const render = (kind: "models" | "auth", editable: boolean) =>
  renderToStaticMarkup(
    <FeedbackProvider>
      <TabFileControls kind={kind} editable={editable} disabled={false} onClear={() => undefined} />
    </FeedbackProvider>,
  );

describe("TabFileControls", () => {
  it("auth 可编辑：显示可编辑药丸和清空按钮", () => {
    setupI18n("zh-CN");
    const html = render("auth", true);
    expect(html).toContain("可编辑");
    expect(html).toContain("清空");
  });

  it("auth 只读：显示只读药丸且不渲染清空，避免点了没反应", () => {
    setupI18n("zh-CN");
    const html = render("auth", false);
    expect(html).toContain("只读");
    expect(html).not.toContain("清空");
  });

  it("models：只有清空按钮，没有状态药丸", () => {
    setupI18n("zh-CN");
    const html = render("models", true);
    expect(html).toContain("清空");
    expect(html).not.toContain("可编辑");
  });
});
