import { Children, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18next from "i18next";
import { setupI18n } from "../i18n";
import { claudePresets, codexPresets, presetConnections, presetConnectionName, presetEntries, type ClientPreset, type PresetConnection } from "../presets";
import { AppSegmentedControl } from "./AppSegmentedControl";
import PresetGrid from "./PresetGrid";
import { AppTabBar } from "./AppTabBar";
import { AppSearchInput } from "./AppSearchInput";
import { ProviderCreate, ProviderPresetControls } from "./ProviderSetup";
import { api } from "../api";
import { webInvoke } from "../api/web-mock";
import type { ClaudeProfileDetail, ClaudeProfileSummary } from "../types";
import ClaudeProfileEdit from "../features/claude/ClaudeProfileEdit";
import CodexProfileEdit from "../features/codex/CodexProfileEdit";
import ConfigTextEditor from "./ConfigTextEditor";
import { ProviderIdentityFields, ProviderModelFields, ProviderSecretField } from "./ProviderFields";

// 不启动浏览器：只替换 React 状态存储，调用真实的选择回调并检查两步流程的元素树。
const hooks = vi.hoisted(() => ({
  values: [] as unknown[], cursor: 0, effects: [] as Array<() => void | (() => void)>,
}));
vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  const useState = (initial: unknown) => {
    const index = hooks.cursor++;
    if (!(index in hooks.values)) hooks.values[index] = typeof initial === "function" ? initial() : initial;
    return [hooks.values[index], (next: unknown) => {
      hooks.values[index] = typeof next === "function" ? next(hooks.values[index]) : next;
    }];
  };
  return { ...actual, useState, useRef: (initial: unknown) => useState({ current: initial })[0],
    useMemo: (factory: () => unknown) => factory(),
    useEffect: (effect: () => void | (() => void)) => { hooks.effects.push(effect); },
    useLayoutEffect: (effect: () => void | (() => void)) => { hooks.effects.push(effect); } };
});
vi.mock("react-i18next", async (importOriginal) => ({
  ...await importOriginal<typeof import("react-i18next")>(),
  useTranslation: (namespace: "profiles" | "claude") => ({ t: i18next.getFixedT(null, namespace) }),
}));
vi.mock("../app/Feedback", () => ({ useFeedback: () => ({ success: vi.fn(), error: vi.fn() }) }));
vi.mock("./ConfigTextEditor", () => ({ default: () => null }));

type Props = { children?: ReactNode; [key: string]: unknown };
function elements(node: ReactNode): ReactElement<Props>[] {
  return Children.toArray(node).flatMap((child) => isValidElement<Props>(child) ? [child, ...elements(child.props.children)] : []);
}
function click(node: ReactElement<Props>) { (node.props.onClick as () => void)(); }

beforeEach(() => {
  setupI18n("zh-CN");
  hooks.values = [];
  hooks.cursor = 0;
  hooks.effects = [];
});

describe("供应商创建的两步流程", () => {
  it.each([{ presets: codexPresets }, { presets: claudePresets }])("页头搜索按名称、套餐与官网筛选，叠加分类且支持清空", ({ presets }) => {
    const render = () => {
      hooks.cursor = 0;
      return ProviderCreate({ presets, onBack: vi.fn(), children: () => null });
    };
    let page = render();
    const search = (value: string) => {
      const input = elements(page).find((node) => node.type === AppSearchInput)!;
      (input.props.onChange as (value: string) => void)(value);
      page = render();
    };
    const grid = () => {
      const node = elements(page).find((item) => item.type === PresetGrid)!;
      return PresetGrid({ presets, selectedKind: "", onSelect: vi.fn(),
        category: node.props.category as "all" | "api" | "plans" | "accounts", query: node.props.query as string });
    };
    const names = () => elements(grid()).filter((node) => node.type === "button").map((node) =>
      elements(node).find((child) => child.props.className === "setting-title block truncate font-medium!")!.props.children);
    const header = elements(page).find((node) => node.type === "header")!;
    expect(elements(header).some((node) => node.type === AppSearchInput && node.props.placeholder === "搜索供应商、套餐或官网地址")).toBe(true);
    search("  mINimAx  ");
    expect(names()).toEqual(["MiniMax · M Plan"]);
    search("platform.minimax.cn");
    expect(names()).toEqual(["MiniMax · M Plan"]);
    search("Token Plan");
    expect(names().length).toBeGreaterThan(0);
    expect(names()).toContain("小米 MiMo");
    expect(names().every((name) => String(name).includes("Token Plan") || name === "小米 MiMo")).toBe(true);
    const bar = elements(page).find((node) => node.type === AppTabBar)!;
    (bar.props.onSelect as (id: string) => void)("api");
    page = render();
    expect(names()).toEqual([]);
    search("不存在的供应商");
    expect(names()).toEqual([]);
    expect(elements(grid()).find((node) => node.props.role === "status")!.props.children).toBe("没有匹配项");
    search("");
    expect(elements(page).find((node) => node.type === AppSearchInput)!.props.value).toBe("");
    expect(names()).toContain("DeepSeek");
    expect(names().some((name) => String(name).includes("Token Plan"))).toBe(false);
  });

  it.each([{ presets: codexPresets }, { presets: claudePresets }])("复用设置页层级：页头、固定分类栏、独立滚动列表", ({ presets }) => {
    const page = ProviderCreate({ presets, onBack: vi.fn(), children: () => null });
    const section = elements(page).find((node) => node.type === "section")!;
    expect(String(section.props.className)).toContain("apple-scroll-page");
    const header = elements(section).find((node) => node.type === "header")!;
    expect(header.props.className).toBe("apple-page-bar");
    const siblings = Children.toArray(section.props.children) as ReactElement<Props>[];
    expect(siblings[1].type).toBe(AppTabBar);
    expect(siblings[2].props.className).toBe("apple-edit-content");
    expect(elements(siblings[2]).some((node) => node.type === AppTabBar)).toBe(false);
    expect(elements(siblings[2]).some((node) => node.type === PresetGrid)).toBe(true);
  });

  it.each([{ presets: codexPresets }, { presets: claudePresets }])("逐层返回主界面，重新选择入口时保留独立草稿", ({ presets }) => {
    const onBack = vi.fn();
    const renderEditor = vi.fn((preset: ClientPreset, onChangeProvider: () => void) => <button type="button" onClick={onChangeProvider}>{preset.kind}</button>);
    const render = () => {
      hooks.cursor = 0;
      return ProviderCreate({ presets, onBack, children: renderEditor });
    };
    let tree = render();
    expect(renderEditor).not.toHaveBeenCalled();
    expect(elements(tree).some((node) => node.type === PresetGrid)).toBe(true);
    expect(elements(tree).some((node) => String(node.props.className).includes("apple-edit-toolbar--footer"))).toBe(false);
    click(elements(tree).find((node) => node.type === "button" && node.props["aria-label"] === "新建供应商")!);
    expect(onBack).toHaveBeenCalledOnce();
    onBack.mockClear();
    const pick = (kind: string) => {
      const grid = elements(tree).find((node) => node.type === PresetGrid)!;
      (grid.props.onSelect as (kind: string) => void)(presetEntries(presets).find((entry) => entry.preset.kind === kind)!.key);
      tree = render();
    };
    pick("kimi");
    expect(elements(tree).some((node) => node.type === PresetGrid)).toBe(false);
    const original = elements(tree).find((node) => node.key?.includes("kimi"))!;
    click(elements(tree).find((node) => node.type === "button" && node.props.children === "kimi")!);
    tree = render();
    expect(elements(tree).find((node) => node.key === original.key)?.props.className).toBe("hidden");
    click(elements(tree).find((node) => node.type === "button" && node.props.className === "apple-page-header apple-back-button")!);
    expect(onBack).toHaveBeenCalledOnce();
    onBack.mockClear();
    tree = render();
    expect(elements(tree).some((node) => node.type === PresetGrid)).toBe(true);
    expect(elements(tree).some((node) => node.props["aria-label"] === "新建供应商")).toBe(true);
    pick("kimi");
    click(elements(tree).find((node) => node.type === "button" && node.props.children === "kimi")!);
    tree = render();
    pick("deepseek");
    expect(elements(tree).find((node) => node.key === original.key)?.props.className).toBe("hidden");
    click(elements(tree).find((node) => node.type === "button" && node.props.children === "deepseek")!);
    tree = render();
    pick("kimi");
    expect(elements(tree).find((node) => node.key === original.key)?.props.className).toBe("contents");
    expect(hooks.values[0]).toEqual([presetEntries(presets).find((entry) => entry.preset.kind === "kimi")!.key, "deepseek"]);
    expect(onBack).not.toHaveBeenCalled();
  });

  it("列表拆分同品牌套餐，保留自定义与官方账号入口", () => {
    const markup = renderToStaticMarkup(<PresetGrid presets={claudePresets} category="all" selectedKind="" onSelect={() => {}} />);
    expect(markup).toContain("Kimi");
    expect(markup).toContain("多版本");
    expect(markup).toContain("Kimi · Kimi Code");
    expect(markup).not.toContain("阿里云百炼 Coding Plan");
    expect(markup).not.toContain("百度千帆 Coding Plan");
    expect(markup).toContain("自定义");
    expect(markup).toContain("Claude Account");
    expect(markup).toContain("Anthropic API");
    expect(markup).toContain("通义千问 · Coding Plan");
    expect(markup).toContain("通义千问 · Token Plan");
    const single = claudePresets.find((preset) => preset.kind === "custom")!;
    hooks.values = [];
    hooks.cursor = 0;
    expect(renderToStaticMarkup(<PresetGrid presets={[single]} category="all" selectedKind="" onSelect={() => {}} />))
      .toContain('<span class="invisible shrink-0" aria-hidden="true"><span class="plan-badge font-normal!">多版本</span></span>');
  });

  it.each([
    { presets: codexPresets, total: 20, apiCount: 9, planCount: 9, account: "ChatGPT", plan: "火山方舟豆包 · Agent Plan", planKey: "doubao:Agent Plan" },
    { presets: claudePresets, total: 26, apiCount: 11, planCount: 13, account: "Claude Account", plan: "Kimi · Kimi Code", planKey: "kimi:Kimi Code" },
  ])("分类数量与筛选结果一致，不重复计算区域且保留账号与自定义", ({ presets, total, apiCount, planCount, account, plan, planKey }) => {
    const onSelect = vi.fn();
    const render = () => {
      hooks.cursor = 0;
      const page = ProviderCreate({ presets, onBack: vi.fn(), children: () => null });
      const grid = elements(page).find((node) => node.type === PresetGrid)!;
      return <>{page}{PresetGrid({ presets, category: grid.props.category as "all" | "api" | "plans" | "accounts", selectedKind: planKey, onSelect })}</>;
    };
    let tree = render();
    const names = () => elements(tree).filter((node) => node.props.className === "setting-title block truncate font-medium!").map((node) => node.props.children);
    const selectCategory = (label: string) => {
      const bar = elements(tree).find((node) => node.type === AppTabBar)!;
      const navigation = AppTabBar(bar.props as Parameters<typeof AppTabBar>[0]);
      click(elements(navigation).find((node) => node.type === "button" && node.props.children === label)!);
      tree = render();
    };
    expect(names()).toHaveLength(total);
    expect(names()).toEqual(expect.arrayContaining(["自定义", account, "DeepSeek", plan]));
    const allLabel = `全部（${total}）`;
    expect(elements(tree).find((node) => node.type === AppTabBar)!.props.items).toEqual(expect.arrayContaining([{ id: "all", label: allLabel }]));
    expect((elements(tree).find((node) => node.type === AppTabBar)!.props.items as Array<{id: string}>).map((item) => item.id)).toEqual(["all", "accounts", "plans", "api"]);
    expect(elements(tree).filter((node) => node.type === "h2").map((node) => node.props.children)).toEqual(["账号授权（1）", `订阅套餐（${planCount}）`, `按量付费（${apiCount}）`, "自定义（1）"]);
    selectCategory("账号授权（1）");
    expect(names()).toEqual([account]);
    expect(elements(tree).some((node) => node.type === "h2")).toBe(false);
    selectCategory(`按量付费（${apiCount}）`);
    const apiNames = names();
    expect(apiNames).toHaveLength(apiCount);
    expect(apiNames).toContain("DeepSeek");
    expect(apiNames).not.toContain("自定义");
    expect(apiNames).not.toContain(account);
    expect(apiNames).not.toContain(plan);
    expect(apiNames).not.toContain("小米 MiMo");
    selectCategory(`订阅套餐（${planCount}）`);
    expect(names()).toHaveLength(planCount);
    expect(names()).toContain(plan);
    expect(names()).toContain("小米 MiMo");
    expect(names().some((name) => apiNames.includes(name))).toBe(false);
    expect(names()).not.toContain("自定义");
    expect(names()).not.toContain(account);
    click(elements(tree).find((node) => node.type === "button" && elements(node).some((child) => child.props.children === plan))!);
    expect(onSelect).toHaveBeenCalledWith(planKey);
    expect(elements(tree).filter((node) => node.props["aria-pressed"] === true)).toHaveLength(1);
    selectCategory(allLabel);
    expect(names()).toHaveLength(total);
    expect(names()).toEqual(expect.arrayContaining(["自定义", account]));
  });
});

describe("供应商套餐入口与区域", () => {
  it("火山 Coding Plan 与 Agent Plan 的入口、地址和草稿容器相互独立", () => {
    const entries = presetEntries(codexPresets).filter((entry) => entry.preset.kind === "doubao");
    expect(entries.map((entry) => entry.name)).toEqual(["火山方舟豆包 · Coding Plan", "火山方舟豆包 · Agent Plan"]);
    expect(entries.map((entry) => entry.preset.endpoints.map((endpoint) => endpoint.region))).toEqual([["cn", "global"], ["cn"]]);
    expect(entries[1].preset.base_url).toBe("https://ark.cn-beijing.volces.com/api/plan/v3");
    const children = vi.fn((preset: ClientPreset, onChangeProvider: () => void) => <button onClick={onChangeProvider}>{preset.plan}</button>);
    const render = () => { hooks.cursor = 0; return ProviderCreate({ presets: codexPresets, onBack: vi.fn(), children }); };
    let tree = render();
    const pick = (key: string) => {
      const grid = elements(tree).find((node) => node.type === PresetGrid)!;
      (grid.props.onSelect as (key: string) => void)(key);
      tree = render();
    };
    pick(entries[0].key);
    const codingKey = elements(tree).find((node) => node.props.className === "contents")!.key;
    click(elements(tree).find((node) => node.type === "button" && node.props.children === "Coding Plan")!);
    tree = render();
    pick(entries[1].key);
    expect(elements(tree).find((node) => node.key === codingKey)?.props.className).toBe("hidden");
    expect(elements(tree).find((node) => node.props.className === "contents")?.key).not.toBe(codingKey);
    expect(hooks.values[0]).toEqual(entries.map((entry) => entry.key));
  });

  it.each(["Coding Plan", "Agent Plan"])("Codex %s 初始化使用所选入口地址，仍读取原内置模板", async (plan) => {
    const preset = presetEntries(codexPresets).find((entry) => entry.key === `doubao:${plan}`)!.preset;
    const template = 'model_provider = "fixture"\nmodel = "fixture-model"\n[model_providers.fixture]\nname = "Fixture"\nbase_url = "https://template.example.test/v1"\n';
    const mcp = vi.spyOn(api, "codexGetMcpSectionToml").mockResolvedValue("");
    const builtin = vi.spyOn(api, "codexGetBuiltinConfig").mockResolvedValue(template);
    try {
      const onBack = vi.fn();
      const onChangeProvider = vi.fn();
      const flow = CodexProfileEdit({ profile: null, create: true, authStatus: { authenticated: false, default_account_id: null, accounts: [], external: [] }, authStatusReady: true, onBack, onChanged: vi.fn(), onManageChatgptAccounts: vi.fn() });
      const editor = flow.props.children(preset, onChangeProvider);
      const render = () => { hooks.cursor = 0; return editor.type(editor.props) as ReactNode; };
      const initialTree = render();
      const identity = elements(initialTree).find((node) => node.type === ProviderIdentityFields)!.props;
      expect(identity.name).toContain(plan);
      expect(identity.icon).toBe(preset.icon);
      expect(elements(initialTree).find((node) => node.type === "fieldset")?.props.disabled).toBe(true);
      expect(elements(initialTree).find((node) => node.type === ConfigTextEditor)).toBeUndefined();
      for (const effect of hooks.effects.splice(0)) effect();
      await vi.waitFor(() => {
        const tree = render();
        expect(elements(tree).find((node) => node.type === ConfigTextEditor)?.props.value).toContain(preset.base_url);
      });
      const tree = render();
      expect(elements(tree).find((node) => node.type === ProviderIdentityFields)?.props.icon).toBe(identity.icon);
      expect(elements(tree).find((node) => node.type === "fieldset")?.props.disabled).toBe(false);
      expect(builtin).toHaveBeenCalledExactlyOnceWith("doubao");
      expect(elements(tree).find((node) => node.type === ProviderPresetControls)?.props.baseUrl).toBe(preset.base_url);
      expect(elements(tree).find((node) => node.type === ConfigTextEditor)?.props.value).toContain(preset.base_url);
      click(elements(tree).find((node) => node.type === "button" && node.props["aria-label"] === "返回")!);
      expect(onChangeProvider).toHaveBeenCalledOnce();
      expect(onBack).not.toHaveBeenCalled();
    } finally {
      mcp.mockRestore();
      builtin.mockRestore();
    }
  });

  it.each(["cn", "global"])("Claude Kimi Code %s 版本保存正确端点、kind 和鉴权键，并保留名称、模型与高级设置", async (region) => {
    const preset = presetEntries(claudePresets).find((entry) => entry.preset.kind === "kimi-code")!.preset;
    const onBack = vi.fn();
    const onChangeProvider = vi.fn();
    const flow = ClaudeProfileEdit({ profile: null, create: true, onBack, onChanged: vi.fn() });
    const editor = flow.props.children(preset, onChangeProvider);
    const render = () => {
      hooks.cursor = 0;
      return editor.type(editor.props) as ReactNode;
    };
    const field = (tree: ReactNode, type: unknown) => elements(tree).find((node) => node.type === type)!.props;
    const code = presetConnections(preset, [preset]).find((connection) => connection.region === region)!;
    let tree = render();
    (field(tree, ProviderPresetControls).onSelect as (connection: PresetConnection) => void)(code);
    tree = render();
    expect(field(tree, ProviderIdentityFields).name).toBe("Kimi · Kimi Code");
    // 即使手动名称恰好等于另一档默认名，也不能再被自动命名覆盖。
    const customName = "Kimi · 全球 · API";
    (field(tree, ProviderIdentityFields).onName as (value: string) => void)(customName);
    (field(tree, ProviderIdentityFields).onDescription as (value: string) => void)("保留描述");
    (field(tree, ProviderSecretField).onChange as (value: string) => void)("fixture-key");
    const mappings = field(tree, ProviderModelFields).mappingFields as Array<{ key: string; onChange: (value: string) => void }>;
    mappings.find((item) => item.key === "ANTHROPIC_MODEL")!.onChange("fixture-model");
    (field(tree, ConfigTextEditor).onChange as (value: string) => void)('{"permissions":{"defaultMode":"plan"},"env":{"EXTRA":"keep"}}');
    tree = render();
    (field(tree, ProviderPresetControls).onSelect as (connection: PresetConnection) => void)(code);
    tree = render();
    const env = JSON.parse(field(tree, ConfigTextEditor).value as string).env;
    expect(env).toMatchObject({ ANTHROPIC_API_KEY: "fixture-key", ANTHROPIC_MODEL: "fixture-model", EXTRA: "keep" });
    expect(env).not.toHaveProperty("ANTHROPIC_AUTH_TOKEN");
    expect(field(tree, ProviderIdentityFields).name).toBe(customName);
    click(elements(tree).find((node) => node.type === "button" && node.props["aria-label"] === "返回")!);
    expect(onChangeProvider).toHaveBeenCalledOnce();
    expect(onBack).not.toHaveBeenCalled();
    (field(tree, ProviderPresetControls).onSelect as (connection: PresetConnection) => void)(code);
    tree = render();
    const save = vi.spyOn(api, "claudeSaveProfile").mockImplementation((input) => webInvoke<ClaudeProfileDetail>("claude_save_profile", input));
    let saved: ClaudeProfileSummary | undefined;
    try {
      click(elements(tree).find((node) => node.type === "button" && node.props.className === "apple-action-button app-button--primary")!);
      await vi.waitFor(() => expect(onBack).toHaveBeenCalledOnce());
      saved = await save.mock.results[0].value;
      const detail = await webInvoke<ClaudeProfileDetail>("claude_get_profile", { id: saved!.id });
      expect(detail).toMatchObject({ kind: "kimi-code", name: customName, description: "保留描述", model: "fixture-model", base_url: code.base_url });
      const settings = JSON.parse(detail.raw_settings!);
      expect(settings.permissions).toEqual({ defaultMode: "plan" });
      expect(settings.env).toMatchObject({ ANTHROPIC_API_KEY: "fixture-key", ANTHROPIC_MODEL: "fixture-model", EXTRA: "keep" });
      expect(settings.env).not.toHaveProperty("ANTHROPIC_AUTH_TOKEN");
    } finally {
      save.mockRestore();
      if (saved) await webInvoke("claude_delete_profile", { id: saved.id });
    }
  });

  it("每个套餐只显示自己的区域，切换区域不更换套餐", () => {
    const entries = presetEntries(claudePresets);
    const preset = entries.find((entry) => entry.key === "qwen:Token Plan")!.preset;
    let baseUrl = preset.base_url;
    let selected: PresetConnection | undefined;
    const render = () => ProviderPresetControls({ preset, baseUrl, disabled: false, onChangeProvider: vi.fn(), onSelect: (connection) => {
      selected = connection;
      baseUrl = connection.base_url;
    } });
    const select = (label: string, index: number) => {
      const group = elements(render()).find((node) => node.type === AppSegmentedControl && node.props.label === label)!;
      click(Children.toArray(group.props.children)[index] as ReactElement<Props>);
    };
    expect(elements(render()).filter((node) => node.type === AppSegmentedControl)).toHaveLength(1);
    select("版本", 1);
    expect(selected).toMatchObject({ kind: "qwen", region: "global", label: "Token Plan" });
    expect(elements(render()).filter((node) => node.type === AppSegmentedControl)).toHaveLength(1);
    // 区域由滑块展示，自动名称不随区域变化
    expect(presetConnectionName(preset)).toBe("通义千问 · Token Plan");
    select("版本", 0);
    expect(selected).toMatchObject({ kind: "qwen", region: "cn", label: "Token Plan" });
    const coding = entries.find((entry) => entry.key === "qwen:Coding Plan")!.preset;
    const codingControls = ProviderPresetControls({ preset: coding, baseUrl: coding.base_url, disabled: false, onChangeProvider: vi.fn(), onSelect: vi.fn() });
    expect(elements(codingControls).filter((node) => node.type === AppSegmentedControl)).toHaveLength(1);
    expect(coding.endpoints.map((endpoint) => endpoint.base_url)).toEqual(["https://coding.dashscope.aliyuncs.com/apps/anthropic", "https://coding-intl.dashscope.aliyuncs.com/apps/anthropic"]);
  });

  it.each([{ presets: codexPresets }, { presets: claudePresets }])("小米只切换按量付费和 Token Plan，不展示区域；手动地址不冒充官方档位", ({ presets }) => {
    const entries = presetEntries(presets).filter((entry) => entry.preset.kind === "mimo");
    expect(entries).toHaveLength(1);
    const preset = entries[0].preset;
    expect(entries[0].name).toBe("小米 MiMo");
    expect(preset.endpoints).toHaveLength(2);
    const onSelect = vi.fn();
    const render = (baseUrl: string) => ProviderPresetControls({ preset, baseUrl, disabled: false, onChangeProvider: vi.fn(), onSelect });
    const group = elements(render(preset.base_url)).find((node) => node.type === AppSegmentedControl)!;
    const buttons = Children.toArray(group.props.children) as ReactElement<Props>[];
    expect(buttons.map((button) => button.props.children)).toEqual(["按量付费", "Token Plan"]);
    click(buttons[1]);
    expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "mimo", base_url: preset.endpoints![1].base_url }));
    expect(elements(render(preset.endpoints![1].base_url)).find((node) => node.type === AppSegmentedControl)?.props.selectedIndex).toBe(1);
    expect(elements(render(preset.endpoints![1].base_url)).filter((node) => node.type === AppSegmentedControl)).toHaveLength(1);
    click(buttons[0]);
    expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "mimo", base_url: preset.base_url }));
    expect(elements(render(preset.base_url)).filter((node) => node.type === AppSegmentedControl)).toHaveLength(1);
    const custom = renderToStaticMarkup(render("https://custom.example.test"));
    expect(custom).toContain('data-custom="true"');
    expect(custom).not.toContain('aria-pressed="true"');
    expect(custom).toContain("当前使用自定义地址");
  });

  it("单一接入方式不展示切换条，保存期间禁用更换与切换，Enter 不保存背景表单", () => {
    const preset = codexPresets.find((item) => item.kind === "deepseek")!;
    const tree = ProviderPresetControls({ preset, baseUrl: preset.base_url, disabled: true, onChangeProvider: vi.fn(), onSelect: vi.fn() });
    expect(elements(tree).some((node) => node.type === AppSegmentedControl)).toBe(false);
    expect(elements(tree).find((node) => node.type === "button")?.props.disabled).toBe(true);
    const stopPropagation = vi.fn();
    tree.props.onKeyDown({ key: "Enter", stopPropagation });
    expect(stopPropagation).toHaveBeenCalledOnce();
  });

  it("分组保留原 kind、端点和客户端边界", () => {
    for (const presets of [codexPresets, claudePresets]) {
      for (const preset of presets.filter((item) => item.group)) expect(presets.some((item) => item.kind === preset.group)).toBe(true);
      for (const preset of presets.filter((item) => !item.group)) {
        for (const connection of presetConnections(preset, presets)) {
          const source = presets.find((item) => item.kind === connection.kind)!;
          expect(source.group ?? source.kind).toBe(preset.kind);
          expect((source.endpoints ?? [source]).some((item) => item.base_url === connection.base_url && item.admin_url === connection.admin_url)).toBe(true);
        }
      }
    }
    expect(presetConnections(codexPresets.find((item) => item.kind === "qwen")!, codexPresets).some((item) => item.kind === "qwen-coding-plan")).toBe(false);
    expect(presetConnections(claudePresets.find((item) => item.kind === "kimi")!, claudePresets).some((item) => item.kind === "kimi-code")).toBe(true);
  });
});
