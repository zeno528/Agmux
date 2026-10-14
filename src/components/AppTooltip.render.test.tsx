import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { AppTooltip } from "./AppTooltip";

describe("AppTooltip 服务端渲染", () => {
  it("关闭时不渲染面板，开启路径下不残留 visibility:hidden 的空壳", () => {
    // 回归：position 被误清成 null 时，面板会一直带 visibility:hidden 挂载，
    // 表现为"悬停完全没反应"。这里锁住关闭态必须根本不渲染面板。
    const closed = renderToStaticMarkup(createElement(AppTooltip, { label: "帮助", children: createElement("p", null, "内容") }));
    expect(closed).not.toContain('role="tooltip"');
    // 触发按钮仍在，说明组件本身挂载正常
    expect(closed).toContain("aria-label=\"帮助\"");
  });
});
