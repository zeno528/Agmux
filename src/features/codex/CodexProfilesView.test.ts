// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { codexActionFor } from "./CodexProfilesView";

const source = readFileSync(new URL("./CodexProfilesView.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const cardSource = readFileSync(new URL("../profiles/ProfileCard.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");

describe("ProfilesView 拖拽预览", () => {
  it("将状态与操作合并，且只让右侧操作区触发启动或重启", () => {
    expect(codexActionFor(true)).toBe("restart");
    expect(codexActionFor(false)).toBe("start");
    expect(source).toContain('className="codex-status__action"');
    expect(source).toContain('onClick={() => void restart(false)}');
    expect(source).toContain('role="status"');
  });

  it("codex 按钮动作语义分离：重启全程 RefreshCw 自旋，启动中用项目旋转指示器，禁止混用", () => {
    expect(source).toContain('{codexAction === "start" ? <LoadingSpinner size="md" /> : (codexAction ?? nextCodexAction) === "restart" ? <RefreshCw className={`h-4 w-4 ${codexAction ? "animate-spin" : ""}`} strokeWidth={2} /> : <Play className="h-4 w-4" strokeWidth={2} />}');
    expect(source).not.toContain("{codexAction ? <LoadingSpinner");
  });

  it("成功通知区分启动与重启，不写死「已重启」", () => {
    expect(source).toContain('t(action === "restart" ? "feedback.codexRestarted" : "feedback.codexStarted")');
  });

  it("供应商页头展示 Codex 标识，更新入口移到全局侧栏", () => {
    expect(source).toContain('<div className="provider-page-brand">');
    expect(source).toContain('<img src="/codex.svg"');
    expect(source).toContain('apple-page-bar flex-wrap justify-between gap-4');
  });

  it("切换期间添加供应商按钮保留禁用语义但不闪烁", () => {
    expect(source).toContain('className="apple-action-button app-button--primary disabled:!opacity-100" disabled={busy}');
  });

  it("进入编辑页先预载详情再切换，首帧不空窗", () => {
    expect(source).toContain("const openEdit = async (profile: CodexProfileSummary) => {");
    expect(source).toContain("detail = await api.codexGetProfile(profile.id);");
    expect(source).toContain("onEdit={() => void openEdit(profile)}");
    expect(source).toContain("initialDetail={editDetail}");
  });

  it("将拖拽浮层挂到 body，避免被页面 transform 容器偏移", () => {
    expect(source).toContain('import { createPortal } from "react-dom";');
    expect(source).toContain("createPortal(<DragOverlay");
    expect(source).toContain("document.body");
  });

  it("源卡片在拖拽收尾时立即恢复显隐，避免释放瞬间渐隐", () => {
    expect(cardSource).not.toContain("group flex cursor-pointer select-none flex-col gap-4 px-5 py-4.5 transition-opacity duration-150 sm:flex-row");
  });

  it("排序保存成功后同步父级状态，切页回来仍保留新顺序", () => {
    expect(source).toContain("await api.codexReorderProfiles(next.map((item) => item.id));\n      await onRefresh();");
  });

  it("切换后先更新激活高亮，再按需重启并合并成功通知", () => {
    const applyStart = source.indexOf("const codexApplyProfile = async");
    const applyEnd = source.indexOf("const removeProfile = async", applyStart);
    const applySource = source.slice(applyStart, applyEnd);
    const refreshIndex = applySource.indexOf("await onRefresh();");
    const restartIndex = applySource.indexOf("await restart(true, false)");

    expect(refreshIndex).toBeGreaterThan(-1);
    expect(restartIndex).toBeGreaterThan(refreshIndex);
    expect(applySource).toContain('feedback.success(t("feedback.applySuccess", { ns: "common" }))');
    expect(applySource.match(/feedback.success/g)).toHaveLength(1);
  });

  it("激活卡的拖拽预览复用品牌渐变且不再覆盖旧底色", () => {
    expect(styles).not.toContain(".profile-drag-preview.is-active {\n  background-image: linear-gradient(");
    expect(styles).not.toContain("--profile-active-bg:");
    expect(styles).not.toContain(":root.dark .profile-drag-preview.is-active {");
  });

  it("激活卡拖拽预览的官网按钮沿用主题文字层级", () => {
    // 选择器用稳定类名而非中文 title/aria-label：文案会随界面语言变化
    expect(styles).toContain(".profile-list > .apple-group.brand-gradient-surface .profile-card-content__text .apple-icon-button,\n.profile-list > .apple-group.brand-gradient-surface .profile-card-action-buttons > .apple-icon-button:not(.app-button--primary) {\n  color: var(--text-primary);");
  });
});

describe("ProfilesView 编辑器加载策略", () => {
  it("ProfileEdit 静态加载：进编辑页零延迟，不出现懒加载骨架", () => {
    expect(source).toContain('import CodexProfileEdit from "./CodexProfileEdit";');
    expect(source).not.toContain("lazy(");
    expect(source).not.toContain("Suspense");
  });
});
