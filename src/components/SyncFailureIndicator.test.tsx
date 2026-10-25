import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { setupI18n } from "../i18n";
import type { SyncFailure } from "../types";
import { SyncFailureIndicator } from "./SyncFailureIndicator";

// 展开既有悬浮卡的内容以检查原因文案；浮层定位与交互沿用 AppTooltip 的测试。
vi.mock("./AppTooltip", () => ({
  AppTooltip: ({ label, icon, children }: { label: string; icon: ReactNode; children: ReactNode }) => <div aria-label={label}>{icon}{children}</div>,
}));

beforeEach(() => setupI18n("zh-CN"));

describe("回写故障提示", () => {
  it("正常时没有感叹号", () => {
    expect(renderToStaticMarkup(<SyncFailureIndicator failure={undefined} />)).toBe("");
  });

  it.each([
    ["codex", "~/.codex/config.toml", "TOML"],
    ["claude", "~/.claude/settings.json", "JSON"],
  ] as const)("%s 用三段说明状态、原因和处理方式", (client, file, format) => {
    const html = renderToStaticMarkup(<SyncFailureIndicator failure={{ client, reason: "live_parse_error", profile: "Provider", position: [49, 1] }} />);
    expect(html).toContain("lucide-circle-alert");
    expect(html).toContain("配置回写已暂停");
    expect(html).toContain(file);
    expect(html).not.toContain("Provider");
    expect(html).toContain(`${format} 格式错误`);
    expect(html).toContain("数据库配置已保留");
    expect(html).toContain("修复文件或切换供应商");
    expect(html).toContain("第 49 行，第 1 列");
    expect(html.match(/<p\b/g)).toHaveLength(3);
  });

  it.each([
    ["live_unreadable", "无法读取 ~/.claude/settings.json"],
    ["write_failed", "无法将实时配置写入数据库"],
    ["internal", "读取或保存数据库配置时发生错误"],
  ] as const)("区分 %s", (reason: SyncFailure["reason"], message) => {
    const html = renderToStaticMarkup(<SyncFailureIndicator failure={{ client: "claude", reason, profile: null, position: null }} />);
    expect(html).toContain(message);
    expect(html).not.toContain("供应商：");
    expect(html).not.toContain("权限规则及其他设置不会生效");
    expect(html).not.toContain("第 ");
  });
});
