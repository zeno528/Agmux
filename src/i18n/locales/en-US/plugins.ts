/** Plugins page strings. */
export default {
  title: "Plugins",
  inventory: {
    label: "Codex content type",
    apps: "Apps",
    loadingApps: "Loading installed Codex apps…",
    noApps: "No matching installed apps",
  },
  marketplace: "Marketplace",
  nav: {
    backToPlugins: "Back to plugins",
  },
  // Data values for plugin origin / marketplace kind → display labels
  origin: {
    official: "Official marketplace",
    thirdParty: "Third-party marketplace",
  },
  source: {
    openGithub: "Open on GitHub",
    openSource: "Open link",
    openTitle: "Open link: {{url}}",
  },
  action: {
    install: "Install",
    toggleLabel: "Enable or disable {{name}}",
    managed: "Managed by Codex",
    managedHelp: "Enable or disable this plugin in the Codex desktop app",
    uninstall: "Uninstall",
    checkUpdates: "Check for updates",
    upgradeAll: "Update all",
    removeMarket: "Remove marketplace",
  },
  status: {
    on: "Enabled",
    off: "Disabled",
  },
  detail: {
    marketplace: "Marketplace",
    installPath: "Install location",
    skillCount: "Skills: {{count}}",
    loadingSkills: "Loading Skill details...",
    skillsEmpty: "This plugin has no readable Skill details",
  },
  marketDetail: {
    thirdPartyNotice: "You are using a third-party model: plugin packages are kept, but plugins with App or MCP connectors may fail to load in Codex. To authorize or use connectors, switch to the official ChatGPT subscription provider",
  },
  market: {
    removeAria: "Remove plugin marketplace {{name}}",
    foundUpdates: "{{client}}: {{count}} plugin updates available",
    upToDateToast: "{{client}}: Plugins are up to date",
    upToDate: "All plugins are up to date",
    allUpgraded: "All third-party plugins updated",
    removedToast: "Removed the plugin marketplace \"{{name}}\"",
    removeConfirmTitle: "Remove third-party marketplace",
    removeConfirm: "Remove \"{{name}}\"? This only removes the marketplace source; plugins already installed from it are not uninstalled",
  },
  recommended: {
    openaiCurated: "OpenAI's official Codex plugin marketplace, offering installable app integrations and development workflows",
    ponytail: "A development workflow biased toward minimal implementations, emphasizing YAGNI, the standard library, and native capabilities",
  },
  add: {
    marketSourceTitle: "Add marketplace source",
    marketPlaceholder: "owner/repo, https://...git, or a local marketplace folder",
  },
  list: {
    empty: "No plugins installed yet. Click \"Marketplaces\" in the top right to add a third-party marketplace",
    searchPlaceholder: "Search plugins...",
    noMatch: "No matching plugins",
    uninstallAria: "Uninstall {{name}}",
    sourceKept: "the marketplace source is kept",
    sourceKeptMarket: "the marketplace source {{marketplace}} is kept",
    removeConfirm: "Uninstall \"{{name}}\"? This removes the plugin via the codex CLI ({{source}}); you can reinstall it from Codex or from here later",
  },
  confirm: {
    uninstallPluginTitle: "Uninstall plugin",
  },
  toast: {
    installed: "Installed \"{{name}}\". Restart Codex to take effect",
    installedNeedsAuth: "\"{{name}}\" installed. Please complete authorization in the Codex desktop app",
    uninstalled: "Uninstalled \"{{name}}\"",
  },
  // Dual-engine switch (mirrors the MCP page): Codex and Claude Code plugin systems are independent
  target: {
    label: "Choose plugin engine",
    codex: "Codex",
    claude: "Claude",
  },
  // Board hierarchy: engine dimension → full-width search → filter chips → section grids (shared by Codex / Claude)
  board: {
    codexInstalledHelp: "Toggles update user settings; project or managed settings may override them. Existing sessions may need a restart. Toggle plugins labeled “Managed by Codex” in the Codex desktop app",
    codexAvailableHelp: "From your current Codex account and added marketplaces. Complete authorization in the Codex desktop app after installation when required",
    filterAll: "All",
    filterInstalled: "Installed",
    filterAvailable: "Available",
    filterUpdates: "Updates",
    sectionInstalled: "Installed",
    sectionAvailable: "Available",
    sectionUpdates: "Updates available",
    upgradeTo: "Update to v{{version}}",
  },
  // Marketplace sources retreat to a dialog; browsing happens in the Available section
  sources: {
    add: "Add",
    title: "Marketplaces",
    description: "Manage third-party marketplaces, then browse their plugins under Available",
    addedToast: "Added plugin marketplace \"{{name}}\"",
  },
  // Claude Code plugin management (all through claude plugin subcommands, never touching CLI-internal files)
  claudePlugin: {
    installedHelp: "User-level toggles are shared across provider profiles. Project and local toggles affect only their project; project settings can override user settings",
    availableHelp: "From added marketplaces, installed to the user scope by default. Project settings can control whether they are enabled",
    noMarkets: "No plugin marketplaces added yet",
    showMore: "Show more",
    scopeUser: "User",
    scopeProject: "Project",
    scopeLocal: "Local",
    refreshMarketAria: "Refresh plugin marketplace {{name}}",
    empty: "No Claude Code plugins installed yet. Pick one in the Available section, or install the CLI in Agent Tools first",
    toggleLabel: "Enable or disable {{name}}",
    action: {
      upgradeTitle: "Upgrade to v{{version}} (restart the Claude Code session to apply)",
      upgradeTo: "Upgrade to v{{version}}",
      uninstall: "Uninstall",
      uninstallAria: "Uninstall {{name}}",
      removeMarket: "Remove Marketplace",
      removeMarketAria: "Remove plugin marketplace {{name}}",
      refreshMarkets: "Refresh marketplace catalogs",
    },
    confirm: {
      uninstallTitle: "Uninstall plugin",
      uninstallBody: "Uninstall \"{{name}}\" ({{scope}} scope)? This runs the Claude Code CLI; you can reinstall it later",
      removeMarketTitle: "Remove plugin marketplace",
      removeMarketBody: "Remove \"{{name}}\"? Plugins installed from this marketplace are removed as well",
    },
    toast: {
      installed: "Installed \"{{name}}\". Restart the Claude Code session to take effect",
      uninstalled: "Uninstalled \"{{name}}\"",
      updated: "\"{{name}}\" upgraded from {{old}} to {{new}}. Restart the Claude Code session to take effect",
      upToDate: "\"{{name}}\" is already up to date ({{version}})",
      marketRemoved: "Removed plugin marketplace \"{{name}}\"",
      marketsRefreshed: "Refreshed {{count}} marketplace catalogs",
    },
    addMarket: {
      description: "Supports owner/repo, Git/HTTPS URLs, or local marketplace directories",
    },
  },
} as const;
