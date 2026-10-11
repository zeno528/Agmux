import i18next from "i18next";

// 展示元数据：网格选择时的展示 / provider id 推断 / "测试连通" 按钮可用性判定。
// config.toml 原文由后端 builtin 模板单一来源，模型目录同。

/** 官方区域端点档；region 只用于展示，写入配置的永远是 URL 本身。 */
export interface PresetEndpoint {
  region: "cn" | "global" | "sg" | "us" | "hk";
  /** 同一区域存在多条计费通道时的档位名（如 Coding Plan / Agent Plan）；缺省时菜单只显示区域徽标。 */
  label?: string;
  base_url: string;
  admin_url: string | null;
}

export function presetRegionName(region: PresetEndpoint["region"]): string {
  const keys = { cn: "endpointRegionCn", global: "endpointRegionGlobal", sg: "endpointRegionSg", us: "endpointRegionUs", hk: "endpointRegionHk" } as const;
  return i18next.t(`profiles:edit.${keys[region]}`);
}

export interface ClientPreset {
  kind: string;
  /** 同一供应商的其他套餐指向主条目；仅用于创建页分组，不改变保存的 kind。 */
  group?: string;
  plan?: string;
  /** 声明后不按套餐拆入口，在版本滑块内切换计费方式（如小米按量 / Token Plan 共用卡片）。 */
  combine_plans?: boolean;
  name: string;
  provider: string | null;
  icon: string;
  base_url: string;
  admin_url: string | null;
  model: string;
  /** 区域或计费通道端点档；首项必须与 base_url/admin_url（默认选项）一致。 */
  endpoints?: readonly PresetEndpoint[];
}

/** 支持余额/用量查询的供应商（卡片统一读 profile.provider，但 Codex 卡片装的是 provider_id、Claude 卡片装的是 kind；同一厂商两端 id 不同时两行都要写，如 ZAI / zhipu）；加供应商时在这里加一行即可 */
export const balanceQueryProviders = new Set(["deepseek", "minimax", "ZAI", "zhipu"]);

/** Claude providers with the same supported usage endpoints. */
export const claudeBalanceQueryKinds = new Set(["deepseek", "minimax", "zhipu"]);

/** 文案使用“用量”的供应商；DeepSeek 保持“余额”，ChatGPT 也统一使用“用量”。 */
export const usageQueryProviders = new Set(["minimax", "ZAI", "zhipu"]);

/** 余额/用量胶囊变色（已用 <70% 绿 / 70-89 9、橙 / ≥90% 红；负余额红色） */
export function balanceChipClass(usagePercent: number | null, totalBalance: string | null = null): string {
  if (usagePercent == null) return Number(totalBalance) < 0 ? "chip-danger" : "chip-success";
  if (usagePercent >= 90) return "chip-danger";
  if (usagePercent >= 70) return "chip-warn";
  return "chip-success";
}

// 地址/密钥留空由表单写入；顶层不带 model 行，模型输入框留空，
// 用户输入或「获取模型列表」选择后经 patchModelValue 插入
export const codexCustomConfigTemplate = `model_provider = "custom"
model_reasoning_effort = "high"
disable_response_storage = true
model_catalog_json = "~/.codex/models.json"

[model_providers.custom]
name = "custom"
base_url = ""
wire_api = "responses"
experimental_bearer_token = ""`;

/** 模型目录按 Codex 官方目录的完整字段集（对照 assets/builtin/zhipu-models.json，
 * 每条 21 个字段）：slug 是模型标识；base_instructions 与 supports_reasoning_summaries
 * 为解析器必填字段，缺失会让 Codex 拒载整个目录文件。 */
export const codexCustomCatalogTemplate = `{
  "models": [
    {
      "slug": "gpt-5.6-sol",
      "display_name": "GPT 5.6 Sol",
      "description": "GPT 5.6 Sol",
      "default_reasoning_level": "high",
      "supported_reasoning_levels": [
        { "effort": "low", "description": "Light reasoning" },
        { "effort": "high", "description": "Deep reasoning" }
      ],
      "shell_type": "shell_command",
      "visibility": "list",
      "supported_in_api": true,
      "priority": 0,
      "base_instructions": "",
      "supports_reasoning_summaries": true,
      "default_reasoning_summary": "none",
      "support_verbosity": false,
      "apply_patch_tool_type": "freeform",
      "truncation_policy": { "mode": "bytes", "limit": 10000 },
      "context_window": 128000,
      "max_context_window": 128000,
      "effective_context_window_percent": 95,
      "supports_parallel_tool_calls": true,
      "experimental_supported_tools": [],
      "input_modalities": ["text"]
    }
  ]
}`;

// 共享身份信息；只声明实际支持的客户端，每边独立填写地址、模型和区域端点。
// 中文展示名继续用 getter，避免语言切换后名称停留在模块加载时的语言。
export interface ProviderPreset extends Pick<ClientPreset, "kind" | "group" | "name" | "icon"> {
  codex?: Omit<ClientPreset, "kind" | "group" | "name" | "icon">;
  claude?: Omit<ClientPreset, "kind" | "group" | "name" | "icon">;
}

export const providerPresets: ProviderPreset[] = [
  { kind: "custom", get name() { return i18next.t("preset.custom"); }, icon: "custom",
    codex: {
      provider: null,
      base_url: "",
      admin_url: null,
      model: "自定义" // i18n-exempt: 模型名会写入 config.toml，不能随界面语言变化
    },
    claude: { provider: null, base_url: "", admin_url: null, model: "" },
  },
  { kind: "claude-account", name: "Claude Account", icon: "claude",
    claude: { provider: null, base_url: "", admin_url: "https://claude.ai", model: "" },
  },
  { kind: "anthropic", name: "Anthropic API", icon: "anthropic",
    claude: { provider: null, base_url: "https://api.anthropic.com", admin_url: "https://console.anthropic.com/settings/keys", model: "" },
  },
  { kind: "chatgpt", name: "ChatGPT", icon: "openai-chatgpt",
    codex: {
      provider: null,
      base_url: "",
      admin_url: "https://openai.com/chatgpt/pricing",
      model: "",
    },
  },
  { kind: "deepseek", name: "DeepSeek", icon: "deepseek",
    codex: {
      provider: "deepseek",
      base_url: "https://api.deepseek.com/",
      admin_url: "https://platform.deepseek.com",
      model: "",
    },
    claude: {
      provider: "deepseek",
      base_url: "https://api.deepseek.com/anthropic",
      admin_url: "https://platform.deepseek.com",
      model: "",
    },
  },
  // M Plan 与按量 API 共用端点，套餐计费由订阅 Key 区分。
  { kind: "minimax", name: "MiniMax", icon: "minimax",
    codex: {
      plan: "M Plan",
      provider: "minimax",
      base_url: "https://api.minimax.cn/v1",
      admin_url: "https://platform.minimax.cn",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.minimax.cn/v1", admin_url: "https://platform.minimax.cn" },
        { region: "global", base_url: "https://api.minimax.io/v1", admin_url: "https://platform.minimax.io" },
      ],
    },
    claude: {
      plan: "M Plan",
      provider: "minimax",
      base_url: "https://api.minimax.cn/anthropic",
      admin_url: "https://platform.minimax.cn",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.minimax.cn/anthropic", admin_url: "https://platform.minimax.cn" },
        { region: "global", base_url: "https://api.minimax.io/anthropic", admin_url: "https://platform.minimax.io" },
      ],
    },
  },
  { kind: "zhipu", get name() { return i18next.t("preset.zhipu"); }, icon: "zhipu",
    codex: {
      plan: "Coding Plan",
      provider: "ZAI",
      base_url: "https://open.bigmodel.cn/api/v1",
      admin_url: "https://open.bigmodel.cn",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://open.bigmodel.cn/api/v1", admin_url: "https://open.bigmodel.cn" },
        { region: "global", base_url: "https://api.z.ai/api/v1", admin_url: "https://z.ai/model-api" },
      ],
    },
    claude: {
      plan: "Coding Plan",
      provider: "ZAI",
      base_url: "https://open.bigmodel.cn/api/anthropic",
      admin_url: "https://open.bigmodel.cn",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://open.bigmodel.cn/api/anthropic", admin_url: "https://open.bigmodel.cn" },
        { region: "global", base_url: "https://api.z.ai/api/anthropic", admin_url: "https://z.ai/model-api" },
      ],
    },
  },
  { kind: "opencode", name: "OpenCode", icon: "opencode",
    codex: {
      plan: "Go",
      provider: "opencode-go",
      base_url: "https://opencode.ai/zen/go/v1",
      admin_url: "https://opencode.ai/go",
      model: "grok-4.6",
    },
    claude: { plan: "Go", provider: "opencode-go", base_url: "https://opencode.ai/zen/go", admin_url: "https://opencode.ai/go", model: "" },
  },
  { kind: "openrouter", name: "OpenRouter", icon: "openrouter",
    codex: {
      provider: "openrouter",
      base_url: "https://openrouter.ai/api/v1",
      admin_url: "https://openrouter.ai/settings/keys",
      model: "openai/gpt-5.6-sol",
    },
    claude: {
      provider: "openrouter",
      base_url: "https://openrouter.ai/api",
      admin_url: "https://openrouter.ai/settings/keys",
      model: "",
    },
  },
  { kind: "mimo", get name() { return i18next.t("preset.mimo"); }, icon: "xiaomi-mimo",
    codex: {
      provider: "mimo",
      combine_plans: true,
      base_url: "https://api.xiaomimimo.com/v1",
      admin_url: "https://platform.xiaomimimo.com/#/console/api-keys",
      model: "mimo-v2.5-pro",
      endpoints: [
        { region: "cn", base_url: "https://api.xiaomimimo.com/v1", admin_url: "https://platform.xiaomimimo.com/#/console/api-keys" },
        { region: "cn", label: "Token Plan", base_url: "https://token-plan-cn.xiaomimimo.com/v1", admin_url: "https://platform.xiaomimimo.com/#/console/plan-manage" },
      ],
    },
    claude: {
      provider: "mimo",
      combine_plans: true,
      base_url: "https://api.xiaomimimo.com/anthropic",
      admin_url: "https://platform.xiaomimimo.com/#/console/api-keys",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.xiaomimimo.com/anthropic", admin_url: "https://platform.xiaomimimo.com/#/console/api-keys" },
        { region: "cn", label: "Token Plan", base_url: "https://token-plan-cn.xiaomimimo.com/anthropic", admin_url: "https://platform.xiaomimimo.com/#/console/plan-manage" },
      ],
    },
  },
  { kind: "kimi", name: "Kimi", icon: "kimi",
    codex: {
      provider: "kimi",
      base_url: "https://api.moonshot.cn/v1",
      admin_url: "https://platform.kimi.com/console/api-keys",
      model: "kimi-k3",
      endpoints: [
        { region: "cn", base_url: "https://api.moonshot.cn/v1", admin_url: "https://platform.kimi.com/console/api-keys" },
        { region: "global", base_url: "https://api.moonshot.ai/v1", admin_url: "https://platform.kimi.ai/console/api-keys" },
      ],
    },
    claude: {
      provider: "kimi",
      base_url: "https://api.moonshot.cn/anthropic",
      admin_url: "https://platform.kimi.com/console/api-keys",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.moonshot.cn/anthropic", admin_url: "https://platform.kimi.com/console/api-keys" },
        { region: "global", base_url: "https://api.moonshot.ai/anthropic", admin_url: "https://platform.kimi.ai/console/api-keys" },
      ],
    },
  },
  { kind: "kimi-code", group: "kimi", get name() { return i18next.t("preset.kimiCode"); }, icon: "kimi",
    codex: {
      plan: "Kimi Code", provider: "kimi-code", base_url: "https://api.kimi.com/coding/v1",
      admin_url: "https://www.kimi.com/code/docs/third-party-tools/codex", model: "k3-256k",
      endpoints: [
        { region: "cn", base_url: "https://api.kimi.com/coding/v1", admin_url: "https://www.kimi.com/code/docs/third-party-tools/codex" },
        { region: "global", base_url: "https://api.kimi.ai/coding/v1", admin_url: "https://www.kimi.com/code/docs/en/third-party-tools/codex" },
      ],
    },
    claude: {
      plan: "Kimi Code",
      provider: "kimi-code",
      base_url: "https://api.kimi.com/coding/",
      admin_url: "https://www.kimi.com/code/docs/third-party-tools/claude-code.html",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.kimi.com/coding/", admin_url: "https://www.kimi.com/code/docs/third-party-tools/claude-code.html" },
        { region: "global", base_url: "https://api.kimi.ai/coding/", admin_url: "https://www.kimi.com/code/docs/en/" },
      ],
    },
  },
  { kind: "qwen", get name() { return i18next.t("preset.qwen"); }, icon: "qwen",
    codex: {
      plan: "Token Plan",
      provider: "Model_Studio_Token_Plan_Personal",
      base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
      admin_url: "https://bailian.console.aliyun.com",
      model: "qwen3.8-max",
      endpoints: [
        { region: "cn", base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1", admin_url: "https://bailian.console.aliyun.com" },
        { region: "global", base_url: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1", admin_url: "https://modelstudio.console.alibabacloud.com/ap-southeast-1" },
      ],
    },
    claude: {
      plan: "Token Plan",
      provider: "Model_Studio_Token_Plan_Personal",
      base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic",
      admin_url: "https://bailian.console.aliyun.com",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic", admin_url: "https://bailian.console.aliyun.com" },
        { region: "global", base_url: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/apps/anthropic", admin_url: "https://modelstudio.console.alibabacloud.com/ap-southeast-1" },
      ],
    },
  },
  { kind: "qwen-coding-plan", group: "qwen", get name() { return i18next.t("preset.qwenCodingPlan"); }, icon: "qwen",
    claude: {
      plan: "Coding Plan",
      provider: "Model_Studio_Coding_Plan",
      base_url: "https://coding.dashscope.aliyuncs.com/apps/anthropic",
      admin_url: "https://bailian.console.aliyun.com",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://coding.dashscope.aliyuncs.com/apps/anthropic", admin_url: "https://bailian.console.aliyun.com" },
        { region: "global", base_url: "https://coding-intl.dashscope.aliyuncs.com/apps/anthropic", admin_url: "https://modelstudio.console.alibabacloud.com/ap-southeast-1" },
      ],
    },
  },
  { kind: "hunyuan", get name() { return i18next.t("preset.hunyuan"); }, icon: "hunyuan",
    codex: {
      plan: "API",
      provider: "hy3-tokenhub",
      base_url: "https://tokenhub.tencentmaas.com/v1",
      admin_url: "https://console.cloud.tencent.com/tokenhub/apikey",
      model: "hy3",
      endpoints: [
        { region: "cn", base_url: "https://tokenhub.tencentmaas.com/v1", admin_url: "https://console.cloud.tencent.com/tokenhub/apikey" },
        { region: "global", base_url: "https://tokenhub-intl.tencentmaas.com/v1", admin_url: null },
      ],
    },
    claude: {
      plan: "API",
      provider: "hy3-tokenhub",
      base_url: "https://tokenhub.tencentmaas.com",
      admin_url: "https://console.cloud.tencent.com/tokenhub/apikey",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://tokenhub.tencentmaas.com", admin_url: "https://console.cloud.tencent.com/tokenhub/apikey" },
        { region: "global", base_url: "https://tokenhub-intl.tencentmaas.com", admin_url: null },
      ],
    },
  },
  { kind: "tencent-token-plan", group: "hunyuan", get name() { return i18next.t("preset.tencentTokenPlan"); }, icon: "tencentcloud",
    claude: {
      plan: "Token Plan",
      provider: "tencent-token-plan",
      base_url: "https://api.lkeap.cloud.tencent.com/plan/anthropic",
      admin_url: "https://cloud.tencent.com/document/product/1823/130060",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.lkeap.cloud.tencent.com/plan/anthropic", admin_url: "https://cloud.tencent.com/document/product/1823/130060" },
        { region: "global", base_url: "https://tokenhub-intl.tencentcloudmaas.com/plan/anthropic", admin_url: "https://console.tencentcloud.com/tokenhub/tokenplan" },
      ],
    },
  },
  { kind: "tencent-token-plan-enterprise", group: "hunyuan", get name() { return i18next.t("preset.tencentTokenPlanEnterprise"); }, icon: "hunyuan",
    codex: {
      plan: "Token Plan Enterprise",
      provider: "Token_Plan_Ent",
      base_url: "https://tokenhub.tencentmaas.com/plan/v3",
      admin_url: "https://cloud.tencent.com/document/product/1823/130660",
      model: "auto",
      endpoints: [
        { region: "cn", base_url: "https://tokenhub.tencentmaas.com/plan/v3", admin_url: "https://cloud.tencent.com/document/product/1823/130660" },
        { region: "global", base_url: "https://tokenhub-intl.tencentmaas.com/plan/v3", admin_url: "https://cloud.tencent.com/document/product/1823/130660" },
      ],
    },
    claude: {
      plan: "Token Plan Enterprise", provider: "Token_Plan_Ent", base_url: "https://tokenhub.tencentmaas.com/plan/anthropic",
      admin_url: "https://cloud.tencent.com/document/product/1823/130659", model: "",
      endpoints: [
        { region: "cn", base_url: "https://tokenhub.tencentmaas.com/plan/anthropic", admin_url: "https://cloud.tencent.com/document/product/1823/130659" },
        { region: "global", base_url: "https://tokenhub-intl.tencentmaas.com/plan/anthropic", admin_url: "https://cloud.tencent.com/document/product/1823/130659" },
      ],
    },
  },
  // Codex 国内提供 Coding Plan / Agent Plan，海外仅 Coding Plan；bytepluses.com 的 /api/v3 按量另计。
  { kind: "doubao", get name() { return i18next.t("preset.doubao"); }, icon: "volcengine",
    codex: {
      provider: "volcengine-coding-plan",
      base_url: "https://ark.cn-beijing.volces.com/api/coding/v3",
      admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey",
      model: "ark-code-latest",
      endpoints: [
        { region: "cn", label: "Coding Plan", base_url: "https://ark.cn-beijing.volces.com/api/coding/v3", admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey" },
        { region: "cn", label: "Agent Plan", base_url: "https://ark.cn-beijing.volces.com/api/plan/v3", admin_url: "https://console.volcengine.com/ark/region:cn-beijing/openManagement?advancedActiveKey=agentPlan" },
        { region: "global", label: "Coding Plan", base_url: "https://ark.ap-southeast.bytepluses.com/api/coding/v3", admin_url: "https://ai.byteplus.com/ark/region:ap-southeast-1/apikey" },
      ],
    },
    claude: {
      plan: "Coding Plan",
      provider: "volcengine-coding-plan",
      base_url: "https://ark.cn-beijing.volces.com/api/coding",
      admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey",
      model: "",
      endpoints: [
        { region: "cn", label: "Coding Plan", base_url: "https://ark.cn-beijing.volces.com/api/coding", admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey" },
        { region: "cn", label: "Agent Plan", base_url: "https://ark.cn-beijing.volces.com/api/plan", admin_url: "https://console.volcengine.com/ark/region:cn-beijing/openManagement?advancedActiveKey=agentPlan" },
        { region: "global", label: "Coding Plan", base_url: "https://ark.ap-southeast.bytepluses.com/api/coding", admin_url: "https://ai.byteplus.com/ark/region:ap-southeast-1/apikey" },
      ],
    },
  },
  { kind: "qianfan", get name() { return i18next.t("preset.qianfan"); }, icon: "baiducloud",
    codex: {
      plan: "API",
      provider: "qianfan",
      base_url: "https://qianfan.baidubce.com/v2",
      admin_url: "https://console.bce.baidu.com/qianfan/",
      model: "glm-5.1",
    },
    claude: {
      plan: "API",
      provider: "qianfan",
      base_url: "https://qianfan.baidubce.com/anthropic",
      admin_url: "https://console.bce.baidu.com/qianfan/",
      model: "",
    },
  },
  { kind: "qianfan-coding-plan", group: "qianfan", get name() { return i18next.t("preset.qianfanCodingPlan"); }, icon: "baiducloud",
    claude: {
      plan: "Coding Plan",
      provider: "qianfan-coding-plan",
      base_url: "https://qianfan.baidubce.com/anthropic/coding",
      admin_url: "https://cloud.baidu.com/product/codingplan",
      model: "",
    },
  },
  { kind: "siliconflow", get name() { return i18next.t("preset.siliconflow"); }, icon: "siliconcloud-siliconflow",
    claude: {
      provider: "siliconflow",
      base_url: "https://api.siliconflow.cn/",
      admin_url: "https://cloud.siliconflow.cn/account/ak",
      model: "",
      endpoints: [
        { region: "cn", base_url: "https://api.siliconflow.cn/", admin_url: "https://cloud.siliconflow.cn/account/ak" },
        { region: "global", base_url: "https://api.siliconflow.com/", admin_url: "https://cloud.siliconflow.com/account/ak" },
      ],
    },
  },
  { kind: "longcat", get name() { return i18next.t("preset.longcat"); }, icon: "longcat",
    claude: {
      provider: "longcat",
      base_url: "https://api.longcat.chat/anthropic",
      admin_url: "https://longcat.chat/platform/docs/ClaudeCode.html",
      model: "",
    },
  },
  { kind: "xai", get name() { return i18next.t("preset.xai"); }, icon: "xai",
    codex: {
      provider: "xai",
      base_url: "https://api.x.ai/v1",
      admin_url: "https://console.x.ai/team/default/api-keys",
      model: "grok-4.7",
      endpoints: [
        { region: "global", base_url: "https://api.x.ai/v1", admin_url: "https://console.x.ai/team/default/api-keys" },
        { region: "us", base_url: "https://us.api.x.ai/v1", admin_url: "https://console.x.ai/team/default/api-keys" },
      ],
    },
  },
  { kind: "opencode-zen", group: "opencode", name: "OpenCode", icon: "opencode",
    codex: {
      plan: "API", provider: "opencode-zen", base_url: "https://opencode.ai/zen/v1",
      admin_url: "https://opencode.ai/zen", model: "gpt-5.6-sol",
    },
    claude: {
      plan: "API", provider: "opencode-zen", base_url: "https://opencode.ai/zen",
      admin_url: "https://opencode.ai/zen", model: "",
    },
  },
  { kind: "qwen-api", group: "qwen", get name() { return i18next.t("preset.qwen"); }, icon: "qwen",
    codex: {
      plan: "API", provider: "qwen-api", base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
      admin_url: "https://bailian.console.aliyun.com", model: "",
      endpoints: [
        { region: "cn", base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", admin_url: "https://bailian.console.aliyun.com" },
        { region: "sg", base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", admin_url: "https://modelstudio.console.alibabacloud.com/ap-southeast-1" },
        { region: "us", base_url: "https://dashscope-us.aliyuncs.com/compatible-mode/v1", admin_url: "https://modelstudio.console.alibabacloud.com/us-east-1" },
        { region: "hk", base_url: "https://cn-hongkong.dashscope.aliyuncs.com/compatible-mode/v1", admin_url: "https://bailian.console.aliyun.com" },
      ],
    },
    claude: {
      plan: "API", provider: "qwen-api", base_url: "https://dashscope.aliyuncs.com/apps/anthropic",
      admin_url: "https://bailian.console.aliyun.com", model: "",
      endpoints: [
        { region: "cn", base_url: "https://dashscope.aliyuncs.com/apps/anthropic", admin_url: "https://bailian.console.aliyun.com" },
        { region: "sg", base_url: "https://dashscope-intl.aliyuncs.com/apps/anthropic", admin_url: "https://modelstudio.console.alibabacloud.com/ap-southeast-1" },
        { region: "us", base_url: "https://dashscope-us.aliyuncs.com/apps/anthropic", admin_url: "https://modelstudio.console.alibabacloud.com/us-east-1" },
        { region: "hk", base_url: "https://cn-hongkong.dashscope.aliyuncs.com/apps/anthropic", admin_url: "https://bailian.console.aliyun.com" },
      ],
    },
  },
  { kind: "doubao-api", group: "doubao", get name() { return i18next.t("preset.doubao"); }, icon: "volcengine",
    codex: {
      plan: "API", provider: "doubao-api", base_url: "https://ark.cn-beijing.volces.com/api/v3",
      admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey", model: "",
      endpoints: [
        { region: "cn", base_url: "https://ark.cn-beijing.volces.com/api/v3", admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey" },
        { region: "global", base_url: "https://ark.ap-southeast.bytepluses.com/api/v3", admin_url: "https://ai.byteplus.com/ark/region:ap-southeast-1/apikey" },
      ],
    },
    claude: {
      plan: "API", provider: "doubao-api", base_url: "https://ark.cn-beijing.volces.com/api/compatible",
      admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey", model: "",
      endpoints: [
        { region: "cn", base_url: "https://ark.cn-beijing.volces.com/api/compatible", admin_url: "https://ark.volcengine.com/region:cn-beijing/apikey" },
        { region: "global", base_url: "https://ark.ap-southeast.bytepluses.com/api/compatible", admin_url: "https://ai.byteplus.com/ark/region:ap-southeast-1/apikey" },
      ],
    },
  },
  { kind: "tencent-coding-plan", group: "hunyuan", get name() { return i18next.t("preset.hunyuan"); }, icon: "tencentcloud",
    claude: {
      plan: "Coding Plan", provider: "tencent-coding-plan", base_url: "https://api.lkeap.cloud.tencent.com/coding/anthropic",
      admin_url: "https://cloud.tencent.com/document/product/1823/130092", model: "",
    },
  },
];

/** 两个客户端共用列表投影；不从另一客户端继承任何接入默认值。 */
export function clientPresets(providers: readonly ProviderPreset[], client: "codex" | "claude"): ClientPreset[] {
  return providers.flatMap((preset) => {
    const connection = preset[client];
    return connection ? [{
      kind: preset.kind,
      group: preset.group,
      get name() { return preset.name; },
      icon: preset.icon,
      ...connection,
    }] : [];
  });
}

export const codexPresets = clientPresets(providerPresets, "codex");
export const claudePresets = clientPresets(providerPresets, "claude");

export interface PresetConnection extends PresetEndpoint {
  kind: string;
  label: string;
}

/** 合并同品牌套餐与区域端点，仍保留每个客户端原本的 kind 和接入配置。 */
export function presetConnections(preset: ClientPreset, presets: readonly ClientPreset[]): PresetConnection[] {
  const group = preset.group ?? preset.kind;
  return presets.filter((item) => (item.group ?? item.kind) === group).flatMap((item) => (
    item.endpoints ?? [{ region: "cn" as const, base_url: item.base_url, admin_url: item.admin_url }]
  ).map((endpoint) => ({ ...endpoint, kind: item.kind, label: endpoint.label ?? item.plan ?? "API" })));
}

/** 创建入口按套餐拆分；声明 combine_plans 的供应商保留单入口，在版本内切换计费方式。原 kind 继续用于模板读取和保存。 */
export function presetEntries(presets: readonly ClientPreset[]) {
  return presets.filter((preset) => !preset.group).flatMap((primary) => {
    const connections = presetConnections(primary, presets);
    const combined = Boolean(primary.combine_plans);
    const plans = combined ? ["API"] : [...new Set(connections.map((connection) => connection.label))];
    return plans.map((plan) => {
      const endpoints = combined ? connections : connections.filter((connection) => connection.label === plan);
      const source = presets.find((preset) => preset.kind === endpoints[0].kind)!;
      return {
        key: plans.length > 1 ? `${primary.kind}:${plan}` : primary.kind,
        get name() { return plan !== "API" || plans.length > 1 ? `${primary.name} · ${plan === "API" ? i18next.t("profiles:edit.payAsYouGo") : plan}` : primary.name; },
        preset: {
          ...source,
          group: undefined,
          plan,
          get name() { return primary.name; },
          base_url: endpoints[0].base_url,
          admin_url: endpoints[0].admin_url,
          endpoints,
        } satisfies ClientPreset,
      };
    });
  });
}

/** 分类数量、筛选和全部视图分组共用；自定义只在全部中显示。 */
export function presetEntryCategory(preset: ClientPreset) {
  if (!preset.base_url) return preset.kind === "custom" ? null : "accounts";
  return preset.combine_plans || preset.plan && preset.plan !== "API" ? "plans" : "api";
}

/** 入口默认名：非按量套餐带套餐名后缀；区域由版本滑块展示，不进名称。 */
export function presetConnectionName(preset: ClientPreset): string {
  return preset.plan && preset.plan !== "API" ? `${preset.name} · ${preset.plan}` : preset.name;
}

export function codexPresetByKind(kind: string): ClientPreset | undefined {
  return codexPresets.find((preset) => preset.kind === kind);
}

export function claudePresetByKind(kind: string | null | undefined): ClientPreset | undefined {
  return kind ? claudePresets.find((preset) => preset.kind === kind) : undefined;
}

/** 哪些内置供应商自带静态模型目录档。models.json 槽位的存在判定统一走这里。 */
export const CODEX_BUILTINS_WITH_CATALOG: ReadonlySet<string> = new Set([
  "deepseek", "minimax", "zhipu", "opencode", "mimo", "kimi-code",
]);

export function codexBuiltinHasCatalog(kind: string | null | undefined): boolean {
  return kind != null && CODEX_BUILTINS_WITH_CATALOG.has(kind);
}
