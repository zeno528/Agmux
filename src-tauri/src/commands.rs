use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

use crate::auth::codex_oauth::{
    parse_external_auth_json, AuthStatus, BrowserLoginStart, CodexOAuthManager, CodexOAuthState,
    ManagedAccount,
};
use crate::codex::config as codex_config;
use crate::codex_builtin;
use crate::error::{app_err, AppResult};
use crate::models::{
    AppState, AuthSource, ClaudeProfileDetail, ClaudeProfileInput, ClaudeProfileSummary,
    CodexAppStatus, CodexProfileDetail, CodexProfileSummary, McpDiffEntryAction, McpServerSpec,
    McpSyncPreview, ProfileBalanceInfo, Settings, TrayClickAction,
};
use crate::services::{
    AppContext, CliFailure, CliStatus, CliUpdate, CodexProfileConnectionResult, DatabaseBackupInfo,
    MarketplacePlugin, PluginMarketplace, PluginPreview, PluginSkill, PluginSummary, PluginUpdate,
    ProfileBalance, SkillSummary, SkillTool,
};

/// CLI 状态检测走阻塞线程池；两端命令共用壳，兜底文案只此一份。
async fn cli_status_blocking(
    state: State<'_, AppContext>,
    detect: fn(&AppContext) -> Result<CliStatus, CliFailure>,
) -> Result<CliStatus, CliFailure> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || detect(&state))
        .await
        .map_err(|_| CliFailure {
            stage: "detect",
            kind: "internal",
            message: "CLI 检测任务失败".into(),
        })?
}

#[tauri::command]
pub async fn claude_get_cli_status(state: State<'_, AppContext>) -> Result<CliStatus, CliFailure> {
    cli_status_blocking(state, AppContext::claude_get_cli_status).await
}

#[tauri::command]
pub async fn claude_check_cli_update(
    state: State<'_, AppContext>,
) -> Result<CliUpdate, CliFailure> {
    state.claude_check_cli_update().await
}

#[tauri::command]
pub async fn claude_install_cli(state: State<'_, AppContext>) -> Result<CliStatus, CliFailure> {
    state.claude_run_cli(true).await
}

#[tauri::command]
pub async fn claude_update_cli(state: State<'_, AppContext>) -> Result<CliStatus, CliFailure> {
    state.claude_run_cli(false).await
}

#[tauri::command]
pub async fn codex_get_cli_status(state: State<'_, AppContext>) -> Result<CliStatus, CliFailure> {
    cli_status_blocking(state, AppContext::codex_get_cli_status).await
}

#[tauri::command]
pub async fn codex_check_cli_update(state: State<'_, AppContext>) -> Result<CliUpdate, CliFailure> {
    state.codex_check_cli_update().await
}

#[tauri::command]
pub async fn codex_install_cli(state: State<'_, AppContext>) -> Result<CliStatus, CliFailure> {
    state.codex_run_cli(true).await
}

#[tauri::command]
pub async fn codex_update_cli(state: State<'_, AppContext>) -> Result<CliStatus, CliFailure> {
    state.codex_run_cli(false).await
}

fn should_try_next_account_credential(result: &CodexProfileConnectionResult) -> bool {
    matches!(result.status, Some(401 | 403))
        && !result
            .error
            .as_deref()
            .is_some_and(|error| error.contains("地区限制"))
}

async fn test_account_connection(
    state: &AppContext,
    manager: &CodexOAuthManager,
    account_id: &str,
) -> AppResult<CodexProfileConnectionResult> {
    // live 凭证只作为同步输入；验证统一使用托管账号记录中的有效 access_token。
    state.sync_live_oauth_auth(manager).await?;
    let workspace = manager.workspace_of(account_id).await;
    let log_context = format!(
        "{} source=oauth",
        manager.account_subject_for(account_id).await
    );
    let token = manager
        .get_valid_token_for_account(account_id)
        .await
        .map_err(|error| app_err!("{error}"))?;
    let result = state
        .test_subscription_connection(&token, &log_context)
        .await?;
    if !should_try_next_account_credential(&result) {
        return Ok(result);
    }

    let auth_json = manager
        .refresh_codex_auth_json(account_id)
        .await
        .map_err(|error| app_err!("{error}"))?;
    let auth = parse_external_auth_json(&auth_json)
        .filter(|auth| auth.account_id == workspace)
        .ok_or_else(|| app_err!("刷新后账号标识不匹配"))?;
    state
        .test_subscription_connection(&auth.access_token, &log_context)
        .await
}

#[tauri::command]
pub fn get_state(state: State<'_, AppContext>) -> AppResult<AppState> {
    state.get_state()
}

/// 前端启动里程碑上报（只写日志不落库）：与 native 埋点共用同一把进程起点尺子。
/// stage 由调用方给定（state_ready / window_shown），detail 是补充定位（如 raf 路径）。
#[tauri::command]
pub fn report_startup_mark(
    state: State<'_, crate::StartupClock>,
    stage: String,
    frontend_elapsed_ms: u64,
    detail: Option<String>,
) {
    tauri_plugin_log::log::info!(
        "[app.startup] stage={stage} frontend_elapsed_ms={frontend_elapsed_ms} rust_elapsed_ms={} detail={:?} msg=\"前端启动里程碑\"",
        state.0.elapsed().as_millis(),
        detail.as_deref().unwrap_or("-"),
    );
}

#[tauri::command]
pub fn get_codex_status(state: State<'_, AppContext>) -> AppResult<CodexAppStatus> {
    state.codex_status()
}

#[tauri::command]
pub async fn list_plugins(state: State<'_, AppContext>) -> AppResult<Vec<PluginSummary>> {
    state.list_plugins().await
}

#[tauri::command]
pub async fn list_skills(state: State<'_, AppContext>) -> AppResult<Vec<SkillSummary>> {
    state.list_skills().await
}

#[tauri::command]
pub async fn get_skill_content(name: String, state: State<'_, AppContext>) -> AppResult<String> {
    state.get_skill_content(&name).await
}

#[tauri::command]
pub async fn get_import_skill_content(
    source_path: String,
    state: State<'_, AppContext>,
) -> AppResult<String> {
    state.get_import_skill_content(&source_path).await
}

#[tauri::command]
pub async fn scan_unmanaged_skills(
    state: State<'_, AppContext>,
) -> AppResult<Vec<super::services::SkillCandidate>> {
    state.scan_unmanaged_skills().await
}

#[tauri::command]
pub async fn import_skill(source_path: String, state: State<'_, AppContext>) -> AppResult<usize> {
    state.import_skill(&source_path).await
}

#[tauri::command]
pub async fn enable_skill(
    name: String,
    tool: SkillTool,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.enable_skill(&name, tool).await
}

#[tauri::command]
pub async fn disable_skill(
    name: String,
    tool: SkillTool,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.disable_skill(&name, tool).await
}

#[tauri::command]
pub async fn delete_skill(name: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.delete_skill(&name).await
}

#[tauri::command]
pub fn claude_list_profiles(state: State<'_, AppContext>) -> AppResult<Vec<ClaudeProfileSummary>> {
    state.claude_list()
}

#[tauri::command]
pub fn claude_get_profile(
    id: String,
    state: State<'_, AppContext>,
) -> AppResult<ClaudeProfileDetail> {
    state.claude_get(&id)
}

#[tauri::command]
pub fn claude_capture_profile(
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<ClaudeProfileDetail> {
    state.claude_capture(&name)
}

#[tauri::command]
pub fn claude_get_common_settings(state: State<'_, AppContext>) -> AppResult<Option<String>> {
    state.claude_common_settings()
}

/// 在新终端里启动一个绑定该供应商的 Claude Code 会话（`claude --settings` 覆盖）。
/// 只读旁路：不切激活、不改 live、不触发回写。
/// async 是给 macOS 的：osascript 要等 Terminal 自动化授权弹窗，同步 command 跑在
/// 主线程会把整个 UI 冻到用户处理完弹窗（Tauri 官方文档：无 async 的 command 在主线程执行）。
#[tauri::command]
pub async fn claude_open_terminal(
    id: String,
    cwd: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.claude_open_terminal(&id, cwd.as_deref())
}

#[tauri::command]
pub fn claude_save_common_settings(
    text: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.claude_save_common_settings(text.as_deref())
}

// ponytail: 参数即表单字段一一对应，IPC 边界保持散参（camelCase 自动映射），service 层才收拢成 Input
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn claude_save_profile(
    id: Option<String>,
    name: String,
    base_url: Option<String>,
    auth_token: Option<String>,
    model: Option<String>,
    description: Option<String>,
    fetched_models: Option<Vec<String>>,
    kind: Option<String>,
    admin_url: Option<String>,
    extra_env: Option<String>,
    raw_settings: Option<String>,
    icon: Option<String>,
    show_balance: bool,
    state: State<'_, AppContext>,
) -> AppResult<ClaudeProfileDetail> {
    state.claude_save(
        id.as_deref(),
        ClaudeProfileInput {
            name,
            base_url,
            auth_token,
            model,
            description,
            fetched_models,
            kind,
            admin_url,
            extra_env,
            raw_settings,
            icon,
            show_balance,
        },
    )
}

#[tauri::command]
pub fn claude_set_profile_icon(
    id: String,
    icon: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.claude_set_icon(&id, icon)
}

#[tauri::command]
pub fn claude_set_profile_show_balance(
    id: String,
    enabled: bool,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.claude_set_show_balance(&id, enabled)
}

#[tauri::command]
pub async fn claude_fetch_models(base_url: String, auth_token: String) -> AppResult<Vec<String>> {
    crate::services::fetch_claude_models(&base_url, &auth_token).await
}

/// 创建态表单的测试连通：空请求判活，返回与 Codex 相同的状态和诊断结果。
#[tauri::command]
pub async fn claude_test_connection(
    base_url: String,
    auth_token: String,
) -> AppResult<CodexProfileConnectionResult> {
    crate::services::test_claude_connection(&base_url, &auth_token).await
}

#[tauri::command]
pub fn claude_delete_profile(id: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.claude_delete(&id)
}

#[tauri::command]
pub fn claude_apply_profile(id: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.claude_apply(&id)
}

#[tauri::command]
pub fn claude_reorder_profiles(ids: Vec<String>, state: State<'_, AppContext>) -> AppResult<()> {
    state.claude_reorder(&ids)
}

#[tauri::command]
pub fn claude_duplicate_profile(
    id: String,
    state: State<'_, AppContext>,
) -> AppResult<ClaudeProfileDetail> {
    state.claude_duplicate(&id)
}

#[tauri::command]
pub async fn claude_test_profile(
    id: String,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileConnectionResult> {
    state.claude_test_profile(&id).await
}

#[tauri::command]
pub async fn list_plugin_skills(
    name: String,
    store_path: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<Vec<PluginSkill>> {
    state.list_plugin_skills(&name, store_path.as_deref()).await
}

#[tauri::command]
pub async fn list_plugin_marketplaces(
    state: State<'_, AppContext>,
) -> AppResult<Vec<PluginMarketplace>> {
    state.list_plugin_marketplaces().await
}

#[tauri::command]
pub async fn list_marketplace_plugins(
    marketplace: String,
    root: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<Vec<MarketplacePlugin>> {
    state
        .list_marketplace_plugins(&marketplace, root.as_deref())
        .await
}

#[tauri::command]
pub async fn add_plugin_marketplace(
    url: String,
    state: State<'_, AppContext>,
) -> AppResult<PluginMarketplace> {
    state.add_plugin_marketplace(&url).await
}

#[tauri::command]
pub async fn remove_plugin_marketplace(
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.remove_plugin_marketplace(&name).await
}

#[tauri::command]
pub async fn install_marketplace_plugin(
    marketplace: String,
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<PluginSummary> {
    state.install_marketplace_plugin(&marketplace, &name).await
}

#[tauri::command]
pub async fn check_plugin_updates(state: State<'_, AppContext>) -> AppResult<Vec<PluginUpdate>> {
    state.check_plugin_updates().await
}

#[tauri::command]
pub async fn upgrade_marketplace_plugin(
    marketplace: String,
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.upgrade_marketplace_plugin(&marketplace, &name).await
}

#[tauri::command]
pub async fn preview_plugin(url: String, state: State<'_, AppContext>) -> AppResult<PluginPreview> {
    state.preview_plugin(&url).await
}

#[tauri::command]
pub async fn install_plugin(
    url: String,
    sub_path: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<PluginSummary> {
    state.install_plugin(&url, sub_path.as_deref()).await
}

#[tauri::command]
pub async fn uninstall_plugin(name: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.uninstall_plugin(&name).await
}

#[tauri::command]
pub fn codex_capture_profile(
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileSummary> {
    state.codex_capture_profile(&name)
}

#[tauri::command]
pub fn codex_add_builtin_profile(
    kind: String,
    description: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    admin_url: Option<String>,
    account_id: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileSummary> {
    state.codex_add_builtin_profile(
        &kind,
        description.as_deref(),
        base_url.as_deref(),
        api_key.as_deref(),
        admin_url.as_deref(),
        account_id.as_deref(),
    )
}

#[tauri::command]
pub fn codex_get_builtin_catalog(
    kind: String,
    state: State<'_, AppContext>,
) -> AppResult<Option<String>> {
    state.codex_get_builtin_catalog(&kind)
}

// 内置供应商的 config.toml 渲染结果（与创建/应用一致）：前端创建页预览的唯一来源。
// minimax 的 model_catalog_json 行由 render_config 插入，返回原文会让预览缺该行、模型目录 tab 不显示。
#[tauri::command]
pub fn codex_get_builtin_config(kind: String) -> AppResult<String> {
    let template = codex_builtin::template(&kind)?;
    Ok(String::from_utf8_lossy(&template.render_config(None)?).into_owned())
}

#[tauri::command]
// 参数个数受前端 IPC 调用约束（一次性提交 config/catalog/auth 三件套），不宜拆结构体
#[allow(clippy::too_many_arguments)]
pub fn codex_add_custom_profile(
    name: String,
    description: Option<String>,
    config_text: String,
    base_url: Option<String>,
    api_key: Option<String>,
    admin_url: Option<String>,
    catalog_text: Option<String>,
    auth_text: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileSummary> {
    state.codex_add_custom_profile(
        &name,
        description.as_deref(),
        &config_text,
        base_url.as_deref(),
        api_key.as_deref(),
        admin_url.as_deref(),
        catalog_text.as_deref(),
        auth_text.as_deref(),
    )
}

#[tauri::command]
pub async fn codex_test_profile_connection(
    id: String,
    base_url: Option<String>,
    api_key: Option<String>,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> AppResult<CodexProfileConnectionResult> {
    // 官方订阅：测认证连通性（token 有效 + 网络可达），走 Codex 官方后端端点
    if state.is_subscription_profile(&id)? {
        let source = state.profile_auth_source(&id)?;
        let bound = state.bound_account_id(&id)?;
        return match source {
            Some(AuthSource::Oauth) => {
                let account_id = bound.ok_or_else(|| app_err!("OAuth 配置未绑定订阅账号"))?;
                test_account_connection(&state, &oauth.0, &account_id).await
            }
            Some(AuthSource::Desktop) => {
                // 用配置自身数据库快照的 token：live auth.json 切换后是别账号的认证
                let token = state
                    .desktop_profile_access_token(&id)?
                    .ok_or_else(|| app_err!("该 Codex 配置尚未保存有效登录"))?;
                // 日志主体带配置名：id 无法对人区分配置
                let name = state.codex_get_profile(&id)?.name;
                let log_context = format!(
                    "profile_id={id} profile_name=\"{}\" source=desktop",
                    name.replace('"', "'")
                );
                state
                    .test_subscription_connection(&token, &log_context)
                    .await
            }
            None => Err(app_err!("官方配置缺少认证来源")),
        };
    }
    state
        .codex_test_profile_connection(&id, base_url.as_deref(), api_key.as_deref())
        .await
}

// 创建态表单测试连通：供应商尚未保存，没有 profile id，地址/密钥实时传入
#[tauri::command]
pub async fn codex_test_provider_connection(
    base_url: String,
    api_key: String,
) -> AppResult<CodexProfileConnectionResult> {
    crate::services::test_provider_connection(&base_url, &api_key).await
}

// 获取 ChatGPT 订阅账号的 Codex 模型目录。
#[tauri::command]
pub async fn codex_fetch_chatgpt_models(
    id: Option<String>,
    source: AuthSource,
    account_id: Option<String>,
    refresh: bool,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> AppResult<Vec<crate::services::ChatgptModel>> {
    state
        .codex_fetch_chatgpt_models(
            id.as_deref(),
            source,
            account_id.as_deref(),
            &oauth.0,
            refresh,
        )
        .await
}

// 获取供应商可用模型 ID 列表（OpenAI 兼容 GET /models）。
#[tauri::command]
pub async fn codex_fetch_provider_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    crate::services::fetch_models(&base_url, &api_key).await
}

#[tauri::command]
pub async fn codex_get_profile_balance(
    id: String,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> AppResult<ProfileBalance> {
    state.codex_get_profile_balance(&id, &oauth.0).await
}

#[tauri::command]
pub async fn claude_get_profile_balance(
    id: String,
    state: State<'_, AppContext>,
) -> AppResult<ProfileBalance> {
    state.claude_get_profile_balance(&id).await
}

#[tauri::command]
pub fn export_database(state: State<'_, AppContext>) -> AppResult<String> {
    Ok(state.export_database()?.display().to_string())
}

#[tauri::command]
pub fn export_database_to(directory: String, state: State<'_, AppContext>) -> AppResult<String> {
    Ok(state.export_database_to(&directory)?.display().to_string())
}

#[tauri::command]
pub async fn import_database(
    path: String,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> AppResult<()> {
    state.import_database(&path)?;
    // 数据库整体替换后，内存中的订阅账号同步重载
    oauth.0.reload_from_database()?;
    Ok(())
}

#[tauri::command]
pub fn list_database_backups(state: State<'_, AppContext>) -> AppResult<Vec<DatabaseBackupInfo>> {
    state.list_database_backups()
}

#[tauri::command]
pub async fn restore_database(
    name: String,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> AppResult<()> {
    state.restore_database(&name)?;
    oauth.0.reload_from_database()?;
    Ok(())
}

#[tauri::command]
pub fn delete_database_backup(name: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.delete_database_backup(&name)
}

#[tauri::command]
pub fn rename_database_backup(
    old_name: String,
    title: String,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.rename_database_backup(&old_name, &title)
}

#[tauri::command]
pub fn rename_profile(
    id: String,
    name: String,
    tool: Option<SkillTool>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.rename_profile(&id, &name, tool.unwrap_or(SkillTool::Codex))
}

#[tauri::command]
pub fn codex_set_profile_icon(
    id: String,
    icon: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.codex_set_profile_icon(&id, icon.as_deref())
}

#[tauri::command]
pub fn codex_set_profile_show_balance(
    id: String,
    enabled: bool,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.codex_set_profile_show_balance(&id, enabled)
}

#[tauri::command]
pub fn codex_set_profile_fetched_models(
    id: String,
    models: Vec<String>,
    efforts: Option<std::collections::BTreeMap<String, Vec<String>>>,
    defaults: Option<std::collections::BTreeMap<String, String>>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.codex_set_profile_fetched_models(&id, models, efforts, defaults)
}

#[tauri::command]
pub fn set_profile_balance(
    id: String,
    info: ProfileBalanceInfo,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.set_profile_balance(&id, &info)
}

#[tauri::command]
pub async fn codex_set_profile_account(
    id: String,
    account_id: Option<String>,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    state
        .codex_set_profile_account_and_apply_active(&id, account_id.as_deref(), &oauth.0)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn codex_duplicate_profile(
    id: String,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileSummary> {
    state.codex_duplicate_profile(&id)
}

#[tauri::command]
pub fn codex_get_profile(
    id: String,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileDetail> {
    state.codex_get_profile(&id)
}

#[tauri::command]
pub fn codex_update_profile(
    id: String,
    name: String,
    description: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    admin_url: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileSummary> {
    state.codex_update_profile(
        &id,
        &name,
        description.as_deref(),
        base_url.as_deref(),
        api_key.as_deref(),
        admin_url.as_deref(),
    )
}

#[tauri::command]
pub fn codex_set_profile_model(
    id: String,
    model: Option<String>,
    effort: Option<String>,
    fast: Option<bool>,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileDetail> {
    state.codex_set_profile_model(&id, model.as_deref(), effort.as_deref(), fast)
}

#[tauri::command]
pub fn codex_update_profile_config(
    id: String,
    config_text: String,
    catalog_text: Option<String>,
    auth_text: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<CodexProfileDetail> {
    state.codex_update_profile_config(
        &id,
        &config_text,
        catalog_text.as_deref(),
        auth_text.as_deref(),
    )
}

#[tauri::command]
pub fn codex_patch_chatgpt_context_config(
    config_text: String,
    enabled: bool,
    compact_token_limit: i64,
) -> AppResult<String> {
    crate::codex::config::patch_context_override(&config_text, enabled, compact_token_limit)
}

#[tauri::command]
pub fn codex_patch_system_proxy_config(config_text: String, enabled: bool) -> AppResult<String> {
    crate::codex::config::patch_system_proxy(&config_text, enabled)
}

#[tauri::command]
pub fn codex_patch_context_management_config(
    config_text: String,
    enabled: bool,
) -> AppResult<String> {
    crate::codex::config::patch_context_management(&config_text, enabled)
}

// async 让解析跑在 tokio 线程池而非主线程，避免大文档校验阻塞 UI
#[tauri::command]
pub async fn validate_toml(text: String) -> Vec<crate::codex::config::TomlDiagnostic> {
    crate::codex::config::validate_document(&text)
}

#[tauri::command]
pub fn format_toml(text: String) -> String {
    crate::codex::config::format_document(&text)
}

#[tauri::command]
pub fn codex_delete_profile(id: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.codex_delete_profile(&id)
}

#[tauri::command]
pub fn codex_reorder_profiles(ids: Vec<String>, state: State<'_, AppContext>) -> AppResult<()> {
    state.codex_reorder_profiles(&ids)
}

#[tauri::command]
pub async fn codex_apply_profile(
    id: String,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    let result = state.codex_apply_profile_with_auth(&id, &oauth.0).await;
    if let Err(error) = &result {
        tauri_plugin_log::log::warn!(
            "[apply.profile.switch] profile_id={id} outcome=failure failure_kind=internal error={error:?} msg=\"配置切换失败\""
        );
    }
    result.map_err(|error| error.to_string())
}

/// 重启期间后端会阻塞数秒（优雅退出等待 + 启动轮询），必须放到专用 blocking
/// 线程：同步命令在主线程内联执行，会把窗口消息泵占死导致整窗无响应。
#[tauri::command]
pub async fn restart_codex(state: State<'_, AppContext>) -> AppResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.restart_codex())
        .await
        .map_err(|error| app_err!("Codex 重启任务失败: {error}"))?
}

/// MCP 服务器管理：直接读写 live ~/.codex/config.toml 的 [mcp_servers.*] 段。
#[tauri::command]
pub fn codex_list_mcp_servers(state: State<'_, AppContext>) -> AppResult<Vec<McpServerSpec>> {
    state.codex_list_mcp_servers()
}

#[tauri::command]
pub fn claude_list_mcp_servers(state: State<'_, AppContext>) -> AppResult<Vec<McpServerSpec>> {
    state.claude_list_mcp_servers()
}

#[tauri::command]
pub fn claude_get_mcp_server_json(
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<Option<String>> {
    state.claude_get_mcp_server_json(&name)
}

#[tauri::command]
pub fn claude_save_mcp_server(
    original_name: Option<String>,
    name: String,
    json: String,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.claude_save_mcp_server(original_name.as_deref(), &name, &json)
}

#[tauri::command]
pub fn claude_delete_mcp_server(name: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.claude_delete_mcp_server(&name)
}

/// 测试 MCP 最小 initialize 握手；include_tools 为 true 时才额外读取 tools/list。
/// manual=true（手动点击）时成功记 Info 日志，进页静默探测只记 Debug。
#[tauri::command]
pub async fn probe_mcp_server(
    name: String,
    include_tools: bool,
    manual: Option<bool>,
    tool: SkillTool,
    state: State<'_, AppContext>,
) -> AppResult<crate::models::McpProbeResult> {
    state
        .probe_mcp_server(&name, include_tools, manual.unwrap_or(false), tool)
        .await
}

#[tauri::command]
pub fn codex_save_mcp_server(
    original_name: Option<String>,
    spec: McpServerSpec,
    fragment: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.save_mcp_server_with_fragment(original_name.as_deref(), spec, fragment.as_deref())
}

#[tauri::command]
pub fn codex_delete_mcp_server(name: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.codex_delete_mcp_server(&name)
}

/// MCP 引擎级开关：只移除该引擎用户范围 live 条目，数据库镜像保留以便恢复。
#[tauri::command]
pub fn set_mcp_server_enabled(
    name: String,
    tool: SkillTool,
    enabled: bool,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.set_mcp_server_enabled(&name, tool, enabled)
}

/// 差异处理"同步"：改写数据库镜像中的单个条目（fragment 为空表示删除该条目；均不触碰 live）。
#[tauri::command]
pub fn set_mcp_mirror(
    name: String,
    fragment: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.set_mcp_mirror_entry(&name, fragment.as_deref())
}

/// 差异处理"撤销"：仅将单个条目恢复为数据库内容写回 live（fragment 为空时从 live 移除）。
#[tauri::command]
pub fn revert_mcp_live(
    name: String,
    fragment: Option<String>,
    state: State<'_, AppContext>,
) -> AppResult<()> {
    state.revert_mcp_live_entry(&name, fragment.as_deref())
}

/// 差异处理"同步"批量：一次写入多条数据库镜像条目（不触碰 live）。
#[tauri::command]
pub fn set_mcp_mirror_entries(
    actions: Vec<McpDiffEntryAction>,
    state: State<'_, AppContext>,
) -> AppResult<usize> {
    state.set_mcp_mirror_entries(&actions)
}

/// 差异处理"撤销"批量：一次写回多条 config.toml 条目（fragment 为空表示从 live 移除）。
#[tauri::command]
pub fn revert_mcp_live_entries(
    actions: Vec<McpDiffEntryAction>,
    state: State<'_, AppContext>,
) -> AppResult<usize> {
    state.revert_mcp_live_entries(&actions)
}

/// 创建表单预填用：优先数据库 MCP 镜像，首次无镜像时回退 live。
#[tauri::command]
pub fn codex_get_mcp_section_toml(state: State<'_, AppContext>) -> AppResult<String> {
    state.codex_mcp_section_toml()
}

/// 用户显式恢复：数据库镜像写回 live config.toml，返回恢复的服务器数量。
#[tauri::command]
pub fn restore_mcp_from_database(state: State<'_, AppContext>) -> AppResult<usize> {
    state.restore_mcp_from_database()
}

/// 对比 live config.toml 与数据库镜像的 MCP 差异（只读不写），供同步前人工裁决。
/// 窗口激活时会与 get_state 一起被调用，而它会等 operation 锁——重启/切换持锁数秒，
/// 同步命令在主线程等锁会把窗口消息泵占死。与 restart_codex 同理丢到 blocking 线程：
/// 这里确实会阻塞数秒，不能占着 async runtime 的工作线程。
#[tauri::command]
pub async fn codex_mcp_sync_preview(state: State<'_, AppContext>) -> AppResult<McpSyncPreview> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.codex_mcp_sync_preview())
        .await
        .map_err(|error| app_err!("MCP 差异检查任务失败: {error}"))?
}

#[tauri::command]
pub async fn claude_mcp_sync_preview(state: State<'_, AppContext>) -> AppResult<McpSyncPreview> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.mcp_sync_preview(SkillTool::Claude))
        .await
        .map_err(|error| app_err!("MCP 差异检查任务失败: {error}"))?
}

#[tauri::command]
pub fn claude_resolve_mcp_entries(
    actions: Vec<McpDiffEntryAction>,
    adopt: bool,
    state: State<'_, AppContext>,
) -> AppResult<usize> {
    state.claude_resolve_mcp_entries(&actions, adopt)
}

/// MCP 编辑页初始化：读取 live 中指定服务器的原始片段（含未建模键与注释）。
#[tauri::command]
pub fn codex_get_mcp_server_toml(
    name: String,
    state: State<'_, AppContext>,
) -> AppResult<Option<String>> {
    state.codex_mcp_server_toml(&name)
}

/// MCP 编辑页实时同步：把表单建模字段写进单服务器片段（表单 → 编辑器）。
#[tauri::command]
pub fn patch_mcp_fragment(toml: String, spec: McpServerSpec) -> AppResult<String> {
    let mut spec = spec;
    // 表单的 enabled 是应用开关状态；原生 enabled 只由配置源码管理。
    spec.enabled = codex_config::parse_mcp_fragment(&toml).map_or(None, |source| source.enabled);
    codex_config::patch_mcp_fragment(&toml, &spec)
}

/// MCP 编辑页实时同步：单服务器片段解析为建模字段（编辑器 → 表单）。
#[tauri::command]
pub fn parse_mcp_fragment(toml: String) -> AppResult<McpServerSpec> {
    codex_config::parse_mcp_fragment(&toml)
}

/// 前端已加载的应用状态直接更新托盘；不在原生启动路径额外读取供应商。
#[tauri::command]
pub fn set_tray_menu(
    app: AppHandle,
    language: String,
    profiles: Vec<crate::TrayProfile>,
    active_profile_id: Option<String>,
) -> AppResult<()> {
    let menu = crate::tray_menu(&app, &language, &profiles, active_profile_id.as_deref())
        .map_err(|error| app_err!("更新托盘菜单失败: {error}"))?;
    app.tray_by_id("main")
        .ok_or_else(|| app_err!("找不到系统托盘"))?
        .set_menu(Some(menu))
        .map_err(|error| app_err!("更新托盘菜单失败: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn set_window_theme(dark: bool, app: AppHandle) -> AppResult<()> {
    #[cfg(not(windows))]
    {
        let _ = (dark, app);
    }
    #[cfg(windows)]
    {
        use crate::error::app_err;
        use std::ffi::c_void;
        use tauri::Manager; // get_webview_window
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
        use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_NCACTIVATE};

        if let Some(window) = app.get_webview_window("main") {
            let hwnd = window
                .hwnd()
                .map_err(|error| app_err!("无法获取窗口句柄: {error}"))?;
            let ours = HWND(hwnd.0);
            let value = i32::from(dark);
            let raw: *const c_void = &value as *const i32 as *const c_void;
            unsafe {
                DwmSetWindowAttribute(
                    ours,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    raw,
                    std::mem::size_of::<i32>() as u32,
                )
            }
            .map_err(|error| app_err!("无法设置窗口标题栏主题: {error}"))?;
            // 强制立即重绘标题栏，避免 DWM 延迟刷新导致与内容主题切换不同步
            unsafe {
                SendMessageW(ours, WM_NCACTIVATE, Some(WPARAM(0)), Some(LPARAM(0)));
                SendMessageW(ours, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn auth_start_browser_login(
    state: State<'_, CodexOAuthState>,
) -> Result<BrowserLoginStart, String> {
    state
        .0
        .start_browser_login()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn auth_poll_browser_login(
    state: State<'_, CodexOAuthState>,
) -> Result<Option<ManagedAccount>, String> {
    state
        .0
        .poll_browser_login()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn auth_cancel_browser_login(state: State<'_, CodexOAuthState>) -> Result<(), String> {
    state.0.cancel_browser_login();
    Ok(())
}

#[tauri::command]
pub async fn auth_get_status(
    app: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> Result<AuthStatus, String> {
    app.sync_live_oauth_auth(&oauth.0)
        .await
        .map_err(|error| error.to_string())?;
    let mut status = oauth.0.get_status().await;
    // Desktop 账号真源是数据库认证快照：不随切换覆写的 live auth.json 漂移；
    // 与 OAuth 账号各自展示，来源由配置绑定显式决定，不在状态层合并。
    status.external = app
        .desktop_auth_accounts()
        .map_err(|error| error.to_string())?;
    if !status.external.is_empty() {
        status.authenticated = true;
    }
    Ok(status)
}

#[tauri::command]
pub async fn auth_get_quota(
    source: AuthSource,
    account_id: Option<String>,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> Result<ProfileBalance, String> {
    state
        .get_auth_quota(source, account_id.as_deref(), &oauth.0)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn auth_warmup(
    source: AuthSource,
    account_id: String,
    state: State<'_, AppContext>,
    oauth: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    state
        .warmup_auth_account(source, &account_id, &oauth.0)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn auth_preview(
    account_id: String,
    oauth: State<'_, CodexOAuthState>,
) -> Result<Option<String>, String> {
    if let Some(cached) = oauth.0.cached_auth_json(&account_id).await {
        return Ok(Some(cached));
    }
    oauth
        .0
        .codex_auth_json(&account_id)
        .await
        .map(Some)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn auth_remove_account(
    account_id: String,
    state: State<'_, CodexOAuthState>,
) -> Result<(), String> {
    state
        .0
        .remove_account(&account_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_url(url: String) -> AppResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(app_err!("仅支持打开 http(s) 链接"));
    }
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let url_wide = HSTRING::from(&url);
        let operation = HSTRING::from("open");
        let result =
            unsafe { ShellExecuteW(None, &operation, &url_wide, None, None, SW_SHOWNORMAL) };
        if result.0 as usize <= 32 {
            if url.starts_with("https://auth.openai.com/oauth/authorize?") {
                tauri_plugin_log::log::warn!(
                    "[auth.login.browser] stage=browser_open outcome=failure failure_kind=io_error shell_code={} msg=\"OAuth 授权浏览器打开失败\"",
                    result.0 as usize
                );
            }
            return Err(app_err!("无法打开系统浏览器"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = std::process::Command::new("open").arg(&url).spawn() {
            if url.starts_with("https://auth.openai.com/oauth/authorize?") {
                tauri_plugin_log::log::warn!(
                    "[auth.login.browser] stage=browser_open outcome=failure failure_kind=io_error error_kind={:?} msg=\"OAuth 授权浏览器打开失败\"",
                    error.kind()
                );
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppContext>) -> AppResult<Settings> {
    state.settings()
}

/// 只接管网络配置与资源注册，下载、签名验证和安装继续由官方更新插件执行。
#[tauri::command]
pub async fn check_app_update(webview: tauri::Webview) -> AppResult<Option<serde_json::Value>> {
    use tauri_plugin_updater::UpdaterExt;

    let network = crate::network::Network::current()
        .await
        .map_err(|error| app_err!("{error}"))?;
    let updater = network
        .updater(webview.updater_builder())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| app_err!("创建更新器失败: {error}"))?;
    let Some(update) = updater
        .check()
        .await
        .map_err(|error| app_err!("检查更新失败: {error}"))?
    else {
        return Ok(None);
    };
    let mut metadata = serde_json::json!({
        "currentVersion": update.current_version,
        "version": update.version,
        "date": update.raw_json.get("pub_date"),
        "body": update.body,
        "rawJson": update.raw_json,
    });
    metadata["rid"] = serde_json::json!(webview.resources_table().add(update));
    Ok(Some(metadata))
}

/// 应用内更新安装成功前写入「已更新到 vX」标记（须在启动安装器/退出进程前完成落盘）。
#[tauri::command]
pub fn set_update_marker(version: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.set_update_marker(&version)
}

/// 新版本启动时消费标记：返回版本号并清除文件，无标记返回 null。
/// rollback=true（安装失败取回）时日志记 Warn 回滚，正常启动消费记 Info 落地。
#[tauri::command]
pub fn take_update_marker(
    rollback: Option<bool>,
    state: State<'_, AppContext>,
) -> AppResult<Option<String>> {
    state.take_update_marker(rollback.unwrap_or(false))
}

fn require_update_version(version: Option<String>) -> AppResult<String> {
    version
        .filter(|version| !version.trim().is_empty())
        .ok_or_else(|| app_err!("更新日志缺少版本号"))
}

/// CLI 更新计时器的跳过决策留痕：真实检查的成败由 services/cli.rs 落 Info/Warn，
/// 这里只补"这轮为什么没查"的痕迹。Debug 级，release 自动消失。
#[tauri::command]
pub fn report_cli_update_tick(client: String, decision: String) {
    let msg = match decision.as_str() {
        "cooldown_skip" => "冷却中跳过本轮更新检查",
        "not_native" => "非原生安装，不参与更新检查",
        "detect_failed" => "本地检测失败，跳过本轮更新检查",
        _ => "更新检查计时器决策",
    };
    tauri_plugin_log::log::debug!(
        "[app.cli.update] client={client:?} decision={decision} outcome=skipped msg={msg:?}"
    );
}

#[tauri::command]
pub fn log_update_event(event: String, version: Option<String>) -> AppResult<()> {
    match event.as_str() {
        "check_available" => {
            let version = require_update_version(version)?;
            tauri_plugin_log::log::info!(
                "[update.check] version={version:?} outcome=success msg=\"检测到应用新版本\""
            );
        }
        "check_latest" => {
            tauri_plugin_log::log::info!("[update.check] outcome=success msg=\"应用已是最新版本\"");
        }
        "check_failure" => {
            tauri_plugin_log::log::warn!(
                "[update.check] outcome=failure failure_kind=network_error msg=\"应用检查更新失败\""
            );
        }
        "download_start" => {
            let version = require_update_version(version)?;
            tauri_plugin_log::log::info!(
                "[update.download] version={version:?} outcome=success msg=\"开始下载更新\""
            );
        }
        "download_complete" => {
            let version = require_update_version(version)?;
            tauri_plugin_log::log::info!(
                "[update.download] version={version:?} outcome=success msg=\"更新包下载完成\""
            );
        }
        "download_failure" => {
            let version = require_update_version(version)?;
            tauri_plugin_log::log::warn!(
                "[update.download] version={version:?} outcome=failure failure_kind=network_error msg=\"更新包下载失败\""
            );
        }
        "install_complete" => {
            let version = require_update_version(version)?;
            tauri_plugin_log::log::info!(
                "[update.install] version={version:?} outcome=success msg=\"安装器执行完成\""
            );
        }
        "install_failure" => {
            let version = require_update_version(version)?;
            tauri_plugin_log::log::warn!(
                "[update.install] version={version:?} outcome=failure failure_kind=internal msg=\"安装或重启失败\""
            );
        }
        _ => return Err(app_err!("不支持的更新日志事件")),
    }
    Ok(())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    settings: Settings,
    state: State<'_, AppContext>,
) -> AppResult<Settings> {
    if !matches!(
        settings.auto_backup_interval_hours,
        0 | 6 | 12 | 24 | 48 | 168
    ) {
        return Err(app_err!("不支持的自动备份间隔"));
    }
    if !matches!(
        settings.database_backup_keep_count,
        3 | 5 | 10 | 15 | 20 | 30
    ) {
        return Err(app_err!("不支持的备份保留数量"));
    }
    let show_menu = settings.tray_click_action == TrayClickAction::ShowMenu;
    let state = state.inner().clone();
    let saved = tauri::async_runtime::spawn_blocking(move || state.save_settings(&settings))
        .await
        .map_err(|error| app_err!("设置保存任务失败: {error}"))??;
    // 存库成功后才应用托盘点击模式，保存失败即保持原状，无需回滚。
    let tray_mode = app.state::<crate::TrayClickMode>();
    if show_menu != tray_mode.0.load(Ordering::Relaxed) {
        app.tray_by_id("main")
            .ok_or_else(|| app_err!("找不到系统托盘"))?
            .set_show_menu_on_left_click(show_menu)
            .map_err(|error| app_err!("更新托盘点击行为失败: {error}"))?;
        tray_mode.0.store(show_menu, Ordering::Relaxed);
    }
    sync_autostart(&app, &saved)?;
    Ok(saved)
}

#[tauri::command]
pub async fn get_proxy_status() -> AppResult<Option<String>> {
    crate::network::Network::current()
        .await
        .map(|network| network.display)
        .map_err(|error| app_err!("{error}"))
}

fn sync_autostart(app: &AppHandle, settings: &Settings) -> AppResult<()> {
    use tauri_plugin_autostart::ManagerExt;
    // 同上：dev 构建不得写入指向 target/debug 的开机自启
    if tauri::is_dev() {
        return Ok(());
    }
    if settings.autostart_enabled {
        app.autolaunch()
            .enable()
            .map_err(|error| app_err!("同步开机自启设置失败: {error}"))
    } else {
        let _ = app.autolaunch().disable();
        Ok(())
    }
}

#[tauri::command]
pub fn open_path(path: String, state: State<'_, AppContext>) -> AppResult<()> {
    state.open_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_editor_patch_preserves_source_enabled_instead_of_app_toggle() {
        for (native, toggle) in [
            (None, Some(false)),
            (Some(false), None),
            (Some(true), Some(false)),
        ] {
            let mut source =
                "[mcp_servers.fixture]\nurl = \"https://example.test/mcp\"\n".to_string();
            if let Some(enabled) = native {
                source.push_str(&format!("enabled = {enabled}\n"));
            }
            let mut spec = codex_config::parse_mcp_fragment(&source).unwrap();
            spec.enabled = toggle;
            spec.url = Some("https://example.test/updated".to_string());
            let patched = patch_mcp_fragment(source, spec).unwrap();
            let parsed = codex_config::parse_mcp_fragment(&patched).unwrap();
            assert_eq!(parsed.enabled, native, "{patched}");
            assert_eq!(parsed.url.as_deref(), Some("https://example.test/updated"));
        }
        let spec = codex_config::parse_mcp_fragment(
            "[mcp_servers.fixture]\nurl = \"https://example.test/mcp\"\n",
        )
        .unwrap();
        assert!(patch_mcp_fragment(String::new(), spec).is_ok());
        assert!(patch_mcp_fragment("[invalid".to_string(), McpServerSpec::default()).is_err());
    }

    #[test]
    fn cached_auth_failure_allows_same_account_fallback_but_not_network_or_region_errors() {
        assert!(should_try_next_account_credential(
            &CodexProfileConnectionResult {
                ok: false,
                latency_ms: Some(10),
                status: Some(401),
                error: Some("ChatGPT 登录已失效".to_string()),
            }
        ));
        assert!(should_try_next_account_credential(
            &CodexProfileConnectionResult {
                ok: false,
                latency_ms: Some(10),
                status: Some(403),
                error: Some("ChatGPT 登录已失效".to_string()),
            }
        ));
        assert!(!should_try_next_account_credential(
            &CodexProfileConnectionResult {
                ok: false,
                latency_ms: Some(10),
                status: Some(403),
                error: Some("认证请求被地区限制拦截".to_string()),
            }
        ));
        assert!(!should_try_next_account_credential(
            &CodexProfileConnectionResult {
                ok: false,
                latency_ms: None,
                status: None,
                error: Some("网络错误".to_string()),
            }
        ));
    }
}
