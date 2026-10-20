/** 插件页文案。 */
export default {
  title: "插件",
  inventory: {
    label: "Codex 内容类型",
    apps: "应用",
    loadingApps: "正在读取 Codex 已安装应用…",
    noApps: "没有匹配的已安装应用",
  },
  marketplace: "插件市场",
  nav: {
    backToPlugins: "返回插件",
  },
  // 插件来源 / 市场类型的数据值 → 展示标签
  origin: {
    official: "官方市场",
    thirdParty: "第三方市场",
  },
  source: {
    openGithub: "打开 GitHub",
    openSource: "打开来源",
    openTitle: "打开来源：{{url}}",
  },
  action: {
    install: "安装",
    toggleLabel: "启用或禁用 {{name}}",
    managed: "由 Codex 管理",
    managedHelp: "请在 Codex 桌面端启用或禁用此插件",
    uninstall: "卸载",
    checkUpdates: "检查更新",
    upgradeAll: "全部更新",
    removeMarket: "卸载市场",
  },
  status: {
    on: "已启用",
    off: "已禁用",
  },
  detail: {
    marketplace: "市场",
    installPath: "安装位置",
    skillCount: "{{count}} 个 Skill",
    loadingSkills: "加载 Skill 明细…",
    skillsEmpty: "这个插件没有可读取的 Skill 明细",
  },
  marketDetail: {
    thirdPartyNotice: "当前为第三方模型：插件包会保留；带 App 或 MCP 连接器的插件可能无法在 Codex 中加载。需要授权或使用连接器时，请切换到 ChatGPT 官方订阅配置",
  },
  market: {
    pluginCount: "{{count}} 个插件",
    removeAria: "卸载插件市场 {{name}}",
    foundUpdates: "{{client}}：{{count}} 个插件可更新",
    upToDateToast: "{{client}}：插件已是最新",
    upToDate: "所有插件都已是最新版本",
    allUpgraded: "第三方插件已全部更新",
    removedToast: "已卸载插件市场「{{name}}」",
    removeConfirmTitle: "卸载第三方市场",
    removeConfirm: "确定移除「{{name}}」吗？只会移除市场来源，不会自动卸载其中已经安装的插件",
  },
  recommended: {
    openaiCurated: "OpenAI 官方 Codex 插件市场，提供可安装的应用集成与开发工作流",
    ponytail: "偏向最小实现的开发工作流，强调 YAGNI、标准库和原生能力",
  },
  add: {
    marketSourceTitle: "添加市场来源",
    marketPlaceholder: "owner/repo、https://...git 或本地市场目录",
  },
  list: {
    empty: "还没有安装插件。点击右上角「市场源」添加第三方市场",
    searchPlaceholder: "搜索插件…",
    noMatch: "没有匹配的插件",
    uninstallAria: "卸载 {{name}}",
    sourceKept: "市场源保留",
    sourceKeptMarket: "市场源 {{marketplace}} 保留",
    removeConfirm: "确定卸载「{{name}}」吗？将通过 codex CLI 卸载（{{source}}），之后可在 Codex 或这里重新安装",
  },
  confirm: {
    uninstallPluginTitle: "卸载插件",
    removePlugin: "确定卸载「{{name}}」吗？将通过 Codex CLI 移除插件文件",
  },
  toast: {
    installed: "已安装插件「{{name}}」，重启 Codex 后生效",
    installedNeedsAuth: "「{{name}}」安装成功，请在 Codex 桌面端完成授权",
    uninstalled: "已卸载插件「{{name}}」",
  },
  // 双引擎切换（与 MCP 页同构）：Codex 与 Claude Code 插件体系独立
  target: {
    label: "选择插件引擎",
    codex: "Codex",
    claude: "Claude",
  },
  // 看板层级：客户端维度 → 通栏搜索 → 一级过滤 chips → 分区网格（Codex / Claude 两端共用）
  board: {
    codexInstalledHelp: "开关修改用户级设置，项目或管理员设置可覆盖；已有会话可能需重启。标注“由 Codex 管理”的插件，请在 Codex 桌面端启停",
    codexAvailableHelp: "来自当前 Codex 账号和已添加的市场。需要授权的插件，安装后请在 Codex 桌面端完成授权",
    filterAll: "全部",
    filterInstalled: "已安装",
    filterAvailable: "可安装",
    filterUpdates: "有更新",
    sectionInstalled: "已安装",
    sectionAvailable: "可安装",
    sectionUpdates: "有更新",
    upgradeTo: "升级到 v{{version}}",
  },
  // 市场源管理：从主流程退到弹层，浏览在「可安装」区完成
  sources: {
    add: "添加",
    title: "市场源",
    description: "管理第三方插件市场，添加后可在「可安装」中浏览插件",
    addedToast: "已添加插件市场「{{name}}」",
  },
  // Claude Code 插件管理（全部经 claude plugin 子命令，不读写 CLI 内部文件）
  claudePlugin: {
    installedHelp: "用户级开关由所有供应商共用；项目级和本地级开关仅影响对应项目，项目设置可覆盖用户级设置",
    availableHelp: "来自已添加的市场，默认安装到用户级；项目设置可单独控制是否启用",
    noMarkets: "还没有添加插件市场",
    showMore: "显示更多",
    scopeUser: "用户级",
    scopeProject: "项目级",
    scopeLocal: "本地",
    refreshMarketAria: "刷新插件市场 {{name}}",
    empty: "还没有安装 Claude Code 插件，可从「可安装」区选择安装；尚未安装 CLI 时，请先到「Agent 工具」安装",
    disabled: "已禁用",
    toggleLabel: "启用或禁用 {{name}}",
    action: {
      upgradeTitle: "升级到 v{{version}}（升级后重启 Claude Code 会话生效）",
      upgradeTo: "升级到 v{{version}}",
      uninstall: "卸载",
      uninstallAria: "卸载 {{name}}",
      removeMarket: "移除市场",
      removeMarketAria: "移除插件市场 {{name}}",
      refreshMarkets: "刷新市场目录",
    },
    confirm: {
      uninstallTitle: "卸载插件",
      uninstallBody: "确定卸载「{{name}}」（{{scope}} 范围）吗？将通过 Claude Code CLI 卸载，之后可重新安装",
      removeMarketTitle: "移除插件市场",
      removeMarketBody: "确定移除「{{name}}」吗？该市场已安装的插件会一并移除",
    },
    toast: {
      installed: "已安装插件「{{name}}」，重启 Claude Code 会话后生效",
      uninstalled: "已卸载插件「{{name}}」",
      updated: "「{{name}}」已从 {{old}} 升级到 {{new}}，重启 Claude Code 会话后生效",
      upToDate: "「{{name}}」已是最新版本（{{version}}）",
      marketAdded: "已添加插件市场「{{name}}」",
      marketRemoved: "已移除插件市场「{{name}}」",
      marketsRefreshed: "已刷新 {{count}} 个市场目录",
    },
    addMarket: {
      description: "支持 owner/repo、Git/HTTPS 地址或本地市场目录",
      placeholder: "owner/repo 或 https://...git",
      confirm: "添加",
    },
  },
} as const;
