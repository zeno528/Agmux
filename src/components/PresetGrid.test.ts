// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./PresetGrid.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../style.css", import.meta.url), "utf8");
// 类名断言只针对代码：先剥掉注释，免得注释里提到旧类名时误报
const code = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");

describe("预设选择网格", () => {
  it("默认项只使用全局卡片细描边，选中项使用强调色", () => {
    expect(code).toContain('"shadow-[0_0_0_1px_var(--accent)] bg-(--active-bg)"');
    expect(code).toContain('"shadow-(--card-edge-shadow) hover:bg-(--hover-bg)"');
  });

  it("外层不再套卡片框，层次由供应商项自身提供", () => {
    // apple-edit-surface 刻意不带底色/圆角/裁切；这里再多包一层带描边的容器
    // 只会把末行的空当圈起来，并多出一级包装深度
    expect(code).toContain('<div className="apple-panel-section">');
    expect(code).not.toContain("border-[var(--panel-border)]");
    expect(code).not.toMatch(/className="rounded-[^"]*border/);
  });

  it("列数由全局样式统一定义：普通窗口两列，视口超过 96rem 四列", () => {
    expect(code).toContain("apple-tile-grid apple-tile-grid--labeled");
    expect(code).not.toContain("grid-cols-");
    expect(styles).toMatch(/\.apple-tile-grid--providers\s*\{\s*grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)/);
    expect(styles).toMatch(/@media \(min-width: 96rem\)\s*\{\s*\.apple-tile-grid--providers\s*\{\s*grid-template-columns: repeat\(4, minmax\(0, 1fr\)\)/);
  });

  it("供应商按名字挑选，左侧图标和名称，右侧提示多个版本", () => {
    expect(code).toContain("<ProfileIconTile");
    expect(code).toContain('className="min-w-0 flex-1"');
    expect(code).toContain("apple-tile-grid--providers");
    expect(code).toContain("presetEntries(presets)");
  });
});
