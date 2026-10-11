# Agofra 中英术语表（i18n Glossary）

界面中英文案的术语与硬规则基线，**纯参考、不做机器强制**——改文案只需同步 zh/en 两个 locale 文件。范围约定：P0 = 通用层 + 首页，P1 = 业务页，P2 = Rust 报错文案。

## 一、在用术语表

| 中文 | 英文 | 备注 |
|---|---|---|
| 供应商配置 | Providers | 侧边栏导航 + 页面标题（`AppShell.tsx`、`ProfilesView.tsx`） |
| 供应商 | Provider | 泛指一家模型供应商，如 DeepSeek、Kimi |
| 配置卡片 | Provider Card | 首页单张卡片（`ProfileCard.tsx`） |
| 连通性 / 连通性测试 | Connectivity / Connectivity Test | 卡片上的测试按钮与结果 |
| 余额 | Balance | 账户余额 |
| 额度 | Quota | 用量语境标准词；`Allowance` 偏「津贴」不贴切 |
| 用量 | Usage | |
| 周期 | Period | 与「用量」「额度」同组 |
| 5 小时 / 7 天 | 5h / 7d | 余额标签短周期，用缩写省空间 |
| 登录 / 未登录 | Sign in / Not signed in | |
| 跟随 Codex 登录 | Sign in with Codex | `ProfileCard.tsx` 的 authSource=desktop 分支 |
| OAuth 登录 | OAuth sign-in | 名词短语作标签时用 sign-in |
| 认证 | Authentication | |
| 授权码 | Authorization code | |
| 订阅 | Subscription | |
| API 密钥 | API key | `key` 小写，句中一律小写 |
| 备份 / 备份目录 | Backup / Backup folder | |
| 数据库 | Database | |
| 导出 / 导入 | Export / Import | |
| 恢复 | Restore | |
| 重命名 | Rename | |
| 主题 | Theme | |
| 跟随系统 / 浅色 / 深色 | System / Light / Dark | 主题项；与语言项的「自动检测」**有意区分**，勿同屏统一 |
| 自动检测 / 简体中文 / English | Auto-detect / 简体中文 / English | 语言项；语言名按惯例用母语显示 |
| 检查更新 | Check for updates | |
| 升级至 | Update to | 关于页检测到新版本后的入口 |
| 立即重启更新 | Restart to update | 更新弹窗确认按钮；原「立即升级」 |
| MCP 管理 | MCP Servers | 说明管的是什么；嫌长用 `MCP` |
| Skill | Skill | Codex 专有概念，不译 |
| 应用（动词） | Apply | 应用配置；名词（应用数据目录）用 `App`，逐处判断、不全局替换 |
| 未设置 | Not set | `profile.model` 等空值占位 |
| 打开官网 | Open website | |
| 点击重命名 | Click to rename | |
| 刷新 | Refresh | |

## 二、什么该翻、什么绝不翻（硬规则）

**只翻用户能看到的。** 判断标准是「这个字符串会不会被渲染给用户」，不是「它是不是中文」。

**绝不翻译（本仓库已实际踩过的实例）：**

| 实例 | 位置 | 为什么不能翻 |
|---|---|---|
| `console.error("Agofra 界面渲染失败", …)` | `src/app/AppErrorBoundary.tsx` | 只写控制台，用户看不到 |
| `throw new Error("额度查询未返回数据")` | `src/features/settings/ChatGPTAccount.tsx` | 消息只被存下、渲染时仅当布尔用，正文从不显示 |
| `label === "7天"` | 同上 | 与后端下发数据值比对，不是界面文案 |
| `model: "自定义"` | `src/presets.ts` | 会写进用户 config.toml，翻了配置内容会随界面语言变 |
| `localeCompare(a, b, "zh")` | `src/icons.ts` | 排序参数，不产生可见文本 |
| `throw new Error("useAppUpdate 必须在 AppUpdateProvider 内使用")` | `src/features/updates/AppUpdateProvider.tsx` | 开发者契约错误，用户不可见 |

此外一律不动：代码注释 / JSDoc、class 名、Tauri 命令名、文件路径、URL、正则片段、仅调试模式（`web-mock.ts`）使用的数据。

**必须翻译**：渲染出的 JSX 文本、`title` / `aria-label` / `placeholder`、toast 文案、弹窗标题与按钮、空状态、表单标签与说明、状态提示。

**日期与数字**：会渲染给用户的，locale 参数用 `i18n.language`（**不写死 `"zh-CN"`，也不留空跟随系统**）；只用于排序的保持原样。

**拿不准就不翻** —— 在该行加 `// i18n-exempt: 待确认 —— <疑问>` 并在评审里单列。宁可漏翻一条由复核补上，也不要多翻一条。

### 三道自动防线（`pnpm check` 全跑）

| 防线 | 拦住什么 |
|---|---|
| TypeScript 类型化键名（`src/i18n/types.d.ts`） | `t("打错的.键")` 编译不过 |
| `src/i18n/locales.test.ts` 键对齐 | 只加中文键、没加英文键 → 测试红 |
| `scripts/check-i18n-leaks.mjs` | ① 代码里写死中文；② 英文资源的值里出现中文（漏翻） |

两道豁免，且都会在失效时报错、防止腐烂：

- **整文件 WHITELIST**：尚未迁移的文件，写明理由，迁移一个摘一个
- **行内 `// i18n-exempt: <理由>`**：确属不能翻的单行，理由写清楚

`check-i18n-leaks.mjs` 里的 `EN_ALLOWLIST` 是英文资源允许保留中文的**唯一**白名单，目前只有一项（语言名按惯例用母语显示）。

### 新增文案时的工作流

1. 中文键加到 `locales/zh-CN/<ns>.ts`
2. 同名键加到 `locales/en-US/<ns>.ts`（缺了会被键对齐测试拦下）
3. 组件里写 `t("...")`（写死中文会被扫描拦下）

三步都有机器兜底，漏不掉；但要**真的写出英文**，这一步没有工具能替代。

## 三、格式约定

- 中文全角标点（`「」`、`（）`、`：`）转为英文半角（`"`、`()`、`:`）
- 中文书名号/引号 `「」` → 英文双引号 `"`；套用 `'` 时用单引号
- 短标签（按钮、导航、字段名）**句末不加句号**；完整句子加句号
- 英文省略号用 `...`（三连点），不用中文 `……`
- 首字母大小写：导航项与按钮用句子式大小写（`Check for updates`），不用标题式（`Check For Updates`）
- 占位符沿用 i18next 的 `{{name}}` 语法，不与中文 `{}` 混用
- 英文比中文长约 30–50%，按钮/导航栏需在 `pnpm dev:tauri` 里人工确认不截断
