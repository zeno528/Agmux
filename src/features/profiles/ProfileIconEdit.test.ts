// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { setupI18n } from "../../i18n";
import zh from "../../i18n/locales/zh-CN/profiles";
import { providerIcons } from "../../icons";
import ProfileIconEdit from "./ProfileIconEdit";

const source = readFileSync(new URL("./ProfileIconEdit.tsx", import.meta.url), "utf8");
// 类名断言只针对代码：先剥掉注释，免得注释里提到旧类名时误报
const code = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
// providerIcons() 里的中文名是 getter，必须在 setupI18n 之后取，否则拿到的是键名
setupI18n("zh-CN");
const icons = providerIcons();

function render(icon: string | null) {
  return renderToStaticMarkup(createElement(ProfileIconEdit, { icon, onBack: () => {}, onSave: () => {} }));
}

describe("图标选择页", () => {
  it("格子只显示图标，名称交给 aria-label 与 title", () => {
    const markup = render("deepseek");
    expect(icons.length).toBeGreaterThan(6);
    for (const item of icons) {
      expect(markup).toContain(`aria-label="${item.label}"`);
      expect(markup).toContain(`title="${item.label}"`);
    }
    // 名称不再是格内可见文字：写死列数时每个格子的名称行就是那 4 行高度的来源
    for (const item of icons) {
      expect(markup).not.toContain(`>${item.label}<`);
    }
  });

  it("状态并进「不使用图标」那一行，不另起一行", () => {
    const label = icons.find((item) => item.id === "deepseek")?.label ?? "";
    expect(label).not.toBe("");
    const text = zh.icons.selected.replace("{{name}}", label);
    // 选中图标：那一行显示已选名称，全页只出现这一次（没有多出一条状态行）
    const picked = render("deepseek");
    expect(picked).toContain(text);
    expect(picked.split(text)).toHaveLength(2);
    expect(picked).not.toContain(zh.icons.none);
    // 未选中：同一行改回「不使用图标」，并显示为已选
    const cleared = render(null);
    expect(cleared).toContain(zh.icons.none);
    expect(cleared).not.toContain(text);
    // 此时没有任何图标被选中，markup 里唯一的 aria-pressed="true" 就是它
    expect(cleared).toContain('aria-pressed="true"');
  });

  it("底部那一行字重适中：12px 常规太细、600 太重，卡在 medium", () => {
    expect(code).toMatch(/py-2\.5 text-xs font-medium/);
    // 只加一档字重，不动字号：升到 field-label(13px/600) 会过重，降回常规又太细
    expect(code).not.toMatch(/py-2\.5 field-label/);
    expect(code).not.toMatch(/font-semibold|font-bold/);
  });

  it("默认态不带描边，悬停才浮现中性描边，选中才是强调色", () => {
    expect(code).toContain('"shadow-[0_0_0_1px_var(--accent)] bg-(--active-bg)"');
    // 每一处 panel-ring 都必须跟在 hover: 后面——默认态不该有描边
    const rings = code.match(/var\(--panel-ring\)/g) ?? [];
    const hoverRings = code.match(/hover:shadow-\[0_0_0_1px_var\(--panel-ring\)\]/g) ?? [];
    expect(rings).toHaveLength(hoverRings.length);
  });

  it("列数随可用宽度自适应：复用 apple-tile-grid 全局类（机制定义在 style.css）", () => {
    expect(code).toContain('className="apple-tile-grid"');
  });

  it("复用 ProfileIconTile 承载图标", () => {
    expect(code).toContain('import { ProfileIconTile } from "../../components/ProfileIconTile";');
    expect(code).toContain("<ProfileIconTile");
    // 图标地址由 ProfileIconTile 统一负责，页面不再自己拼一遍
    expect(code).not.toContain("item.url");
  });
});
