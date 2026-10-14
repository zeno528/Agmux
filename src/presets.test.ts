import { describe, expect, it } from "vitest";
// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import i18next from "i18next";
import { resources } from "./i18n";
import { codexPresets, claudePresets, clientPresets, codexCustomCatalogTemplate, providerPresets, presetEntries, presetEntryCategory, codexBuiltinHasCatalog, balanceQueryProviders, type ProviderPreset } from "./presets";

describe("供应商目录的客户端边界", () => {
  it("每个 Codex 入口都有匹配的 Rust 注册、模板默认值及目录声明", () => {
    const registry = readFileSync(new URL("../src-tauri/src/codex_builtin.rs", import.meta.url), "utf8");
    for (const preset of codexPresets.filter((item) => item.kind !== "custom")) {
      const config = readFileSync(new URL(`../src-tauri/assets/builtin/${preset.kind}.toml`, import.meta.url), "utf8");
      const symbol = preset.kind.replace(/-/g, "_").toUpperCase();
      expect(registry).toContain(`kind: KIND_${symbol},`);
      expect(registry).toContain(`config: ${symbol}_CONFIG,`);
      expect(config.match(/^model = "(.*)"/m)?.[1] ?? "").toBe(preset.model);
      if (preset.provider) {
        expect(config).toContain(`model_provider = "${preset.provider}"`);
        expect(config).toContain(`base_url = "${preset.base_url}"`);
        expect(config).toContain('wire_api = "responses"');
      }
      const entry = registry.split(`kind: KIND_${symbol},`)[1].split("    },")[0];
      expect(entry.includes("catalog: Some(")).toBe(codexBuiltinHasCatalog(preset.kind));
    }
  });

  it("按量入口不误用套餐用量查询；仅有 Chat 协议的套餐不进入 Codex", () => {
    for (const kind of ["qwen-api", "doubao-api", "opencode-zen"]) {
      for (const presets of [codexPresets, claudePresets]) {
        const preset = presets.find((item) => item.kind === kind)!;
        expect(preset.plan).toBe("API");
        expect(balanceQueryProviders.has(preset.provider!)).toBe(false);
      }
    }
    for (const kind of ["tencent-token-plan", "tencent-coding-plan", "qwen-coding-plan", "qianfan-coding-plan", "siliconflow", "longcat"]) {
      expect(codexPresets.some((preset) => preset.kind === kind)).toBe(false);
      expect(claudePresets.some((preset) => preset.kind === kind)).toBe(true);
    }
    expect(claudePresets.find((preset) => preset.kind === "tencent-token-plan-enterprise")?.endpoints).toHaveLength(2);
    expect(presetEntries(claudePresets).find((entry) => entry.key === "doubao:Agent Plan")?.preset.base_url).toBe("https://ark.cn-beijing.volces.com/api/plan");
  });
  it.each([{ presets: codexPresets }, { presets: claudePresets }])("套餐入口标题完整，套餐与普通 API 分类准确", async ({ presets }) => {
    await i18next.init({ lng: "zh-CN", defaultNS: "common", resources });
    const entries = presetEntries(presets);
    for (const entry of entries) {
      if (entry.preset.plan !== "API") {
        expect(entry.name).toContain(` · ${entry.preset.plan}`);
        expect(presetEntryCategory(entry.preset)).toBe("plans");
      }
    }
    for (const [kind, plan] of [["minimax", "M Plan"], ["zhipu", "Coding Plan"], ["qwen", "Token Plan"]]) {
      expect(entries.find((entry) => entry.preset.kind === kind)?.preset.plan).toBe(plan);
    }
    for (const kind of ["minimax", "zhipu"]) {
      expect(entries.filter((entry) => entry.preset.kind === kind)).toHaveLength(1);
      expect(presets.some((preset) => preset.kind === `${kind}-api`)).toBe(false);
      expect(presets.find((preset) => preset.kind === kind)?.endpoints).toHaveLength(2);
    }
    for (const kind of ["kimi", "hunyuan"]) {
      const entry = entries.find((entry) => entry.preset.kind === kind && entry.preset.plan === "API");
      expect(entry, `${kind} 的普通 API 不能误标为会员套餐`).toBeDefined();
      expect(presetEntryCategory(entry!.preset)).toBe("api");
    }
    for (const kind of ["deepseek", "openrouter", "kimi", "hunyuan", "qianfan", ...(presets === codexPresets ? ["mimo", "xai"] : ["anthropic", "siliconflow", "longcat"])]) {
      const entry = entries.find((item) => item.preset.kind === kind && item.preset.plan === "API")!;
      const needsSuffix = (presets === codexPresets ? ["kimi", "hunyuan"] : ["kimi", "hunyuan", "qianfan"]).includes(kind);
      expect(entry.name).toBe(needsSuffix ? `${entry.preset.name} · 按量付费` : entry.preset.name);
    }
  });

  it("支持获取模型列表的四个 Codex 预设不预填模型 ID", () => {
    for (const kind of ["chatgpt", "deepseek", "zhipu", "minimax"]) {
      expect(codexPresets.find((preset) => preset.kind === kind)?.model).toBe("");
    }
  });

  it("双客户端各用自己的配置，单客户端条目只出现在对应列表", () => {
    const providers: ProviderPreset[] = [
      { kind: "both", name: "Both", icon: "custom",
        codex: { provider: "codex-provider", base_url: "https://codex.example.test", admin_url: "https://codex.example.test/admin", model: "codex-model",
          endpoints: [{ region: "cn", base_url: "https://codex.example.test", admin_url: null }] },
        claude: { provider: null, base_url: "https://claude.example.test", admin_url: null, model: "" },
      },
      { kind: "claude-only", name: "Claude only", icon: "custom",
        claude: { provider: null, base_url: "https://only.example.test", admin_url: null, model: "claude-model" },
      },
      { kind: "codex-only", name: "Codex only", icon: "custom",
        codex: { provider: null, base_url: "", admin_url: null, model: "codex-only-model" },
      },
    ];
    const codex = clientPresets(providers, "codex");
    const claude = clientPresets(providers, "claude");
    expect(codex.map(({ kind }) => kind)).toEqual(["both", "codex-only"]);
    expect(claude.map(({ kind }) => kind)).toEqual(["both", "claude-only"]);
    expect(codex[0]).toMatchObject(providers[0].codex!);
    expect(claude[0]).toMatchObject(providers[0].claude!);
    expect(claude[0].endpoints).toBeUndefined();
    expect(claude[0].model).toBe("");
    expect(codexPresets.some(({ kind }) => kind === "claude-account" || kind === "anthropic")).toBe(false);
    expect(claudePresets.some(({ kind }) => kind === "chatgpt")).toBe(false);
    expect(new Set(providerPresets.map(({ kind }) => kind)).size).toBe(providerPresets.length);
  });

  it("投影后语言切换仍更新共享名称，不改变接入配置", async () => {
    const previousLanguage = i18next.language;
    await i18next.init({ lng: "zh-CN", defaultNS: "common", resources: {
      "zh-CN": { common: { preset: { custom: "名称" } } },
      "en-US": { common: { preset: { custom: "Name" } } },
    } });
    try {
      const providers: ProviderPreset[] = [{ kind: "fixture", get name() { return i18next.t("preset.custom"); }, icon: "custom",
        codex: { provider: null, base_url: "https://codex.example.test", admin_url: null, model: "codex-model" },
        claude: { provider: null, base_url: "https://claude.example.test", admin_url: null, model: "claude-model" },
      }];
      const codex = clientPresets(providers, "codex");
      const claude = clientPresets(providers, "claude");
      expect([codex[0].name, claude[0].name]).toEqual(["名称", "名称"]);
      await i18next.changeLanguage("en-US");
      expect([codex[0].name, claude[0].name]).toEqual(["Name", "Name"]);
      expect([codex[0].model, claude[0].model]).toEqual(["codex-model", "claude-model"]);
    } finally {
      await i18next.changeLanguage(previousLanguage ?? "zh-CN");
    }
  });
});

it("Claude 官方账号与 API 是独立卡片，紧随自定义", () => {
  expect(claudePresets.slice(0, 3).map(({ kind, name }) => [kind, name])).toEqual([
    ["custom", claudePresets[0].name], ["claude-account", "Claude Account"], ["anthropic", "Anthropic API"],
  ]);
  expect(claudePresets[1].base_url).toBe("");
});

describe("codexCustomCatalogTemplate", () => {
  it("follows the Codex catalog schema with parser-required fields", () => {
    // 回归：旧模板用 id/name（Codex 目录格式是 slug），且 base_instructions 与
    // supports_reasoning_summaries 缺失会让 Codex 拒载整个目录文件
    const catalog = JSON.parse(codexCustomCatalogTemplate) as { models: Array<Record<string, unknown>> };
    expect(catalog.models.length).toBeGreaterThan(0);
    for (const model of catalog.models) {
      expect(model.slug).toBeTruthy();
      expect(model).toHaveProperty("base_instructions");
      expect(model).toHaveProperty("supports_reasoning_summaries");
      expect(model.id).toBeUndefined();
      expect(model.name).toBeUndefined();
    }
  });

  it("carries the full 21-field set of the official catalogs", () => {
    // 对照 assets/builtin/zhipu-models.json 的字段集（CCswitch 与官方目录每条 21 字段）
    const expected = [
      "slug", "display_name", "description", "default_reasoning_level", "supported_reasoning_levels",
      "shell_type", "visibility", "supported_in_api", "priority", "base_instructions",
      "supports_reasoning_summaries", "default_reasoning_summary", "support_verbosity",
      "apply_patch_tool_type", "truncation_policy", "context_window", "max_context_window",
      "effective_context_window_percent", "supports_parallel_tool_calls",
      "experimental_supported_tools", "input_modalities",
    ];
    const catalog = JSON.parse(codexCustomCatalogTemplate) as { models: Array<Record<string, unknown>> };
    for (const field of expected) {
      expect(catalog.models[0]).toHaveProperty(field);
    }
  });
});
import { balanceChipClass } from "./presets";

describe("balanceChipClass", () => {
  it("marks a negative balance as danger when usage is unavailable", () => {
    expect(balanceChipClass(null, "-1.00")).toBe("chip-danger");
  });

  it("keeps zero and positive balances successful", () => {
    expect(balanceChipClass(null, "0.00")).toBe("chip-success");
    expect(balanceChipClass(null, "110.00")).toBe("chip-success");
  });

  it("uses usage thresholds before the total balance", () => {
    expect(balanceChipClass(70, "110.00")).toBe("chip-warn");
    expect(balanceChipClass(90, "110.00")).toBe("chip-danger");
  });
});

describe("codexPresets 新增 responses 供应商", () => {
  it("内置 kimi / qwen / hunyuan / doubao 四条预设且 base_url 为官方端点", () => {
    const expected: Record<string, string> = {
      kimi: "https://api.moonshot.cn/v1",
      qwen: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
      hunyuan: "https://tokenhub.tencentmaas.com/v1",
      "tencent-token-plan-enterprise": "https://tokenhub.tencentmaas.com/plan/v3",
      doubao: "https://ark.cn-beijing.volces.com/api/coding/v3",
    };
    for (const [kind, base] of Object.entries(expected)) {
      const preset = codexPresets.find((p) => p.kind === kind);
      expect(preset, `缺少内置预设 ${kind}`).toBeDefined();
      expect(preset?.base_url).toBe(base);
      expect(preset?.model).toBeTruthy();
    }
  });
});

describe("codexPresets 区域与计费通道端点档", () => {
  it("仅官方确认的选项携带端点档，且首项与默认 base_url/admin_url 一致", () => {
    const dual = ["doubao", "doubao-api", "hunyuan", "kimi", "kimi-code", "mimo", "minimax", "qwen", "qwen-api", "tencent-token-plan-enterprise", "xai", "zhipu"];
    const carried = codexPresets.filter((p) => p.endpoints).map((p) => p.kind).sort();
    expect(carried, "只有官方文档确认的区域或计费通道才带端点档").toEqual(dual);

    for (const preset of codexPresets) {
      const endpoints = preset.endpoints ?? [];
      const urls = endpoints.map((ep) => ep.base_url);
      expect(new Set(urls).size, `${preset.kind} 端点档 URL 应互不相同`).toBe(urls.length);
      if (endpoints.length > 0) {
        expect(endpoints[0].base_url, `${preset.kind} 首档应为默认区域`).toBe(preset.base_url);
        expect(endpoints[0].admin_url, `${preset.kind} 首档控制台应为默认控制台`).toBe(preset.admin_url);
        // 同一区域可能有多条计费通道（label 区分），档位名不得为空串
        for (const ep of endpoints) {
          expect(ep.label === undefined || ep.label.length > 0, `${preset.kind} 档位名不得为空`).toBe(true);
        }
      }
    }
  });

  it("火山方舟提供国内双套餐与海外 BytePlus Coding Plan 档", () => {
    const doubao = codexPresets.find((p) => p.kind === "doubao");
    const endpoints = doubao?.endpoints ?? [];
    expect(endpoints.map((ep) => `${ep.region}/${ep.base_url}`)).toEqual([
      "cn/https://ark.cn-beijing.volces.com/api/coding/v3",
      "cn/https://ark.cn-beijing.volces.com/api/plan/v3",
      "global/https://ark.ap-southeast.bytepluses.com/api/coding/v3",
    ]);
    // 按量 /api/v3 不在档内（不消耗 Coding Plan 套餐额度）
    expect(doubao?.model).toBe("ark-code-latest");
    expect(doubao?.provider).toBe("volcengine-coding-plan");
  });
});

describe("claudePresets Claude Code 模板", () => {
  it("Kimi Code 提供国内和海外版本，保持独立套餐入口", () => {
    const entries = presetEntries(claudePresets).filter((entry) => entry.preset.kind === "kimi-code");
    expect(entries).toHaveLength(1);
    expect(entries[0].preset.endpoints.map(({ region, base_url }) => ({ region, base_url }))).toEqual([
      { region: "cn", base_url: "https://api.kimi.com/coding/" },
      { region: "global", base_url: "https://api.kimi.ai/coding/" },
    ]);
    expect(entries[0].preset.base_url).toBe(entries[0].preset.endpoints[0].base_url);
  });

  it("只预填接入信息，不预填 Claude 模型", () => {
    const expected: Record<string, string> = {
      "qwen-coding-plan": "https://coding.dashscope.aliyuncs.com/apps/anthropic",
      "tencent-token-plan": "https://api.lkeap.cloud.tencent.com/plan/anthropic",
      longcat: "https://api.longcat.chat/anthropic",
      "kimi-code": "https://api.kimi.com/coding/",
      siliconflow: "https://api.siliconflow.cn/",
      "qianfan-coding-plan": "https://qianfan.baidubce.com/anthropic/coding",
    };
    for (const [kind, baseUrl] of Object.entries(expected)) {
      const preset = claudePresets.find((item) => item.kind === kind);
      expect(preset, `缺少 Claude Code 模板 ${kind}`).toBeDefined();
      expect(preset).toMatchObject({ base_url: baseUrl });
    }
    // 全量兜底：所有 Claude 模板一律不带默认模型
    expect(claudePresets.every((preset) => preset.model === "")).toBe(true);
    const personalPlan = presetEntries(claudePresets).find((entry) => entry.preset.kind === "tencent-token-plan")!;
    expect(personalPlan.preset.endpoints.map(({ region, base_url }) => [region, base_url])).toEqual([
      ["cn", "https://api.lkeap.cloud.tencent.com/plan/anthropic"],
      ["global", "https://tokenhub-intl.tencentcloudmaas.com/plan/anthropic"],
    ]);
    expect(personalPlan.preset.endpoints[1].admin_url).toBe("https://console.tencentcloud.com/tokenhub/tokenplan");
  });
});
