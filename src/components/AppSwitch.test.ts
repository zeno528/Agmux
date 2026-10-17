// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const styleSource = readFileSync(new URL("../style.css", import.meta.url), "utf8");

describe("AppSwitch styles", () => {
  it("开启态统一用项目蓝，两套主题同色", () => {
    expect(styleSource).toContain('.app-switch[data-state="checked"] .app-switch-rail { background-color: var(--accent); }');
    // 滑块保持基色白，不再翻成主题按钮文字色（深色下会变黑）
    expect(styleSource).not.toContain("background: var(--primary-button-text);");
  });
});
