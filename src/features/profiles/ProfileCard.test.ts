// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createInstance, type TFunction } from "i18next";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { authQuotaErrorKind } from "../../app/authQuotaCache";
import type { CodexProfileSummary } from "../../types";
import { connectionGate, ProfileCardActions } from "./ProfileCard";
import { profileConnectionGate } from "../codex/CodexProfileCard";

const source = readFileSync(new URL("./ProfileCard.tsx", import.meta.url), "utf8");
const hookSource = readFileSync(new URL("./useProfileBalance.ts", import.meta.url), "utf8");
const sortableCardSource = readFileSync(new URL("../../components/SortableCard.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");

it("供应商卡片文字区域固定保留标题和元信息行高度", () => {
  expect(styles).toMatch(/\.profile-card-content__text\s*\{[^}]*height: calc\(1\.75rem \+ 0\.25rem \+ 18px\);/);
});

it("没有第二行时标题居中，有第二行时保留原布局", () => {
  expect(styles).toMatch(/\.profile-card-content__text:not\(:has\(\.profile-card-meta\)\)\s*\{[^}]*justify-content: center;[^}]*transform: none;/);
});

describe("ProfileCard 官网入口", () => {
  it("官网入口悬停居中覆盖在 Logo 容器上，沿用 Globe 图标", () => {
    expect(source).toContain("Globe");
    expect(source).toContain("overlay={");
    expect(source).toContain("profile.admin_url ?");
    expect(source).toContain('title={t("card.openWebsite")}');
    expect(source).toContain("group-hover/tile:opacity-100");
    expect(source).toContain("focus-visible:opacity-100");
    // 标题行的旧官网按钮已移除，不再与「点击名称重命名」抢注意力
    expect(source).not.toContain('className="apple-icon-button !h-6 !w-7 shrink-0 text-accent"');
  });

  it("所有配置激活时复用全局品牌渐变", () => {
    // 卡片外壳（含激活渐变与拖拽手柄）抽到共享 SortableCard，两列表页共用
    expect(sortableCardSource).toContain('active ? " is-active brand-gradient-surface" : ""');
    expect(source).not.toContain('profile.kind === "official" ? " brand-gradient-surface" : ""');
    expect(source).not.toContain("third-party-gradient");
    expect(styles).not.toContain(".profile-list > .apple-group.is-active:not(.brand-gradient-surface)");
    expect(styles).toContain("--brand-gradient-start: #263b63;");
    expect(styles).toContain("--brand-gradient-middle: #3f72b8;");
    expect(styles).toContain("--brand-gradient-start-mix: 42%;");
    expect(styles).toContain("--brand-gradient-middle-mix: 58%;");
    expect(styles).toContain(":root.dark {\n  color-scheme: dark;");
    expect(styles).toContain(".brand-gradient-surface {\n  background-image: linear-gradient(90deg, color-mix(in srgb, var(--brand-gradient-start) var(--brand-gradient-start-mix), transparent) 0%, color-mix(in srgb, var(--brand-gradient-blend) var(--brand-gradient-blend-mix), transparent) 20%, color-mix(in srgb, var(--brand-gradient-middle) var(--brand-gradient-middle-mix), transparent) 40%, color-mix(in srgb, var(--brand-gradient-middle) var(--brand-gradient-fade-mix), transparent) 58%, transparent 100%);");
  });

  it("激活卡沿用主题文字层级", () => {
    expect(styles).not.toContain("--active-card-text-primary");
    expect(styles).not.toContain("--active-card-text-secondary");
    expect(styles).toContain(".profile-list > .apple-group.brand-gradient-surface .profile-card-meta {\n  color: var(--text-secondary);");
    expect(styles).not.toContain(":root.dark .profile-list > .apple-group.brand-gradient-surface .profile-card-meta .apple-chip");
    expect(styles).toContain(".profile-list > .apple-group.brand-gradient-surface .drag-handle {\n  color: var(--text-secondary);");
  });

  it("激活时保留卡片边缘", () => {
    expect(styles).not.toContain(".profile-list > .apple-group.is-active {");
    expect(styles).not.toContain(".profile-list > .apple-group.brand-gradient-surface {");
    expect(styles).toContain(".profile-list > .apple-group:hover,\n.profile-drag-preview {\n  outline: 1px solid");
    expect(styles).not.toContain(":root.dark .profile-list > .apple-group.is-active {");
  });

  it("激活渐变卡沿用默认卡片的圆角和几何", () => {
    expect(styles).toContain(".apple-group {\n  overflow: hidden;\n  border-radius: var(--radius-card);");
  });

  it("提高渐变卡片的文字与图标对比度", () => {
    expect(styles).toContain(".profile-list > .apple-group.brand-gradient-surface .profile-card-content__text {\n  color: var(--text-primary);");
    // 选择器用稳定类名而非中文 title/aria-label：文案会随界面语言变化
    expect(styles).toContain(".profile-list > .apple-group.brand-gradient-surface .profile-card-action-buttons > .apple-icon-button:not(.app-button--primary) {");
  });

  it("胶囊底色统一定义在 --chip-bg，配置卡片浅色药丸复用主容器底色", () => {
    expect(styles).toContain("--chip-bg: #e9e9e6;");
    expect(styles).toContain(".apple-chip {\n  align-items: center;\n  background: var(--chip-bg);");
    expect(styles).toContain(".profile-card-meta .apple-chip {\n  background: var(--main-surface-bg);\n  border-color: transparent;\n  font-size: 12px;");
    expect(styles).toContain(".profile-list > .apple-group.brand-gradient-surface .profile-card-meta .apple-chip {\n  border-color: transparent;\n  background: var(--main-surface-bg);\n  color: color-mix(in srgb, var(--primary-button-bg) 82%, transparent);");
  });

  it("让浅色模式的用量成功百分比使用高对比度绿色", () => {
    expect(styles).toContain("--success-text: #27c840;");
    expect(styles).toContain(".chip-success {\n  color: var(--success-text);");
  });

  it("用普通文字显示推理等级", () => {
    expect(source).toContain('{profile.reasoning_effort ? <><span aria-hidden="true">·</span><span>{profile.reasoning_effort}</span></> : null}');
    expect(source).not.toContain('<span className="apple-chip">{profile.reasoning_effort}</span>');
  });

  it("仅在端点或 API Key 缺失时禁用连通测试，缺什么报什么", () => {
    // 门控抽到共享 connectionGate（Codex 卡片、拖拽预览与 Claude 卡片共用同一判定）
    const titles = { ready: "ready", missingEndpoint: "no-endpoint", missingKey: "no-key" };
    expect(connectionGate(true, true, titles)).toEqual({ disabled: false, title: "ready" });
    expect(connectionGate(false, true, titles)).toEqual({ disabled: true, title: "no-endpoint" });
    expect(connectionGate(true, false, titles)).toEqual({ disabled: true, title: "no-key" });
    expect(connectionGate(false, false, titles)).toEqual({ disabled: true, title: "no-endpoint" });
    expect(source).not.toContain("missingApiCredentialsWarning");
  });

  it("卡片只常驻使用与更多，菜单保留测试和删除门控", () => {
    const i18n = createInstance();
    void i18n.init({ lng: "en", resources: { en: { profiles: {} } }, initAsync: false });
    const html = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(ProfileCardActions, {
      active: false, busy: true, testing: false, allowInactiveDeleteWhileBusy: true, connectionDisabled: false, connectionTitle: "Test",
    })));
    const buttons = html.match(/<button\b[^>]*>/g) ?? [];
    expect(buttons).toHaveLength(2);
    expect(buttons[1]).toContain('aria-haspopup="menu"');
    // 不传 terminalProfileId 的卡片（Codex）不渲染终端入口，只有 Claude 卡传
    expect(html).not.toContain('aria-label="terminal.open"');
  });

  it("订阅与普通供应商共用同一套连通性悬停文案", () => {
    // 订阅不再单独定义一份「测试订阅认证连通性」，避免同一动作两套文案；
    // 无 provider 的官方订阅永不禁用，第三方供应商才走缺什么报什么
    const t = ((key: string) => key) as unknown as TFunction<"profiles">;
    expect(profileConnectionGate({ provider: null } as CodexProfileSummary, t)).toEqual({ disabled: false, title: "connection.test" });
    expect(profileConnectionGate({ provider: "gw", has_base_url: true, has_key: true } as CodexProfileSummary, t)).toEqual({ disabled: false, title: "connection.test" });
    expect(profileConnectionGate({ provider: "gw", has_base_url: false, has_key: true } as CodexProfileSummary, t)).toEqual({ disabled: true, title: "connection.missingApiEndpointWarning" });
    expect(profileConnectionGate({ provider: "gw", has_base_url: true, has_key: false } as CodexProfileSummary, t)).toEqual({ disabled: true, title: "connection.missingApiKeyWarning" });
    expect(source).not.toContain("connection.testSubscription");
  });

  it("卡片操作按钮自持悬停文案，不继承卡片的「单击编辑」", () => {
    // 图标操作按钮仍自持 title，避免继承 <article> 的 card.clickToEdit。
    expect(source).toContain('title={active ? t("actions.inUse") : t("actions.switch")} onClick={onApply}');
  });

  it("使用按钮两种状态共用固定尺寸，保留可见图标与禁用条件", () => {
    const i18n = createInstance();
    void i18n.init({ lng: "en", resources: { en: { profiles: {} } }, initAsync: false });
    for (const [active, busy] of [[false, false], [true, false], [false, true]]) {
      const html = renderToStaticMarkup(createElement(I18nextProvider, { i18n }, createElement(ProfileCardActions, {
        active, busy, testing: false, connectionDisabled: false, connectionTitle: "Test",
      })));
      const button = html.match(/<button\b[^>]*>[\s\S]*?<\/button>/)?.[0] ?? "";
      expect(button).toContain('class="apple-action-button relative !h-[var(--icon-button-size)] !rounded-[var(--radius-control)] app-button--primary');
      expect(button).toContain(`aria-pressed="${active}"`);
      expect(button.includes('disabled=""')).toBe(active || busy);
      if (!active) expect(button).toContain("disabled:!opacity-100");
      expect(button.includes('<svg')).toBe(active);
      if (active) expect(button).toContain('width="18" height="18"');
      expect(button).toContain("actions.switch");
      expect(button).toContain(active ? "absolute inset-0 m-auto" : 'class=""');
    }
    expect(styles).toMatch(/\.apple-icon-button\s*\{[^}]*width: var\(--icon-button-size\);[^}]*height: var\(--icon-button-size\);/);
  });

  it("激活卡片和拖拽预览的图标颜色不覆盖使用按钮的主按钮前景色", () => {
    expect(styles.match(/\.profile-card-action-buttons > \.apple-icon-button:not\(\.app-button--primary\)/g)).toHaveLength(1);
    expect(styles).not.toContain(".profile-card-delete");
  });

  it("仅主动点击余额药丸才播放刷新动效，刷新逻辑保持原样", () => {
    // 动效只由点击回调开关，静默路径（挂载/聚焦/轮询）不触发；刷新逻辑抽到 useProfileBalance
    expect(hookSource).toContain("const [balanceRefreshing, setBalanceRefreshing] = useState(false);");
    expect(source).toContain("{balanceRefreshing ? <LoadingSpinner size=\"sm\" /> : <Gauge");
    expect(source).toContain("aria-busy={balanceRefreshing}");
    expect(hookSource).toContain("setBalanceRefreshing(true);");
    expect(hookSource).toContain("void fetchBalance(manual).finally(() => setBalanceRefreshing(false));");
    expect(authQuotaErrorKind("refresh_token 被服务端拒绝，该账号需要重新登录")).toBe("auth_expired"); // i18n-exempt: Backend error fixture.
    expect(authQuotaErrorKind("Network request timed out")).toBe("query_failed");
    expect(hookSource).toContain('feedback.error(t(authInvalid ? "balance.authInvalidToast" : "balance.queryFailedToast"));');
    // 单飞去重把在途 promise 交回调用方：指示器跟随真正落地的查询，不留真空期
    expect(hookSource).toContain("if (balanceInFlightRef.current) return balanceInFlightRef.current;");
  });

  it("静默用量刷新只在冷启动窗口内延后，其余场景零等待", () => {
    // 延迟的唯一理由是"别跟首屏抢资源"；窗口已经起来之后，切页和聚焦都不该再等。
    // 两条路各自独立：冷启动走 900/1200，日常走 0（setTimeout 立即宏任务）；逻辑在 useProfileBalance
    expect(hookSource).toContain("window.setTimeout(() => void fetchBalance(), coldStart ? (active ? 900 : 1200) : 0);");
    expect(source).not.toContain("deferBalanceRef");
    expect(source).not.toContain("lastSeenEpoch");
    expect(hookSource).not.toContain("deferBalanceRef");
    expect(hookSource).not.toContain("lastSeenEpoch");
  });
});
