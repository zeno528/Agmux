import type { ChatgptModel } from "../../types";

/** 远程获取和账号缓存共用同一份模型选项转换。 */
export function chatgptModelOptions(catalog: ChatgptModel[]) {
  return {
    models: catalog.map((model) => model.slug),
    efforts: Object.fromEntries(catalog.map((model) => [model.slug, model.supported_reasoning_levels.map((level) => level.effort)])),
    defaults: Object.fromEntries(catalog.map((model) => [model.slug, model.default_reasoning_level ?? "none"])),
  };
}

export interface ProviderFields {
  base_url: string;
  experimental_bearer_token: string;
  found: boolean;
  tokenMasked: boolean;
}

export type AuthSourceValue = "desktop" | "oauth" | null;

/** 模型目录广告的档位优先于客户端通用选项；空数组表示该模型不支持推理档位。 */
export function readCatalogOptions(text: string | null) {
  const models: string[] = [];
  const efforts: Record<string, string[] | undefined> = Object.create(null);
  const defaults: Record<string, string> = Object.create(null);
  const options = { models, efforts, defaults };
  if (!text?.trim()) return options;
  const catalog = JSON.parse(text);
  if (!Array.isArray(catalog?.models)) return options;
  const entries = catalog.models.filter((model: { slug?: unknown } | null) => typeof model?.slug === "string");
  entries.sort((a: { priority?: number }, b: { priority?: number }) => (a.priority ?? 0) - (b.priority ?? 0));
  for (const model of entries) {
    if (!model.slug.trim()) continue;
    if (!model.visibility || model.visibility === "list") models.push(model.slug);
    defaults[model.slug] = typeof model.default_reasoning_level === "string" ? model.default_reasoning_level : "";
    efforts[model.slug] = undefined;
    if (Array.isArray(model.supported_reasoning_levels)) {
      efforts[model.slug] = [...new Set<string>(model.supported_reasoning_levels
        .map((level: { effort?: unknown } | null) => level?.effort)
        .filter((level: unknown): level is string => typeof level === "string" && !!level))];
    }
  }
  return options;
}

/**
 * auth_source 三态解析：后端对第三方档明确返回 null（无认证语义）。
 * 只允许在字段缺失（undefined，旧数据）时推断；`??` 会把 null 一并吞成
 * "desktop"，导致第三方档被当成带认证处理。
 */
export function resolveAuthSource(
  detail: {
    auth_source?: AuthSourceValue;
    account_id?: string | null;
    provider?: string | null;
  } | null,
): AuthSourceValue {
  if (detail?.auth_source !== undefined) return detail.auth_source;
  if (detail?.account_id) return "oauth";
  return detail != null && detail.provider === null ? "desktop" : null;
}

// 读取 [model_providers.*] 段里的 base_url / 密钥，供编辑器回填表单。
export function readProviderFields(text: string): ProviderFields {
  const values: ProviderFields = {
    base_url: "",
    experimental_bearer_token: "",
    found: false,
    tokenMasked: false,
  };
  const lines = text.split("\n");
  let providerId: string | null = null;
  for (const line of lines) {
    const match = /^model_provider\s*=\s*"([^"]+)"/.exec(line.trim());
    if (match) {
      providerId = match[1];
      break;
    }
  }
  let inProvider = false;
  let done = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (/^\[.+\]$/.test(trimmed)) {
      const section = /^\[model_providers\.(.+)\]$/.exec(trimmed);
      if (section && !done) {
        // 只处理 model_provider 指向的段；无 model_provider 时退化为第一段。
        inProvider = providerId === null || section[1] === providerId;
        if (inProvider) {
          done = true;
          values.found = true;
        }
      } else {
        inProvider = false;
      }
      continue;
    }
    if (!inProvider) continue;
    const assignment = /^(base_url|experimental_bearer_token)\s*=/.exec(trimmed);
    if (!assignment) continue;
    const quoted = /^("(?:\\.|[^"\\])*"|'[^']*')\s*(?:#.*)?$/.exec(
      trimmed.slice(assignment[0].length).trim(),
    )?.[1];
    if (!quoted) {
      values.found = false;
      return values;
    }
    let value: string;
    try {
      value = quoted.startsWith('"') ? JSON.parse(quoted) : quoted.slice(1, -1);
    } catch {
      values.found = false;
      return values;
    }
    const field = assignment[1] as "base_url" | "experimental_bearer_token";
    values[field] = value;
    if (field === "experimental_bearer_token") values.tokenMasked = /^[•*]+$/.test(value);
  }
  return values;
}

// 把表单里的地址/密钥写回编辑器 provider 段；缺失的行在段尾补上。
export function patchProviderFields(text: string, baseUrl: string, apiKey: string): string {
  const base = baseUrl.trim();
  const key = apiKey.trim();
  const lines = text.split("\n");
  let providerId: string | null = null;
  for (const line of lines) {
    const match = /^model_provider\s*=\s*"([^"]+)"/.exec(line.trim());
    if (match) {
      providerId = match[1];
      break;
    }
  }
  let inProvider = false;
  let done = false;
  let replacedBase = false;
  let replacedKey = false;
  const out: string[] = [];
  const flushMissing = () => {
    if (!inProvider) return;
    if (base && !replacedBase) out.push(`base_url = ${JSON.stringify(base)}`);
    if (key && !replacedKey) {
      out.push(`experimental_bearer_token = ${JSON.stringify(key)}`);
    }
    inProvider = false;
  };
  for (const line of lines) {
    const trimmed = line.trim();
    if (/^\[.+\]$/.test(trimmed)) {
      flushMissing();
      const section = /^\[model_providers\.(.+)\]$/.exec(trimmed);
      if (section && !done) {
        inProvider = providerId === null || section[1] === providerId;
        if (inProvider) done = true;
      } else {
        inProvider = false;
      }
      replacedBase = false;
      replacedKey = false;
      out.push(line);
      continue;
    }
    if (!inProvider) {
      out.push(line);
      continue;
    }
    const match = /^(base_url|experimental_bearer_token)\s*=/.exec(trimmed);
    if (!match) {
      out.push(line);
      continue;
    }
    const field = match[1];
    const value = field === "base_url" ? base : key;
    const indent = line.slice(0, line.length - line.trimStart().length);
    if (field === "base_url") replacedBase = true;
    else replacedKey = true;
    const singleQuoted = trimmed.slice(match[0].length).trimStart().startsWith("'")
      && !value.includes("'") && !/[\r\n]/.test(value);
    out.push(`${indent}${field} = ${singleQuoted ? `'${value}'` : JSON.stringify(value)}`);
  }
  flushMissing();
  return out.join("\n");
}

export function withoutApiKeyPlaceholder(text: string): string {
  const fields = readProviderFields(text);
  return fields.found && /^<.*>$/.test(fields.experimental_bearer_token)
    ? patchProviderFields(text, fields.base_url, "")
    : text;
}

export function withMcpSection(base: string, mcpSection: string): string {
  return mcpSection ? `${base.trimEnd()}\n\n${mcpSection.trimEnd()}\n` : base;
}

/** 读取顶层 `model = "..."` 的值（剥引号）；无该行返回 null。
 * `^model\s*=` 不匹配 model_provider / model_reasoning_effort 等前缀键。 */
export function readModelValue(text: string, key: "model" | "model_reasoning_effort" | "service_tier" | "model_catalog_json" = "model"): string | null {
  for (const line of text.split("\n")) {
    if (line.trimStart().startsWith("[")) break;
    const match = new RegExp(`^${key}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s#]+))`).exec(line.trim());
    if (match) return match[1] ?? match[2] ?? match[3] ?? "";
  }
  return null;
}

/**
 * 把表单的模型值写回顶层 `model = "..."` 行：
 * - 有 model 行 → 替换第一处；后续重复的 model 行（历史缺陷产生的非法重复键）一并收敛
 * - 没有该顶层键时插到 `model_provider` 行后（仍无则插到文本开头）
 * 注意不能逐行边扫边插：智谱等配置 model_provider 排在 model 前，
 * 先遇到 provider 就插入会造出重复的 model 键（TOML 非法）。
 */
export function patchModelValue(text: string, model: string, key: "model" | "model_reasoning_effort" | "service_tier" = "model"): string {
  const value = model.trim();
  const lines = text.split("\n");
  const out: string[] = [];
  let replaced = false;
  let inTable = false;
  for (const line of lines) {
    if (line.trimStart().startsWith("[")) inTable = true;
    if (!inTable && new RegExp(`^${key}\\s*=`).test(line.trim())) {
      if (!replaced && value !== "") {
        const indent = line.slice(0, line.length - line.trimStart().length);
        out.push(`${indent}${key} = ${JSON.stringify(value)}`);
      }
      // 首个 model 行被替换（或 value 为空被删除）后，其余重复行一律丢弃
      replaced = true;
      continue;
    }
    out.push(line);
  }
  if (replaced) return out.join("\n");
  if (value === "") return text;

  const providerIndex = out.findIndex((line) => /^model_provider\s*=/.test(line.trim()));
  if (providerIndex === -1) return [`${key} = ${JSON.stringify(value)}`, ...out].join("\n");
  const provider = out[providerIndex];
  const indent = provider.slice(0, provider.length - provider.trimStart().length);
  out.splice(providerIndex + 1, 0, `${indent}${key} = ${JSON.stringify(value)}`);
  return out.join("\n");
}
