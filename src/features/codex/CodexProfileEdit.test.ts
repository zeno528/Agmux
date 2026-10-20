// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { webInvoke } from "../../api/web-mock";
import { setupI18n } from "../../i18n";
import type { AppState, CodexProfileDetail, CodexProfileSummary } from "../../types";
import CodexProfileEdit from "./CodexProfileEdit";

vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ error: vi.fn() }) }));
// CodeMirror 依赖浏览器 DOM；回归测试仅渲染真实表单和下拉组件。
vi.mock("../../components/ConfigTextEditor", () => ({ default: () => null }));

it("认证预览保留 OAuth 路径，浏览器不能伪造本机桌面认证", async () => {
  const preview = await webInvoke<string>("auth_preview", { accountId: "fixture-account" });
  expect(JSON.parse(preview).tokens.account_id).toBe("fixture-account");
  await expect(webInvoke("auth_preview", { accountId: null })).rejects.toThrow("无法读取本机");
});

it.each([{ models: [] }, { models: ["fetched-model"] }])("仅有缓存时可用模型补入手输 ID，模型列表为 $models", async ({ models }) => {
  setupI18n("zh-CN");
  const profile = await webInvoke<CodexProfileSummary>("codex_add_builtin_profile", { kind: "opencode", apiKey: "fixture-key" });
  try {
    await webInvoke("codex_set_profile_fetched_models", { id: profile.id, models });
    const detail = await webInvoke<CodexProfileDetail>("codex_set_profile_model", { id: profile.id, model: "manual-model", effort: "" });
    const state = await webInvoke<AppState>("get_state");
    const markup = renderToStaticMarkup(createElement(CodexProfileEdit, {
      profile, initialDetail: detail, authStatus: state.auth_status, authStatusReady: true,
      onBack: () => {}, onChanged: () => {}, onManageChatgptAccounts: () => {},
    }));
    const select = markup.match(/<button\b[^>]*aria-label="(?:请先获取模型列表|选择模型)"[^>]*>[\s\S]*?<\/button>/)?.[0];
    expect(select).toBeDefined();
    expect(select!.includes("manual-model")).toBe(models.length > 0);
    expect(select!.includes('disabled=""')).toBe(models.length === 0);
    expect(markup).toContain('value="manual-model"');
  } finally { await webInvoke("codex_delete_profile", { id: profile.id }); }
});

it.each([
  { name: "无效对象仍可编辑", catalog: "{", model: null },
  { name: "无效数组仍可编辑", catalog: '{"models": [}', model: null },
  { name: "自定义目录优先", catalog: JSON.stringify({ models: [{ slug: "local-model", default_reasoning_level: "low",
    supported_reasoning_levels: [{ effort: "low" }] }] }), model: "local-model" },
  { name: "远程缓存保留动态默认", catalog: null, model: "fixture-model" },
])("官方编辑页：$name", async ({ catalog, model }) => {
  setupI18n("zh-CN");
  const profile = await webInvoke<CodexProfileSummary>("codex_add_builtin_profile", { kind: "chatgpt" });
  try {
    await webInvoke("codex_set_profile_fetched_models", { id: profile.id, models: ["fixture-model"],
      efforts: { "fixture-model": ["low", "max"] }, defaults: { "fixture-model": "low" } });
    const detail = await webInvoke<CodexProfileDetail>("codex_set_profile_model", { id: profile.id, model: "", effort: "" });
    const state = await webInvoke<AppState>("get_state");
    if (catalog !== null) {
      detail.raw_catalog = catalog;
      detail.raw_config = 'model_catalog_json = "models.json"\n';
    } else {
      expect(state.codex_profiles.find((item) => item.id === profile.id))
        .toMatchObject({ model: "fixture-model", reasoning_effort: "low" });
    }
    const markup = renderToStaticMarkup(createElement(CodexProfileEdit, {
      profile, initialDetail: detail, authStatus: state.auth_status, authStatusReady: true,
      onBack: () => {}, onChanged: () => {}, onManageChatgptAccounts: () => {},
    }));
    const fetchButton = markup.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)?.find((button) => button.includes("获取模型列表"));
    expect(fetchButton).toBeDefined();
    expect(fetchButton!.includes('disabled=""')).toBe(catalog !== null);
    if (catalog !== null) expect(markup).toContain("models.json");
    if (model) {
      expect(markup).toMatch(new RegExp(`aria-label="选择模型"[^>]*>[\\s\\S]*?${model}`));
      expect(markup).toContain(`value="${model}"`);
      expect(markup).toMatch(/aria-label="推理强度"[^>]*>[\s\S]*?low/);
    }
    if (model === "local-model") expect(markup).not.toContain("fixture-model");
    expect(markup.indexOf("登录方式")).toBeLessThan(markup.indexOf("模型 ID"));
    expect(markup.indexOf("模型 ID")).toBeLessThan(markup.indexOf("可用模型"));
    expect(markup.indexOf("可用模型")).toBeLessThan(markup.indexOf("推理强度"));
    expect(detail.model_values.model).toBeUndefined();
    expect(detail.model_values.model_reasoning_effort).toBeUndefined();
  } finally { await webInvoke("codex_delete_profile", { id: profile.id }); }
});

const source = readFileSync(new URL("./CodexProfileEdit.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const endpointFieldSource = readFileSync(new URL("../../components/EndpointField.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const zhLocale = readFileSync(new URL("../../i18n/locales/zh-CN/profiles.ts", import.meta.url), "utf8");
const enLocale = readFileSync(new URL("../../i18n/locales/en-US/profiles.ts", import.meta.url), "utf8");

describe("ProfileEdit 用量查询", () => {
  it("新增态按所选预设显示支持的用量开关", () => {
    expect(source).toContain('const supportsBalance = create ? presetKind === "chatgpt" || balanceQueryProviders.has(selectedPreset?.provider ?? "") : isOfficial || balanceQueryProviders.has(detail?.provider ?? "");');
    expect(source).not.toContain('const isUsageProvider = create ? usageQueryProviders.has(selectedPreset?.provider ?? "") : usageQueryProviders.has(detail?.provider ?? "");');
    expect(source).not.toContain("{!create && supportsBalance ?");
  });

  it("用全局设置行呈现，并在新增后保存开启状态", () => {
    expect(source).toContain('className="mt-4 flex min-h-[var(--input-min-height)] items-center justify-between gap-4"');
    expect(source).toContain('className="setting-title"');
    expect(source).toContain('className="setting-description mt-0.5"');
    expect(source).toContain('t("edit.balanceUsage")');
    expect(source).not.toContain('t("edit.balanceChatgpt")');
    expect(source).not.toContain('t("edit.balanceBalance")');
    expect(source).not.toContain('t("edit.balanceBoth")');
    expect(source).not.toContain('title={t("edit.balanceAutoRefreshTitle")}');
    expect(source).toContain("if (showBalance) await api.codexSetProfileShowBalance(created.id, true);");
    expect(source).not.toContain('<div className="app-input mt-4 flex items-center justify-between gap-3">');
  });

  it("新增态余额开关默认开启，编辑态仍由已存值覆盖", () => {
    expect(source).toContain('const isOfficial = create ? presetKind === "chatgpt" : profile?.kind === "official";');
    expect(source).toContain("const [showBalance, setShowBalance] = useState(create || Boolean(profile?.show_balance));");
    expect(source).toContain('setShowBalance(kind === "chatgpt" || balanceQueryProviders.has(preset.provider ?? ""));');
    expect(source).toContain("setShowBalance(loaded.show_balance);");
    expect(source).not.toContain("setShowBalance(false);");
  });

  it("自定义预设不预填名称和模型输入框", () => {
    expect(source).toContain('setName(kind === "custom" ? "" : presetConnectionName(');
    expect(source).toContain('setModelValue(kind === "custom" ? "" : preset.model);');
  });

  it("保留说明文字，但移除说明文字的悬停提示", () => {
    expect(source).toContain('{t("edit.balanceAutoRefresh")}');
    expect(zhLocale).toContain('balanceAutoRefresh: "显示在供应商卡片，窗口激活时自动刷新"');
    expect(enLocale).toContain('balanceAutoRefresh: "Shown on provider cards; refreshes when the window is active"');
  });

  it("新建和编辑配置缺少密钥或 API 端点时只发合并通知并仍允许保存", () => {
    expect(source).toContain("const notifySaved = (message: string) => {");
    expect(source).toContain("if (!isOfficial && showProviderFields && (missingApiKey || missingBaseUrl)) {");
    expect(source).toContain('feedback.warning(t("edit.savedWithMissingFields", {');
    expect(source).toContain('message: create ? t("edit.providerAdded") : t("edit.providerUpdated"),');
    expect(source).toContain('notifySaved(t("edit.customProviderAdded"));');
    expect(source).not.toContain('feedback.warning(t("edit.apiKeySaveWarning"));');
    expect(zhLocale).toContain('savedWithMissingFields: "{{message}}；未填写{{fields}}，可能无法测试连通和用量查询"');
    expect(enLocale).toContain('savedWithMissingFields: "{{message}}; {{fields}} missing, connection tests and usage queries may not work"');
  });

  it("编辑官方配置时复用全局 OAuth 账号快照，未完成认证刷新前不揭示空选项", () => {
    expect(source).toContain("authStatus: AuthStatus;");
    expect(source).toContain("authStatusReady: boolean;");
    expect(source).toContain("const authAccounts = authStatus.accounts;");
    expect(source).toContain('const needsAuthStatus = create ? isOfficial : profile?.auth_source === "oauth" || Boolean(profile?.account_id);');
    expect(source).toContain("const authStatusPending = needsAuthStatus && !authStatusReady;");
    expect(source).not.toContain("loadAuthStatus");
  });

  it("以预载详情初始化 detail，详情就绪后才挂载即首帧完整揭示", () => {
    expect(source).toContain("initialDetail?: CodexProfileDetail | null;");
    expect(source).toContain("useState<CodexProfileDetail | null>(initialDetail)");
  });

  it("表单与编辑器内容状态以预载详情惰性初始化，首帧不为空壳", () => {
    expect(source).toContain('const [configText, setConfigText] = useState(() => initialConfigText);');
    expect(source).toContain('const [catalogText, setCatalogText] = useState(() => initialDetail?.raw_catalog ?? initialDetail?.catalog_content ?? "");');
    expect(source).toContain('const [authText, setAuthText] = useState(() => initialDetail?.raw_auth ?? "");');
    expect(source).toContain('const [configInitial, setConfigInitial] = useState(() => initialConfigText);');
    expect(source).toContain('const [catalogInitial, setCatalogInitial] = useState(() => initialDetail?.raw_catalog ?? initialDetail?.catalog_content ?? "");');
    expect(source).toContain('const [authInitial, setAuthInitial] = useState(() => initialDetail?.raw_auth ?? "");');
    expect(source).toContain('const [baseUrl, setBaseUrl] = useState(() => initialDetail?.base_url ?? initialPreset?.base_url ?? "");');
    expect(source).toContain('const [apiKey, setApiKey] = useState(() => initialDetail?.api_key ?? "");');
    expect(source).toContain('const [modelValue, setModelValue] = useState(() => readModelValue(initialConfigText) ?? initialPreset?.model ?? "");');
    expect(source).toContain("const [fetchedModels, setFetchedModels] = useState<string[]>(() => initialDetail?.fetched_models ?? []);");
    expect(source).toContain('const [adminUrl, setAdminUrl] = useState(() => initialDetail?.admin_url ?? initialPreset?.admin_url ?? "");');
    expect(source).toContain("const [boundAccountId, setBoundAccountId] = useState<string | null>(() => initialDetail?.account_id ?? null);");
  });
});

describe("编辑器文件头布局", () => {
  it("tab 与格式化同一行，附属条嵌进编辑器托盘顶部并按 tab 切换：config 快捷设置、models/auth 文件操作", () => {
    expect(source).toContain('<div className="flex min-h-8 items-center justify-between gap-2">');
    expect(source).toContain('<div className="editor-attach-group mt-2">');
    expect(source).toContain('<div className="editor-attach-bar">');
    expect(source).toContain("<TabFileControls");
    expect(source).toContain('kind={activeTab === "models" ? "models" : "auth"}');
    expect(source).toContain('editable={activeTab === "models" || !authPreviewOnly}');
    expect(source).toContain('onClear={() => (activeTab === "models" ? setCatalogText("") : setAuthText(""))}');
  });

  it("格式化入口在文件标签行最右侧（ghost 权重），底部工具栏不再重复", () => {
    expect(source).toContain('className="editor-ghost editor-ghost--format ml-auto shrink-0"');
    expect(source.match(/formatCurrentDocument\(\)/g)).toHaveLength(1);
  });

  it("清空草稿保存时把空目录原文交给后端归一，而不是被 || null 吞成不动", () => {
    expect(source).toContain("liveCatalogPath && catalogDirty ? catalogText : null");
    expect(source).not.toContain("liveCatalogPath && catalogDirty ? catalogText || null : null");
  });
});

describe("编辑页回车保存", () => {
  it("只在编辑器外响应回车，且不再要求 Ctrl", () => {
    expect(source).toContain('event.key === "Enter" && !event.nativeEvent.isComposing && !(event.target instanceof Element && event.target.closest(".apple-editor-shell"))');
    expect(source).not.toContain('event.ctrlKey && event.key === "Enter"');
  });
});

describe("端点区域下拉", () => {
  it("双区域端点为可输入 combobox：地址可自由编辑，弹层复用全局 app-select-menu 基建（抽到共享 EndpointField，两编辑页共用）", () => {
    // 双区域档走共享 combobox；无档供应商回落普通输入（同一受控值）
    expect(source).toContain("<EndpointField");
    expect(source).toContain('baseUrlField("app-input")');
    expect(source).toContain("endpoints={presetEndpoints}");
    expect(endpointFieldSource).toContain("useFixedMenuPosition(open, rootRef.current, menuRef");
    expect(endpointFieldSource).toContain("useMenuDismiss(open, rootRef, menuRef, setOpen");
    expect(endpointFieldSource).toContain('className="app-select-menu app-popover"');
    expect(endpointFieldSource).toContain("presetRegionName(endpoint.region)");
    expect(endpointFieldSource).not.toContain("options.unshift");
    expect(endpointFieldSource).not.toContain("renderEndpointOption");
    expect(endpointFieldSource).not.toContain("renderEndpointMenuItem");
    expect(endpointFieldSource).not.toContain("closeOnOutsidePointer");
  });
});
