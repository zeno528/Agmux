// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const componentSource = readFileSync(new URL("./DiagnosticsChip.tsx", import.meta.url), "utf8");
const profileEditSource = readFileSync(new URL("../features/codex/CodexProfileEdit.tsx", import.meta.url), "utf8");
const claudeMcpEditSource = readFileSync(new URL("../features/mcp/ClaudeMcpEdit.tsx", import.meta.url), "utf8");

describe("DiagnosticsChip 共享诊断 chip", () => {
  it("两个编辑页都改用共享组件，不再各留一份内联 chip", () => {
    expect(profileEditSource).toContain("<DiagnosticsChip");
    expect(claudeMcpEditSource).toContain("<DiagnosticsChip");
    for (const source of [profileEditSource, claudeMcpEditSource]) {
      // 内联 chip 的原始渲染要件不得散落在编辑页里（回退即红）
      expect(source).not.toContain("bg-(--danger)/10 px-2.5");
      expect(source).not.toContain("chip-danger");
      expect(source).not.toContain('aria-live="polite"');
      expect(source).not.toContain("edit.diagnosticsErrors");
      expect(source).not.toContain("edit.diagnosticsLine");
    }
  });

  it("组件自持渲染要件并经 profiles ns 解析两条文案", () => {
    expect(componentSource).toContain("useTranslation(\"profiles\")");
    expect(componentSource).toContain("edit.diagnosticsErrors");
    expect(componentSource).toContain("edit.diagnosticsLine");
    expect(componentSource).toContain("chip-danger");
    expect(componentSource).toContain('aria-live="polite"');
    expect(componentSource).toContain("className=\"h-1.5 w-1.5 rounded-full bg-(--danger)\"");
    // count > 0 才渲染
    expect(componentSource).toContain("diagnostics.count <= 0");
  });
});
