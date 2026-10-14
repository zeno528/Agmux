# 供应商模板审计（2026-10-11）

范围：现有供应商品牌的按量、编程套餐、官方区域端点，以及 Codex Responses / Claude Code Anthropic 接入。以下是文档与配置核验结果，未使用真实 Key 逐项调用。

名称、图标、品牌分组共享；两端接入配置独立声明。没有对应协议的套餐不进入该客户端列表。同端点、同 Key 体系、仅额度不同的套餐等级不重复建模板。

## 当前覆盖

| 供应商 | Codex | Claude Code | 区域与计费 |
| --- | --- | --- | --- |
| ChatGPT | 账号登录 | — | 订阅等级随账号，不建重复模板 |
| Anthropic | — | Claude Account / Anthropic API | 原生账号与按量 API 分开 |
| DeepSeek | 原生 Responses | Anthropic | 单一按量入口，标题不加后缀 |
| MiniMax | M Plan | M Plan | 国内、全球；按用户要求删除同地址的重复按量模板，平台实际计费由 Key 决定 |
| 智谱 / Z.AI | Coding Plan | Coding Plan | 国内、全球；按用户要求删除同地址的重复按量模板，国际按量有账号限制，见下文 |
| OpenCode | Zen 按量 / Go | Zen 按量 / Go | 仅可使用对应协议支持的模型；Go / Go Plus 不重复建入口 |
| OpenRouter | Responses | Anthropic | 聚合网关，单一按量入口 |
| 小米 MiMo | 按量 / Token Plan | 按量 / Token Plan | 按用户要求保留一个卡片，归入订阅套餐，只提供计费方式切换，不展示区域 |
| Kimi | 按量 / Kimi Code | 按量 / Kimi Code | 两种均有国内、全球；Codex Kimi Code 使用官方 k3 / k3-256k 目录 |
| 通义千问 | 按量 / Token Plan | 按量 / Token Plan / Coding Plan | 按量北京 / 新加坡 / 美国 / 香港；Token Plan 国内 / 全球；Coding Plan 国内 / 全球 |
| 腾讯混元 / 腾讯云 | 按量 / Token Plan Enterprise | 按量 / Token Plan / Token Plan Enterprise / Coding Plan | 按量、企业套餐、个人 Token Plan 有国内 / 全球；个人国际站为新加坡；Coding Plan 尚未确认独立海外端点 |
| 火山方舟 / BytePlus | 按量 / Coding Plan / Agent Plan | 按量 / Coding Plan / Agent Plan | 按量、Coding Plan 国内 / 全球；Agent Plan 已确认国内端点，BytePlus 文档有产品提及但尚未确认独立海外端点 |
| 百度千帆 | 按量 | 按量 / Coding Plan | 未找到独立海外套餐端点，不猜测填充 |
| SiliconFlow | — | 按量 | 国内、全球；未确认 Responses，不加入 Codex |
| LongCat | — | 按量（兼容资源包扣费） | 资源包与按量使用相同地址及 Key，不做无法控制扣费方式的版本切换 |
| xAI | Responses | — | 全球、美国；Anthropic 兼容接口已弃用 |

## 需要保留的边界

- “兼容 OpenAI”不等于支持 Responses。百炼 Coding Plan、腾讯个人 Token Plan / Coding Plan、百度 Coding Plan 没有据此新增 Codex 模板。
- 腾讯个人通用 Token Plan 与 Hy Token Plan 的 Anthropic 接入均为 `https://api.lkeap.cloud.tencent.com/plan/anthropic`，共用 Token Plan Key；可用模型由订阅决定，现有 Token Plan 入口覆盖两者，不虚构不同 URL。
- 百炼个人 / 团队 Token Plan、BytePlus 个人 / 团队 Coding Plan、OpenCode Go / Go Plus 的额度等级不会形成新的协议端点。
- 百炼日本、德国等业务空间专属地址需要用户自己的 Workspace ID。模板只预置官方仍支持的公共域名；专属域名在编辑器手动填写，不保存带占位符的不可用地址。
- Z.AI 官方 GLM-5.3 文档明确：曾订阅 GLM Coding Plan（含过期）的账号目前仅能通过 Chat Completions 使用按量模型 API。因此这些账号的国际按量 Responses / Anthropic 接入不能保证可用。
- MiniMax 的按量 Key 与 M Plan Key 不通用；界面只保留 M Plan 入口，其套餐用量查询仍面向订阅 Key。智谱同样只保留 Coding Plan 入口；同地址不代表计费方式或 Key 通用。
- 切换区域不代表自动迁移密钥、模型权限或配额。具体模型仍由对应平台的 Key 与区域权限决定。
- 不改用户已经保存的配置、不新增数据库迁移、不在启动阶段请求供应商文档或模型列表。

## 官方依据

- MiniMax：[M Plan 接入与 Key 隔离](https://platform.minimax.io/docs/m-plan/quickstart)、[Responses API](https://platform.minimax.io/docs/api-reference/responses-create)。
- 智谱：[Responses 按量接口](https://docs.bigmodel.cn/cn/guide/develop/responses/introduction)、[Anthropic 接口](https://docs.bigmodel.cn/cn/guide/develop/claude/introduction)、[Coding Plan 协议](https://docs.bigmodel.cn/cn/coding-plan/tool/others)、[Z.AI 按量与账号限制](https://docs.z.ai/guides/llm/glm-5.3)。
- 小米：[Codex 按量 / Token Plan](https://mimo.mi.com/docs/zh-CN/tokenplan/integration/codex-configuration)、[Token Plan 区域地址](https://mimo.mi.com/docs/zh-CN/tokenplan/Token%20Plan/quick-access)。
- Kimi：[Kimi Code 原生 Codex、国内 / 全球地址与目录](https://www.kimi.com/code/docs/en/third-party-tools/codex)、[Claude Code](https://www.kimi.com/code/docs/third-party-tools/claude-code.html)。
- 百炼：[区域和协议地址矩阵](https://help.aliyun.com/zh/model-studio/base-url)、[Responses](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)、[Anthropic](https://help.aliyun.com/zh/model-studio/anthropic-api-messages)、[国际 Token Plan](https://www.alibabacloud.com/help/en/model-studio/token-plan-personal-quick-start)、[国际 Coding Plan](https://www.alibabacloud.com/help/en/model-studio/coding-plan)、[Coding Plan 协议限制](https://help.aliyun.com/zh/model-studio/coding-plan-faq)。
- 腾讯：[企业 Token Plan 国内 / 全球](https://cloud.tencent.com/document/product/1823/130659)、[个人 Token Plan](https://cloud.tencent.com/document/product/1823/130060)、[国际站个人 Token Plan 新加坡地址](https://intl.cloud.tencent.com/document/product/1300/81315)、[国际站 Claude Code 接入](https://intl.cloud.tencent.com/document/product/1300/81037)、[个人套餐 Codex 限制](https://cloud.tencent.com/document/product/1823/130071)、[Coding Plan](https://cloud.tencent.com/document/product/1823/130092)。
- 火山：[按量接入](https://docs.volcengine.com/docs/ark/integrate-third-party-tools?lang=zh)、[Agent Plan Claude Code](https://docs.volcengine.com/docs/ark/agent-plan-personal-claude-code?lang=zh)、[BytePlus Coding Plan](https://docs.byteplus.com/en/docs/ModelArk/2277823)、[BytePlus Responses](https://docs.byteplus.com/en/docs/ModelArk/create-model-responses-api)、[BytePlus Anthropic](https://docs.byteplus.com/en/docs/ModelArk/messages-api)。
- 百度：[Coding Plan 接口](https://cloud.baidu.com/doc/qianfan/s/imlg0beiu)。
- OpenCode：[Go](https://opencode.ai/docs/go/)、[Zen](https://opencode.ai/docs/zen/)。
- SiliconFlow：[全球 Anthropic 接口](https://docs.siliconflow.com/en/api-reference/chat-completions/messages)。
- LongCat：[资源包](https://longcat.chat/platform/docs/zh/token-pack)、[工具接入](https://longcat.chat/platform/docs/zh/tools-overview)。
- xAI：[区域](https://docs.x.ai/developers/advanced-api-usage/regions)、[弃用的 Anthropic 兼容接口](https://docs.x.ai/developers/rest-api-reference/inference/legacy)。
