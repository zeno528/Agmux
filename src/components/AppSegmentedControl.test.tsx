import { Children } from "react";
import { renderToStaticMarkup } from "react-dom/server";
// @ts-expect-error 测试运行于 Node，浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { AppSegmentedControl } from "./AppSegmentedControl";

it("共享圆角分段控件随项数定位浮起的滑块，保持颜色过渡", () => {
  for (const count of [2, 3]) {
    const children = Array.from({ length: count }, (_, index) => (
      <button key={index} type="button" aria-pressed={index === count - 1}>{index}</button>
    ));
    const control = AppSegmentedControl({ selectedIndex: count - 1, label: "Options", children });
    expect(Children.toArray(control.props.children)).toHaveLength(count);
    const html = renderToStaticMarkup(control);
    expect(html).toContain(`--segment-count:${count};--segment-index:${count - 1}`);
    expect(html).toContain('role="group" aria-label="Options"');
    expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
    expect(html).not.toContain("apple-group");
  }
  const styles = readFileSync(new URL("../style.css", import.meta.url), "utf8");
  const start = styles.indexOf(".app-segmented-control {");
  const sharedStyles = styles.slice(start, styles.indexOf(".apple-action-button {", start));
  expect(styles).toContain(".apple-group,\n.app-segmented-control,\n.apple-list-row,\n.apple-editor-surface {\n  box-shadow: var(--card-edge-shadow);\n}");
  expect(sharedStyles).toContain("padding: 3px;");
  expect(sharedStyles).toContain("inset: 3px auto 3px 3px;");
  expect(sharedStyles).toContain("width: calc((100% - 6px) / var(--segment-count));");
  expect(sharedStyles).toContain("border-radius: 0.75rem;");
  expect(sharedStyles).toContain("border-radius: 0.625rem;");
  expect(sharedStyles).toContain("box-shadow: var(--shadow-sm);");
  expect(sharedStyles).toContain("transform: translateX(calc(var(--segment-index) * 100%));");
  expect(styles).toContain("--motion-segment-slide: 340ms cubic-bezier(0.22, 1, 0.36, 1);");
  expect(sharedStyles).toContain("transition: transform var(--motion-segment-slide), background-color 210ms ease;");
  expect(styles).toContain("transition: transform var(--motion-segment-slide), width var(--motion-segment-slide);");
  expect(sharedStyles).toContain("transition: color 210ms ease;");
  expect(sharedStyles).toContain("background: var(--secondary-button-bg);");
  expect(sharedStyles).toContain("background: var(--main-surface-bg);");
  expect(sharedStyles).toContain('button:is([aria-pressed="true"], [aria-selected="true"]) { color: var(--text-primary); }');
  expect(sharedStyles).not.toMatch(/font-size|font-weight|font-family/);
  expect(sharedStyles).not.toMatch(/background-image|gradient|filter|will-change/);
  expect(styles).toContain(".theme-switching *:not(.app-segmented-control, .app-segmented-control *),");
  expect(styles).toContain(".app-segmented-control::before,\n  .app-segmented-control > button {\n    transition: none;\n  }");
  expect(styles).not.toMatch(/\.apple-segmented-control|\.theme-segmented-control/);
});
