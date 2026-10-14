import { invoke } from "@tauri-apps/api/core";
import type { Update } from "@tauri-apps/plugin-updater";
import type {
  AppState,
  ChatgptModel,
  CliStatus,
  CliUpdate,
  AuthStatus,
  BrowserLoginStart,
  ClaudeProfileDetail,
  ClaudeProfileSummary,
  CodexAppStatus,
  DatabaseBackupInfo,
  ManagedAccount,
  McpServerSpec,
  McpProbeResult,
  McpSyncPreview,
  McpDiffEntryAction,
  PluginMarketplace,
  MarketplacePlugin,
  PluginSkill,
  PluginPreview,
  PluginSummary,
  PluginUpdate,
  SkillSummary,
  SkillCandidate,
  SkillTool,
  ProfileBalance,
  ProfileBalanceInfo,
  CodexProfileDetail,
  CodexProfileConnectionResult,
  CodexProfileSummary,
  Settings,
  TomlDiagnostic,
} from "../types";
export const isTauri = typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
// macOS 使用原生交通灯（titleBarStyle: Overlay），并为交通灯预留窗口控件位
export const isMacWindow = isTauri && /Macintosh/.test(navigator.userAgent);

export type AppUpdateMetadata = ConstructorParameters<typeof Update>[0];

export type UpdateLogEvent =
  | "check_available"
  | "check_latest"
  | "check_failure"
  | "download_start"
  | "download_complete"
  | "download_failure"
  | "install_complete"
  | "install_failure";

// web-mock 动态加载：生产 Tauri 永远不会拉取这个 chunk，浏览器 dev 首次调用时才加载
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) return invoke<T>(command, args);
  const { webInvoke } = await import("./web-mock");
  return webInvoke<T>(command, args);
}

export const api = {
  // CLI 命令失败返回 CliFailure 对象，界面按阶段/类别翻译，不直接显示后端诊断文案。
  claudeGetCliStatus: () => call<CliStatus>("claude_get_cli_status"),
  claudeCheckCliUpdate: () => call<CliUpdate>("claude_check_cli_update"),
  claudeInstallCli: () => call<CliStatus>("claude_install_cli"),
  claudeUpdateCli: () => call<CliStatus>("claude_update_cli"),
  codexGetCliStatus: () => call<CliStatus>("codex_get_cli_status"),
  codexCheckCliUpdate: () => call<CliUpdate>("codex_check_cli_update"),
  codexInstallCli: () => call<CliStatus>("codex_install_cli"),
  codexUpdateCli: () => call<CliStatus>("codex_update_cli"),
  getState: () => call<AppState>("get_state"),
  // 启动里程碑：只写日志（Rust 侧折算到进程起点），无返回值语义
  reportStartupMark: (stage: string, frontendElapsedMs: number, detail?: string) =>
    call<void>("report_startup_mark", { stage, frontendElapsedMs, detail }),
  getCodexStatus: () => call<CodexAppStatus>("get_codex_status"),
  codexFetchProviderModels: (baseUrl: string, apiKey: string) =>
    call<string[]>("codex_fetch_provider_models", { baseUrl, apiKey }),
  codexFetchChatgptModels: (id: string | null, source: "desktop" | "oauth", accountId: string | null, refresh = true) =>
    call<ChatgptModel[]>("codex_fetch_chatgpt_models", { id, source, accountId, refresh }),
  codexCaptureProfile: (name: string) => call<CodexProfileSummary>("codex_capture_profile", { name }),
  codexAddBuiltinProfile: (
    kind: string,
    description: string,
    baseUrl?: string,
    apiKey?: string,
    adminUrl?: string,
    accountId?: string,
  ) => call<CodexProfileSummary>("codex_add_builtin_profile", { kind, description, baseUrl, apiKey, adminUrl, accountId }),
  codexAddCustomProfile: (
    name: string,
    description: string,
    configText: string,
    baseUrl?: string,
    apiKey?: string,
    adminUrl?: string,
    catalogText?: string | null,
    authText?: string | null,
  ) =>
    call<CodexProfileSummary>("codex_add_custom_profile", {
      name,
      description,
      configText,
      baseUrl,
      apiKey,
      adminUrl,
      catalogText,
      authText,
    }),
  codexGetBuiltinCatalog: (kind: string) => call<string | null>("codex_get_builtin_catalog", { kind }),
  codexGetBuiltinConfig: (kind: string) => call<string>("codex_get_builtin_config", { kind }),
  codexTestProfileConnection: (id: string, baseUrl?: string, apiKey?: string) =>
    call<CodexProfileConnectionResult>("codex_test_profile_connection", { id, baseUrl, apiKey }),
  // 创建态表单测试：供应商尚未保存，直接用表单里的地址/密钥
  codexTestProviderConnection: (baseUrl: string, apiKey: string) =>
    call<CodexProfileConnectionResult>("codex_test_provider_connection", { baseUrl, apiKey }),
  codexGetProfileBalance: (id: string) =>
    call<ProfileBalance>("codex_get_profile_balance", { id }),
  claudeGetProfileBalance: (id: string) =>
    call<ProfileBalance>("claude_get_profile_balance", { id }),
  exportDatabase: () => call<string>("export_database"),
  exportDatabaseTo: (directory: string) => call<string>("export_database_to", { directory }),
  importDatabase: (path: string) => call<void>("import_database", { path }),
  listDatabaseBackups: () => call<DatabaseBackupInfo[]>("list_database_backups"),
  restoreDatabase: (name: string) => call<void>("restore_database", { name }),
  deleteDatabaseBackup: (name: string) => call<void>("delete_database_backup", { name }),
  renameDatabaseBackup: (oldName: string, title: string) =>
    call<void>("rename_database_backup", { oldName, title }),
  renameProfile: (id: string, name: string, tool: SkillTool = "codex") => call<void>("rename_profile", { id, name, tool }),
  codexSetProfileIcon: (id: string, icon: string | null) => call<void>("codex_set_profile_icon", { id, icon }),
  codexSetProfileShowBalance: (id: string, enabled: boolean) =>
    call<void>("codex_set_profile_show_balance", { id, enabled }),
  codexSetProfileFetchedModels: (id: string, models: string[], efforts?: Record<string, string[]>, defaults?: Record<string, string>) =>
    call<void>("codex_set_profile_fetched_models", { id, models, ...(efforts ? { efforts } : {}), ...(defaults ? { defaults } : {}) }),
  setProfileBalance: (id: string, info: ProfileBalanceInfo) =>
    call<void>("set_profile_balance", { id, info }),
  codexSetProfileAccount: (id: string, accountId: string | null) =>
    call<void>("codex_set_profile_account", { id, accountId }),
  codexDuplicateProfile: (id: string) => call<CodexProfileSummary>("codex_duplicate_profile", { id }),
  codexGetProfile: (id: string) => call<CodexProfileDetail>("codex_get_profile", { id }),
  codexUpdateProfile: (id: string, name: string, description: string, baseUrl?: string, apiKey?: string, adminUrl?: string) =>
    call<CodexProfileSummary>("codex_update_profile", { id, name, description, baseUrl, apiKey, adminUrl }),
  codexUpdateProfileConfig: (
    id: string,
    configText: string,
    catalogText: string | null,
    authText: string | null,
  ) => call<CodexProfileDetail>("codex_update_profile_config", { id, configText, catalogText, authText }),
  codexSetProfileModel: (id: string, changes: { model?: string; effort?: string; fast?: boolean }) =>
    call<CodexProfileDetail>("codex_set_profile_model", { id, ...changes }),
  codexPatchChatgptContextConfig: (configText: string, enabled: boolean, compactTokenLimit: number) =>
    call<string>("codex_patch_chatgpt_context_config", { configText, enabled, compactTokenLimit }),
  codexPatchSystemProxyConfig: (configText: string, enabled: boolean) =>
    call<string>("codex_patch_system_proxy_config", { configText, enabled }),
  codexPatchContextManagementConfig: (configText: string, enabled: boolean) =>
    call<string>("codex_patch_context_management_config", { configText, enabled }),
  validateToml: (text: string) => call<TomlDiagnostic[]>("validate_toml", { text }),
  formatToml: (text: string) => call<string>("format_toml", { text }),
  listPlugins: () => call<PluginSummary[]>("list_plugins"),
  listSkills: () => call<SkillSummary[]>("list_skills"),
  claudeListProfiles: () => call<ClaudeProfileSummary[]>("claude_list_profiles"),
  claudeGetCommonSettings: () => call<string | null>("claude_get_common_settings"),
  claudeSaveCommonSettings: (text: string | null) => call<void>("claude_save_common_settings", { text }),
  claudeGetProfile: (id: string) => call<ClaudeProfileDetail>("claude_get_profile", { id }),
  claudeCaptureProfile: (name: string) => call<ClaudeProfileDetail>("claude_capture_profile", { name }),
  claudeSaveProfile: (input: { id?: string; name: string; baseUrl?: string | null; authToken?: string | null; model?: string | null; description?: string | null; fetchedModels?: string[] | null; kind?: string | null; adminUrl?: string | null; extraEnv?: string | null; rawSettings?: string | null; icon?: string | null; showBalance: boolean }) =>
    call<ClaudeProfileDetail>("claude_save_profile", input),
  claudeFetchModels: (baseUrl: string, authToken: string) => call<string[]>("claude_fetch_models", { baseUrl, authToken }),
  claudeTestConnection: (baseUrl: string, authToken: string) => call<CodexProfileConnectionResult>("claude_test_connection", { baseUrl, authToken }),
  claudeSetProfileIcon: (id: string, icon: string | null) => call<void>("claude_set_profile_icon", { id, icon }),
  claudeSetProfileShowBalance: (id: string, enabled: boolean) => call<void>("claude_set_profile_show_balance", { id, enabled }),
  claudeReorderProfiles: (ids: string[]) => call<void>("claude_reorder_profiles", { ids }),
  claudeDuplicateProfile: (id: string) => call<ClaudeProfileDetail>("claude_duplicate_profile", { id }),
  claudeTestProfile: (id: string) => call<CodexProfileConnectionResult>("claude_test_profile", { id }),
  claudeOpenTerminal: (id: string, cwd: string | null) => call<void>("claude_open_terminal", { id, cwd }),
  claudeDeleteProfile: (id: string) => call<void>("claude_delete_profile", { id }),
  claudeApplyProfile: (id: string) => call<void>("claude_apply_profile", { id }),
  getSkillContent: (name: string) => call<string>("get_skill_content", { name }),
  getImportSkillContent: (sourcePath: string) => call<string>("get_import_skill_content", { sourcePath }),
  scanUnmanagedSkills: () => call<SkillCandidate[]>("scan_unmanaged_skills"),
  importSkill: (sourcePath: string) => call<number>("import_skill", { sourcePath }),
  enableSkill: (name: string, tool: SkillTool) => call<void>("enable_skill", { name, tool }),
  disableSkill: (name: string, tool: SkillTool) => call<void>("disable_skill", { name, tool }),
  deleteSkill: (name: string) => call<void>("delete_skill", { name }),
  listPluginSkills: (name: string, storePath?: string) => call<PluginSkill[]>("list_plugin_skills", { name, storePath }),
  listPluginMarketplaces: () => call<PluginMarketplace[]>("list_plugin_marketplaces"),
  listMarketplacePlugins: (marketplace: string, root?: string) => call<MarketplacePlugin[]>("list_marketplace_plugins", { marketplace, root }),
  addPluginMarketplace: (url: string) => call<PluginMarketplace>("add_plugin_marketplace", { url }),
  removePluginMarketplace: (name: string) => call<void>("remove_plugin_marketplace", { name }),
  installMarketplacePlugin: (marketplace: string, name: string) =>
    call<PluginSummary>("install_marketplace_plugin", { marketplace, name }),
  checkPluginUpdates: () => call<PluginUpdate[]>("check_plugin_updates"),
  upgradeMarketplacePlugin: (marketplace: string, name: string) =>
    call<void>("upgrade_marketplace_plugin", { marketplace, name }),
  previewPlugin: (url: string) => call<PluginPreview>("preview_plugin", { url }),
  installPlugin: (url: string, subPath: string | null) =>
    call<PluginSummary>("install_plugin", { url, subPath }),
  uninstallPlugin: (name: string) => call<void>("uninstall_plugin", { name }),
  codexDeleteProfile: (id: string) => call<void>("codex_delete_profile", { id }),
  codexReorderProfiles: (ids: string[]) => call<void>("codex_reorder_profiles", { ids }),
  codexApplyProfile: (id: string) => call<void>("codex_apply_profile", { id }),
  codexListMcpServers: () => call<McpServerSpec[]>("codex_list_mcp_servers"),
  claudeListMcpServers: () => call<McpServerSpec[]>("claude_list_mcp_servers"),
  claudeGetMcpServerJson: (name: string) => call<string | null>("claude_get_mcp_server_json", { name }),
  claudeSaveMcpServer: (originalName: string | null, name: string, json: string) =>
    call<void>("claude_save_mcp_server", { originalName, name, json }),
  claudeDeleteMcpServer: (name: string) => call<void>("claude_delete_mcp_server", { name }),
  // manual 用于后端日志分级：手动测试记 Info，进页静默探测只记 Debug；tool 决定按哪侧引擎的名单与开关判定
  probeMcpServer: (name: string, includeTools = false, manual = true, tool: "codex" | "claude" = "codex") =>
    call<McpProbeResult>("probe_mcp_server", { name, includeTools, manual, tool }),
  // 创建表单预填用：优先数据库 MCP 镜像，首次无镜像时回退 live
  codexGetMcpSectionToml: () => call<string>("codex_get_mcp_section_toml"),
  // 显式恢复：数据库镜像写回 live config.toml，返回恢复数量
  restoreMcpFromDatabase: () => call<number>("restore_mcp_from_database"),
  // 同步预览：对比 live 与数据库镜像的 MCP 差异（只读），供同步前人工裁决
  codexMcpSyncPreview: () => call<McpSyncPreview>("codex_mcp_sync_preview"),
  claudeMcpSyncPreview: () => call<McpSyncPreview>("claude_mcp_sync_preview"),
  claudeResolveMcpEntries: (actions: McpDiffEntryAction[], adopt: boolean) => call<number>("claude_resolve_mcp_entries", { actions, adopt }),
  codexSaveMcpServer: (originalName: string | null, spec: McpServerSpec, fragment?: string) =>
    call<void>("codex_save_mcp_server", { originalName, spec, fragment }),
  // MCP 编辑页：读取 live 原始片段（含未建模键与注释），初始化编辑器用
  codexGetMcpServerToml: (name: string) => call<string | null>("codex_get_mcp_server_toml", { name }),
  // MCP 编辑页实时同步：表单建模字段写进片段（表单 → 编辑器）
  patchMcpFragment: (toml: string, spec: McpServerSpec) =>
    call<string>("patch_mcp_fragment", { toml, spec }),
  // MCP 编辑页实时同步：片段解析回建模字段（编辑器 → 表单）
  parseMcpFragment: (toml: string) => call<McpServerSpec>("parse_mcp_fragment", { toml }),
  codexDeleteMcpServer: (name: string) => call<void>("codex_delete_mcp_server", { name }),
  setMcpServerEnabled: (name: string, tool: "codex" | "claude", enabled: boolean) => call<void>("set_mcp_server_enabled", { name, tool, enabled }),
  setMcpMirror: (name: string, fragment: string | null) => call<void>("set_mcp_mirror", { name, fragment }),
  revertMcpLive: (name: string, fragment: string | null) => call<void>("revert_mcp_live", { name, fragment }),
  // 批量差异处理：整批一次写入（只备份/写盘一次），返回实际处理的条目数
  setMcpMirrorEntries: (actions: McpDiffEntryAction[]) => call<number>("set_mcp_mirror_entries", { actions }),
  revertMcpLiveEntries: (actions: McpDiffEntryAction[]) => call<number>("revert_mcp_live_entries", { actions }),
  restartCodex: () => call<void>("restart_codex"),
  setWindowTheme: (dark: boolean) => call<void>("set_window_theme", { dark }),
  setTrayMenu: (language: string, profiles: Pick<CodexProfileSummary, "id" | "name">[], activeProfileId: string | null) =>
    call<void>("set_tray_menu", { language, profiles, activeProfileId }),
  authStartBrowserLogin: () => call<BrowserLoginStart>("auth_start_browser_login"),
  authPollBrowserLogin: () => call<ManagedAccount | null>("auth_poll_browser_login"),
  authCancelBrowserLogin: () => call<void>("auth_cancel_browser_login"),
  authGetStatus: () => call<AuthStatus>("auth_get_status"),
  authGetQuota: (source: "desktop" | "oauth", accountId?: string) =>
    call<ProfileBalance>("auth_get_quota", { source, accountId }),
  authWarmup: (source: "desktop" | "oauth", accountId: string) =>
    call<void>("auth_warmup", { source, accountId }),
  authPreview: (accountId: string) => call<string | null>("auth_preview", { accountId }),
  authRemoveAccount: (accountId: string) =>
    call<void>("auth_remove_account", { accountId }),
  openUrl: (url: string) => call<void>("open_url", { url }),
  getSettings: () => call<Settings>("get_settings"),
  saveSettings: (settings: Settings) => call<Settings>("save_settings", { settings }),
  getProxyStatus: () => call<string | null>("get_proxy_status"),
  logUpdateEvent: (event: UpdateLogEvent, version?: string) =>
    call<void>("log_update_event", { event, version }),
  // CLI 更新计时器的跳过决策留痕（真实检查的成败由后端自行落日志）
  reportCliUpdateTick: (client: "codex" | "claude", decision: "cooldown_skip" | "not_native" | "detect_failed") =>
    call<void>("report_cli_update_tick", { client, decision }),
  checkAppUpdate: () => call<AppUpdateMetadata | null>("check_app_update"),
  setUpdateMarker: (version: string) => call<void>("set_update_marker", { version }),
  takeUpdateMarker: (rollback = false) =>
    call<string | null>("take_update_marker", { rollback }),
  openPath: (path: string) => call<void>("open_path", { path }),
};
