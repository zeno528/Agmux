// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const editSource = readFileSync(new URL("./ClaudeProfileEdit.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const viewSource = readFileSync(new URL("./ClaudeProfilesView.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const templateDialogSource = readFileSync(new URL("./ClaudeCommonTemplateDialog.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

describe("通用模板弹窗编辑器工具条", () => {
  it("格式化与清空都进编辑器附属条而非底部动作区，复用编辑页同一按钮与 i18n", () => {
    expect(templateDialogSource).toContain('className="editor-attach-group"');
    expect(templateDialogSource).toContain('className="editor-attach-bar"');
    expect(templateDialogSource).toContain("editor-ghost editor-ghost--format");
    expect(templateDialogSource).toContain("editor-ghost editor-ghost--danger");
    // 两个按钮都在附属条里（源码顺序：附属条 → 清空 → 格式化 → 底部动作区）
    expect(templateDialogSource.indexOf("clearFile")).toBeGreaterThan(templateDialogSource.indexOf("editor-attach-bar"));
    expect(templateDialogSource.indexOf("editor-ghost--format")).toBeGreaterThan(templateDialogSource.indexOf("clearFile"));
    // 排版逻辑已提进共用的 formatJsonText，弹窗直接复用（行为由 profileEnvText.test.ts 覆盖）
    expect(templateDialogSource).toContain('import { extractClaudeCommonSettings, formatJsonText } from "./profileEnvText";');
    expect(templateDialogSource).toContain("formatJsonText(text)");
  });
});

describe("Claude 新建配置隔离", () => {
  it("名称点击复用 Codex 命名弹窗和重命名接口，卡片仍进入完整编辑", () => {
    expect(viewSource).toContain('import ProfileNameDialog from "../profiles/ProfileNameDialog";');
    expect(viewSource).toContain("onRename={onRename}");
    expect(viewSource).not.toContain("onRename={onEdit}");
    expect(viewSource).toContain("onRename={() => openRename(profile)}");
    expect(viewSource).toContain("onEdit={() => void openEdit(profile)}");
    expect(viewSource).toContain('api.renameProfile(modalProfile.id, profileName.trim(), "claude")');
  });

  it("详情读取失败保留列表，已有配置缺少详情时禁止保存", () => {
    const openEdit = viewSource.slice(viewSource.indexOf("const openEdit ="), viewSource.indexOf("const closeEdit ="));
    expect(openEdit).toMatch(/try \{\s*const detail = await api.claudeGetProfile\(profile.id\);\s*setEditDetail\(detail\);\s*setEditingProfile\(profile\);\s*\} catch \(error\) \{\s*feedback.error/);
    expect(editSource).toContain("(!create && !initialDetail)");
  });
  it("新建从空 settings 开始，不读取当前 live 配置", () => {
    expect(editSource).toMatch(/const initialEnvText = useMemo\(\s*\(\) => create \? patchEnvFields\("\{\}"/);
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
    expect(editSource).toContain("DISABLE_AUTO_COMPACT");
    expect(editSource).toContain("CLAUDE_CODE_AUTO_COMPACT_WINDOW");
    expect(editSource).not.toContain("CLAUDE_CODE_ATTRIBUTION_HEADER");
    expect(editSource).toContain("patchGitAttribution");
    expect(editSource).toContain("gitAttributionLabel");
    expect(editSource).toContain("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS");
    expect(editSource).toContain("CLAUDE_CODE_EFFORT_LEVEL");
    expect(editSource).toContain("editor-ghost-group");
  });

  it("自动压缩开启的唯一高亮是阈值药丸描边（数值保持正常文字色），标签文字不分块转 accent", () => {
    expect(editSource).toContain('<label className="editor-ghost" title={t("advanced.autoCompactTitle")}>');
    expect(editSource).toContain("compact-token-input ${autoCompactDisabled ? \"\" : \"compact-token-input--on\"} h-6 text-center");
  });
});
