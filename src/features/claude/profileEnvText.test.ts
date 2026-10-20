import { describe, expect, it } from "vitest";
import { patchBypassPermissions } from "./profileEnvText";
import { CLAUDE_MANAGED_ENV_KEYS, CLAUDE_MODEL_MAPPING_KEYS, CLAUDE_MODEL_DISPLAY_KEYS, extractClaudeCommonSettings, fillClaudeCommonSettings } from "./profileEnvText";
import { buildSettingsText, formatJsonText, hasOneMillionModelSuffix, patchEnvFields, patchEnvValue, patchGitAttribution, patchModelDisplayNames, patchModelMappings, readAdvancedSettings, readEnvFields, readEnvValue, readGitAttributionDisabled, readModelDisplayNames, readModelMappings, resolveOneMillionByDeclaration, setOneMillionModelSuffix, splitEnvExtras } from "./profileEnvText";

describe("排版按钮的四种结局", () => {
  it("空文本、坏 JSON、已排好、需要写回各自可辨", () => {
    expect(formatJsonText("   ").status).toBe("empty");
    expect(formatJsonText("{oops").status).toBe("failed");
    expect(formatJsonText('{\n  "a": 1\n}').status).toBe("unchanged");
    expect(formatJsonText('{"a":1}')).toEqual({ status: "formatted", text: '{\n  "a": 1\n}' });
  });
});

describe("Claude 通用模板", () => {
  it("默认权限模式不进入模板，权限规则和组织限制仍保留", () => {
    const permissions = { deny: ["WebFetch", "WebSearch"], allow: ["Read"], ask: ["Bash"], disableBypassPermissionsMode: "disable" };
    for (const defaultMode of ["bypassPermissions", "plan"]) {
      const template = JSON.stringify({ permissions: { ...permissions, defaultMode } });
      expect(JSON.parse(extractClaudeCommonSettings(template)!)).toEqual({ permissions });
      expect(JSON.parse(fillClaudeCommonSettings("{}", template)!)).toEqual({ permissions });
      expect(JSON.parse(fillClaudeCommonSettings('{"permissions":{"defaultMode":"plan"}}', template)!))
        .toEqual({ permissions: { defaultMode: "plan", ...permissions } });
    }
    expect(extractClaudeCommonSettings('{"permissions":{"defaultMode":"bypassPermissions"}}')).toBe("{}");
  });

  it("排除所有表单和开关键，保留 hooks、权限及未知字段", () => {
    const env = Object.fromEntries([
      ...CLAUDE_MANAGED_ENV_KEYS, ...CLAUDE_MODEL_MAPPING_KEYS, ...CLAUDE_MODEL_DISPLAY_KEYS,
      "DISABLE_AUTO_COMPACT", "CLAUDE_CODE_AUTO_COMPACT_WINDOW", "CLAUDE_CODE_ATTRIBUTION_HEADER",
      "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "CLAUDE_CODE_EFFORT_LEVEL",
    ].map((key) => [key, "managed"]));
    const common = {
      env: { CUSTOM: "keep", CLAUDE_USER_SETTING: "keep" },
      hooks: { Stop: [{ hooks: [{ type: "command", command: "echo fixture" }] }] },
      permissions: { allow: ["Read"] },
      attribution: { sessionUrl: "https://example.test/session" },
      model: "manual-top-level-model",
      unknown: { nested: [1, false, null] },
    };
    expect(JSON.parse(extractClaudeCommonSettings(JSON.stringify({
      ...common, env: { ...env, ...common.env },
      attribution: { ...common.attribution, commit: "", pr: "" }, includeCoAuthoredBy: false,
    }))!)).toEqual(common);
    expect(extractClaudeCommonSettings('{"env":{"ANTHROPIC_AUTH_TOKEN":"secret"},"attribution":{"commit":"","pr":""}}')).toBe("{}");
  });

  it("通用字段以模板为准，保留供应商字段与模板未声明项，重复填充不改动文本", () => {
    const current = {
      env: { ANTHROPIC_AUTH_TOKEN: "current-token", KEEP: "current" },
      permissions: { allow: [], deny: ["Write"] },
      hooks: { Stop: [{ hooks: [{ command: "current" }] }], PostToolUse: [{ matcher: "Write|Edit", hooks: [{ command: "lint" }] }] },
      enabledPlugins: { shared: false, local: true },
      nullable: null, disabled: false, empty: "",
    };
    const template = {
      env: { ANTHROPIC_AUTH_TOKEN: "template-token", DISABLE_AUTO_COMPACT: "1", KEEP: "template", EXTRA: "new" },
      attribution: { commit: "template", sessionUrl: "https://example.test/session" },
      permissions: { allow: ["Read"], ask: ["Bash"] },
      hooks: { Stop: [{ hooks: [{ command: "summary", timeout: 10 }] }], SessionStart: [],
        PostToolUse: [{ matcher: "Write|Edit", hooks: [{ command: "lint" }] }, { matcher: "Write|Edit|MultiEdit|NotebookEdit", hooks: [{ command: "count", timeout: 10 }] }] },
      enabledPlugins: { shared: true },
      nullable: { nested: true }, disabled: true, empty: "fallback", unknown: [1],
    };
    const filled = fillClaudeCommonSettings(JSON.stringify(current), JSON.stringify(template))!;
    expect(JSON.parse(filled)).toEqual({
      ...current, env: { ...current.env, KEEP: "template", EXTRA: "new" },
      attribution: { sessionUrl: "https://example.test/session" },
      permissions: { ...current.permissions, allow: ["Read"], ask: ["Bash"] },
      hooks: template.hooks, enabledPlugins: { shared: true, local: true },
      nullable: { nested: true }, disabled: true, empty: "fallback", unknown: [1],
    });
    expect(fillClaudeCommonSettings(filled, JSON.stringify(template))).toBe(filled);
    expect(fillClaudeCommonSettings("{\n  \"env\": {}\n}\n", "{}")).toBe("{\n  \"env\": {}\n}\n");
  });

  it("拒绝非法 JSON 和 env，不把无效草稿当作空配置", () => {
    for (const invalid of ["", "{", "null", "[]", '{"env":null}', '{"env":[]}', '{"env":{"KEY":42}}']) {
      expect(extractClaudeCommonSettings(invalid)).toBeNull();
      expect(fillClaudeCommonSettings(invalid, "{}")).toBeNull();
      expect(fillClaudeCommonSettings("{}", invalid)).toBeNull();
    }
  });

  it("未知对象键不读取或修改 JavaScript 原型", () => {
    const template = '{"__proto__":{"polluted":true},"constructor":{"custom":1},"toString":"keep"}';
    const filled = fillClaudeCommonSettings("{}", template)!;
    expect(JSON.parse(filled)).toEqual(JSON.parse(template));
    expect(Object.prototype).not.toHaveProperty("polluted");
    expect(fillClaudeCommonSettings(filled, template)).toBe(filled);
  });

  it("旧模板中的顶层开关和推理强度也不能进入配置，其他通用字段保留", () => {
    const template = JSON.stringify({
      autoCompactEnabled: false, autoCompactWindow: 500000, effortLevel: "high",
      attribution: false, includeCoAuthoredBy: false,
      modelSettings: { "fixture-model": { effortLevel: "low" } },
      hooks: { Stop: [] }, env: { KEEP: "keep", CLAUDE_CODE_EFFORT_LEVEL: "max" },
    });
    const common = { modelSettings: { "fixture-model": { effortLevel: "low" } }, hooks: { Stop: [] }, env: { KEEP: "keep" } };
    expect(JSON.parse(extractClaudeCommonSettings(template)!)).toEqual(common);
    expect(JSON.parse(fillClaudeCommonSettings("{}", template)!)).toEqual(common);
    const current = '{"autoCompactEnabled":true,"autoCompactWindow":200000,"effortLevel":"low","attribution":false}';
    expect(JSON.parse(fillClaudeCommonSettings(current, template)!)).toEqual({ ...JSON.parse(current), ...common });
  });

  it("更多中的开关不进入通用模板，也不从旧模板补入配置", () => {
    const template = JSON.stringify({ autoMemoryEnabled: false, bashEditDiffEnabled: true,
      env: { CLAUDE_CODE_DISABLE_AUTO_MEMORY: "1", CLAUDE_CODE_BASH_EDIT_DIFF: "1", KEEP: "keep" },
      autoMemoryDirectory: "/fixture/memory", permissions: { deny: ["WebFetch"] }, hooks: { Stop: [] },
    });
    const common = { env: { KEEP: "keep" }, autoMemoryDirectory: "/fixture/memory", permissions: { deny: ["WebFetch"] }, hooks: { Stop: [] } };
    expect(JSON.parse(extractClaudeCommonSettings(template)!)).toEqual(common);
    expect(JSON.parse(fillClaudeCommonSettings("{}", template)!)).toEqual(common);
    const current = '{"autoMemoryEnabled":true,"env":{"CLAUDE_CODE_DISABLE_AUTO_MEMORY":"0"}}';
    expect(readAdvancedSettings(fillClaudeCommonSettings(current, template)!)).toMatchObject({ autoMemoryEnabled: true });
    expect(readAdvancedSettings(fillClaudeCommonSettings(current, template)!).bashEditDiffEnabled).toBe(false);
  });
});

describe("Claude 默认跳过权限确认", () => {
  it("可回显和切换，仅修改默认模式，保留用户规则与组织限制", () => {
    const current = {
      permissions: { defaultMode: "plan", deny: ["WebFetch", "WebSearch"], disableBypassPermissionsMode: "disable" },
      hooks: { Stop: [] }, env: { KEEP: "keep" },
    };
    const raw = JSON.stringify(current);
    expect(readAdvancedSettings("{}").bypassPermissionsEnabled).toBe(false);
    expect(readAdvancedSettings(raw).bypassPermissionsEnabled).toBe(false);
    expect(patchBypassPermissions(raw, false)).toBe(raw);
    const enabled = patchBypassPermissions(raw, true);
    expect(readAdvancedSettings(enabled).bypassPermissionsEnabled).toBe(true);
    expect(JSON.parse(enabled)).toEqual({ ...current, permissions: { ...current.permissions, defaultMode: "bypassPermissions" } });
    expect(patchBypassPermissions(enabled, true)).toBe(enabled);
    const disabled = patchBypassPermissions(enabled, false);
    expect(readAdvancedSettings(disabled).bypassPermissionsEnabled).toBe(false);
    expect(JSON.parse(disabled)).toEqual({ ...current, permissions: { deny: ["WebFetch", "WebSearch"], disableBypassPermissionsMode: "disable" } });
    expect(JSON.parse(patchBypassPermissions(patchBypassPermissions("{}", true), false))).toEqual({});
    for (const invalid of ["{", "null", "[]", '{"permissions":null}', '{"permissions":[]}', '{"permissions":"plan"}']) {
      expect(patchBypassPermissions(invalid, true)).toBe(invalid);
      expect(patchBypassPermissions(invalid, false)).toBe(invalid);
      expect(readAdvancedSettings(invalid).bypassPermissionsEnabled).toBe(false);
    }
  });
});

describe("settings.json 全文与表单同步", () => {
  it("清空全文时表单字段归零，未写完的 JSON 保持原值", () => {
    expect(readEnvFields(" \n ")).toEqual({ baseUrl: "", authToken: "", model: "" });
    expect(readEnvFields("{")).toBeNull();
  });

  it("清空后重新填写表单会生成 settings.json，未写完的 JSON 不被覆盖", () => {
    const fields = { baseUrl: "https://api.example.test", authToken: "token", model: "model-a" };
    const text = patchEnvFields("", fields);
    expect(readEnvFields(text)).toEqual(fields);
    const mapped = patchModelMappings("", { ...readModelMappings(""), ANTHROPIC_DEFAULT_OPUS_MODEL: "model-b" });
    expect(readModelMappings(mapped).ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("model-b");
    expect(patchEnvFields("{", fields)).toBe("{");
  });

  it("官方 API 使用 API Key 回显，账号模式撤下认证覆盖并保留普通设置", () => {
    const raw = JSON.stringify({ apiKeyHelper: "fixture-helper", hooks: { Stop: [] }, env: {
      ANTHROPIC_AUTH_TOKEN: "relay-token", CLAUDE_CODE_OAUTH_TOKEN: "override", CLAUDE_CODE_USE_VERTEX: "1", KEEP: "keep",
    } });
    const fields = { baseUrl: "https://api.example.test", authToken: "api-key", model: "sonnet" };
    expect(readEnvFields(raw, "anthropic")?.authToken).toBe("relay-token");
    const api = patchEnvFields(raw, fields, "anthropic");
    expect(readEnvFields(api, "anthropic")).toEqual(fields);
    expect(JSON.parse(api).env.ANTHROPIC_AUTH_TOKEN).toBeUndefined();
    expect(JSON.parse(extractClaudeCommonSettings(api)!).env.ANTHROPIC_API_KEY).toBeUndefined();
    const account = patchEnvFields(api, fields, "claude-account");
    expect(JSON.parse(account)).toEqual({ hooks: { Stop: [] }, env: { ANTHROPIC_MODEL: "sonnet", KEEP: "keep" } });
    expect(patchEnvFields(account, fields, "claude-account")).toBe(account);
    expect(patchEnvFields('{', fields, "claude-account")).toBe('{');
    expect(JSON.parse(patchEnvFields('{"apiKeyHelper":"fixture-helper"}', { baseUrl: "", authToken: "", model: "" }, "claude-account"))).toEqual({ env: {} });
  });

  it("Shell 文件改动开关回显遵循 env 优先，切换保留其他配置", () => {
    expect(readAdvancedSettings("{}").bashEditDiffEnabled).toBe(false);
    expect(readAdvancedSettings('{"bashEditDiffEnabled":true}').bashEditDiffEnabled).toBe(true);
    expect(readAdvancedSettings('{"bashEditDiffEnabled":true,"env":{"CLAUDE_CODE_BASH_EDIT_DIFF":"0"}}').bashEditDiffEnabled).toBe(false);
    expect(readAdvancedSettings('{"bashEditDiffEnabled":false,"env":{"CLAUDE_CODE_BASH_EDIT_DIFF":"1"}}').bashEditDiffEnabled).toBe(true);
    const raw = '{"bashEditDiffEnabled":false,"hooks":{"Stop":[]},"permissions":{"deny":["WebFetch"]},"env":{"KEEP":"keep"}}';
    const enabled = patchEnvValue(raw, "CLAUDE_CODE_BASH_EDIT_DIFF", "1", "bashEditDiffEnabled");
    expect(readAdvancedSettings(enabled).bashEditDiffEnabled).toBe(true);
    const disabled = patchEnvValue(enabled, "CLAUDE_CODE_BASH_EDIT_DIFF", null, "bashEditDiffEnabled");
    expect(readAdvancedSettings(disabled).bashEditDiffEnabled).toBe(false);
    expect(JSON.parse(disabled)).toEqual({ hooks: { Stop: [] }, permissions: { deny: ["WebFetch"] }, env: { KEEP: "keep" } });
  });

  const raw = '{\n  "model": "opus",\n  "permissions": { "allow": ["Read"] },\n  "env": { "ANTHROPIC_BASE_URL": "https://a.example", "CUSTOM": "keep" }\n}\n';

  it("快照原文完整回显，表单读取嵌套 env", () => {
    expect(buildSettingsText({ raw_settings: raw, base_url: null, auth_token: null, model: null, extra_env: null })).toBe(raw);
    expect(readEnvFields(raw)).toEqual({ baseUrl: "https://a.example", authToken: "", model: "" });
  });

  it("表单仅修改 env，保留顶层配置与其他 env 键", () => {
    expect(patchEnvFields(raw, { baseUrl: "https://a.example", authToken: "", model: "" })).toBe(raw);
    const patched = patchEnvFields(raw, { baseUrl: "https://b.example", authToken: "tok", model: "" });
    expect(JSON.parse(patched)).toEqual({
      model: "opus", permissions: { allow: ["Read"] },
      env: { ANTHROPIC_BASE_URL: "https://b.example", ANTHROPIC_AUTH_TOKEN: "tok", CUSTOM: "keep" },
    });
    expect(readEnvFields(patched)).toEqual({ baseUrl: "https://b.example", authToken: "tok", model: "" });
  });

  it("按 Claude Code 模板切换 API_KEY、AUTH_TOKEN 与 OpenRouter 空 API_KEY", () => {
    const fields = { baseUrl: "https://provider.example", authToken: "tok", model: "model" };
    for (const kind of ["kimi-code", "siliconflow"]) {
      const patched = patchEnvFields('{"env":{"ANTHROPIC_AUTH_TOKEN":"stale"}}', fields, kind);
      expect(JSON.parse(patched).env).toMatchObject({ ANTHROPIC_API_KEY: "tok" });
      expect(JSON.parse(patched).env.ANTHROPIC_AUTH_TOKEN).toBeUndefined();
    }
    const token = patchEnvFields('{"env":{"ANTHROPIC_API_KEY":"stale"}}', fields, "qwen-coding-plan");
    expect(JSON.parse(token).env).toMatchObject({ ANTHROPIC_AUTH_TOKEN: "tok" });
    expect(JSON.parse(token).env.ANTHROPIC_API_KEY).toBeUndefined();
    const openrouter = patchEnvFields("{}", fields, "openrouter");
    expect(JSON.parse(openrouter).env.ANTHROPIC_API_KEY).toBe("");
    expect(splitEnvExtras(openrouter)).toBeNull();
  });

  it("预设模型映射与表单字段一起同步后保持稳定", () => {
    const mappings = readModelMappings(JSON.stringify({ env: {
      ANTHROPIC_MODEL: "LongCat-2.5-Preview",
      ANTHROPIC_DEFAULT_FABLE_MODEL: "LongCat-2.5-Preview",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "LongCat-2.5-Preview",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "LongCat-2.5-Preview",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "LongCat-2.5-Preview",
      CLAUDE_CODE_SUBAGENT_MODEL: "LongCat-2.5-Preview",
    } }));
    const initial = patchEnvFields("{}", { baseUrl: "https://api.longcat.chat/anthropic", authToken: "", model: "LongCat-2.5-Preview" }, "longcat");
    const withMappings = patchModelMappings(initial, mappings);
    const fields = readEnvFields(withMappings, "longcat");
    expect(fields).not.toBeNull();
    const resynced = patchModelMappings(patchEnvFields(withMappings, fields!, "longcat"), mappings);
    expect(resynced).toBe(withMappings);
  });

  it("旧快照构造完整文件，保存拒绝无效 env", () => {
    expect(JSON.parse(buildSettingsText({ raw_settings: null, base_url: "https://a.example", auth_token: "tok", model: null, extra_env: '{"CUSTOM":"keep"}' }))).toEqual({
      env: { CUSTOM: "keep", ANTHROPIC_BASE_URL: "https://a.example", ANTHROPIC_AUTH_TOKEN: "tok" },
    });
    expect(JSON.parse(splitEnvExtras(raw)!)).toEqual({ CUSTOM: "keep" });
    expect(splitEnvExtras('{"model":"opus"}')).toBeNull();
    expect(splitEnvExtras('{"env":{"ANTHROPIC_MODEL":42}}')).toBeUndefined();
    expect(splitEnvExtras('{"env":[]}')).toBeUndefined();
    expect(splitEnvExtras("not json {")).toBeUndefined();
  });

  it("读取并保存 Claude Code 的多模型映射，保留其他 settings 内容", () => {
    const mappings = readModelMappings('{"permissions":{"allow":["Read"]},"env":{"ANTHROPIC_MODEL":"main","ANTHROPIC_DEFAULT_OPUS_MODEL":"opus-gateway","CLAUDE_CODE_SUBAGENT_MODEL":"haiku-gateway","IGNORED":"keep"}}');
    expect(mappings).toMatchObject({
      ANTHROPIC_MODEL: "main",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "opus-gateway",
      CLAUDE_CODE_SUBAGENT_MODEL: "haiku-gateway",
    });
    const patched = patchModelMappings('{"permissions":{"allow":["Read"]},"env":{"ANTHROPIC_MODEL":"main","IGNORED":"keep"}}', {
      ...mappings,
      ANTHROPIC_MODEL: "new-main",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "haiku-gateway",
    });
    expect(JSON.parse(patched)).toEqual({
      permissions: { allow: ["Read"] },
      env: {
        ANTHROPIC_MODEL: "new-main",
        ANTHROPIC_DEFAULT_HAIKU_MODEL: "haiku-gateway",
        ANTHROPIC_DEFAULT_OPUS_MODEL: "opus-gateway",
        CLAUDE_CODE_SUBAGENT_MODEL: "haiku-gateway",
        IGNORED: "keep",
      },
    });
  });

  it("给每个模型 ID 添加或移除 Claude Code 的 1M 后缀", () => {
    expect(hasOneMillionModelSuffix("glm-5.3-flash[1m]")).toBe(true);
    expect(setOneMillionModelSuffix("glm-5.3-flash", true)).toBe("glm-5.3-flash[1M]");
    expect(setOneMillionModelSuffix("glm-5.3-flash[1m]", false)).toBe("glm-5.3-flash");
    expect(setOneMillionModelSuffix("", true)).toBe("");
  });

  it("卡片选主会话模型按本配置既有声明决定 1M 后缀", () => {
    const text = '{"env":{"ANTHROPIC_MODEL":"glm-5.3[1M]","ANTHROPIC_DEFAULT_OPUS_MODEL":"m3[1M]","ANTHROPIC_DEFAULT_SONNET_MODEL":"glm-5.3-flash"}}';
    expect(resolveOneMillionByDeclaration(text, "m3")).toBe("m3[1M]");
    expect(resolveOneMillionByDeclaration(text, "glm-5.3")).toBe("glm-5.3[1M]");
    expect(resolveOneMillionByDeclaration(text, "glm-5.3-flash")).toBe("glm-5.3-flash");
    expect(resolveOneMillionByDeclaration(text, "glm-4.6")).toBe("glm-4.6");
    expect(resolveOneMillionByDeclaration(text, "")).toBe("");
    expect(resolveOneMillionByDeclaration("not json", "m3")).toBe("m3");
  });

  it("读取和保存模型选择器显示名称，保留其他 env", () => {
    const raw = '{"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"sonnet-gateway","ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"Sonnet 网关","KEEP":"keep"}}';
    const names = readModelDisplayNames(raw);
    expect(names.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("Sonnet 网关");
    const patched = patchModelDisplayNames(raw, { ...names, ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "Sonnet 自定义" });
    expect(JSON.parse(patched)).toEqual({
      env: { ANTHROPIC_DEFAULT_SONNET_MODEL: "sonnet-gateway", ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "Sonnet 自定义", KEEP: "keep" },
    });
  });

  it("读取和保存 Claude Code 高级 env 字段，保留其他设置", () => {
    const raw = '{"env":{"DISABLE_AUTO_COMPACT":"1","KEEP":"keep"}}';
    expect(readEnvValue(raw, "DISABLE_AUTO_COMPACT")).toBe("1");
    const enabled = patchEnvValue(raw, "DISABLE_AUTO_COMPACT", null);
    expect(JSON.parse(enabled)).toEqual({ env: { KEEP: "keep" } });
    const configured = patchEnvValue(enabled, "CLAUDE_CODE_EFFORT_LEVEL", "max");
    expect(JSON.parse(configured)).toEqual({ env: { KEEP: "keep", CLAUDE_CODE_EFFORT_LEVEL: "max" } });
  });

  it("按官方优先级回显顶层自动压缩设置，切换和清空时移除冲突写法", () => {
    const raw = '{"autoCompactEnabled":false,"autoCompactWindow":500000,"hooks":{"Stop":[]},"env":{"KEEP":"keep"}}';
    expect(readAdvancedSettings(raw)).toMatchObject({ autoCompactDisabled: true, autoCompactWindow: "500000" });
    expect(readAdvancedSettings('{"autoCompactEnabled":false,"env":{"DISABLE_AUTO_COMPACT":"0"}}').autoCompactDisabled).toBe(true);
    expect(readAdvancedSettings('{"autoCompactEnabled":true,"autoCompactWindow":500000,"env":{"DISABLE_AUTO_COMPACT":"1","CLAUDE_CODE_AUTO_COMPACT_WINDOW":"200000"}}'))
      .toMatchObject({ autoCompactDisabled: true, autoCompactWindow: "200000" });
    const enabled = patchEnvValue(raw, "DISABLE_AUTO_COMPACT", null, "autoCompactEnabled");
    expect(readAdvancedSettings(enabled).autoCompactDisabled).toBe(false);
    const configured = patchEnvValue(enabled, "CLAUDE_CODE_AUTO_COMPACT_WINDOW", "300000", "autoCompactWindow");
    expect(readAdvancedSettings(configured).autoCompactWindow).toBe("300000");
    expect(JSON.parse(configured)).not.toHaveProperty("autoCompactWindow");
    const cleared = patchEnvValue(configured, "CLAUDE_CODE_AUTO_COMPACT_WINDOW", null, "autoCompactWindow");
    expect(JSON.parse(cleared)).toEqual({ hooks: { Stop: [] }, env: { KEEP: "keep" } });
    expect(readAdvancedSettings(cleared).autoCompactWindow).toBe("");
  });

  it("推理强度优先读取 env，支持各档位并清除顶层强制值", () => {
    const raw = '{"effortLevel":"high","modelSettings":{"fixture-model":{"effortLevel":"low"}},"env":{"KEEP":"keep"}}';
    expect(readAdvancedSettings(raw).effortLevel).toBe("high");
    for (const level of ["low", "medium", "high", "xhigh", "max"]) {
      const configured = patchEnvValue(raw, "CLAUDE_CODE_EFFORT_LEVEL", level, "effortLevel");
      expect(readAdvancedSettings(configured).effortLevel).toBe(level);
      expect(JSON.parse(configured)).not.toHaveProperty("effortLevel");
      expect(JSON.parse(configured).modelSettings).toEqual(JSON.parse(raw).modelSettings);
      expect(JSON.parse(configured).env.KEEP).toBe("keep");
    }
    const cleared = patchEnvValue(raw, "CLAUDE_CODE_EFFORT_LEVEL", null, "effortLevel");
    expect(readAdvancedSettings(cleared).effortLevel).toBe("");
    expect(readAdvancedSettings('{"effortLevel":"high","env":{"CLAUDE_CODE_EFFORT_LEVEL":"auto"}}').effortLevel).toBe("");
    expect(readAdvancedSettings('{"effortLevel":"high","env":{"CLAUDE_CODE_EFFORT_LEVEL":"future-level"}}').effortLevel).toBe("future-level");
    expect(patchEnvValue("{", "CLAUDE_CODE_EFFORT_LEVEL", "max", "effortLevel")).toBe("{");
    expect(patchEnvValue('{"env":[]}', "DISABLE_AUTO_COMPACT", null, "autoCompactEnabled")).toBe('{"env":[]}');
  });

  it("自动记忆默认开启，env 覆盖顶层设置，切换保留用户数据", () => {
    expect(readAdvancedSettings("{}").autoMemoryEnabled).toBe(true);
    expect(readAdvancedSettings('{"autoMemoryEnabled":false}').autoMemoryEnabled).toBe(false);
    expect(readAdvancedSettings('{"autoMemoryEnabled":false,"env":{"CLAUDE_CODE_DISABLE_AUTO_MEMORY":"0"}}').autoMemoryEnabled).toBe(true);
    expect(readAdvancedSettings('{"autoMemoryEnabled":true,"env":{"CLAUDE_CODE_DISABLE_AUTO_MEMORY":"1"}}').autoMemoryEnabled).toBe(false);
    const raw = '{"autoMemoryEnabled":false,"autoMemoryDirectory":"/fixture/memory","hooks":{"Stop":[]},"env":{"KEEP":"keep"}}';
    const enabled = patchEnvValue(raw, "CLAUDE_CODE_DISABLE_AUTO_MEMORY", null, "autoMemoryEnabled");
    expect(readAdvancedSettings(enabled).autoMemoryEnabled).toBe(true);
    const disabled = patchEnvValue(enabled, "CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1", "autoMemoryEnabled");
    expect(readAdvancedSettings(disabled).autoMemoryEnabled).toBe(false);
    expect(JSON.parse(patchEnvValue(disabled, "CLAUDE_CODE_DISABLE_AUTO_MEMORY", null, "autoMemoryEnabled"))).toEqual({
      autoMemoryDirectory: "/fixture/memory", hooks: { Stop: [] }, env: { KEEP: "keep" },
    });
  });

  it("识别新版 attribution:false，并能恢复为兼容旧版的对象形式", () => {
    const raw = '{"attribution":false,"hooks":{"Stop":[]},"env":{"KEEP":"keep"}}';
    expect(readGitAttributionDisabled(raw)).toBe(true);
    const hidden = patchGitAttribution(raw, true);
    expect(JSON.parse(hidden)).toEqual({ attribution: { sessionUrl: false, commit: "", pr: "" }, hooks: { Stop: [] }, env: { KEEP: "keep" } });
    expect(JSON.parse(patchGitAttribution(raw, false))).toEqual({ hooks: { Stop: [] }, env: { KEEP: "keep" } });
    expect(readGitAttributionDisabled('{"attribution":{"sessionUrl":false},"includeCoAuthoredBy":false}')).toBe(true);
    expect(readGitAttributionDisabled('{"attribution":{"commit":"custom","pr":""},"includeCoAuthoredBy":false}')).toBe(false);
  });

  it("使用官方顶层 attribution 关闭提交和 PR 归属，并保留其他字段", () => {
    const raw = '{"model":"opus","attribution":{"sessionUrl":"https://claude.ai/code/session-x"},"env":{"KEEP":"keep"}}';
    expect(readGitAttributionDisabled(raw)).toBe(false);
    const disabled = patchGitAttribution(raw, true);
    expect(JSON.parse(disabled)).toEqual({
      model: "opus",
      attribution: { sessionUrl: "https://claude.ai/code/session-x", commit: "", pr: "" },
      env: { KEEP: "keep" },
    });
    expect(readGitAttributionDisabled(disabled)).toBe(true);
    expect(JSON.parse(patchGitAttribution(disabled, false))).toEqual({
      model: "opus",
      attribution: { sessionUrl: "https://claude.ai/code/session-x" },
      env: { KEEP: "keep" },
    });
  });

});
