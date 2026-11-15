// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const editSource = readFileSync(new URL("./ClaudeProfileEdit.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const viewSource = readFileSync(new URL("./ClaudeProfilesView.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

describe("Claude 新建配置隔离", () => {
  it("详情读取失败保留列表，已有配置缺少详情时禁止保存", () => {
    const openEdit = viewSource.slice(viewSource.indexOf("const openEdit ="), viewSource.indexOf("const closeEdit ="));
    expect(openEdit).toMatch(/try \{\s*const detail = await api.claudeGetProfile\(profile.id\);\s*setEditDetail\(detail\);\s*setEditingProfile\(profile\);\s*\} catch \(error\) \{\s*feedback.error/);
    expect(editSource).toContain("(!create && !initialDetail)");
  });
  it("新建默认隐藏 AI 署名，不读取当前 live 配置", () => {
    expect(editSource).toMatch(/const initialEnvText = useMemo\(\s*\(\) => create \? patchEnvFields\(patchGitAttribution\("\{\}", true\)/);
    expect(editSource).not.toContain("initialSettingsText");
    expect(viewSource).not.toContain("claudeLiveSettings");
  });

  it("一键设置批量填充模型并保留各行的 1M 声明", () => {
    expect(editSource).toContain("onApplyModel={applyModelToMappings}");
    expect(editSource).toContain("setOneMillionModelSuffix(selected, hasOneMillionModelSuffix(current[key]))");
  });

  it("官网地址沿用供应商页的全局文案和行内按钮", () => {
    expect(editSource).toContain('tProfiles("edit.adminUrlLabel")');
    expect(editSource).toContain('tProfiles("card.openWebsite")');
    expect(editSource).toContain('className="apple-inline-btn apple-inline-btn--quiet !h-5 shrink-0"');
  });

  it("高级控制使用 Claude Code 官方 env 字段并复用编辑器控制样式", () => {
    expect(editSource).toContain("CLAUDE_CODE_AUTO_COMPACT_WINDOW");
    expect(editSource).not.toContain("CLAUDE_CODE_ATTRIBUTION_HEADER");
    expect(editSource).toContain("patchGitAttribution");
    expect(editSource).toContain("gitAttributionLabel");
    expect(editSource).toContain("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS");
    expect(editSource).toContain("CLAUDE_CODE_EFFORT_LEVEL");
    expect(editSource).toContain("editor-ghost-group");
  });

  it("压缩窗口草稿越界阻断保存，未提交草稿随保存落盘", () => {
    expect(editSource).toContain("|| (Number(compactWindowDraft) >= 100000 && Number(compactWindowDraft) <= 1000000)");
    expect(editSource).toContain("if (showCompactWindow && !compactWindowValid) {");
    expect(editSource).toContain('feedback.warning(t("advanced.compactWindowRange"));');
    expect(editSource).toContain('patchEnvValue(envText, "CLAUDE_CODE_AUTO_COMPACT_WINDOW", compactWindowDraft, "autoCompactWindow")');
    expect(editSource).toContain("onBlur={commitCompactWindow}");
  });
});
