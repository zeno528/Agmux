import { describe, expect, it } from "vitest";
import { createInstance } from "i18next";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { FeedbackProvider } from "../../app/Feedback";
import ProfileTerminalMenu, { dirName, nextDirs } from "./ProfileTerminalMenu";

function renderMenu() {
  const i18n = createInstance();
  void i18n.init({ lng: "en", resources: { en: { profiles: {} } }, initAsync: false });
  return renderToStaticMarkup(createElement(
    FeedbackProvider,
    null,
    createElement(I18nextProvider, { i18n }, createElement(ProfileTerminalMenu, { profileId: "p1" })),
  ));
}

describe("最近目录列表", () => {
  it("新目录排最前，重复使用只前移不重复", () => {
    expect(nextDirs(["a", "b"], "c")).toEqual(["c", "a", "b"]);
    expect(nextDirs(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
  });

  it("超出上限时挤掉最旧的", () => {
    const full = Array.from({ length: 8 }, (_, index) => `d${index}`);
    const next = nextDirs(full, "new");
    expect(next).toHaveLength(8);
    expect(next[0]).toBe("new");
    expect(next).not.toContain("d7");
  });

  it("列表项只显示末级目录名，两种分隔符都认", () => {
    expect(dirName("D:\\work\\cg-switch")).toBe("cg-switch");
    expect(dirName("/Users/me/work/cg-switch")).toBe("cg-switch");
    expect(dirName("D:\\work\\")).toBe("work");
  });
});

it("触发器沿用卡片既有图标按钮与 18px 尺寸，未展开时不渲染菜单内容", () => {
  const html = renderMenu();
  expect(html).toContain('aria-haspopup="menu"');
  expect(html).toContain('aria-expanded="false"');
  expect(html).toContain("h-[18px] w-[18px]");
  expect(html).toContain("terminal.open");
  // 列表只在展开时读，未展开不出现空态文案，避免冷启动多余 DOM
  expect(html).not.toContain("terminal.empty");
});
