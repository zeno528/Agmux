import { describe, expect, it } from "vitest";
import { chatgptModelOptions, patchModelValue, patchProviderFields, readCatalogOptions, readModelValue, readProviderFields, resolveAuthSource, withMcpSection } from "./profileEditText";

it("远程模型和账号缓存保留顺序、空档位以及默认档位兜底", () => {
  expect(chatgptModelOptions([
    { slug: "second", display_name: "Second", supported_reasoning_levels: [], default_reasoning_level: null },
    { slug: "first", display_name: "First", supported_reasoning_levels: [{ effort: "low" }], default_reasoning_level: "low" },
  ])).toEqual({ models: ["second", "first"], efforts: { second: [], first: ["low"] }, defaults: { second: "none", first: "low" } });
  expect(chatgptModelOptions([])).toEqual({ models: [], efforts: {}, defaults: {} });
});

describe("model value read/patch", () => {
  it("reads and patches effort only at the top level, including configs without a provider", () => {
    const source = '[profiles.fixture]\nmodel = "nested"\nmodel_reasoning_effort = "high"\n';
    expect(readModelValue(source)).toBeNull();
    expect(readModelValue(source, "model_reasoning_effort")).toBeNull();
    const patched = patchModelValue(source, "low", "model_reasoning_effort");
    expect(patched).toBe('model_reasoning_effort = "low"\n' + source);
    expect(readModelValue(patched, "model_reasoning_effort")).toBe("low");
    expect(patchModelValue(patched, "", "model_reasoning_effort")).toBe(source);
  });

  it("uses advertised catalog efforts, preserving models with empty or unknown capabilities", () => {
    const catalog = JSON.stringify({ models: [
      { slug: "fixture", supported_reasoning_levels: [{ effort: "low" }, { effort: "max" }, { effort: "max" }] },
      { slug: "no-reasoning", supported_reasoning_levels: [] },
      { slug: "unknown" }, { slug: "__proto__", supported_reasoning_levels: [{ effort: "high" }] },
    ] });
    const { efforts } = readCatalogOptions(catalog);
    expect(efforts.fixture).toEqual(["low", "max"]);
    expect(efforts["no-reasoning"]).toEqual([]);
    expect(Object.keys(efforts)).toContain("unknown");
    expect(efforts.unknown).toBeUndefined();
    expect(efforts.__proto__).toEqual(["high"]);
    expect(readCatalogOptions(null).efforts.toString).toBeUndefined();
    expect(() => readCatalogOptions("{")).toThrow();
  });
  it("自定义目录按优先级提供可见模型和默认档位，保留隐藏模型的显式档位", () => {
    const options = readCatalogOptions(JSON.stringify({ models: [
      { slug: "second", priority: 2, default_reasoning_level: "high" },
      { slug: "hidden", priority: 0, visibility: "hide", supported_reasoning_levels: [{ effort: "low" }] },
      { slug: "first", priority: 1, visibility: "list", default_reasoning_level: "low" },
    ] }));
    expect(options.models).toEqual(["first", "second"]);
    expect(options.defaults.first).toBe("low");
    expect(options.efforts.hidden).toEqual(["low"]);
  });
  const source = [
    'model = "glm-5.2"',
    'model_provider = "ZAI"',
    'model_reasoning_effort = "high"',
    "",
    "[model_providers.ZAI]",
    'name = "ZAI"',
  ].join("\n");

  it("reads the top-level model line without touching model_* keys", () => {
    expect(readModelValue(source)).toBe("glm-5.2");
    expect(readModelValue('model_reasoning_effort = "high"\n')).toBeNull();
    expect(readModelValue("model_provider = \"x\"\n")).toBeNull();
  });

  it("patches the model line in place preserving indentation", () => {
    const patched = patchModelValue(source, "glm-5.3");
    expect(patched).toContain('model = "glm-5.3"');
    expect(patched).toContain('model_provider = "ZAI"');
    expect(patched).toContain('model_reasoning_effort = "high"');
  });

  it("inserts a model line after model_provider when missing", () => {
    const noModel = 'model_provider = "ZAI"\nmodel_reasoning_effort = "high"\n';
    const patched = patchModelValue(noModel, "glm-5.2");
    expect(patched).toBe('model_provider = "ZAI"\nmodel = "glm-5.2"\nmodel_reasoning_effort = "high"\n');
  });

  it("round-trips through the editor unchanged when values agree", () => {
    // 表单 → 编辑器 no-op：同值重写产出相同文本（防双向同步死循环）
    expect(patchModelValue(source, "glm-5.2")).toBe(source);
  });

  it("removes the model line when the form value is empty", () => {
    const patched = patchModelValue(source, "");
    expect(patched).not.toContain("model =");
    expect(patched).toContain('model_provider = "ZAI"');
  });

  it("replaces the existing model line when model_provider comes first", () => {
    // 回归：智谱的配置 model_provider 在 model 之前，旧实现逐行扫描先命中
    // provider 就插入新行，不去替换后面已有的 model 行 → 每次切换多出一行
    // model（TOML 重复键非法）
    const providerFirst = [
      'model_provider = "ZAI"',
      'model = "glm-5.3"',
      'model_reasoning_effort = "max"',
    ].join("\n");
    const patched = patchModelValue(providerFirst, "glm-5");
    expect(patched).toBe([
      'model_provider = "ZAI"',
      'model = "glm-5"',
      'model_reasoning_effort = "max"',
    ].join("\n"));
  });

  it("collapses duplicate model lines left by the old defect", () => {
    const polluted = [
      'model_provider = "ZAI"',
      'model = "glm-5"',
      'model = "glm-5-turbo"',
      'model = "glm-5.3"',
      'model_reasoning_effort = "high"',
    ].join("\n");
    const patched = patchModelValue(polluted, "glm-5.3");
    expect(patched.match(/^model = /gm)?.length).toBe(1);
    expect(patched).toContain('model = "glm-5.3"');
    expect(patched).toContain('model_provider = "ZAI"');
  });
});

describe("resolveAuthSource", () => {
  it("keeps the backend's explicit null for third-party profiles", () => {
    // 回归：`??` 会把 null 吞成 "desktop"，导致第三方档被当成带认证处理
    expect(resolveAuthSource({ auth_source: null, provider: "deepseek" })).toBeNull();
  });

  it("returns desktop and oauth unchanged when present", () => {
    expect(resolveAuthSource({ auth_source: "desktop", provider: null })).toBe("desktop");
    expect(resolveAuthSource({ auth_source: "oauth", provider: null, account_id: "a1" })).toBe("oauth");
  });

  it("infers only when the field is missing (legacy data)", () => {
    expect(resolveAuthSource({ provider: null })).toBe("desktop");
    expect(resolveAuthSource({ account_id: "a1", provider: null })).toBe("oauth");
    expect(resolveAuthSource({ provider: "deepseek" })).toBeNull();
    expect(resolveAuthSource(null)).toBeNull();
  });
});

describe("profileEditText", () => {
  it("reads the provider selected by model_provider", () => {
    const text = [
      'model_provider = "target"',
      "",
      "[model_providers.other]",
      'base_url = "https://other.example"',
      "",
      "[model_providers.target]",
      'base_url = "https://target.example"',
      'experimental_bearer_token = "secret"',
    ].join("\n");

    expect(readProviderFields(text)).toEqual({
      base_url: "https://target.example",
      experimental_bearer_token: "secret",
      found: true,
      tokenMasked: false,
    });
  });

  it("marks redacted provider tokens so the form keeps its current key", () => {
    const text = '[model_providers.target]\nexperimental_bearer_token = "••••••••"';

    expect(readProviderFields(text)).toMatchObject({
      experimental_bearer_token: "••••••••",
      found: true,
      tokenMasked: true,
    });
  });

  it("reads an escaped key without doubling it when the form writes back", () => {
    const text = [
      'model_provider = "custom"',
      "[model_providers.custom]",
      'base_url = "https://example.com"',
      'experimental_bearer_token = "\\\\"',
    ].join("\n");

    const fields = readProviderFields(text);
    expect(fields.experimental_bearer_token).toBe("\\");
    expect(patchProviderFields(text, fields.base_url, fields.experimental_bearer_token)).toBe(text);
  });

  it("keeps a broken key line editable and repairs that line from the form", () => {
    const text = [
      'model_provider = "custom"',
      "[model_providers.custom]",
      'base_url = "https://example.com"',
      'experimental_bearer_token = "broken',
    ].join("\n");

    expect(readProviderFields(text).found).toBe(false);
    expect(patchProviderFields(text, "https://example.com", "new-key")).toBe(
      text.replace('experimental_bearer_token = "broken', 'experimental_bearer_token = "new-key"'),
    );
  });

  it("patches modeled fields while preserving unmodeled lines", () => {
    const text = [
      'model_provider = "target"',
      "",
      "[model_providers.target]",
      "# keep this comment",
      'wire_api = "responses"',
      'base_url = "https://old.example"',
      "",
      "[other]",
      'value = "unchanged"',
    ].join("\n");

    const patched = patchProviderFields(text, "https://new.example", "new-secret");
    expect(patched).toContain(
      '# keep this comment\nwire_api = "responses"\nbase_url = "https://new.example"\n\nexperimental_bearer_token = "new-secret"',
    );
    expect(patched).toContain('[other]\nvalue = "unchanged"');
  });

  it("does not clear fields when the referenced provider is unmatched", () => {
    const text = 'model_provider = "missing"\n\n[model_providers.other]\nbase_url = "https://other.example"';
    expect(readProviderFields(text).found).toBe(false);
    expect(patchProviderFields(text, "https://new.example", "new-secret")).toBe(text);
  });

  it("appends a normalized MCP section only when present", () => {
    expect(withMcpSection("base = true\n", '[mcp_servers.demo]\nurl = "https://x"'))
      .toBe('base = true\n\n[mcp_servers.demo]\nurl = "https://x"\n');
    expect(withMcpSection("base = true\n", "")).toBe("base = true\n");
  });
});
