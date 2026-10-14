use super::{
    app_err, atomic_write, backup_file, codex_config, now_ms, AppContext, AppResult,
    CodexProfilePayload,
};
use crate::codex_builtin;

pub(crate) struct ProviderDetail {
    pub(crate) base_url: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) fragment: String,
}

pub(crate) fn parse_provider_detail(body: &str) -> AppResult<ProviderDetail> {
    let document = codex_config::parse_document(body)?;
    let table = document.as_table();
    let value = |key: &str| {
        table
            .get(key)
            .and_then(toml_edit::Item::as_str)
            .map(str::to_string)
    };
    let mut fragment = String::new();
    for (key, item) in table.iter() {
        fragment.push_str(&format!("{key} = {item}\n"));
    }
    Ok(ProviderDetail {
        base_url: value("base_url"),
        api_key: value("experimental_bearer_token"),
        fragment,
    })
}

pub(crate) fn provider_api_key(body: &str) -> Option<String> {
    let document = codex_config::parse_document(body).ok()?;
    document
        .as_table()
        .get("experimental_bearer_token")
        .and_then(toml_edit::Item::as_str)
        .map(str::to_string)
}

/// 供应商已保存的真实 API 密钥（占位符视为未配置）。
pub(crate) fn stored_provider_api_key(payload: &CodexProfilePayload) -> Option<String> {
    payload
        .provider_body
        .as_deref()
        .and_then(provider_api_key)
        .filter(|key| !key.trim().is_empty() && !is_builtin_placeholder(payload, key))
}

pub(crate) fn is_builtin_placeholder(payload: &CodexProfilePayload, key: &str) -> bool {
    payload
        .builtin
        .as_deref()
        .and_then(|kind| codex_builtin::template(kind).ok())
        .is_some_and(|template| template.is_placeholder(key.as_bytes()))
}

pub(crate) fn profile_config_fragment(payload: &CodexProfilePayload) -> String {
    let mut fragment = String::new();
    for (key, raw) in &payload.model_values {
        fragment.push_str(&format!("{key} = {raw}\n"));
    }
    if let (Some(provider_id), Some(body)) = (&payload.provider_id, &payload.provider_body) {
        if let Ok(detail) = parse_provider_detail(body) {
            fragment.push_str(&format!("[model_providers.{provider_id}]\n"));
            fragment.push_str(&detail.fragment);
        }
    }
    fragment
}

/// 使用中的供应商编辑地址/密钥时，只更新 live 的 provider 段。
pub(super) fn write_live_provider_update(
    context: &AppContext,
    profile_id: &str,
    provider_id: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> AppResult<()> {
    let config_path = context.paths.codex_config();
    let original = std::fs::read_to_string(&config_path)
        .map_err(|error| app_err!("无法读取 {}: {error}", config_path.display()))?;
    let mut document = codex_config::parse_document(&original)?;
    codex_config::update_provider_in_document(&mut document, provider_id, base_url, api_key)?;
    let updated = codex_config::normalize_global_section_order(&document.to_string());
    backup_file(&config_path, &context.paths.config_backup, "config")?;
    atomic_write(&config_path, updated.as_bytes())?;
    context.database.record_event(
        Some(profile_id),
        "update",
        "success",
        Some("provider settings written back to live config"),
        &now_ms().to_string(),
    )?;
    Ok(())
}
