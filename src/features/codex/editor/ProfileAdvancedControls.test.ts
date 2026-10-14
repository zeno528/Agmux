// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./ProfileAdvancedControls.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

describe("长上下文组阈值药丸高亮契约", () => {
  it("开关开启的唯一高亮是压缩上限药丸描边（数值保持正常文字色），标签文字不分块转 accent，与 Claude 侧自动压缩组同契约", () => {
    expect(source).toContain('<label className="editor-ghost" title={t("edit.longContextTitle")}>');
    expect(source).toContain("compact-token-input ${advanced.longContextEnabled ? \"compact-token-input--on\" : \"\"} h-6 text-center");
  });
});
