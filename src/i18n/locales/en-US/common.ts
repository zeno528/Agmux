/** Shared UI strings: navigation, window controls. */
export default {
  writeback: {
    overwriteTitle: "Overwrite damaged configuration and switch?",
    overwriteDescription: "The saved configuration for {{name}} will overwrite the damaged {{file}}. The current provider's database configuration will stay unchanged. Unsaved changes in the damaged file will be discarded.",
    overwriteConfirm: "Overwrite and switch",
    failed: "Configuration sync failed",
    paused: "Configuration sync paused",
    position: " (line {{line}}, column {{column}})",
    protected: "The database configuration has been kept.",
    parseAction: "The database configuration has been kept. Repair the file or switch providers.",
    reason: {
      live_parse_error: "{{file}}: invalid {{format}}{{position}}. Settings from this file are not in effect.",
      live_unreadable: "Cannot read {{file}}. Check file permissions, whether it is in use, and that the path points to a valid file.",
      write_failed: "Cannot save the live configuration to the database. Check disk space, database permissions, and diagnostic logs.",
      internal: "An error occurred while reading or saving the database configuration. Check diagnostic logs for the cause.",
    },
  },
  nav: {
    providers: "Providers",
    // 侧边栏给标签只留约 72px；"MCP Servers" 会溢出，导航用短名，完整名见页面标题 mcp:list.title
    mcp: "MCP",
    plugins: "Plugins",
    skills: "Skill",
    accounts: "Accounts",
    agentTools: "Agent tools",
    settings: "Settings",
    groupCodex: "Codex",
    groupClaude: "Claude Code",
  },
  sidebar: {
    expand: "Expand sidebar",
    collapse: "Collapse sidebar",
  },
  window: {
    minimize: "Minimize",
    maximize: "Maximize",
    close: "Close",
  },
  search: {
    clear: "Clear search",
    clearText: "Clear",
  },
  select: {
    placeholder: "Select...",
    optionsLabel: "Options",
    search: "Search...",
    clearSearch: "Clear search",
    noResults: "No matches",
    availableCount: "{{count}} available",
  },
  loading: "Loading...",
  feedback: {
    applySuccess: "Switched successfully",
    cancel: "Cancel",
    confirm: "Confirm",
    dismissToast: "Dismiss notification",
    genericError: "Operation failed",
  },
  error: {
    title: "Failed to load the interface",
    description: "This page ran into an error. Reload to continue using the app",
    reload: "Reload",
    details: "View error details",
  },
  editor: {
    placeholder: "Edit the configuration here...",
    horizontalScrollbar: "Editor horizontal scroll",
    jsonSyntaxError: "JSON syntax error. Check the comma, brackets, or value here",
  },
  preset: {
    custom: "Custom",
    // 中文显示「智谱 GLM」；英文刻意保留国际品牌 Z.ai，不随中文改名。
    zhipu: "Z.ai",
    qwen: "Qwen",
    hunyuan: "Tencent Hunyuan",
    doubao: "Volcengine Doubao",
    mimo: "Xiaomi MiMo",
    qianfan: "Baidu Qianfan",
    xai: "xAI (Grok)",
    kimiCode: "Kimi Code",
    qwenCodingPlan: "Alibaba Cloud Bailian Coding Plan",
    tencentTokenPlan: "Tencent Cloud General Token Plan",
    tencentTokenPlanEnterprise: "Tencent Cloud Token Plan Enterprise",
    qianfanCodingPlan: "Baidu Qianfan Coding Plan",
    siliconflow: "SiliconFlow",
    longcat: "LongCat",
  },
  providerName: {
    hunyuan: "Tencent Hunyuan",
    volcengine: "Volcengine Ark",
  },
  cliUpdate: {
    // 与中文「更新至」同一语义：英文用 Update to X。
    upgrade: "Update to {{version}}",
    upgradeTitle: "{{client}} CLI {{version}} available ({{channel}} channel). Click to update",
  },
} as const;
