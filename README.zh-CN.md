<p align="center">
  <img src="src-tauri/icons/icon.svg" width="112" alt="Agmux logo" />
</p>

<h1 align="center">Agmux</h1>

<p align="center"><strong>你的 AI，汇聚于此。</strong></p>

<p align="center">
  为 OpenAI Codex 与 Claude Code 打造的一站式开源桌面管理器。<br />
  一键切换任一客户端的供应商配置，管理 ChatGPT 账号，统一管理 MCP 服务器、插件与 Skills。
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="https://github.com/zeno528/Agmux/releases/latest">最新发行版</a> ·
  <a href="CHANGELOG.md">更新日志</a> ·
  <a href="https://github.com/zeno528/Agmux/issues">问题反馈</a>
</p>

<p align="center">
  <a href="https://github.com/zeno528/Agmux/releases/latest"><img src="https://img.shields.io/github/v/release/zeno528/Agmux?display_name=tag&sort=semver&style=flat-square" alt="最新发行版" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT License" /></a>
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square&logo=tauri&logoColor=white" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS-555?style=flat-square" alt="Windows 和 macOS" />
</p>

Agmux 面向使用 OpenAI Codex 或 Claude Code 的开发者，围绕电脑上这两个客户端的本地环境工作。它将供应商配置、ChatGPT OAuth 账号、MCP 服务器、插件和 Skills 的管理放进一个桌面应用，减少在多个配置文件和工具之间来回操作。

## Agmux 如何融入工作流

```text
供应商模板或现有客户端配置
              ↓
          保存为配置
              ↓
       编辑 · 测试 · 使用 · 恢复
```

Codex 与 Claude Code 并排管理：两个客户端各有自己的供应商页，在一侧使用配置不会改写另一侧。使用供应商配置前，Agmux 会备份相关文件。切换模型供应商时，MCP、Plugins 和 Skills 等全局资源仍保持独立，不必重新配置。

## 功能

### Codex 供应商配置

- 使用内置供应商模板、捕获当前 `~/.codex/config.toml`，或创建自定义供应商。
- 在应用内编辑 `config.toml`、`models.json` 和 `auth.json`，提供 TOML/JSON 校验。
- 从供应商的 `/models` 接口获取可用模型，并在供应商配置编辑器中选择。
- 重命名、复制、排序、删除和使用配置。
- 使用配置时，尽可能保留 MCP、插件等无关 Codex 配置内容。
- 为配置设置自定义显示名称、供应商图标、管理后台地址、可选描述，并可绑定 ChatGPT 账号。描述只保存在 Agmux，不会写入 Codex 配置。

### 当前支持的供应商模板

创建供应商时按三类展示，并可搜索供应商名、套餐名和官网地址：

- **账号授权** —— 直接使用客户端自身的登录态，不需要填写接口地址。Codex 为 `ChatGPT`，Claude Code 为 `Claude Account`。
- **套餐服务** —— 按订阅套餐计费。套餐名跟在供应商名后面，一眼能看出买的是哪个包，例如 `MiniMax · M Plan`。
- **按量付费** —— 按调用量计费，用 API Key 直接调用。

同一供应商的多个套餐共用图标与主条目，入口名后缀区分套餐；小米 MiMo 的按量与 Token Plan 端点不同，因此不拆入口，在同一张卡片内切换计费方式。区域端点（国内 / 全球）在版本滑块中选择，不进入口名。

两个客户端各自声明支持的模板，同名模板共享名称与图标，但**接口地址、模型和区域端点各自独立，互不继承**：

- **两侧都提供** —— `DeepSeek`、`MiniMax`、`Zhipu`、`OpenCode`、`OpenRouter`、`Xiaomi MiMo`、`Kimi`、`Qwen`、`Tencent Hunyuan`、`Volcengine Doubao`、`Baidu Qianfan` 和 `Custom`。
- **仅 Codex 提供** —— `ChatGPT`、`xAI（Grok）`。
- **仅 Claude Code 提供** —— `Claude Account`、`Anthropic API`、`SiliconFlow`、`LongCat`。

`Qwen`、`Tencent Hunyuan`、`Baidu Qianfan` 的部分套餐并非两个客户端都有，具体以创建页实际显示为准。自定义供应商可以使用 Codex 支持的 Responses API 兼容配置，或 Claude Code 支持的 Anthropic 兼容配置，并自行填写接口地址、密钥和模型目录。

### Claude Code 供应商

- 使用内置 Claude 模板、把当前 `~/.claude/settings.json` 存为快照，或创建自定义供应商。
- 供应商字段写入 `~/.claude/settings.json` 的 `env` 块；整个文件仍可编辑并做 JSON 校验，其他顶层键和 `env` 条目原样保留。
- 第三方网关选择 `Anthropic 兼容`协议与国内或全球端点；密钥按模板写成 `ANTHROPIC_AUTH_TOKEN` 或 `ANTHROPIC_API_KEY`。
- 把请求模型映射到 Claude Code 的模型角色 —— 主模型、Fable、Opus、Sonnet、Haiku 和子代理 —— 也可以追加自定义 `/model` 条目；`1M` 开关会在模型 ID 后追加 `[1m]`，请求百万 token 上下文。
- `Claude Account` 使用 Claude Code 中已登录的账号：先在 Claude Code 里执行 `/login`。使用该配置会移除 `settings.json` 中第三方的接口与认证覆盖项，Agmux 既不读取也不保存登录凭据。
- 连通测试用已保存的凭据请求真实调用路径 `/v1/messages`；获取模型列表是独立的可选步骤，没有模型目录的网关同样能通过连通测试。
- 可把共用字段保存为通用模板，再填入任意配置；接口地址、凭据、模型映射和应用托管字段不在模板范围内，模板改动也不会回写已有配置。
- 在支持查询余额或用量的模板（目前是 DeepSeek 和 MiniMax）上，可按配置开关余额或用量指示条。
- 可在指定项目目录为该供应商开一个独立终端：卡片上点终端图标选择目录（保留最近使用过的目录），Agmux 把该配置写成 `claude --settings` 的进程级覆盖后启动。覆盖文件只进应用自己的临时目录，不写 `~/.claude`，也不参与「使用」与实时回写 —— 终端会话因此与其他配置互不干扰。macOS 可在设置中选择用 Terminal.app 或 Ghostty。
- 高级选项包括自动压缩及其窗口、effort 等级、agent teams、自动记忆、隐藏 AI 署名、跳过权限确认，以及显示 Shell 文件改动。

### ChatGPT 账号与用量

- 在系统浏览器中通过 OAuth 登录 ChatGPT。
- 管理多个账号、设置默认账号，并将账号绑定到指定供应商配置。
- 测试 ChatGPT 认证状态，并在可用时查看配额信息。
- 测试第三方供应商的 API 连通性、响应状态和延迟。
- 查看已支持供应商的余额或用量；ChatGPT 配额单独处理。

### MCP 管理

- 在 Codex 与 Claude Code 之间切换 MCP 页面。Codex 管理 `~/.codex/config.toml` 中全局的 `[mcp_servers.*]` 配置，Claude Code 管理用户级的 `~/.claude.json` 配置。
- 配置本地 `STDIO` 服务器和远程 `HTTP` / Streamable HTTP 服务器。
- 编辑命令、参数、URL、Bearer Token 环境变量、请求头、环境变量和超时。
- 测试服务器连通性并查看其提供的工具，探测经系统代理进行，两个客户端分别探测、分别缓存。
- 启用、停用或卸载只作用于当前所在的客户端，另一侧的服务器与配置保持原样。
- 在结构化表单与源码编辑之间切换 —— Codex 为 TOML，Claude Code 为 JSON —— 并提供校验和格式化。
- 在同步前比较客户端实际配置与 Agmux 数据库镜像，明确选择同步方向。

### Plugins 与 Skills

- 列出已安装的 Codex 插件，查看版本、能力、内容、来源和安装路径。
- 浏览内置及外部插件 Marketplace。
- 市场目录的安装状态与 Codex 桌面端保持一致，通过应用安装的插件桌面端可直接识别。
- 已安装插件排在市场列表前方；支持仅按插件名搜索，快捷键 Ctrl/Cmd+K。
- 安装需要登录授权的插件时，会提示前往 Codex 桌面端完成认证。
- 从 GitHub 简写、Git、SSH 或本地 Marketplace 目录添加市场源。
- 预览并从 GitHub 仓库安装插件，可选指定分支或子目录。
- 检查并升级第三方 Marketplace 插件，也可以通过 Codex CLI 卸载插件。
- 导入本地 Skill，预览 `SKILL.md`，检测更新和冲突，并启用、禁用或删除受 Agmux 管理的 Skill。
- 扫描 `~/.codex/skills`、`~/.claude/skills` 和 `~/.agents/skills` 中的 Skill，同时将 Agmux Skill 注册表与插件内置 Skill 分开管理。
- 受管理的 Skill 按客户端分别启用：Codex 与 Claude Code 两份独立开关，各自显示已启用数量；删除时会一并移除 Agmux 仓库副本和两个客户端副本。

### Agent 工具

- 在同一页集中管理 **Codex CLI** 与 **Claude Code CLI**：查看是否已安装、当前版本，并执行安装与升级。
- 只接管走官方脚本的原生 / 独立安装。检测到其他安装方式或同一工具存在多份安装时会阻止混装并说明原因；桌面端内置的 Codex CLI 可与应用共存，Agmux 不会接管它。
- 检查更新只读取官方渠道的版本信息，确认存在更高版本后才显示升级入口，升级始终由用户主动点击执行。Claude Code 走官方 `update`，Codex 交给官方安装器并校验安装结果与入口未变。
- 更新检查结果全局共享并有 6 小时冷却：进入本页会静默检查一次，后台每小时懒轮询，只有确实发现新版本时才在该客户端的供应商页标题旁显示同款升级胶囊，点击即可升级。
- 两个客户端的任务彼此独立，一端的安装或升级不会影响另一端。

### 桌面体验

- 基于 Tauri 2，提供 Windows 和 macOS 桌面版本。
- 支持浅色、深色和跟随系统主题。
- 支持英文和简体中文界面，并可自动检测系统语言。
- 支持开机启动、静默启动和关闭时最小化到托盘。
- 可设置启动时默认打开的页面；从托盘恢复不受影响，首帧直接落在目标页不闪。
- 系统托盘菜单提供快速操作：切换供应商、打开设置、跳转到账号页、显示主窗口；托盘单击行为可设为"显示主窗口"或"显示菜单"。
- 使用 Codex 配置后可选择自动重启 Codex；Claude Code 配置不会重启任何进程。
- 可自动检查应用更新，安装前先看更新日志，下次启动弹出"已更新到 vX"提示。
- 自动备份本地数据库、配置文件和 Codex 文件；数据库备份可在设置中浏览和恢复。

### 设置

设置页分四个 tab：

- **常规** — 外观与语言（主题、界面语言）；启动行为（开机自启、静默启动、关闭时最小化到托盘、启动页面、macOS 终端应用）。
- **应用** — 应用与更新（托盘单击行为、切换配置后自动重启 Codex、启动时自动检查更新）；ChatGPT 账号管理（OAuth 授权登录、多账号管理、查看用量与额度、可用重置次数、套餐续期日、获取模型与账号预热）。
- **数据管理** — 数据库备份：立即备份、导出到文件、从文件恢复、自动备份（备份频率与最多保留数量独立配置）、备份重命名与删除、可折叠的备份记录。恢复会覆盖当前所有供应商数据且无法撤销，备份中的 MCP 配置会一并写回 `~/.codex/config.toml`。
- **关于** — 应用信息卡片（版本号、检查更新、GitHub / 更新日志链接）和数据与路径列表（应用数据目录、备份目录、日志目录、Codex 配置）。

**网络代理**单独成组，提供三种模式：

- **自动** — 优先沿用环境变量，其次读取 Windows / macOS 系统代理；都没有则直连。
- **关闭** — 强制直连，不读取任何系统代理设置。
- **自定义** — 填写 HTTP(S) 代理地址，只接受有效地址。

代理只作用于应用自身的网络请求（余额与用量查询、连通性测试、插件市场、应用更新和 Agent CLI 安装），不会写进任何客户端配置，也不会改动 Codex 或 Claude Code 的设置。已配置的代理无效或连接失败时不会回退直连。

### MCP 差异

当客户端的实时配置与 Agmux 的 MCP 镜像不一致时，进入专用差异处理页：列出每一项不同的 server，红色 / 绿色 LCS 行级 diff，支持单条或批量 `adopt` / `revert`。Codex 比对 `~/.codex/config.toml`，Claude Code 比对用户级 `~/.claude.json`。「撤销」采用数据库版本时只替换 MCP 区域，配置文件的其余部分原样保留；实时文件被改坏而无法解析时不会盲目覆写，仍保留最后一次有效快照。

## 下载与安装

请从 [GitHub Releases](https://github.com/zeno528/Agmux/releases/latest) 下载最新版本。

| 平台 | 推荐文件 | 说明 |
| --- | --- | --- |
| Windows x64 | `Agmux-v{VERSION}-Windows-setup.exe` | 常规安装程序。 |
| Windows x64 | `Agmux-v{VERSION}-Windows.msi` | 适合部署或 MSI 安装场景。 |
| macOS Apple Silicon | `Agmux-v{VERSION}-macOS-arm64.dmg` | 适用于 Apple Silicon 芯片。 |
| macOS Intel | `Agmux-v{VERSION}-macOS-x64.dmg` | 适用于 Intel 芯片。 |

### macOS 首次打开

打开 DMG，将 **Agmux** 拖入「应用程序」，然后启动。如果 macOS 阻止打开，请先在「系统设置 → 隐私与安全性」中允许。如果仍提示应用无法打开，在终端执行：

```bash
xattr -cr /Applications/Agmux.app
```

如果应用安装在其他位置，请将命令中的路径替换为实际 `.app` 路径。目前官方发行包支持 Windows 和 macOS，暂不提供 Linux 安装包。

## 快速开始

1. 在「Codex」分组进入「供应商」，选择内置模板或「Custom」，填写凭据或绑定 ChatGPT 账号，然后使用。
2. 在「Claude」分组进入「供应商」配置 Claude Code，或把当前 `~/.claude/settings.json` 存为快照。
3. 如果希望使用后自动重启 Codex，在「设置 → 应用」中开启对应选项。
4. 需要管理全局资源时，从「通用」分组进入「MCP」「插件」「Skill」或「Agent 工具」。MCP 一次只管理一个客户端，Skill 则可以同时为两个客户端启用，Agent 工具用于安装或升级 Codex CLI 与 Claude Code CLI。

## 数据与隐私

Agmux 将应用数据保存在当前用户的主目录下，实际文件和目录会随使用过的功能而变化：

```text
~/.agmux/
├── settings.json
├── agmux.db
├── balance-cache.json
├── logs/
│   └── agmux.log
├── skills/            Agmux 管理的 Skill 仓库副本
├── terminal/          Claude Code 终端的 per-session 覆盖文件
├── update-marker
└── backups/
    ├── config/
    ├── database/
    └── codex-files/
```

运行日志写在 `~/.agmux/logs/` 下（1MB × 10 轮转）。

从旧名称版本升级时，应用不会自动迁移旧数据目录。请先在旧版本的「设置 → 数据管理」导出数据库备份，再在 Agmux 中恢复；旧目录会保留。

Agmux 不读取也不保存 Claude Code 的登录态：`Claude Account` 配置只是让 Claude Code 去掉第三方的接口与认证覆盖项。Claude 通用模板保存在 Agmux 数据库中，因此随数据库备份一起备份。

API Key、OAuth 凭据、配置和备份都属于本地数据。Agmux 会在相关配置写入前创建备份，但仍请不要将 `.agmux`、`auth.json`、`~/.claude.json`、API Key 或备份文件提交到 Git 或分享给他人。

## 常见问题与排查

### 点击「使用」会发生什么？

Agmux 会先备份相关文件，再更新供应商相关的 Codex 配置，并尽可能保留其他配置区域。操作完成后是否重启 Codex 由「设置 → 应用」决定。

### 供应商配置、MCP、Plugins 和 Skills 是一回事吗？

不是。供应商配置描述模型和供应商设置；MCP 描述工具服务器；Plugins 是 Codex 扩展包；Skills 是可复用的指令目录。它们在应用的不同区域管理。

### Agmux 会接管我的 Claude Code 登录吗？

不会。先在 Claude Code 里执行 `/login`，再使用 `Claude Account` 配置 —— Agmux 只是移除 `settings.json` 里第三方的接口与认证覆盖项，让 Claude Code 改用自己的账号。它不读取也不保存登录凭据，Shell 环境里的认证变量也需要自行清理。

### 为什么配置了第三方供应商，插件仍然无法使用？

模型供应商的配置并不能保证所有 App 或 MCP Connector 插件都能加载。部分 Connector 插件还需要兼容的官方 ChatGPT 认证或其他依赖。插件已经安装但 Connector 不可用时，请查看插件自身要求。

### 为什么连接测试失败？

第三方供应商先检查接口地址和 API Key；官方 ChatGPT 配置需要先在账号设置中完成登录，并确认选中的账号仍然有效。如果两边都对，可能是网络问题：在「设置 → 网络」确认代理模式是「自动」还是需要手动填写地址 —— 代理无效时应用不会回退直连，因此一台配置了失效代理的机器会让所有网络功能一起失败。运行日志位于「设置 → 关于 → 日志目录」，按域前缀能定位到失败原因。

## 开发

### 环境要求

- Node.js
- pnpm `11.9.0`
- 由 [`src-tauri/rust-toolchain.toml`](src-tauri/rust-toolchain.toml) 固定的 Rust 工具链
- Tauri 2 所需的平台开发依赖

### 安装与启动

```bash
pnpm install
pnpm dev:tauri
```

`pnpm dev:tauri` 会启动 Vite 前端和真实的 Tauri 桌面窗口。只开发前端时可以使用：

```bash
pnpm dev
```

### 检查与构建

```bash
pnpm typecheck
pnpm test:unit
pnpm check
pnpm build
pnpm build:debug
```

`pnpm check` 会运行前端和 Rust 质量检查。`pnpm build` 生成 Web 构建产物；`pnpm build:debug` 生成 Tauri 调试包。需要在本地打包发行安装程序时运行：

```bash
pnpm tauri build
```

发行包位于 `src-tauri/target/release/bundle/`。

## 架构

Agmux 使用 Tauri 2 + Rust 负责原生文件访问、进程管理和与 Codex / Claude Code 的集成，前端为 React，本地数据存放在 SQLite。应用只写自己管理的那部分实时配置，写入前先备份，并在必要时把外部改动回写进数据库。

主要源码目录：

```text
src/
├── main.tsx     React 入口
├── style.css    全局 token、布局与样式
├── presets.ts   供应商目录与预设行
├── icons.ts     内置供应商图标注册表
├── types.ts     共享 TypeScript 类型
├── api/         类型化 IPC 方法和浏览器 mock
├── app/         应用壳层、导航、状态、轮询和管理数据缓存
├── assets/      打包的供应商图标与静态资源
├── components/  共享 UI 组件（AppDialog、AppSelect、ConfigTextEditor、CliUpgradePill 等）
├── features/    profiles、codex、claude、mcp、plugins、skills、accounts、settings、updates
└── i18n/        英文与简体中文文案

src-tauri/src/
├── main.rs          可执行入口
├── lib.rs           Tauri runtime、command 注册与插件装配
├── commands.rs      Tauri command 边界
├── error.rs         类型化应用错误
├── fsutil.rs        文件系统工具（atomic write 等）
├── network.rs       应用网络策略（自动 / 关闭 / 自定义代理）
├── services/        AppContext 与用例
│   ├── codex_profiles.rs / codex_apply.rs / codex_profile_config.rs   Codex 供应商读写与应用
│   ├── claude.rs / claude_terminal.rs   Claude Code 供应商与独立终端
│   ├── cli.rs / codex_cli.rs / claude_cli.rs   Agent CLI 安装、检查更新与升级
│   ├── mcp.rs / mcp_probe.rs   MCP 镜像、差异处理与连通性探测
│   ├── plugins/      插件与 Skill 的目录、商店与安装
│   ├── connections.rs / model_fetch.rs   连通性、余额与用量
│   ├── settings.rs / storage.rs   应用设置与备份存储
│   └── sync/         实时配置 → 数据库的回写时机层
├── codex/           Codex 配置文件、进程与窗口状态
├── auth/            OAuth 与账号认证
├── database.rs      SQLite 连接、schema 与迁移
├── models.rs        Rust 领域模型与 command DTO
├── codex_builtin.rs 内置供应商资源与模板
└── paths.rs         文件系统路径工具
```

## 参与贡献

欢迎提交 Bug、功能建议、文档改进和 Pull Request。代码改动请运行上面的相关检查，并确保凭据、本地数据库和生成的构建产物不会进入提交。

- [提交 Issue](https://github.com/zeno528/Agmux/issues)
- [查看更新日志](CHANGELOG.md)

## 许可证

Agmux 使用 [MIT License](LICENSE) 发布。
