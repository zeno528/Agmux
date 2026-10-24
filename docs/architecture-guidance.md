# Agmux 架构与开发边界

本文按任务查阅，解释跨文件约束与现有 owner；日常工作底线与命令在根目录 `AGENTS.md`，构建/发行等步骤在 `.codex/skills/`。具体字段、常量和检查项以对应源码与测试为准，不在多处维护副本。

## 模块职责与调用链

Agmux 是同时管理 Codex 与 Claude Code 的本地桌面应用，支持 macOS / Windows。

```text
页面与编辑草稿
  → src/api/index.ts（Tauri 调用 / 浏览器 mock 动态路由）
  → src-tauri/src/commands.rs
  → services/AppContext 与各业务服务
  → 客户端实时文件 + SQLite
```

| 职责 | owner |
|---|---|
| 根状态、窗口/首屏、轮询与管理缓存 | `src/app/{AppShell.tsx,appShellHooks.ts,managementDataCache.ts}` |
| 客户端页面与共享配置控件 | `src/features/{codex,claude,profiles}/`；`src/components/` |
| MCP / 插件 / Skills / 账号 / 设置 / 更新 | `src/features/` 对应功能目录；后端 `services/` 对应模块与 `auth/` |
| Codex 配置解析与进程 | `src-tauri/src/codex/` |
| 配置用例与两端同步 | `services/{codex_profiles,codex_apply,claude,codex_accounts}.rs`、`services/sync/` |
| 存储、迁移、恢复与文件保护 | `database.rs`、`services/storage.rs`、`fsutil.rs`、`paths.rs` |
| 网络、CLI 与平台环境 | `network.rs`、`services/{cli,claude_cli,codex_cli,claude_terminal,shell_env}.rs` |
| 共享 UI、主题、文案检查 | `src/components/`、`src/style.css`、`src/i18n/`、`scripts/check-*-leaks.mjs` |

表中未写全的 Rust 路径相对 `src-tauri/src/`。改共享能力先检索全部调用方；只有实际出现独立职责或多个真实契约，才拆分或抽取。不按文件行数、hook 数量或未来平台规划创建适配层。

## 数据与写入边界

### 三种数据归属

- **客户端实时文件**：`~/.codex/` 下的配置、模型目录与认证；Claude Code 的 `~/.claude/settings.json` 与用户级 MCP `~/.claude.json`。这些包含用户的全局状态，不是可随意重建的缓存。
- **应用持久化**：`~/.agmux/agmux.db` 保存供应商快照、账号、MCP 镜像及状态；`settings.json` 保存应用设置。路径统一由 `paths.rs` 构造，SQLite schema 与迁移归 `database.rs`。
- **程序、缓存与开发产物**：客户端安装状态由各自管理接口确认，缓存不能冒充已安装状态。项目工具链、`node_modules/`、`dist/`、`src-tauri/target/` 与用户数据分开，不随数据库恢复。

### 编辑、应用与回写

1. 编辑保存只更新对应数据库记录；应用才把选中配置写入 live。外部更改只应回写当前激活项，不扫描全部供应商全文。
2. 修改这条链路必须同时追踪表单、payload、完整快照与实际文件：Codex 的 `raw_config` / `raw_catalog` / `raw_auth`，Claude 的 `raw_settings` / `extra_env`。不能只让表单显示正确而丢掉未知字段。
3. 应用保留无关的全局 MCP、插件等配置。复用 `backup_file`、`atomic_write`、`with_file_rollback` 和既有操作锁；备份名保持 `<name>-<unix_ms>.bak`，保证恢复逻辑识别。
4. 认证写入同时校验归属与新鲜度，沿用 `sync_external_auth_json` 等既有判定；不能用旧 token 快照覆盖刷新后的凭证，也不能因一次切换结束较晚覆盖更新的激活操作。
5. SQLite 迁移只新增。WAL/SHM 运行文件不能手工删除；config、codex-files、database 三类备份职责不同，不能为减少目录而合并。恢复和多文件写入必须验证中途失败后的数据状态。
6. Claude 通用模板存于 `app_state.claude_common_settings`，只在打开模板或手动填充时读取；复制进草稿后随 Profile 保存，不在启动或应用时叠加。

### MCP 与插件状态

- MCP 的实时内容、数据库镜像与两端安装/开关状态是不同层次。沿用 `services/mcp.rs` 的差异预览与裁决；新增有效条目和开关可按既有流程自动跟随，其他修改/删除不能擅自覆盖。
- Codex 官方应用自动维护的 MCP 条目由 `codex/config.rs` 的 `MANAGED_MCP_SERVERS` 识别：列表、差异与数据库镜像跳过，重建实时 MCP 段时保留。用户条目的未建模字段与注释也应保留，不在规则里再复制一份名称清单。
- Codex 插件的安装清单与操作能力来自 CLI 管理接口，缓存目录不代表已安装，origin 标签不代表操作权限。预览、安装、卸载沿用 `services/plugins/` 的既有职责与官方 CLI 链。

## 客户端与供应商接入

### 新客户端复用同步底座

客户端指拥有独立实时文件并需要 live→库回写的引擎。现有执行时机与统一结果收集在 `services/sync/mod.rs`；新增客户端实现 `ClientSync`，扩展 `ClientId`、产品名 `label()` 与 `registry()`，不要在 feature 内另起同步链。

- 守卫返回与日志约定直接读 `ClientSync` 契约和 `SyncKind`；它们已覆盖无激活项、目标不符、文件缺失、读取/解析失败、一致、写入及写库失败。读取失败与不存在必须区分，激活项必须主键点查，回写结果统一由 `harvest` 记录。
- 服务只实现该客户端的解析、比对与写回；独立存储通过新增 migration 接入，command、API、web mock、中英文文案一起检查。
- 复用 `services::profile_copy_plan` / 浏览器 `profileCopyPlan` 的复制命名与插入位置，应用成功复用 `common:feedback.applySuccess`。
- 回归沿用 `services/tests.rs` 与 `claude.rs` 的 `corrupted_live_*` 场景：外部修改确实写回并带 profile，破坏后返回明确原因且保住最后有效快照，修复后链路自愈。仅断言错误类型不够。

### 供应商目录以声明为准

目录为 `src/presets.ts` 的 `providerPresets`：名称/图标共享，`codex` / `claude` 配置独立；缺哪个客户端声明就不进哪个列表，不跨客户端继承地址、模型或区域端点。

| 需要更新的 owner | Codex | Claude Code |
|---|---|---|
| 目录、图标、中英名称 | `presets.ts`、已有品牌 SVG / `icons.ts`、两份 locale | 同左 |
| 客户端模板 | `assets/builtin/` + `codex_builtin.rs` 注册 | `presets.ts` 的 Claude 声明，不加 Codex 资源 |
| 现有验证 | `presets.test.ts`、`codex_builtin.rs` 测试 | `presets.test.ts`、`profileEnvText.test.ts` 与 `claude.rs` 测试 |

多套餐使用既有 `group` / `plan` 与 `presetEntries`；`plan="API"` 保留为按量入口，不另注册一套菜单。模型与端点的实际值随模板和目录测试维护，不在规则中固定版本。

几个局部源码不容易揭示的双端约束需一起检查：

- `ANTHROPIC_API_KEY` 型供应商集合同时在前端 `profileEnvText.ts` 的 `CLAUDE_API_KEY_KINDS` 与后端 `claude.rs` 的 `uses_api_key`，必须一致。
- OpenRouter 写入空 `ANTHROPIC_API_KEY` 的分支同时在前端 `buildSettingsText` / `patchEnvFields` 与后端 apply；新增同类网关时两侧一起改，不恢复已删除的 `clear_api_key` 标志。
- Claude 预设的模型固定留空；Codex 默认值按 builtin 模板声明。密钥占位符统一 `<YOUR_API_KEY>`，token 使用 `experimental_bearer_token`。

## 前端与启动边界

- AppShell 聚合根 `AppState` 与导航，`managementDataCache` 共享管理数据、在途请求及持久化缓存；feature 保留编辑草稿等局部状态，不另造全局 store。接口类型同时检查 `src/types.ts` 与 Rust `models.rs`。
- 页面通过 `<Activity>` 保活，未访问页面不渲染；新增页补 `renderPages` 的完整 `Record<AppView, ReactNode>`，不以重挂载、reset key 或含 view 的外层 key 清空草稿。重复点击当前侧栏经 `navTo` 弹回一级；托盘等程序化跳转仍用 `setView`。
- 管理页进入时刷新；已有首帧后预热和全局定时任务允许复用，但不得前移到窗口显示或首帧。具体延时、冷却与缓存失效由实现和 `managementDataCache.test.ts` 维护。
- 普通页由 AppShell `<main>` 纵向滚动，编辑页由 `.apple-edit-content` 滚动中间区，代码区随内容撑高；配置编辑器操作栏复用 `.apple-editor-toolbar` 吸顶，不另造纵向滚动容器。页头/操作分别复用 `.apple-page-bar` / `.apple-action-button`，排版归共享样式。
- 组件优先用现有 App 包装；Radix 只在 `components/App*.tsx` 与 `Feedback.tsx` 内使用。反馈/确认走 `useFeedback()`，配置编辑走 `ConfigTextEditor`，等宽分段走 `AppSegmentedControl` 并保持选中索引与 ARIA 状态一致；图标用 lucide，默认 18、按钮内 16、strokeWidth 2。
- 字号、颜色、字体、间距、主题使用 `style.css` token；新增主题 token 同时维护浅深两套，新增全局类注释说明意图。CSS 使用逻辑像素，不固定套用某台机器的截图缩放比。
- 具体禁用 class、页头与滚动约束由 `check-style-leaks.mjs` 执行；例外精确到文件/class 并说明理由。文案遵循中英 locale 与术语表，`check-i18n-leaks.mjs`、键对齐和类型检查共同约束，不在根规则抄录 token 与文案清单。

## 网络、CLI与日志边界

### 系统能力

- 应用 HTTP、更新及安装子进程共用 `network.rs` 的操作级网络快照。自动/关闭/自定义代理只写应用设置，回环直连；代理失效不回退直连，调用方保留超时和认证。HTTP 沿用 reqwest 默认 TLS，不另引入原生 TLS 构建链。
- CLI 共享进程与官方脚本执行链归 `services/cli.rs`，客户端差异归 `claude_cli.rs` / `codex_cli.rs`；Agent 工具页、升级胶囊与后台调度复用同一缓存服务，不复制一套检查逻辑。
- 检查更新只读官方版本；升级须由用户触发，后端复核检查结果、当前版本与入口。只接管原生/官方独立安装，禁止混装，不接管桌面内置 Codex CLI；安装验证与布局交给官方安装器，保留包 SHA256 验证和自定义独立安装目录。
- Claude 引导/升级使用既有临时 settings 隔离用户 `settings.env` 与本次网络快照；成功后核实绝对路径的本机版本。平台 shell、冲突保护、代理注入与进度细节归实现，不在全局规则维护副本。
- Mac 从 Finder 启动时的环境读取归 `shell_env.rs` / `claude_terminal.rs`，与开发终端环境分开；不靠修改全局 shell 配置修复应用启动链。

### 日志

管道单点归 `lib.rs`，业务用 `tauri_plugin_log::log`，不自造包装或逐函数记录。格式为 `[域.子域.动作] <定位字段> outcome=<结果> <诊断字段> msg="静态中文"`，产品名称与 UI 术语一致；时间、级别、轮转在管道维护。

- 同一操作的成功与失败共用事件标签；`outcome` 为 `success` / `failure` / `denied` / `skipped`，守卫与无变化用 skipped，不伪报 success。定位字段只留 client/profile/id、server 或请求状态等必要信息，跳过必须说明 reason。
- 失败附 `failure_kind`：`http_error`、`timeout`、`parse_error`、`io_error`、`auth_error`、`region_blocked`、`network_error`、`not_found`、`validation_error`、`protocol_error` 或 `internal`；增加类别时同步本契约。
- 外部文本按 Rust `{:?}` 编码，空值用 `-`，不输出 `Some(...)` 或枚举/结构体 Debug。外部 error 截断脱敏；内部 ID 不截断；动态值放字段，不拼进 msg。
- 禁止密钥、token、配置原文；email 仅 debug 构建。toml_edit 错误可能带原始配置，不能直接落日志；JSON 行列错误可记录。
- 高频无变化降 Trace，静默成功/周期任务用 Debug，用户里程碑用 Info，最终失败只记一次 Warn，Error 只用于 panic；后台失败不扰用户，主动操作失败必须显示可行动反馈。

## 规则与验证的归属

| 信息 | 唯一维护位置 | 何时读取 |
|---|---|---|
| 项目底线、DEV 命令、任务入口 | 根 `AGENTS.md` | 开始工作 |
| 跨模块职责、数据语义与接入边界 | 本文 | 涉及对应模块 |
| 构建、发行、完整 CI、测量等操作步骤 | `.codex/skills/` 对应技能 | 用户任务需要该流程 |
| 确定性格式、常量、缓存策略、失败保护 | 源码、现有测试、检查脚本 | 实现或验证对应行为 |
| 当前机器安装路径与工具版本快照 | `work/dev-env/README.md` 及工具输出 | 安装或环境排查 |
| 供应商外部依据与历史发布事实 | `docs/provider-audit-*.md`、CHANGELOG | 核查对应事实，不能当作最新接口文档 |

现有机器约束足以在根规则表达，不为每个目录补一份 AGENTS。只有某模块反复需要独立且不与父级重复的约束，才考虑局部规则。新规则应改变具体决策，并有明确 owner；已过时的约束直接删除，不把旧文件整段搬到下一层。
