// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { AppSelect, selectedRowScrollTop } from "./AppSelect";

const styleSource = readFileSync(new URL("../style.css", import.meta.url), "utf8");
const componentSource = readFileSync(new URL("./AppSelect.tsx", import.meta.url), "utf8");
const i18n = createInstance();
void i18n.init({ lng: "en", resources: { en: { translation: {} } }, initAsync: false });

describe("AppSelect checkbox menu", () => {
  const options = [{ value: "first", label: "First" }, { value: "second", label: "Second" }];

  it("复选模式使用真实复选框和独立选中状态，收起时不进入 Tab 顺序", () => {
    const markup = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(AppSelect<string>, {
      value: null, options, checkedValues: ["second"], placeholder: "More", onChange: () => {},
    })));
    expect(markup).toContain('aria-haspopup="menu"');
    expect(markup).toContain('role="menu"');
    expect(markup.match(/type="checkbox"/g)).toHaveLength(2);
    expect(markup).toContain('aria-checked="false"');
    expect(markup).toContain('aria-checked="true"');
    expect(markup.match(/tabindex="-1"/g)).toHaveLength(2);
    expect(markup).toContain(">More</span>");
  });

  it("原有单选模式保留列表语义和当前选项", () => {
    const markup = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(AppSelect<string>, {
      value: "second", options, placeholder: "Select", onChange: () => {},
    })));
    expect(markup).toContain('aria-haspopup="listbox"');
    expect(markup).toContain('role="listbox"');
    expect(markup).toContain('aria-selected="true"');
    expect(markup).not.toContain('type="checkbox"');
    expect(markup).not.toContain('type="search"');
  });

  it("搜索行直接提供输入框，收起菜单后不进入 Tab 顺序，选项保留列表语义", () => {
    const markup = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(AppSelect<string>, {
      value: "second", options, searchable: true, onChange: () => {},
    })));
    expect(markup).toMatch(/<input[^>]*type="search"[^>]*aria-label="select.search"[^>]*tabindex="-1"/);
    expect(markup).toContain('placeholder="select.search select.availableCount"');
    expect(markup).toContain('role="listbox"');
    expect(markup.match(/role="option"/g)).toHaveLength(2);
    expect(markup.indexOf('aria-label="select.search"')).toBeLessThan(markup.indexOf('class="app-select-options"'));
  });
});

describe("AppSelect styles", () => {
  it("三类控件共用侧边栏的悬停和激活高亮", () => {
    const sharedStateSource = styleSource.slice(
      styleSource.indexOf(".app-selection-state {"),
      styleSource.indexOf(".apple-sidebar-nav-button {"),
    );
    expect(styleSource).toContain("--active-bg: var(--tile-bg);");
    expect(sharedStateSource).toContain("background: var(--hover-bg)");
    expect(sharedStateSource).toContain("background: var(--active-bg)");
    expect(sharedStateSource).toContain("color: var(--accent)");
    expect(sharedStateSource).toContain("transition: background-color var(--motion-fast) ease-out, color var(--motion-fast) ease-out;");
    expect(sharedStateSource).not.toContain("background: var(--selection-bg)");
    expect(sharedStateSource).not.toContain("font-weight: 600");
  });

  it("下拉菜单复用全局浅深主题的卡片内描边", () => {
    const menuSource = styleSource.slice(
      styleSource.indexOf(".app-select-menu {"),
      styleSource.indexOf(".app-select-menu[data-open"),
    );
    expect(menuSource).toContain("border: 0;");
    expect(menuSource).toContain("inset var(--card-edge-shadow)");
    expect(menuSource).toContain("0 8px 24px rgb(0 0 0 / 0.12)");
    expect(menuSource).not.toContain("border: 1px solid var(--panel-border)");
  });

  it("下拉与行内菜单共用全局浮层进场动画", () => {
    // 点击触发的浮层只挂进场：关闭即收，不引入出场缩放，避免与进场观感割裂
    expect(componentSource).toContain('className={`app-select-menu app-popover');
    expect(componentSource).toContain('data-popover-in={open ? "" : undefined}');
    expect(componentSource).not.toContain("data-popover-out");
    const popoverSource = styleSource.slice(
      styleSource.indexOf(".app-popover {"),
      styleSource.indexOf(".app-select-menu {"),
    );
    expect(popoverSource).toContain("transform-origin: var(--app-popover-origin, 0 0);");
    expect(popoverSource).toContain("animation: app-popover-in var(--motion-popover) var(--motion-popover-in) both;");
    // 关闭走基础类原有的短淡出
    const menuSource = styleSource.slice(
      styleSource.indexOf(".app-select-menu {"),
      styleSource.indexOf(".app-select-option {"),
    );
    expect(menuSource).toContain("transition: opacity 150ms ease-out, visibility 180ms;");
  });

  it("搜索菜单锁宽高用 offsetWidth，不受进场缩放影响", () => {
    // getBoundingClientRect 会读到动画首帧的缩放尺寸，把菜单永久锁成 80% 大小
    expect(componentSource).toContain("menu.offsetWidth");
    expect(componentSource).toContain("menu.offsetHeight");
    expect(componentSource).not.toContain("getBoundingClientRect()");
  });
});

describe("AppSelect 交互", () => {
  it("模型数量与可视高度变化时仍对齐完整选项行", () => {
    const rowStep = 34.4;
    expect(selectedRowScrollTop(2 * rowStep, 32, 272, rowStep)).toBe(0);
    expect(selectedRowScrollTop(30 * rowStep, 32, 272, rowStep)).toBeCloseTo(27 * rowStep);
    expect(selectedRowScrollTop(30 * rowStep, 32, 150, rowStep)).toBeCloseTo(29 * rowStep);
  });

  it("展开菜单时定位到当前选中项，而不是停留在列表顶部", () => {
    // 契约：打开时按 data-selected 找到选中项并滚动菜单，使其进入可视区
    expect(componentSource).toContain(`querySelector<HTMLButtonElement>('[data-selected="true"]')`);
    expect(componentSource).toContain("menu.scrollTop");
  });

  it("收起逻辑统一走 useMenuDismiss，组件内不再自带外点/滚动监听", () => {
    expect(componentSource).toContain("useMenuDismiss(open, rootRef, menuRef");
    expect(componentSource).not.toContain("closeOnOutsidePointer");
    expect(componentSource).not.toContain('addEventListener("scroll"');
  });

  it("支持模型列表的箭头触发器与独立菜单宽度", () => {
    expect(componentSource).toContain("iconOnly");
    expect(componentSource).toContain("menuWidth");
    expect(componentSource).toContain("menuAlign");
    expect(componentSource).toContain("compact");
    expect(componentSource).toContain("app-select-menu--arrow");
    expect(componentSource).toContain('iconOnly ? "end" : "match"');
  });
});
