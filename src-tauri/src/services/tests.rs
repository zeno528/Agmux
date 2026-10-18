use super::*;
use crate::models::ClaudeProfileInput;
use rusqlite::Connection;
use serde_json::Value;

#[test]
fn update_marker_writes_once_and_consumes_once() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    let context = AppContext::new(paths).unwrap();

    // 无标记 → None；写入 → 读取即消费；再读 → None（一次性）
    assert_eq!(context.take_update_marker(false).unwrap(), None);
    context.set_update_marker("0.13.4").unwrap();
    assert_eq!(
        context.take_update_marker(false).unwrap(),
        Some("0.13.4".to_string())
    );
    assert_eq!(context.take_update_marker(false).unwrap(), None);
}

fn chatgpt_test_context() -> (tempfile::TempDir, AppContext) {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"gpt-5.6\"\n").unwrap();
    let context = AppContext::new(paths).unwrap();
    (home, context)
}

/// 测试夹具：live config 的 [mcp_servers] 段整体镜像进数据库，返回导入的服务器数。
/// （生产侧 import_mcp_from_live 因 UI 从未接线已随命令链一并移除，测试仍需要它构造镜像状态。）
fn import_mcp_from_live(context: &AppContext) -> usize {
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    let document = codex_config::parse_document(&live).unwrap();
    let fragments = codex_config::mcp_server_fragments_from_document(&document);
    let count = fragments.len();
    context.replace_mcp_mirror(&fragments).unwrap();
    count
}

fn chatgpt_auth(account_id: &str, access_token: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let payload =
        URL_SAFE_NO_PAD.encode(format!(r#"{{"chatgpt_account_id":"{account_id}"}}"#).as_bytes());
    format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"e30.{payload}.sig","access_token":"{access_token}"}}}}"#
    )
}

#[tokio::test]
async fn model_cache_reads_never_fetch_and_are_isolated_by_account_source() {
    let (_home, context) = chatgpt_test_context();
    let oauth = crate::auth::codex_oauth::CodexOAuthManager::new(context.database.clone());
    let models = super::connections::parse_chatgpt_models(serde_json::json!({
        "models":[{"slug":"fixture", "display_name":"Fixture", "visibility":"list",
            "supported_reasoning_levels":[{"effort":"low"}], "default_reasoning_level":"low"}]
    }))
    .unwrap();
    context
        .database
        .set_chatgpt_models("oauth:row", &models)
        .unwrap();
    let cached = context
        .codex_fetch_chatgpt_models(None, AuthSource::Oauth, Some("row"), &oauth, false)
        .await
        .unwrap();
    assert_eq!(cached[0].slug, "fixture");
    let desktop = context
        .codex_fetch_chatgpt_models(None, AuthSource::Desktop, Some("row"), &oauth, false)
        .await
        .unwrap();
    assert!(desktop.is_empty());
    context
        .database
        .set_chatgpt_models("desktop:workspace", &models)
        .unwrap();
    let desktop = context
        .codex_fetch_chatgpt_models(None, AuthSource::Desktop, Some("workspace"), &oauth, false)
        .await
        .unwrap();
    assert_eq!(desktop[0].slug, "fixture");
    assert!(context
        .codex_fetch_chatgpt_models(None, AuthSource::Oauth, Some("missing"), &oauth, false)
        .await
        .unwrap()
        .is_empty());
    assert!(context
        .codex_fetch_chatgpt_models(None, AuthSource::Desktop, None, &oauth, false)
        .await
        .unwrap()
        .is_empty());
}

/// id_token 带官方嵌套套餐 claim 的认证快照（plan_type 徽标链路用）
fn chatgpt_auth_with_plan(account_id: &str, access_token: &str, plan: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let payload = URL_SAFE_NO_PAD.encode(
        serde_json::json!({
            "chatgpt_account_id": account_id,
            "https://api.openai.com/auth": { "chatgpt_plan_type": plan }
        })
        .to_string()
        .as_bytes(),
    );
    format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"e30.{payload}.sig","access_token":"{access_token}"}}}}"#
    )
}

fn oauth_auth(account_id: &str, access_token: &str, refresh_token: &str, id_token: &str) -> String {
    format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{id_token}","access_token":"{access_token}","refresh_token":"{refresh_token}","account_id":"{account_id}"}}}}"#
    )
}

#[test]
fn live_codex_auth_is_available_for_matching_account_only() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let payload =
        URL_SAFE_NO_PAD.encode(br#"{"chatgpt_account_id":"acc-live","email":"live@example.com"}"#);
    let auth = format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"e30.{payload}.sig","access_token":"live-access"}}}}"#
    );
    std::fs::write(paths.codex_home.join("auth.json"), auth).unwrap();

    let context = AppContext::new(paths).unwrap();

    assert_eq!(
        context
            .external_codex_access_token_for_account("acc-live")
            .unwrap()
            .as_deref(),
        Some("live-access")
    );
    assert!(context
        .external_codex_access_token_for_account("acc-other")
        .unwrap()
        .is_none());
}

#[test]
fn connection_error_body_detects_provider_level_failures() {
    let parse = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|json| connections::connection_error_from_body(&json))
    };
    // 智谱风格：HTTP 200 包装 401
    assert_eq!(
        parse(r#"{"code":401,"msg":"令牌已过期或验证不正确","success":false}"#).as_deref(),
        Some("令牌已过期或验证不正确")
    );
    // OpenAI 风格：error.message
    assert_eq!(
        parse(r#"{"error":{"message":"Incorrect API key provided"}}"#).as_deref(),
        Some("Incorrect API key provided")
    );
    // error 为字符串
    assert_eq!(
        parse(r#"{"error":"unauthorized"}"#).as_deref(),
        Some("unauthorized")
    );
    // 字符串业务错误码
    assert_eq!(
        parse(r#"{"code":"401","msg":"invalid key"}"#).as_deref(),
        Some("invalid key")
    );
    // 正常模型列表 / 2xx 业务码不应误判
    assert_eq!(parse(r#"{"data":[{"id":"glm-5.3"}]}"#), None);
    assert_eq!(parse(r#"{"code":200,"msg":"ok","success":true}"#), None);
}

#[test]
fn card_model_selection_only_updates_top_level_config_and_preserves_other_files() {
    let (_home, context) = chatgpt_test_context();
    let config = "model = \"fixture\" # keep\nmodel_reasoning_effort = \"high\"\nbase_instructions = '''\nmodel = \"inside-string\"\n'''\n[profiles.other]\nmodel = \"nested\"\n[mcp_servers.keep]\ncommand = \"node\"\n";
    std::fs::write(context.paths.codex_config(), config).unwrap();
    let profile = context.codex_capture_profile("Fixture").unwrap();
    assert!(!profile.fast_mode);
    context
        .codex_set_profile_model(&profile.id, None, Some("low"), None)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_config()).unwrap(),
        config
    );
    context.codex_apply_profile(&profile.id).unwrap();
    let auth_path = context.paths.codex_home.join("auth.json");
    std::fs::write(&auth_path, "{\"keep\":true}").unwrap();
    context
        .codex_set_profile_model(&profile.id, None, None, Some(true))
        .unwrap();
    let before = context.codex_get_profile(&profile.id).unwrap();
    assert!(
        crate::database::codex_profile_summary(
            &context.database.codex_profile(&profile.id).unwrap()
        )
        .fast_mode
    );
    assert_eq!(
        before.model_values.get("service_tier").map(String::as_str),
        Some("\"fast\"")
    );
    context
        .codex_set_profile_model(&profile.id, Some("next"), None, None)
        .unwrap();
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    let document = codex_config::parse_document(&live).unwrap();
    assert_eq!(document["model"].as_str(), Some("next"));
    assert_eq!(document["model_reasoning_effort"].as_str(), Some("low"));
    assert_eq!(document["service_tier"].as_str(), Some("fast"));
    assert_eq!(
        document["profiles"]["other"]["model"].as_str(),
        Some("nested")
    );
    assert_eq!(
        document["base_instructions"].as_str(),
        Some("model = \"inside-string\"\n")
    );
    assert!(live.contains("# keep") && live.contains("[mcp_servers.keep]"));
    assert_eq!(
        std::fs::read_to_string(&auth_path).unwrap(),
        "{\"keep\":true}"
    );
    assert_eq!(
        context.codex_get_profile(&profile.id).unwrap().raw_catalog,
        before.raw_catalog
    );
    context
        .codex_set_profile_model(&profile.id, Some(""), Some(""), None)
        .unwrap();
    context
        .codex_set_profile_model(&profile.id, None, None, Some(false))
        .unwrap();
    let defaults = context.codex_get_profile(&profile.id).unwrap();
    assert!(!defaults.model_values.contains_key("model"));
    assert!(!defaults.model_values.contains_key("model_reasoning_effort"));
    assert!(!defaults.model_values.contains_key("service_tier"));
    assert!(
        !crate::database::codex_profile_summary(
            &context.database.codex_profile(&profile.id).unwrap()
        )
        .fast_mode
    );
    let third = context.codex_add_custom_profile("Third", None,
        "model_provider = \"fixture\"\n[model_providers.fixture]\nbase_url = \"https://example.test\"\n",
        None, None, None, None, None).unwrap();
    assert!(context
        .codex_set_profile_model(&third.id, None, None, Some(true))
        .is_err());
    std::fs::write(context.paths.codex_config(), "model = [").unwrap();
    assert!(context
        .codex_set_profile_model(&profile.id, Some("invalid"), None, None)
        .is_err());
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .model_values,
        defaults.model_values
    );
}

#[test]
fn fetched_models_survive_profile_reload() {
    let (_home, context) = chatgpt_test_context();
    let profile = context.codex_capture_profile("模型缓存").unwrap();

    context
        .codex_set_profile_fetched_models(
            &profile.id,
            vec!["gpt-5.6".into(), "gpt-5.6-mini".into()],
            Some(BTreeMap::from([("gpt-5.6".into(), vec!["high".into()])])),
            Some(BTreeMap::from([("gpt-5.6".into(), "high".into())])),
        )
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    context.get_state().unwrap();

    assert_eq!(
        context
            .codex_get_profile(&profile.id)
            .unwrap()
            .fetched_models,
        vec!["gpt-5.6", "gpt-5.6-mini"]
    );
    let reloaded = AppContext::new(context.paths.clone()).unwrap();
    assert_eq!(
        reloaded
            .codex_get_profile(&profile.id)
            .unwrap()
            .fetched_model_defaults,
        BTreeMap::from([("gpt-5.6".into(), "high".into())])
    );
    assert_eq!(
        reloaded
            .codex_get_profile(&profile.id)
            .unwrap()
            .fetched_model_efforts,
        BTreeMap::from([("gpt-5.6".into(), vec!["high".into()])])
    );
    reloaded
        .codex_set_profile_model(&profile.id, Some(""), Some(""), None)
        .unwrap();
    let restarted = AppContext::new(context.paths.clone()).unwrap();
    let state = restarted.get_state().unwrap();
    let summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert_eq!(summary.model.as_deref(), Some("gpt-5.6"));
    assert_eq!(summary.reasoning_effort.as_deref(), Some("high"));
    assert!(!restarted
        .codex_get_profile(&profile.id)
        .unwrap()
        .model_values
        .contains_key("model"));
}

#[test]
fn desktop_model_cache_survives_token_refresh_but_not_identity_changes() {
    for manual in [false, true] {
        let (_home, context) = chatgpt_test_context();
        let auth_path = context.paths.codex_home.join("auth.json");
        let profile = context.codex_capture_profile("缓存身份").unwrap();
        context.codex_apply_profile(&profile.id).unwrap();
        std::fs::write(&auth_path, chatgpt_auth("first", "old-token")).unwrap();
        context.get_state().unwrap();
        context
            .codex_set_profile_fetched_models(
                &profile.id,
                vec!["fixture".into()],
                Some(BTreeMap::from([("fixture".into(), vec!["high".into()])])),
                Some(BTreeMap::from([("fixture".into(), "high".into())])),
            )
            .unwrap();
        let update_auth = |auth: &str| {
            if manual {
                let detail = context.codex_get_profile(&profile.id).unwrap();
                context
                    .codex_update_profile_config(
                        &profile.id,
                        &detail.config_fragment,
                        None,
                        Some(auth),
                    )
                    .unwrap();
            } else {
                std::fs::write(&auth_path, auth).unwrap();
                context.get_state().unwrap();
            }
        };
        update_auth(&chatgpt_auth("first", "new-token"));
        let detail = context.codex_get_profile(&profile.id).unwrap();
        assert_eq!(detail.fetched_models, ["fixture"]);
        assert_eq!(detail.fetched_model_efforts["fixture"], ["high"]);
        assert_eq!(detail.fetched_model_defaults["fixture"], "high");
        update_auth(&chatgpt_auth("second", "second-token"));
        let detail = context.codex_get_profile(&profile.id).unwrap();
        assert!(detail.fetched_models.is_empty());
        assert!(detail.fetched_model_efforts.is_empty());
        assert!(detail.fetched_model_defaults.is_empty());
    }
}

#[test]
fn official_summary_uses_custom_catalog_instead_of_remote_defaults() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context
        .codex_set_profile_fetched_models(
            &profile.id,
            vec!["remote".into()],
            None,
            Some(BTreeMap::from([("remote".into(), "ultra".into())])),
        )
        .unwrap();
    let catalog = serde_json::json!({"models": [
        {"slug": "second", "display_name": "Second", "visibility": "list", "priority": 2,
         "supported_reasoning_levels": [{"effort": "high"}], "default_reasoning_level": "high"},
        {"slug": "first", "display_name": "First", "visibility": "list", "priority": 1,
         "supported_reasoning_levels": [{"effort": "low"}], "default_reasoning_level": "low"}
    ]})
    .to_string();
    context
        .codex_update_profile_config(
            &profile.id,
            "model_catalog_json = \"models.json\"\n",
            Some(&catalog),
            None,
        )
        .unwrap();
    let state = context.get_state().unwrap();
    let summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert_eq!(summary.model.as_deref(), Some("first"));
    assert_eq!(summary.reasoning_effort.as_deref(), Some("low"));
}

#[test]
fn capture_and_apply_profile_round_trip() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"

[mcp_servers.keep]
command = "node"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret"
"#,
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("GLM High").unwrap();
    std::fs::write(
        context.paths.codex_config(),
        r#"
model = "other-model"
model_provider = "ZAI"
model_reasoning_effort = "low"

[mcp_servers.keep]
command = "node"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://old.example"
experimental_bearer_token = "old"
"#,
    )
    .unwrap();

    context.codex_apply_profile(&profile.id).unwrap();
    let state = context.get_state().unwrap();
    let text = std::fs::read_to_string(context.paths.codex_config()).unwrap();

    assert_eq!(
        state.active_codex_profile_id.as_deref(),
        Some(profile.id.as_str())
    );
    assert!(text.contains("glm-5.3"));
    assert!(text.contains("https://api.example"));
    assert!(text.contains("[mcp_servers.keep]"));
    assert!(context.paths.config_backup.read_dir().unwrap().count() > 0);
}

#[test]
fn failed_codex_apply_restores_files_and_keeps_active_snapshot() {
    for builtin in [false, true] {
        for (table, operation) in [("app_state", "UPDATE"), ("switch_events", "INSERT")] {
            let (home, context) = chatgpt_test_context();
            let paths = &context.paths;
            let database = &context.database;
            let config_path = paths.codex_config();
            let auth_path = context.paths.codex_home.join("auth.json");
            let original_auth = r#"{"OPENAI_API_KEY":"fixture-old"}"#;
            let active = context.codex_capture_profile("Active").unwrap();
            context.codex_apply_profile(&active.id).unwrap();
            std::fs::write(&auth_path, original_auth).unwrap();
            context.get_state().unwrap();
            let original_config = std::fs::read(&config_path).unwrap();
            let original_payload = database.codex_profile(&active.id).unwrap().payload;
            let catalog = if builtin {
                paths.codex_home.join("models.json")
            } else {
                home.path().join("external-models.json")
            };
            if builtin {
                std::fs::write(&catalog, r#"{"models":[],"fixture":"old"}"#).unwrap();
            }
            let original_catalog = std::fs::read(&catalog).ok();
            let target = if builtin {
                context
                    .codex_add_builtin_profile(
                        "deepseek",
                        None,
                        None,
                        Some("fixture-key"),
                        None,
                        None,
                    )
                    .unwrap()
            } else {
                context.codex_capture_profile("Target").unwrap()
            };
            let mut stored = database.codex_profile(&target.id).unwrap();
            stored.payload.raw_catalog = Some(r#"{"models":[]}"#.into());
            stored.payload.raw_auth = Some(r#"{"OPENAI_API_KEY":"fixture-new"}"#.into());
            stored.payload.auth_auto_sync = Some(false);
            if !builtin {
                let path = serde_json::to_string(&catalog.to_string_lossy()).unwrap();
                stored
                    .payload
                    .model_values
                    .insert("model_catalog_json".into(), path.clone());
                stored.payload.raw_config = Some(format!(
                    "model = \"fixture-new\"\nmodel_catalog_json = {path}\n"
                ));
            }
            database
                .codex_update_profile(&target.id, &stored.name, &stored.payload, "2")
                .unwrap();
            let connection = Connection::open(&paths.database).unwrap();
            connection
                .execute_batch(&format!(
                    "CREATE TRIGGER reject_apply BEFORE {operation} ON {table}
                 BEGIN SELECT RAISE(FAIL, 'fixture commit failure'); END;"
                ))
                .unwrap();

            let error = context.codex_apply_profile(&target.id).unwrap_err();
            assert!(error.0.contains("fixture commit failure"), "{error}");
            assert_eq!(std::fs::read(&config_path).unwrap(), original_config);
            assert_eq!(std::fs::read_to_string(&auth_path).unwrap(), original_auth);
            assert_eq!(std::fs::read(&catalog).ok(), original_catalog);
            let state = context.get_state().unwrap();
            assert_eq!(state.active_codex_profile_id.as_ref(), Some(&active.id));
            assert_eq!(
                database.codex_profile(&active.id).unwrap().payload,
                original_payload
            );
            assert_eq!(
                database.codex_latest_applied_profile().unwrap().as_ref(),
                Some(&active.id)
            );

            connection
                .execute_batch("DROP TRIGGER reject_apply")
                .unwrap();
            context.codex_apply_profile(&target.id).unwrap();
            assert_eq!(
                database.app_state().unwrap().0.as_deref(),
                Some(target.id.as_str())
            );
            assert_eq!(
                std::fs::read_to_string(&auth_path).unwrap(),
                r#"{"OPENAI_API_KEY":"fixture-new"}"#
            );
            assert_eq!(
                std::fs::read_to_string(&catalog).unwrap(),
                r#"{"models":[]}"#
            );
        }
    }
}

#[test]
fn apply_profile_autosyncs_accumulated_active_profile() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let context = AppContext::new(paths).unwrap();
    let write = |text: &str| std::fs::write(context.paths.codex_config(), text).unwrap();

    write(
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret"
"#,
    );
    let profile_a = context.codex_capture_profile("A").unwrap();
    // A 显式激活，成为唯一“使用中”来源
    context.codex_apply_profile(&profile_a.id).unwrap();

    // A 使用期间 live 配置累计了新的模型键和 provider 字段
    write(
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"
model_catalog_json = "zai.json"

[mcp_servers.keep]
command = "node"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret"
new_field = "accumulated"
"#,
    );
    std::thread::sleep(std::time::Duration::from_millis(2));
    let profile_b = context.codex_capture_profile("B").unwrap();

    // 切到 B：autosync 应把 A 使用期间的累计改动写回 A 的快照
    context.codex_apply_profile(&profile_b.id).unwrap();

    let stored_a = context.database.codex_profile(&profile_a.id).unwrap();
    assert_eq!(
        stored_a
            .payload
            .model_values
            .get("model_catalog_json")
            .map(|raw| raw.trim().trim_matches('"')),
        Some("zai.json")
    );
    assert!(stored_a
        .payload
        .provider_body
        .as_deref()
        .unwrap()
        .contains("new_field = \"accumulated\""));
    assert_eq!(
        context
            .get_state()
            .unwrap()
            .active_codex_profile_id
            .as_deref(),
        Some(profile_b.id.as_str())
    );
}

#[test]
fn card_defaults_use_account_model_cache_and_keep_explicit_and_local_values() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.raw_auth = Some(chatgpt_auth("workspace", "fixture-token"));
    stored.payload.fetched_models = vec!["obsolete-profile-cache".into()];
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();
    let models = super::connections::parse_chatgpt_models(serde_json::json!({"models": [
        {"slug": "account-model", "display_name": "Fixture", "visibility": "list",
         "supported_reasoning_levels": [{"effort": "low"}], "default_reasoning_level": "low"}
    ]}))
    .unwrap();
    context
        .database
        .set_chatgpt_models("desktop:workspace", &models)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    context
        .codex_set_profile_model(&profile.id, None, Some("high"), None)
        .unwrap();
    let check = |model: &str, effort: &str| {
        let state = context.get_state().unwrap();
        let summary = state
            .codex_profiles
            .iter()
            .find(|item| item.id == profile.id)
            .unwrap();
        assert_eq!(summary.model.as_deref(), Some(model));
        assert_eq!(summary.reasoning_effort.as_deref(), Some(effort));
    };
    check("account-model", "high");
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(!codex_config::parse_document(&live)
        .unwrap()
        .contains_key("model"));
    context
        .codex_set_profile_model(&profile.id, None, Some(""), None)
        .unwrap();
    check("account-model", "low");
    context
        .codex_set_profile_model(&profile.id, Some("manual-model"), Some("high"), None)
        .unwrap();
    check("manual-model", "high");
    context.codex_update_profile_config(&profile.id, "model_catalog_json = \"models.json\"\n",
        Some(r#"{"models":[{"slug":"local-model","display_name":"Local","visibility":"list","supported_reasoning_levels":[{"effort":"max"}],"default_reasoning_level":"max"}]}"#), None).unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    check("local-model", "max");
}

#[test]
fn corrupt_model_cache_row_skips_backfill_without_breaking_state() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.raw_auth = Some(chatgpt_auth("workspace", "fixture-token"));
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();
    // 直接写入坏 JSON：坏缓存行只跳过回填，get_state 不失败。
    rusqlite::Connection::open(&context.paths.database)
        .unwrap()
        .execute(
            "INSERT INTO chatgpt_model_cache(cache_key, models_json, updated_at) \
             VALUES('desktop:workspace', '{bad', '0')",
            [],
        )
        .unwrap();
    let state = context.get_state().unwrap();
    let summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert_eq!(summary.model, None);
}

#[test]
fn desktop_profile_plan_badge_reads_own_database_snapshot() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.raw_auth = Some(chatgpt_auth_with_plan("desktop-ws", "token-1", "plus"));
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();

    // 回归：live auth.json 被切换覆写为 free 账号后，卡片套餐仍来自自身快照
    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        chatgpt_auth_with_plan("managed-ws", "oauth-live", "free"),
    )
    .unwrap();
    let state = context.get_state().unwrap();
    let summary = state
        .codex_profiles
        .iter()
        .find(|summary| summary.id == profile.id)
        .unwrap();
    assert_eq!(summary.plan_type.as_deref(), Some("plus"));
}

#[tokio::test]
async fn desktop_accounts_derive_from_database_snapshot_not_live_auth() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.raw_auth = Some(chatgpt_auth("desktop-ws", "token-1"));
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();

    // 身份来自数据库快照
    let accounts = context.desktop_auth_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "desktop-ws");

    // 回归：live auth.json 被切换覆写为其他账号时，Desktop 身份不受影响
    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        chatgpt_auth("managed-ws", "oauth-live"),
    )
    .unwrap();
    let accounts = context.desktop_auth_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "desktop-ws");

    // 快照匹配额度查询：按 workspace 命中自身，错过他账号
    let snapshot = context
        .desktop_auth_snapshot_for_account("desktop-ws")
        .unwrap();
    assert_eq!(
        snapshot.expect("desktop snapshot should resolve"),
        ("token-1".to_string(), None)
    );
    assert!(context
        .desktop_auth_snapshot_for_account("other-ws")
        .unwrap()
        .is_none());
    let oauth = crate::auth::codex_oauth::CodexOAuthManager::new(context.database.clone());
    let (token, workspace, _) = context
        .auth_account_credentials(AuthSource::Desktop, Some("desktop-ws"), &oauth)
        .await
        .unwrap();
    assert_eq!(
        (token.as_str(), workspace.as_deref()),
        ("token-1", Some("desktop-ws"))
    );
    assert!(context
        .auth_account_credentials(AuthSource::Desktop, Some("other-ws"), &oauth)
        .await
        .is_err());
    let (token, workspace, _) = context
        .auth_account_credentials(AuthSource::Desktop, Some("managed-ws"), &oauth)
        .await
        .unwrap();
    assert_eq!(
        (token.as_str(), workspace.as_deref()),
        ("oauth-live", Some("managed-ws"))
    );
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_home.join("auth.json")).unwrap(),
        chatgpt_auth("managed-ws", "oauth-live")
    );
}

#[test]
fn desktop_accounts_dedupe_same_login_across_profiles() {
    let (_home, context) = chatgpt_test_context();
    for updated_at in ["2", "3"] {
        let profile = context
            .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
            .unwrap();
        let mut stored = context.database.codex_profile(&profile.id).unwrap();
        stored.payload.raw_auth = Some(chatgpt_auth("desktop-ws", "token-1"));
        context
            .database
            .codex_update_profile(&profile.id, &stored.name, &stored.payload, updated_at)
            .unwrap();
    }
    // 同一登录存在于多个 Desktop 配置：账号页只出一张卡
    let accounts = context.desktop_auth_accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "desktop-ws");
}

#[test]
fn desktop_profile_syncs_and_restores_auth_snapshot() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .auth_source,
        Some(crate::models::AuthSource::Desktop)
    );

    // 创建只写数据库；激活空快照会清理当前 live 认证。
    let initial_live = chatgpt_auth("desktop-account", "initial-live");
    std::fs::write(context.paths.codex_home.join("auth.json"), &initial_live).unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    assert!(!context.paths.codex_home.join("auth.json").exists());

    // 官方认证完成后，聚焦/刷新把新文件回写当前 Desktop 配置。
    let refreshed_live = chatgpt_auth("desktop-account", "refreshed-live");
    std::fs::write(context.paths.codex_home.join("auth.json"), &refreshed_live).unwrap();
    context.get_state().unwrap();
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .raw_auth,
        Some(refreshed_live.clone())
    );
    assert_eq!(
        context
            .codex_get_profile(&profile.id)
            .unwrap()
            .desktop_login
            .as_deref(),
        Some("desktop-account")
    );

    // 再次激活恢复数据库快照。
    std::fs::remove_file(context.paths.codex_home.join("auth.json")).unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_home.join("auth.json")).unwrap(),
        refreshed_live
    );
}

#[test]
fn desktop_profile_restores_saved_auth_snapshot_even_when_manual() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.raw_auth = Some(chatgpt_auth("desktop-account", "stale-snapshot"));
    stored.payload.auth_auto_sync = Some(false);
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();
    let live_auth = chatgpt_auth("desktop-account", "desktop-live");
    std::fs::write(context.paths.codex_home.join("auth.json"), &live_auth).unwrap();

    context.codex_apply_profile(&profile.id).unwrap();

    assert_eq!(
        std::fs::read_to_string(context.paths.codex_home.join("auth.json")).unwrap(),
        chatgpt_auth("desktop-account", "stale-snapshot")
    );
}

#[test]
fn apply_bound_oauth_profile_leaves_auth_for_oauth_writer() {
    let (_home, context) = chatgpt_test_context();
    context
        .database
        .upsert_account(&crate::database::StoredAccount {
            id: "oauth-account".into(),
            email: None,
            id_token: None,
            refresh_token: "refresh-token".into(),
            auth_json: None,
            chatgpt_account_id: Some("oauth-account".into()),
            user_identity: None,
            plan_type: None,
            authenticated_at: 1,
        })
        .unwrap();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, Some("oauth-account"))
        .unwrap();
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .auth_source,
        Some(crate::models::AuthSource::Oauth)
    );
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.raw_auth = Some(chatgpt_auth("oauth-account", "stale-snapshot"));
    stored.payload.auth_auto_sync = Some(false);
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();
    let live_auth = chatgpt_auth("desktop-account", "desktop-live");
    std::fs::write(context.paths.codex_home.join("auth.json"), &live_auth).unwrap();

    context.codex_apply_profile(&profile.id).unwrap();

    assert_eq!(
        std::fs::read_to_string(context.paths.codex_home.join("auth.json")).unwrap(),
        live_auth
    );
}

#[test]
fn auth_source_is_fixed_and_oauth_accounts_can_switch() {
    let (_home, context) = chatgpt_test_context();
    for id in ["oauth-one", "oauth-two"] {
        context
            .database
            .upsert_account(&crate::database::StoredAccount {
                id: id.to_string(),
                email: None,
                id_token: None,
                refresh_token: format!("refresh-{id}"),
                auth_json: None,
                authenticated_at: 1,
                chatgpt_account_id: Some(id.to_string()),
                user_identity: None,
                plan_type: None,
            })
            .unwrap();
    }
    let desktop = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    assert!(context
        .codex_set_profile_account(&desktop.id, Some("oauth-one"))
        .is_err());
    assert_eq!(context.bound_account_id(&desktop.id).unwrap(), None);

    let oauth = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, Some("oauth-one"))
        .unwrap();
    context
        .codex_set_profile_fetched_models(
            &oauth.id,
            vec!["fixture-model".into()],
            Some(BTreeMap::from([(
                "fixture-model".into(),
                vec!["high".into()],
            )])),
            Some(BTreeMap::from([("fixture-model".into(), "high".into())])),
        )
        .unwrap();
    context
        .codex_set_profile_account(&oauth.id, Some("oauth-one"))
        .unwrap();
    assert_eq!(
        context
            .database
            .codex_profile(&oauth.id)
            .unwrap()
            .payload
            .fetched_models,
        ["fixture-model"]
    );
    context
        .codex_set_profile_account(&oauth.id, Some("oauth-two"))
        .unwrap();
    let switched = context.database.codex_profile(&oauth.id).unwrap();
    assert!(switched.payload.fetched_models.is_empty());
    assert!(switched.payload.fetched_model_efforts.is_empty());
    assert!(switched.payload.fetched_model_defaults.is_empty());
    assert_eq!(
        context.bound_account_id(&oauth.id).unwrap().as_deref(),
        Some("oauth-two")
    );
    assert!(context.codex_set_profile_account(&oauth.id, None).is_err());
    assert_eq!(
        context.profile_auth_source(&oauth.id).unwrap(),
        Some(crate::models::AuthSource::Oauth)
    );
}

#[test]
fn get_state_syncs_desktop_auth_into_profile_snapshot() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"account-1"}}"#,
    )
    .unwrap();
    context.get_state().unwrap();

    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(
        stored.payload.raw_auth,
        Some(r#"{"auth_mode":"chatgpt","tokens":{"access_token":"account-1"}}"#.into())
    );
}

#[test]
fn focus_refresh_keeps_last_desktop_snapshot_when_live_auth_is_invalid() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    let valid_auth = chatgpt_auth("desktop-account", "valid-access");
    std::fs::write(context.paths.codex_home.join("auth.json"), &valid_auth).unwrap();
    context.get_state().unwrap();

    std::fs::write(context.paths.codex_home.join("auth.json"), "{invalid-json").unwrap();
    context.get_state().unwrap();

    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .raw_auth,
        Some(valid_auth)
    );
}

#[tokio::test]
async fn oauth_activation_and_account_switch_write_only_the_bound_account_snapshot() {
    let (_home, context) = chatgpt_test_context();
    let initial_oauth_auth = oauth_auth(
        "oauth-account",
        "cached-access",
        "refresh-token",
        "id-token",
    );
    let initial_second_oauth_auth = oauth_auth(
        "oauth-second",
        "cached-second-access",
        "second-refresh-token",
        "second-id-token",
    );
    context
        .database
        .upsert_account(&crate::database::StoredAccount {
            id: "oauth-account".into(),
            email: None,
            id_token: Some("id-token".into()),
            refresh_token: "refresh-token".into(),
            auth_json: Some(initial_oauth_auth.clone()),
            chatgpt_account_id: Some("oauth-account".into()),
            user_identity: None,
            plan_type: None,
            authenticated_at: 1,
        })
        .unwrap();
    context
        .database
        .upsert_account(&crate::database::StoredAccount {
            id: "oauth-second".into(),
            email: None,
            id_token: Some("second-id-token".into()),
            refresh_token: "second-refresh-token".into(),
            auth_json: Some(initial_second_oauth_auth.clone()),
            chatgpt_account_id: Some("oauth-second".into()),
            user_identity: None,
            plan_type: None,
            authenticated_at: 2,
        })
        .unwrap();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, Some("oauth-account"))
        .unwrap();
    let oauth = crate::auth::codex_oauth::CodexOAuthManager::new(context.database.clone());
    oauth
        .seed_access_token_for_test("oauth-account", "fresh-oauth-access")
        .await;

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let auth_path = context.paths.codex_home.join("auth.json");
        let original_auth = chatgpt_auth("external-account", "fixture-old");
        std::fs::write(&auth_path, &original_auth).unwrap();
        let original_config = std::fs::read(context.paths.codex_config()).unwrap();
        let auth_lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&auth_path)
            .unwrap();
        let error = context
            .codex_apply_profile_with_auth(&profile.id, &oauth)
            .await
            .unwrap_err();
        assert!(error.0.contains("无法替换"), "{error}");
        drop(auth_lock);
        assert_eq!(
            std::fs::read(context.paths.codex_config()).unwrap(),
            original_config
        );
        assert_eq!(std::fs::read_to_string(&auth_path).unwrap(), original_auth);
        assert_eq!(context.get_state().unwrap().active_codex_profile_id, None);
    }

    context
        .codex_apply_profile_with_auth(&profile.id, &oauth)
        .await
        .unwrap();

    let live: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(context.paths.codex_home.join("auth.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(live["tokens"]["access_token"], "fresh-oauth-access");
    assert_eq!(live["tokens"]["refresh_token"], "refresh-token");
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .raw_auth,
        None
    );

    // 外部刷新的 live access_token 需为带 iat 的 JWT 且新于 authenticated_at(1)，同步才吸收
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let refreshed_access = {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(br#"{"iat":100}"#);
        format!("{header}.{payload}.sig")
    };
    let externally_refreshed = oauth_auth(
        "oauth-account",
        &refreshed_access,
        "rotated-refresh-token",
        "rotated-id-token",
    );
    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        &externally_refreshed,
    )
    .unwrap();
    oauth
        .seed_access_token_for_test("oauth-second", "fresh-second-access")
        .await;

    context
        .codex_set_profile_account_and_apply_active(&profile.id, Some("oauth-second"), &oauth)
        .await
        .unwrap();
    assert_eq!(
        context.bound_account_id(&profile.id).unwrap().as_deref(),
        Some("oauth-second")
    );
    let live: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(context.paths.codex_home.join("auth.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(live["tokens"]["access_token"], "fresh-second-access");
    assert_eq!(live["tokens"]["refresh_token"], "second-refresh-token");
    let account = context
        .database
        .accounts()
        .unwrap()
        .into_iter()
        .find(|account| account.id == "oauth-account")
        .unwrap();
    assert_eq!(account.refresh_token, "rotated-refresh-token");
    assert_eq!(
        account.auth_json.as_deref(),
        Some(externally_refreshed.as_str())
    );
}

#[test]
fn focus_refresh_keeps_oauth_auth_out_of_profile_snapshot() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let (_home, context) = chatgpt_test_context();
    context
        .database
        .upsert_account(&crate::database::StoredAccount {
            id: "account-1".into(),
            email: Some("one@example.com".into()),
            id_token: Some("id-token".into()),
            refresh_token: "refresh-token".into(),
            auth_json: None,
            chatgpt_account_id: Some("account-1".into()),
            user_identity: None,
            plan_type: None,
            authenticated_at: 0,
        })
        .unwrap();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, Some("account-1"))
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    let id_token = |account_id: &str| {
        let payload = URL_SAFE_NO_PAD
            .encode(format!(r#"{{"chatgpt_account_id":"{account_id}"}}"#).as_bytes());
        format!("e30.{payload}.sig")
    };
    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{}","access_token":"account-1"}}}}"#,
            id_token("account-1")
        ),
    )
    .unwrap();
    context.get_state().unwrap();

    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{}","access_token":"account-2"}}}}"#,
            id_token("account-2")
        ),
    )
    .unwrap();
    context.get_state().unwrap();

    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(stored.payload.raw_auth, None);
}

#[test]
fn desktop_auth_clear_repopulates_after_focus_refresh() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"manual-source"}}"#,
    )
    .unwrap();
    context.get_state().unwrap();

    let config = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    context
        .codex_update_profile_config(&profile.id, &config, None, Some(""))
        .unwrap();
    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"should-repopulate"}}"#,
    )
    .unwrap();
    context.get_state().unwrap();

    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(stored.payload.auth_auto_sync, Some(true));
    assert_eq!(
        stored.payload.raw_auth,
        Some(r#"{"auth_mode":"chatgpt","tokens":{"access_token":"should-repopulate"}}"#.into())
    );
}

#[test]
fn legacy_empty_desktop_auth_snapshot_repopulates_after_focus_refresh() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    let mut stored = context.database.codex_profile(&profile.id).unwrap();
    stored.payload.auth_auto_sync = Some(false);
    stored.payload.raw_auth = None;
    context
        .database
        .codex_update_profile(&profile.id, &stored.name, &stored.payload, "2")
        .unwrap();

    std::fs::write(
        context.paths.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"legacy-repopulate"}}"#,
    )
    .unwrap();
    context.get_state().unwrap();

    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(stored.payload.auth_auto_sync, Some(true));
    assert_eq!(
        stored.payload.raw_auth,
        Some(r#"{"auth_mode":"chatgpt","tokens":{"access_token":"legacy-repopulate"}}"#.into())
    );
}

#[test]
fn capture_keeps_active_and_autosyncs_previous() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let context = AppContext::new(paths).unwrap();
    let write = |text: &str| std::fs::write(context.paths.codex_config(), text).unwrap();

    write(
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret"
"#,
    );
    let profile_a = context.codex_capture_profile("A").unwrap();
    assert_eq!(
        context
            .get_state()
            .unwrap()
            .active_codex_profile_id
            .as_deref(),
        None
    );
    context.codex_apply_profile(&profile_a.id).unwrap();

    // A 使用期间 live 累计了新键，再次捕获 B：A 快照被同步，激活仍保持在 A
    write(
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"
model_catalog_json = "zai.json"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret"
new_field = "accumulated"
"#,
    );
    let profile_b = context.codex_capture_profile("B").unwrap();

    let state = context.get_state().unwrap();
    assert_eq!(
        state.active_codex_profile_id.as_deref(),
        Some(profile_a.id.as_str())
    );
    assert_ne!(
        state.active_codex_profile_id.as_deref(),
        Some(profile_b.id.as_str())
    );
    let stored_a = context.database.codex_profile(&profile_a.id).unwrap();
    assert!(stored_a
        .payload
        .provider_body
        .as_deref()
        .unwrap()
        .contains("new_field = \"accumulated\""));
}

#[test]
fn get_profile_returns_raw_file_contents() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_catalog_json = "zai.json"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret-token"
"#,
    )
    .unwrap();
    std::fs::write(
        paths.codex_home.join("zai.json"),
        r#"{"models":[{"id":"glm-5.3","api_key":"sk-secret"}]}"#,
    )
    .unwrap();
    std::fs::write(
        paths.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"raw-token"}}"#,
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("ZAI").unwrap();

    // 捕获默认不设为使用中；验证“未使用”只读库快照
    let inactive = context.codex_get_profile(&profile.id).unwrap();
    assert_eq!(inactive.catalog_content, None);

    // 使用中：live 文件是唯一事实源
    context.codex_apply_profile(&profile.id).unwrap();
    let detail = context.codex_get_profile(&profile.id).unwrap();

    assert!(detail.config_fragment.contains("experimental_bearer_token"));
    assert!(detail.config_fragment.contains("secret-token"));
    assert!(!detail.config_fragment.contains("••••••••"));
    assert_eq!(detail.api_key.as_deref(), Some("secret-token"));
    assert_eq!(
        detail.catalog_content.as_deref(),
        Some(r#"{"models":[{"id":"glm-5.3","api_key":"sk-secret"}]}"#)
    );
}

#[test]
fn update_profile_config_clears_active_live_auth_when_editor_is_empty() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"gpt-5.6\"\n").unwrap();
    std::fs::write(
        paths.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"access_token":"live"}}"#,
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    let detail = context.codex_get_profile(&profile.id).unwrap();

    let updated = context
        .codex_update_profile_config(&profile.id, &detail.config_fragment, None, Some(""))
        .unwrap();

    assert_eq!(updated.raw_auth, None);
    assert!(!context.paths.codex_home.join("auth.json").exists());
}

#[test]
fn update_profile_writes_back_to_active_live_config() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://old.example"
experimental_bearer_token = "old-key"
"#,
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("ZAI").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    context
        .codex_update_profile(
            &profile.id,
            "ZAI",
            None,
            Some("https://new.example"),
            Some("new-key"),
            None,
        )
        .unwrap();

    let text = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(text.contains(r#"base_url = "https://new.example""#));
    assert!(text.contains(r#"experimental_bearer_token = "new-key""#));
    assert!(!text.contains("old-key"));

    let detail = context.codex_get_profile(&profile.id).unwrap();
    assert_eq!(detail.base_url.as_deref(), Some("https://new.example"));
    assert_eq!(detail.api_key.as_deref(), Some("new-key"));
}

#[test]
fn sync_active_profile_from_live_persists_external_mcp_changes() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        "model = \"gpt-5.6\"\n\n[mcp_servers.computer-use]\ncommand = \"SkyComputerUseClient\"\nenabled = true\n",
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("电脑自动化").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    std::fs::write(
        context.paths.codex_config(),
        "model = \"gpt-5.6\"\n\n[mcp_servers.computer-use]\ncommand = \"SkyComputerUseClient\"\nenabled = false\n",
    )
    .unwrap();

    if let Some(document) = context.live_document() {
        context.sync_active_profile_document(&document).unwrap();
    }
    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert!(stored
        .payload
        .raw_config
        .as_deref()
        .is_some_and(|text| !text.contains("[mcp_servers.computer-use]")));
}

#[test]
fn corrupted_live_config_reports_parse_error_and_keeps_last_valid_snapshot() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"gpt-5.6\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("A").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    // 正常外部漂移：应当收敛
    std::fs::write(context.paths.codex_config(), "model = \"gpt-5.7\"\n").unwrap();
    let document = context.live_document().unwrap();
    let outcome = context.sync_active_profile_document(&document).unwrap();
    assert_eq!(outcome.kind, crate::services::sync::SyncKind::Wrote);
    // 结局必须带出配置名，否则日志里只剩"没回写"，看不出是谁
    assert_eq!(outcome.profile.as_deref(), Some("A"));
    let last_valid = context
        .database
        .codex_profile(&profile.id)
        .unwrap()
        .payload
        .raw_config
        .clone();

    // 外部把 live 改坏：残缺的 TOML，严格读必须拒绝它
    std::fs::write(context.paths.codex_config(), "model = \n").unwrap();
    assert!(matches!(
        context.live_document_checked(),
        Err(super::codex_profiles::LiveReadError::Parse)
    ));

    // 走真实被动收割（get_state 同一条路径）：损坏内容不得覆盖最后一次有效快照
    crate::services::sync::registry().harvest_passive(
        &context,
        &crate::services::sync::SyncTrigger::StateRefresh,
        crate::services::sync::SyncMaterial::default(),
    );
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .raw_config,
        last_valid
    );

    // 读不了（权限/占用）必须与解析失败分开：目录占位稳定复现非 NotFound 的 io 失败，
    // 同一条被动路径同样不得覆盖最后一次有效快照
    std::fs::remove_file(context.paths.codex_config()).unwrap();
    std::fs::create_dir(context.paths.codex_config()).unwrap();
    assert!(matches!(
        context.live_document_checked(),
        Err(super::codex_profiles::LiveReadError::Unreadable)
    ));
    crate::services::sync::registry().harvest_passive(
        &context,
        &crate::services::sync::SyncTrigger::StateRefresh,
        crate::services::sync::SyncMaterial::default(),
    );
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .raw_config,
        last_valid
    );

    // 修好后同一路径自愈
    std::fs::remove_dir(context.paths.codex_config()).unwrap();
    std::fs::write(context.paths.codex_config(), "model = \"gpt-5.8\"\n").unwrap();
    crate::services::sync::registry().harvest_passive(
        &context,
        &crate::services::sync::SyncTrigger::StateRefresh,
        crate::services::sync::SyncMaterial::default(),
    );
    assert!(context
        .database
        .codex_profile(&profile.id)
        .unwrap()
        .payload
        .raw_config
        .as_deref()
        .is_some_and(|text| text.contains("gpt-5.8")));
}

#[test]
fn only_exposed_paths_can_be_opened() {
    let home = tempfile::tempdir().unwrap();
    let context = AppContext::new(crate::paths::from_home(home.path()).unwrap()).unwrap();

    // 设置页只暴露四处：应用数据目录 / 数据库备份目录 / 日志目录 / Codex 配置（见 path_info）
    assert!(context.is_managed_path(&context.paths.root.display().to_string()));
    assert!(context.is_managed_path(&context.paths.codex_config().display().to_string()));
    assert!(context.is_managed_path(&context.paths.database_backup.display().to_string()));
    assert!(!context.is_managed_path(&context.paths.root.join("backups").display().to_string()));
    // 未单独暴露的具体文件路径不在白名单内，open_path 应拒绝
    assert!(!context.is_managed_path(&context.paths.database.display().to_string()));
    assert!(!context.is_managed_path("C:\\unmanaged-path"));
}

#[test]
fn update_profile_allows_duplicate_name() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    context.codex_capture_profile("First").unwrap();
    // 供应商 id 取毫秒时间戳，同毫秒内二次捕获会撞 id；真实 UI 不可能，测试里隔开
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = context.codex_capture_profile("Second").unwrap();

    // 名字不是唯一键，重命名为已存在的名字应允许，靠 ID 区分
    let updated = context
        .codex_update_profile(&second.id, "first", None, None, None, None)
        .unwrap();
    assert_eq!(updated.name, "first");
}

#[test]
fn icon_ids_are_validated() {
    assert_eq!(validated_icon(None).unwrap(), None);
    assert_eq!(validated_icon(Some("  ")).unwrap(), None);
    assert_eq!(
        validated_icon(Some(" zhipu ")).unwrap().as_deref(),
        Some("zhipu")
    );
    assert!(validated_icon(Some("Zhipu")).is_err());
    assert!(validated_icon(Some("a!b")).is_err());
    assert!(validated_icon(Some(&"x".repeat(41))).is_err());
}

#[test]
fn add_builtin_profile_creates_snapshot_without_activating() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let original = "model = \"glm-5.3\"\n";
    std::fs::write(paths.codex_config(), original).unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile(
            "deepseek",
            None,
            Some("https://custom.example"),
            Some("sk-test"),
            None,
            None,
        )
        .unwrap();

    assert_eq!(profile.name, "DeepSeek");
    assert_eq!(profile.model, None);
    assert_eq!(profile.provider.as_deref(), Some("deepseek"));
    assert_eq!(profile.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(profile.icon.as_deref(), Some("deepseek"));
    assert!(profile.has_key);

    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(stored.payload.builtin.as_deref(), Some("deepseek"));
    assert_eq!(
        stored
            .payload
            .model_values
            .get("model_catalog_json")
            .map(|raw| raw.trim().trim_matches('"')),
        Some("~/.codex/models.json")
    );
    assert!(stored
        .payload
        .provider_body
        .as_deref()
        .unwrap()
        .contains("sk-test"));
    assert!(stored
        .payload
        .provider_body
        .as_deref()
        .unwrap()
        .contains("https://custom.example"));

    // 添加只存快照，不写生产配置
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_config()).unwrap(),
        original
    );
    let state = context.get_state().unwrap();
    assert_eq!(state.active_codex_profile_id, None);
    assert_eq!(state.codex_profiles.len(), 1);

    // 同名模板允许重复添加，名字相同，靠 ID 区分
    let duplicate = context
        .codex_add_builtin_profile("deepseek", None, None, Some("sk-test"), None, None)
        .unwrap();
    assert_eq!(duplicate.name, "DeepSeek");
    assert_ne!(duplicate.id, profile.id);
}

#[test]
fn get_builtin_catalog_returns_embedded_file_content() {
    let home = tempfile::tempdir().unwrap();
    let context = AppContext::new(crate::paths::from_home(home.path()).unwrap()).unwrap();

    assert_eq!(
        context.codex_get_builtin_catalog("deepseek").unwrap(),
        Some(String::from_utf8_lossy(crate::codex_builtin::DEEPSEEK_MODELS).into_owned())
    );
    assert_eq!(
        context.codex_get_builtin_catalog("zhipu").unwrap(),
        Some(String::from_utf8_lossy(crate::codex_builtin::ZHIPU_MODELS).into_owned())
    );
    assert_eq!(
        context.codex_get_builtin_catalog("minimax").unwrap(),
        Some(String::from_utf8_lossy(crate::codex_builtin::MINIMAX_CATALOG).into_owned())
    );
    assert_eq!(context.codex_get_builtin_catalog("chatgpt").unwrap(), None);
    assert!(context.codex_get_builtin_catalog("unknown").is_err());
}

#[tokio::test]
async fn balance_rejects_unsupported_or_keyless() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
    let context = AppContext::new(paths).unwrap();
    let oauth = crate::auth::codex_oauth::CodexOAuthManager::new(context.database.clone());

    // 不支持余额/用量查询的供应商拒绝
    let unsupported = context
        .codex_add_builtin_profile("opencode", None, None, Some("opencode-key"), None, None)
        .unwrap();
    let error = context
        .codex_get_profile_balance(&unsupported.id, &oauth)
        .await
        .unwrap_err();
    assert!(error.0.contains("该供应商不支持余额/用量查询"));

    // MiniMax 但只有占位符密钥（未配置真实密钥）拒绝
    let keyless = context
        .codex_add_builtin_profile("minimax", None, None, None, None, None)
        .unwrap();
    let error = context
        .codex_get_profile_balance(&keyless.id, &oauth)
        .await
        .unwrap_err();
    assert!(error.0.contains("没有配置 API Key"));
}

#[tokio::test]
async fn chatgpt_models_require_the_profiles_own_credentials() {
    let (_home, context) = chatgpt_test_context();
    let oauth = crate::auth::codex_oauth::CodexOAuthManager::new(context.database.clone());
    let official = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let other = context
        .codex_add_builtin_profile("minimax", None, None, None, None, None)
        .unwrap();
    let live_auth = chatgpt_auth("unrelated-account", "unrelated-token");
    let auth_path = context.paths.codex_home.join("auth.json");
    std::fs::write(&auth_path, &live_auth).unwrap();
    let error = context
        .codex_fetch_chatgpt_models(Some(&official.id), AuthSource::Desktop, None, &oauth, true)
        .await
        .unwrap_err();
    assert!(error.0.contains("该 Codex 配置尚未保存有效登录"));
    let error = context
        .codex_fetch_chatgpt_models(Some(&other.id), AuthSource::Desktop, None, &oauth, true)
        .await
        .unwrap_err();
    assert!(error.0.contains("该配置不是 ChatGPT 订阅配置"));
    assert_eq!(std::fs::read_to_string(&auth_path).unwrap(), live_auth);
}

#[test]
fn minimax_remains_converts_remaining_to_used_percent() {
    // statusline.ps1 实测形态：general 条目，remaining_percent 是“剩余”
    let entry = connections::MiniMaxModelRemains {
        model_name: "general".into(),
        current_interval_remaining_percent: Some(85.0),
        current_weekly_remaining_percent: Some(96.0),
        remains_time: Some(8_580_000),          // 2h23m
        weekly_remains_time: Some(507_600_000), // 5d21h
    };
    assert_eq!(
        connections::used_percent(entry.current_interval_remaining_percent),
        Some(15)
    );
    assert_eq!(
        connections::used_percent(entry.current_weekly_remaining_percent),
        Some(4)
    );
    assert_eq!(
        entry
            .remains_time
            .and_then(|ms| connections::format_reset(ms, false))
            .as_deref(),
        Some("2h23m")
    );
    assert_eq!(
        entry
            .weekly_remains_time
            .and_then(|ms| connections::format_reset(ms, true))
            .as_deref(),
        Some("5d21h")
    );

    // 剩余 100% → 用量 0
    let entry = connections::MiniMaxModelRemains {
        model_name: "general".into(),
        current_interval_remaining_percent: Some(100.0),
        current_weekly_remaining_percent: Some(100.0),
        remains_time: Some(60_000),
        weekly_remains_time: None,
    };
    assert_eq!(
        connections::used_percent(entry.current_interval_remaining_percent),
        Some(0)
    );
    assert_eq!(
        entry
            .remains_time
            .and_then(|ms| connections::format_reset(ms, false)),
        None
    );

    // 无百分比数据时返回 None（卡片显示“查询失败”而不是假数字）
    let empty = connections::MiniMaxModelRemains {
        model_name: "general".into(),
        current_interval_remaining_percent: None,
        current_weekly_remaining_percent: None,
        remains_time: None,
        weekly_remains_time: None,
    };
    assert_eq!(
        connections::used_percent(empty.current_interval_remaining_percent),
        None
    );
}

#[test]
fn format_reset_rounds_up_so_unused_windows_show_full_window() {
    // 未使用的 ChatGPT 5h 窗口重置点会随查询滑动（窗口未开始），
    // 截断会把“剩 4h59m59s”显示成 4h59m，看起来像窗口已走掉 1 分钟
    assert_eq!(
        connections::format_reset(17_999_000, false),
        Some("5h".into())
    );
    // 恰好整分钟不进位
    assert_eq!(
        connections::format_reset(8_580_000, false),
        Some("2h23m".into())
    );
    // 周窗口同理：5d20h59m59s 向上进到 5d21h
    assert_eq!(
        connections::format_reset(507_540_001, true),
        Some("5d21h".into())
    );
    // 不足 1 分钟不显示（既有契约）
    assert_eq!(connections::format_reset(60_000, false), None);
}

#[test]
fn zhipu_quota_maps_rolling_windows_to_usage() {
    let now_ms = 1_800_000_000_000_i64;
    let value = serde_json::json!({
        "data": {
            "limits": [
                {
                    "type": "TOKENS_LIMIT",
                    "percentage": 32.4,
                    "nextResetTime": now_ms + 8_580_000,
                    "unit": 3
                },
                {
                    "type": "TOKENS_LIMIT",
                    "percentage": 44.6,
                    "nextResetTime": now_ms + 507_600_000,
                    "unit": 6
                }
            ]
        }
    });
    let info = connections::zhipu_quota_info(&value, now_ms).unwrap();

    assert_eq!(info.usage_percent, Some(32));
    assert_eq!(info.usage_label.as_deref(), Some("5小时"));
    assert_eq!(info.usage_reset.as_deref(), Some("2h23m"));
    assert_eq!(info.weekly_usage_percent, Some(45));
    assert_eq!(info.weekly_label.as_deref(), Some("7天"));
    assert_eq!(info.weekly_reset.as_deref(), Some("5d21h"));
}

#[test]
fn chatgpt_quota_maps_windows_to_remaining_display_data() {
    let reset_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 3_600;
    let info = connections::chatgpt_quota_info(connections::ChatgptUsageResponse {
        rate_limit_reset_credits: Some(connections::ChatgptResetCreditsSummary {
            available_count: Some(2),
        }),
        rate_limit: Some(connections::ChatgptRateLimit {
            primary_window: Some(connections::ChatgptRateLimitWindow {
                used_percent: Some(18.0),
                limit_window_seconds: Some(18_000),
                reset_at: Some(reset_at),
            }),
            secondary_window: Some(connections::ChatgptRateLimitWindow {
                used_percent: Some(42.0),
                limit_window_seconds: Some(2_592_000),
                reset_at: Some(reset_at + 86_400),
            }),
        }),
    })
    .unwrap();

    assert_eq!(info.usage_percent, Some(18));
    assert_eq!(info.usage_label.as_deref(), Some("5小时"));
    assert!(info.usage_reset.is_some());
    assert_eq!(info.usage_reset_at, Some(reset_at * 1_000));
    assert_eq!(info.weekly_usage_percent, Some(42));
    assert_eq!(info.weekly_label.as_deref(), Some("30天"));
    assert!(info.weekly_reset.is_some());
    assert_eq!(info.weekly_reset_at, Some((reset_at + 86_400) * 1_000));
    assert_eq!(info.reset_credits_available, Some(2));
}

#[test]
fn chatgpt_reset_credit_expiry_parses_official_rfc3339_timestamp() {
    assert_eq!(
        connections::chatgpt_reset_credit_expiry(Some("2026-10-04T05:12:00Z")),
        Some(1_791_090_720_000)
    );
    assert_eq!(connections::chatgpt_reset_credit_expiry(None), None);
}

#[test]
fn chatgpt_quota_uses_a_seven_day_primary_window_without_faking_five_hours() {
    let reset_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        + 86_400;
    let info = connections::chatgpt_quota_info(connections::ChatgptUsageResponse {
        rate_limit_reset_credits: None,
        rate_limit: Some(connections::ChatgptRateLimit {
            primary_window: Some(connections::ChatgptRateLimitWindow {
                used_percent: Some(62.0),
                limit_window_seconds: Some(604_800),
                reset_at: Some(reset_at),
            }),
            secondary_window: None,
        }),
    })
    .unwrap();

    assert_eq!(info.usage_percent, Some(62));
    assert_eq!(info.usage_label.as_deref(), Some("7天"));
    assert!(info.usage_reset.is_some());
    assert_eq!(info.usage_reset_at, Some(reset_at * 1_000));
    assert_eq!(info.weekly_usage_percent, None);
    assert_eq!(info.weekly_label, None);
}

#[test]
fn chatgpt_quota_skips_empty_primary_window() {
    let info = connections::chatgpt_quota_info(connections::ChatgptUsageResponse {
        rate_limit_reset_credits: None,
        rate_limit: Some(connections::ChatgptRateLimit {
            primary_window: Some(connections::ChatgptRateLimitWindow {
                used_percent: None,
                limit_window_seconds: Some(18_000),
                reset_at: None,
            }),
            secondary_window: Some(connections::ChatgptRateLimitWindow {
                used_percent: Some(62.0),
                limit_window_seconds: Some(604_800),
                reset_at: None,
            }),
        }),
    })
    .unwrap();

    assert_eq!(info.usage_percent, Some(62));
    assert_eq!(info.usage_label.as_deref(), Some("7天"));
    assert_eq!(info.weekly_usage_percent, None);
}

#[test]
fn get_state_tags_active_profile_from_live_config() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\nmodel_reasoning_effort = \"high\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.example\"\nexperimental_bearer_token = \"secret\"\n",
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("ZAI High").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    // 数据库快照滞后：DB 里推理强度是 old，live 配置手动改成 medium 并累计新键
    let mut payload = context.database.codex_profile(&profile.id).unwrap().payload;
    payload
        .model_values
        .insert("model_reasoning_effort".into(), "\"old\"".into());
    context
        .database
        .codex_update_profile(&profile.id, "ZAI High", &payload, &now_ms().to_string())
        .unwrap();
    std::fs::write(
        context.paths.codex_config(),
        "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\nmodel_reasoning_effort = \"medium\"\nmodel_catalog_json = \"zai.json\"\n\n[mcp_servers.keep]\ncommand = \"node\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.example\"\nexperimental_bearer_token = \"secret\"\n",
    )
    .unwrap();

    let state = context.get_state().unwrap();
    assert_eq!(
        state.active_codex_profile_id.as_deref(),
        Some(profile.id.as_str())
    );
    let summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert_eq!(summary.model.as_deref(), Some("glm-5.3"));
    assert_eq!(summary.provider.as_deref(), Some("ZAI"));
    assert_eq!(summary.reasoning_effort.as_deref(), Some("medium"));
    // get_state 按需同步：外部改动已回写进数据库快照
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .model_values
            .get("model_reasoning_effort")
            .map(|raw| raw.trim().trim_matches('"')),
        Some("medium")
    );
}

#[test]
fn get_state_and_profile_sync_active_snapshot_from_live() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let context = AppContext::new(paths).unwrap();
    let write = |text: &str| std::fs::write(context.paths.codex_config(), text).unwrap();

    write(
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
experimental_bearer_token = "secret"
"#,
    );
    let profile = context.codex_capture_profile("ZAI").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    // 外部把 live 换成另一套配置
    write(
        r#"
model = "glm-5.3-pro"
model_provider = "ZAI"
model_reasoning_effort = "max"
model_catalog_json = "zai.json"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://new.example"
experimental_bearer_token = "secret"
"#,
    );

    // 打开编辑页（codex_get_profile）即触发同步：DB 快照跟随 live
    let detail = context.codex_get_profile(&profile.id).unwrap();
    assert_eq!(
        detail
            .model_values
            .get("model")
            .map(|value| value.trim().trim_matches('"')),
        Some("glm-5.3-pro")
    );
    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(
        stored
            .payload
            .model_values
            .get("model")
            .map(|value| value.trim().trim_matches('"')),
        Some("glm-5.3-pro")
    );
    assert!(stored
        .payload
        .raw_config
        .as_deref()
        .unwrap()
        .contains("model = \"glm-5.3-pro\""));

    // 再外部改一次，get_state（刷新按钮/窗口激活）也会同步
    write(
        r#"
model = "glm-5.4"
model_provider = "ZAI"
model_reasoning_effort = "low"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://new.example"
experimental_bearer_token = "secret"
"#,
    );
    let state = context.get_state().unwrap();
    let summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert_eq!(summary.model.as_deref(), Some("glm-5.4"));
    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert_eq!(
        stored
            .payload
            .model_values
            .get("model")
            .map(|value| value.trim().trim_matches('"')),
        Some("glm-5.4")
    );
}

#[test]
fn show_balance_toggle_survives_live_sync() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.example\"\nexperimental_bearer_token = \"secret\"\n",
    )
    .unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("ZAI").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    assert!(!profile.show_balance); // 默认关闭

    context
        .codex_set_profile_show_balance(&profile.id, false)
        .unwrap();

    // 外部改 live 后触发同步，供应商级开关不能被重置回默认值
    std::fs::write(
        context.paths.codex_config(),
        "model = \"glm-5.4\"\nmodel_provider = \"ZAI\"\nmodel_reasoning_effort = \"low\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://new.example\"\nexperimental_bearer_token = \"secret\"\n",
    )
    .unwrap();
    let state = context.get_state().unwrap();
    let summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert!(!summary.show_balance);
    let stored = context.database.codex_profile(&profile.id).unwrap();
    assert!(!stored.payload.show_balance);
}

#[test]
fn deleting_active_profile_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
    let context = AppContext::new(paths.clone()).unwrap();
    let profile = context.codex_capture_profile("A").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    // 使用中的配置不可删除（前端删除按钮对激活卡片禁用）：拒绝后配置仍在库中
    assert!(context.codex_delete_profile(&profile.id).is_err());
    assert!(context.database.codex_profile(&profile.id).is_ok());
}

#[test]
fn export_and_restore_database_round_trip() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();

    let context = AppContext::new(paths.clone()).unwrap();
    let profile = context.codex_capture_profile("A").unwrap();

    let exported = context.export_database().unwrap();
    assert!(exported.exists());
    let name = exported.file_name().unwrap().to_string_lossy().into_owned();
    assert!(context
        .list_database_backups()
        .unwrap()
        .iter()
        .any(|backup| backup.name == name));

    // 把当前库改乱，再从备份恢复
    context.database.codex_delete_profile(&profile.id).unwrap();
    assert!(context.database.codex_profiles().unwrap().is_empty());
    context.restore_database(&name).unwrap();
    assert_eq!(context.database.codex_profiles().unwrap().len(), 1);

    // 非法文件名拒绝
    assert!(context.restore_database("../evil.db").is_err());
    assert!(context.delete_database_backup("..\\evil.db").is_err());
    assert!(context.restore_database("agmux-export-nothere.db").is_err());
}

/// 恢复备份必须还原备份时的卡片排序（sort_order 随备份走）
#[test]
fn restore_database_preserves_profile_order() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let a = context.codex_capture_profile("A").unwrap();
    let b = context.codex_capture_profile("B").unwrap();
    let c = context.codex_capture_profile("C").unwrap();
    // 拖成 C、A、B 并落库
    context
        .codex_reorder_profiles(&[c.id.clone(), a.id.clone(), b.id.clone()])
        .unwrap();
    let exported = context.export_database().unwrap();
    let name = exported.file_name().unwrap().to_string_lossy().into_owned();

    // 破坏现场：改回创建顺序，确保恢复结果只能来自备份而不是巧合
    context
        .codex_reorder_profiles(&[a.id.clone(), b.id.clone(), c.id.clone()])
        .unwrap();

    context.restore_database(&name).unwrap();
    let names: Vec<String> = context
        .database
        .codex_profiles()
        .unwrap()
        .into_iter()
        .map(|profile| profile.name)
        .collect();
    assert_eq!(
        names,
        vec!["C".to_string(), "A".to_string(), "B".to_string()]
    );
}

/// Claude 供应商全字段随备份走：A/B 差异化字段 → 恢复后逐列对上，排序与激活位随之恢复，
/// 恢复还把激活配置的托管键收敛回 live settings.json（平凡快照走 merge，不整写原文）。
#[test]
fn claude_profiles_survive_backup_restore_round_trip() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
    let context = AppContext::new(paths).unwrap();

    let a = context
        .claude_save(
            None,
            ClaudeProfileInput {
                name: "A".into(),
                base_url: Some("https://a.example".into()),
                auth_token: Some("tok-a".into()),
                model: Some("glm-a".into()),
                description: Some("第一份".into()),
                kind: Some("zhipu".into()),
                admin_url: Some("https://console.example".into()),
                extra_env: Some(r#"{"FOO":"bar"}"#.into()),
                icon: Some("zhipu".into()),
                fetched_models: Some(vec!["glm-a".into(), "glm-a-x".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    let b_raw =
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://b.example","ANTHROPIC_AUTH_TOKEN":"tok-b"}}"#;
    let b = context
        .claude_save(
            None,
            ClaudeProfileInput {
                name: "B".into(),
                raw_settings: Some(b_raw.into()),
                ..Default::default()
            },
        )
        .unwrap();
    context.claude_apply(&b.id).unwrap();
    context
        .claude_reorder(&[b.id.clone(), a.id.clone()])
        .unwrap();

    let exported = context.export_database().unwrap();
    let name = exported.file_name().unwrap().to_string_lossy().into_owned();
    // 破坏现场：清空 Claude 数据与激活位，确保恢复结果只能来自备份
    // （service 层拒绝删除使用中的配置，清场直接走数据库层）
    context.database.delete_claude_profile(&a.id).unwrap();
    context.database.delete_claude_profile(&b.id).unwrap();
    assert!(context.database.claude_profiles().unwrap().is_empty());
    assert_eq!(context.database.active_claude_profile().unwrap(), None);

    context.restore_database(&name).unwrap();
    let restored = context.database.claude_profiles().unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].id, b.id, "排序随备份恢复");
    let ra = restored.iter().find(|profile| profile.id == a.id).unwrap();
    assert_eq!(ra.base_url.as_deref(), Some("https://a.example"));
    assert_eq!(ra.auth_token.as_deref(), Some("tok-a"));
    assert_eq!(ra.model.as_deref(), Some("glm-a"));
    assert_eq!(ra.description.as_deref(), Some("第一份"));
    assert_eq!(ra.kind.as_deref(), Some("zhipu"));
    assert_eq!(ra.admin_url.as_deref(), Some("https://console.example"));
    assert_eq!(ra.extra_env.as_deref(), Some(r#"{"FOO":"bar"}"#));
    assert_eq!(ra.icon.as_deref(), Some("zhipu"));
    assert_eq!(ra.fetched_models.as_deref(), Some(r#"["glm-a","glm-a-x"]"#));
    let rb = restored.iter().find(|profile| profile.id == b.id).unwrap();
    assert_eq!(rb.raw_settings.as_deref(), Some(b_raw));
    assert_eq!(
        context.database.active_claude_profile().unwrap().as_deref(),
        Some(b.id.as_str())
    );
    let live = std::fs::read_to_string(home.path().join(".claude/settings.json")).unwrap();
    // B 的快照只含托管 env 键，属平凡快照：写回落 merge 不再整文件替换（保住用户其余键），
    // 断言收敛到 B 的托管键值即可。
    let live: serde_json::Value = serde_json::from_str(&live).unwrap();
    assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://b.example");
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "tok-b");
}

/// 旧版备份（尚无 claude_profiles 表 / global_enabled 列）恢复：Codex 数据照常恢复，
/// Claude 从零开始；恢复前激活过的配置把托管键从 live 撤下（应用不再管理任何 Claude 配置）。
#[test]
fn restore_legacy_backup_without_claude_tables_starts_claude_empty() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
    let context = AppContext::new(paths).unwrap();
    let settings = home.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        r#"{"model":"opus","permissions":{"allow":["Read"]}}"#,
    )
    .unwrap();
    let active = context
        .claude_save(
            None,
            ClaudeProfileInput {
                name: "A".into(),
                base_url: Some("https://a.example".into()),
                auth_token: Some("tok-a".into()),
                extra_env: Some(r#"{"A_KEY":"1"}"#.into()),
                ..Default::default()
            },
        )
        .unwrap();
    context.claude_apply(&active.id).unwrap();

    // 手工搭旧版库（v4 形状：无 claude_profiles、app_state 无激活位、mcp_servers 无 global_enabled）
    let legacy = context.paths.database_backup.join("cg-backup-legacy.db");
    {
        let connection = Connection::open(&legacy).unwrap();
        connection
            .execute_batch(
                r#"CREATE TABLE app_state (
                     singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                     active_profile_id TEXT,
                     default_account_id TEXT
                   );
                   CREATE TABLE switch_events (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     profile_id TEXT,
                     action TEXT NOT NULL,
                     status TEXT NOT NULL,
                     message TEXT,
                     created_at TEXT NOT NULL
                   );
                   CREATE TABLE profiles (
                     id TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     payload_json TEXT NOT NULL,
                     icon TEXT,
                     kind TEXT NOT NULL,
                     account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
                     created_at TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                   );
                   CREATE TABLE accounts (
                     id TEXT PRIMARY KEY,
                     email TEXT,
                     id_token TEXT,
                     refresh_token TEXT NOT NULL,
                     authenticated_at INTEGER NOT NULL,
                     auth_json TEXT
                   );
                   INSERT INTO accounts VALUES ('acc-1', 'a@example.com', NULL, 'rt-1', 1, NULL);
                   INSERT INTO profiles(id, name, payload_json, kind, created_at, updated_at)
                     VALUES ('p-1', 'GLM', '{}', 'third_party', '1', '1');
                   INSERT INTO app_state(singleton, active_profile_id) VALUES (1, 'p-1');"#,
            )
            .unwrap();
    }

    context.restore_database("cg-backup-legacy.db").unwrap();

    // Codex 数据照常恢复；Claude 表为空、激活位为空
    assert_eq!(context.database.codex_profiles().unwrap()[0].name, "GLM");
    let (codex_active, _default) = context.database.app_state().unwrap();
    assert_eq!(codex_active.as_deref(), Some("p-1"));
    assert!(context.database.claude_profiles().unwrap().is_empty());
    assert_eq!(context.database.active_claude_profile().unwrap(), None);
    // live 托管键撤下，未托管键原样保留
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(doc["model"], "opus");
    assert_eq!(doc["permissions"]["allow"][0], "Read");
    assert!(doc["env"].get("ANTHROPIC_BASE_URL").is_none());
    assert!(doc["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    assert!(doc["env"].get("A_KEY").is_none());
}

/// 恢复数据库后 live settings.json 收敛到恢复出的激活位：
/// 备份时刻激活 B、恢复前激活 A → 恢复后 live 是 B 的键，A 的附加键不残留，未托管键保留。
#[test]
fn restore_reconciles_claude_live_with_restored_active_profile() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
    let context = AppContext::new(paths).unwrap();
    let settings = home.path().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        r#"{"model":"opus","permissions":{"allow":["Read"]}}"#,
    )
    .unwrap();

    let a = context
        .claude_save(
            None,
            ClaudeProfileInput {
                name: "A".into(),
                base_url: Some("https://a.example".into()),
                auth_token: Some("tok-a".into()),
                extra_env: Some(r#"{"A_KEY":"1"}"#.into()),
                ..Default::default()
            },
        )
        .unwrap();
    let b = context
        .claude_save(
            None,
            ClaudeProfileInput {
                name: "B".into(),
                base_url: Some("https://b.example".into()),
                auth_token: Some("tok-b".into()),
                extra_env: Some(r#"{"B_KEY":"2"}"#.into()),
                ..Default::default()
            },
        )
        .unwrap();
    // 备份时刻 B 激活，之后切到 A：live 现在是 A 的键
    context.claude_apply(&b.id).unwrap();
    let exported = context.export_database().unwrap();
    let name = exported.file_name().unwrap().to_string_lossy().into_owned();
    context.claude_apply(&a.id).unwrap();
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "https://a.example");

    context.restore_database(&name).unwrap();

    // 激活位回到 B，live 跟着回到 B：A 的附加键撤下，未托管键保留
    assert_eq!(
        context.database.active_claude_profile().unwrap().as_deref(),
        Some(b.id.as_str())
    );
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(doc["model"], "opus");
    assert_eq!(doc["permissions"]["allow"][0], "Read");
    assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "https://b.example");
    assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "tok-b");
    assert_eq!(doc["env"]["B_KEY"], "2");
    assert!(doc["env"].get("A_KEY").is_none());
}

#[test]
fn apply_builtin_profile_writes_exact_config_and_catalog() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        "model = \"other\"\n[mcp_servers.keep]\ncommand = \"node\"\n",
    )
    .unwrap();
    let old_models = b"{\"models\":[]}";
    std::fs::write(paths.codex_home.join("models.json"), old_models).unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("deepseek", None, None, Some("sk-test"), None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    // 整文件替换，模板之外的键全部清掉，仅密钥占位符被替换；
    // MCP 段例外——跟随 live 携带（全局生效，不随供应商模板丢失）
    let config = std::fs::read(context.paths.codex_config()).unwrap();
    let rendered = crate::codex_builtin::template("deepseek")
        .unwrap()
        .render_config(Some("sk-test"))
        .unwrap();
    let live =
        codex_config::parse_document("model = \"other\"\n[mcp_servers.keep]\ncommand = \"node\"\n")
            .unwrap();
    let expected =
        codex_config::merge_mcp_section(&String::from_utf8_lossy(&rendered), &live).into_bytes();
    assert_eq!(config, expected);
    assert!(!String::from_utf8_lossy(&config).contains("<YOUR_API_KEY>"));
    // 关联文件按本供应商字节写入，旧文件已备份
    let models = std::fs::read(context.paths.codex_home.join("models.json")).unwrap();
    assert_eq!(models, crate::codex_builtin::DEEPSEEK_MODELS);
    let backup = std::fs::read_dir(context.paths.codex_files_backup.clone())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(backup).unwrap(), old_models);
    assert!(context.paths.config_backup.read_dir().unwrap().count() > 0);
}

fn mcp_test_context(live_config: &str) -> (AppContext, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    if !live_config.is_empty() {
        std::fs::write(paths.codex_config(), live_config).unwrap();
    }
    (AppContext::new(paths).unwrap(), home)
}

fn read_config_text(context: &AppContext) -> String {
    String::from_utf8(std::fs::read(context.paths.codex_config()).unwrap()).unwrap()
}

fn config_backup_count(context: &AppContext) -> usize {
    std::fs::read_dir(&context.paths.config_backup)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("config-"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn mcp_save_edit_preserves_unmodeled_keys_and_subtables() {
    let (context, _home) = mcp_test_context(
        r#"[mcp_servers.dev_repl]
command = "node_repl.exe"
args = []
startup_timeout_sec = 120
cwd = "C:\\bin"

# 勿动：桌面版自动维护
[mcp_servers.dev_repl.env]
CODEX_HOME = "C:\\.codex"
"#,
    );

    context
        .codex_save_mcp_server(
            Some("dev_repl"),
            McpServerSpec {
                name: "dev_repl".into(),
                command: Some("node_repl.exe".into()),
                args: vec!["--verbose".into()],
                env: BTreeMap::from([("CODEX_HOME".into(), "C:\\.codex".into())]),
                startup_timeout_sec: Some(120),
                ..Default::default()
            },
        )
        .unwrap();

    let config = read_config_text(&context);
    assert!(config.contains("cwd = \"C:\\\\bin\""), "{config}");
    assert!(config.contains("# 勿动：桌面版自动维护"), "{config}");
    assert!(config.contains("startup_timeout_sec = 120"), "{config}");
    assert!(config.contains("\"--verbose\""), "{config}");
    assert!(context.paths.config_backup.read_dir().unwrap().count() > 0);

    let servers = context.codex_list_mcp_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].args, ["--verbose"]);
    assert_eq!(servers[0].startup_timeout_sec, Some(120));
}

#[test]
fn mcp_save_renames_server() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.old]\nurl = \"https://mcp.example/mcp\"\n");

    context
        .codex_save_mcp_server(
            Some("old"),
            McpServerSpec {
                name: "fresh".into(),
                url: Some("https://mcp.example/mcp".into()),
                ..Default::default()
            },
        )
        .unwrap();

    let config = read_config_text(&context);
    assert!(!config.contains("mcp_servers.old"), "{config}");
    assert!(config.contains("mcp_servers.fresh"), "{config}");
}

#[test]
fn mcp_save_rejects_invalid_input() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n");

    let spec = |name: &str, url: Option<&str>, command: Option<&str>| McpServerSpec {
        name: name.into(),
        url: url.map(str::to_string),
        command: command.map(str::to_string),
        ..Default::default()
    };

    // 重名（新建）
    assert!(context
        .codex_save_mcp_server(None, spec("tavily", Some("https://other/mcp"), None))
        .is_err());
    // 非法名称：点号嵌套 / 空格 / 中文 / 空
    assert!(context
        .codex_save_mcp_server(None, spec("a.b", None, Some("node")))
        .is_err());
    assert!(context
        .codex_save_mcp_server(None, spec("a b", None, Some("node")))
        .is_err());
    assert!(context
        .codex_save_mcp_server(None, spec("中文", None, Some("node")))
        .is_err());
    assert!(context
        .codex_save_mcp_server(None, spec("", None, Some("node")))
        .is_err());
    // 传输互斥与必填
    assert!(context
        .codex_save_mcp_server(None, spec("x", Some("https://a/mcp"), Some("node")))
        .is_err());
    assert!(context
        .codex_save_mcp_server(None, spec("x", None, None))
        .is_err());
    assert!(context
        .codex_save_mcp_server(None, spec("x", Some("ftp://a"), None))
        .is_err());
    // 超时为正
    assert!(context
        .codex_save_mcp_server(
            None,
            McpServerSpec {
                name: "x".into(),
                command: Some("node".into()),
                startup_timeout_sec: Some(0),
                ..Default::default()
            }
        )
        .is_err());
}

/// 应用自己的操作（开关/保存/采纳）不得制造或残留差异：差异只属于外部修改。
#[test]
fn mcp_app_operations_leave_no_sync_diff() {
    // 带原生 enabled=false 的片段：app 内打开开关必须让 live 与镜像同时收敛，
    // 否则语义比（Some(false) vs None）会把 app 自己的操作误报成"已修改"
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\nenabled = false\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );
    import_mcp_from_live(&context);
    context
        .claude_save_mcp_server(None, "b", r#"{"url":"https://claude.example.test/mcp"}"#)
        .unwrap();
    assert!(context.codex_mcp_sync_preview().unwrap().entries.is_empty());

    context
        .set_mcp_server_enabled("a", SkillTool::Codex, true)
        .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(
        preview.entries.is_empty(),
        "app 开关制造了幻影差异: {:?}",
        preview.entries
    );
    let live = read_config_text(&context);
    assert!(!live.contains("enabled"), "原生 enabled 键应被剥离: {live}");

    // 开→关→开 与 Claude 侧开关同样不得留下差异
    context
        .set_mcp_server_enabled("a", SkillTool::Codex, false)
        .unwrap();
    context
        .set_mcp_server_enabled("a", SkillTool::Codex, true)
        .unwrap();
    context
        .set_mcp_server_enabled("b", SkillTool::Claude, false)
        .unwrap();
    context
        .set_mcp_server_enabled("b", SkillTool::Claude, true)
        .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(
        preview.entries.is_empty(),
        "开关循环留下差异: {:?}",
        preview.entries
    );
}

/// 外部把已关条目加回 live 后，差异页"同步"必须真正消掉差异：片段进镜像、开关翻开。
#[test]
fn mcp_adopt_external_readd_clears_diff_and_reenables() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);
    context
        .set_mcp_server_enabled("a", SkillTool::Codex, false)
        .unwrap();

    // 外部把同名片段写回 live：应报告"仅 live 有"
    let external = "[mcp_servers.a]\nurl = \"https://a/mcp\"\ncommand = \"node\"\n";
    std::fs::write(context.paths.codex_config(), external).unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert_eq!(preview.entries.len(), 1);
    assert_eq!(preview.entries[0].kind, McpSyncEntryKind::LiveOnly);

    // 采纳后差异清空，codex 开关翻开（live 里有条目 = on 的事实源）
    context
        .set_mcp_mirror_entry(
            "a",
            Some("[mcp_servers.a]\nurl = \"https://a/mcp\"\ncommand = \"node\"\n"),
        )
        .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(
        preview.entries.is_empty(),
        "采纳后差异未清空: {:?}",
        preview.entries
    );
    assert!(
        context
            .database
            .mcp_server_record("a")
            .unwrap()
            .unwrap()
            .codex_enabled
    );
    let listed = context.codex_list_mcp_servers().unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|server| server.name == "a")
            .unwrap()
            .enabled,
        None
    );
}

#[test]
fn mcp_managed_entry_hidden_and_untouchable() {
    // node_repl 由 Codex 桌面版自动写入/更新，本应用完全跳过且不许碰
    let (context, _home) = mcp_test_context(
        "[mcp_servers.node_repl]\ncommand = \"node_repl.exe\"\nargs = []\n\n[mcp_servers.github]\nurl = \"https://x/mcp\"\n",
    );

    // 列表不显示托管条目
    let servers = context.codex_list_mcp_servers().unwrap();
    let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["github"]);

    // 正常导入一次保证镜像与 live 一致，再往镜像塞入残留的 node_repl 片段
    import_mcp_from_live(&context);
    let mut fragments = context.database.mcp_server_fragments().unwrap();
    fragments.push((
        "node_repl".into(),
        "[mcp_servers.node_repl]\ncommand = \"stale.exe\"\n".into(),
    ));
    context
        .database
        .replace_mcp_server_fragments(&fragments, "2")
        .unwrap();

    // 差异对比对两侧的托管条目都视而不见：无差异
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries.len());

    // 不能以托管名创建/编辑，也不能删除
    assert!(context
        .codex_save_mcp_server(
            None,
            McpServerSpec {
                name: "node_repl".into(),
                command: Some("x".into()),
                ..Default::default()
            }
        )
        .is_err());
    assert!(context.codex_delete_mcp_server("node_repl").is_err());

    // 镜像写回 live：托管条目原样保留（不被 stale 片段覆盖、也不被删除）
    context.restore_mcp_from_database().unwrap();
    let config = read_config_text(&context);
    assert!(config.contains("[mcp_servers.node_repl]"), "{config}");
    assert!(config.contains("node_repl.exe"), "{config}");
    assert!(!config.contains("stale.exe"), "{config}");
    assert!(config.contains("[mcp_servers.github]"), "{config}");
}

#[test]
fn mcp_save_with_fragment_is_wysiwyg() {
    // 片段路径保存：编辑器新增的未建模键/注释保留，片段里删掉的键（cwd）随之消失，
    // 片段缺失的建模字段（url）按 spec 补齐
    let (context, _home) =
        mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\ncwd = \"/old\"\n");
    let fragment = "[mcp_servers.a]\n# 手动维护\ntools = [\"x\"]\n";
    context
        .save_mcp_server_with_fragment(
            Some("a"),
            McpServerSpec {
                name: "a".into(),
                url: Some("https://a/mcp".into()),
                ..Default::default()
            },
            Some(fragment),
        )
        .unwrap();

    let config = read_config_text(&context);
    assert!(config.contains("[mcp_servers.a]"), "{config}");
    assert!(config.contains("# 手动维护"), "{config}");
    assert!(config.contains("tools = [\"x\"]"), "{config}");
    assert!(config.contains("url = \"https://a/mcp\""), "{config}");
    assert!(!config.contains("cwd"), "{config}");
}

#[test]
fn mcp_delete_removes_only_target() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );

    context.codex_delete_mcp_server("a").unwrap();

    let config = read_config_text(&context);
    assert!(!config.contains("mcp_servers.a"), "{config}");
    assert!(config.contains("mcp_servers.b"), "{config}");
    // 不存在的服务器报错（外部并发修改时让用户看见）
    assert!(context.codex_delete_mcp_server("nothere").is_err());
}

#[test]
fn apply_raw_profile_carries_live_mcp_section() {
    let (context, _home) = mcp_test_context(concat!(
        "[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n\n",
        "[marketplaces.ponytail]\nsource = \"https://github.com/DietrichGebert/ponytail.git\"\n\n",
        "[plugins.\"ponytail@ponytail\"]\nenabled = true\n\n",
        "[hooks.state]\nponytail = true\n",
    ));
    let raw = concat!(
        "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\n\n",
        "[mcp_servers.stale]\ncommand = \"old\"\n\n",
        "[marketplaces.stale]\nsource = \"old\"\n\n",
        "[plugins.\"stale@stale\"]\nenabled = true\n\n",
        "[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.z.ai\"\nwire_api = \"responses\"\n",
    );
    let profile = context
        .codex_add_custom_profile("智谱", None, raw, None, None, None, None, None)
        .unwrap();

    context.codex_apply_profile(&profile.id).unwrap();

    let config = read_config_text(&context);
    // live 的 MCP 段被携带进快照供应商；快照里的陈旧段被替换
    assert!(config.contains("mcp_servers.tavily"), "{config}");
    assert!(!config.contains("mcp_servers.stale"), "{config}");
    assert!(config.contains("marketplaces.ponytail"), "{config}");
    assert!(config.contains("ponytail@ponytail"), "{config}");
    assert!(config.contains("ponytail = true"), "{config}");
    assert!(!config.contains("marketplaces.stale"), "{config}");
    assert!(!config.contains("stale@stale"), "{config}");
    assert!(config.contains("model = \"glm-5.3\""), "{config}");
    let second = context
        .codex_add_custom_profile("第二配置", None, raw, None, None, None, None, None)
        .unwrap();
    for enabled in [false, true] {
        let mut live = codex_config::parse_document(&read_config_text(&context)).unwrap();
        live["plugins"]["ponytail@ponytail"]["enabled"] = toml_edit::value(enabled);
        std::fs::write(context.paths.codex_config(), live.to_string()).unwrap();
        for id in [&second.id, &profile.id] {
            context.codex_apply_profile(id).unwrap();
            let config = codex_config::parse_document(&read_config_text(&context)).unwrap();
            assert_eq!(
                config["plugins"]["ponytail@ponytail"]["enabled"].as_bool(),
                Some(enabled)
            );
        }
    }
}

#[test]
fn mcp_list_does_not_write_database_mirror() {
    let (context, _home) = mcp_test_context(concat!(
        "[mcp_servers.github]\n",
        "# 手动维护\n",
        "command = \"node\"\n",
        "cwd = \"/srv\"\n",
    ));

    let servers = context.codex_list_mcp_servers().unwrap();
    assert_eq!(servers.len(), 1);

    assert!(context.database.mcp_server_fragments().unwrap().is_empty());
}

#[test]
fn restore_database_writes_mcp_back_to_live() {
    // 机器 A：显式导入 MCP 镜像后导出备份
    let (source_context, _home_a) =
        mcp_test_context("[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n");
    context_with_profile(&source_context);
    import_mcp_from_live(&source_context);
    let exported = source_context.export_database().unwrap();
    let backup_name = exported.file_name().unwrap().to_string_lossy().into_owned();

    // 机器 B：live 没有 MCP，导入备份后 MCP 写回 live
    let (target_context, _home_b) = mcp_test_context("model = \"gpt-5.6\"\n");
    context_with_profile(&target_context);
    std::fs::copy(
        &exported,
        target_context.paths.database_backup.join(&backup_name),
    )
    .unwrap();
    target_context.restore_database(&backup_name).unwrap();

    let config = read_config_text(&target_context);
    assert!(config.contains("mcp_servers.tavily"), "{config}");
    assert!(config.contains("model = \"gpt-5.6\""), "{config}");
}

fn context_with_profile(context: &AppContext) {
    let raw = "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.z.ai\"\nwire_api = \"responses\"\n";
    context
        .codex_add_custom_profile("智谱", None, raw, None, None, None, None, None)
        .unwrap();
}

#[test]
fn created_profiles_snapshot_prefers_database_mcp_mirror() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.mirrored]\nurl = \"https://mirror/mcp\"\n");
    import_mcp_from_live(&context);
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.live]\nurl = \"https://live/mcp\"\n",
    )
    .unwrap();

    // 自定义供应商：粘贴的配置没有 MCP，保存后快照带上全局段（编辑器打开即见）
    let raw = "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.z.ai\"\nwire_api = \"responses\"\n";
    let custom = context
        .codex_add_custom_profile("智谱", None, raw, None, None, None, None, None)
        .unwrap();
    let detail = context.codex_get_profile(&custom.id).unwrap();
    let stored = detail.raw_config.expect("自定义快照应有 raw_config");
    assert!(stored.contains("mcp_servers.mirrored"), "{stored}");
    assert!(!stored.contains("mcp_servers.live"), "{stored}");

    // 内置供应商：快照同样带上全局段
    let builtin = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    let detail = context.codex_get_profile(&builtin.id).unwrap();
    let stored = detail.raw_config.expect("内置快照应有 raw_config");
    assert!(stored.contains("mcp_servers.mirrored"), "{stored}");
    assert!(!stored.contains("mcp_servers.live"), "{stored}");
}

#[test]
fn created_profiles_fall_back_to_live_mcp_when_mirror_empty() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n");
    let raw = "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://api.z.ai\"\nwire_api = \"responses\"\n";
    let custom = context
        .codex_add_custom_profile("智谱", None, raw, None, None, None, None, None)
        .unwrap();
    let builtin = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();

    for profile in [custom, builtin] {
        let stored = context
            .codex_get_profile(&profile.id)
            .unwrap()
            .raw_config
            .unwrap();
        assert!(stored.contains("mcp_servers.tavily"), "{stored}");
    }
}

#[test]
fn mcp_delete_via_app_clears_database_mirror() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n");

    // 应用内的删除是明确意图：live 与数据库镜像一起清空
    context.codex_delete_mcp_server("tavily").unwrap();
    assert!(context.database.mcp_server_fragments().unwrap().is_empty());

    // 应用内的保存/删除持续整表对齐镜像。
    context
        .codex_save_mcp_server(
            None,
            McpServerSpec {
                name: "a".into(),
                url: Some("https://a/mcp".into()),
                ..Default::default()
            },
        )
        .unwrap();
    context
        .codex_save_mcp_server(
            None,
            McpServerSpec {
                name: "b".into(),
                url: Some("https://b/mcp".into()),
                ..Default::default()
            },
        )
        .unwrap();
    context.codex_delete_mcp_server("a").unwrap();
    let fragments = context.database.mcp_server_fragments().unwrap();
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].0, "b");
}

#[test]
fn mcp_import_from_live_forces_mirror() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n");
    import_mcp_from_live(&context);

    // 外部清空 live 后，显式「从配置导入」让数据库接受空态（放弃保留的镜像）
    std::fs::write(context.paths.codex_config(), "model = \"gpt-5.6\"\n").unwrap();
    let count = import_mcp_from_live(&context);
    assert_eq!(count, 0);
    assert!(context.database.mcp_server_fragments().unwrap().is_empty());
    assert!(context.restore_mcp_from_database().is_err());
}

#[test]
fn restore_mcp_rewrites_only_the_mcp_region() {
    let (context, _home) = mcp_test_context(
        "model = \"gpt-5.6\"\n\n[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n\n[projects.'x']\ntrust_level = \"trusted\"\n",
    );
    import_mcp_from_live(&context);

    // 外部把 tavily 那行的引号删掉：整份文件解析不了，MCP 段之外的内容完好
    std::fs::write(
        context.paths.codex_config(),
        "model = \"gpt-5.6\"\n\n[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\nbearer_token_env_var = \"BROKEN\n\n[projects.'x']\ntrust_level = \"trusted\"\n",
    )
    .unwrap();

    let count = context.restore_mcp_from_database().unwrap();
    assert_eq!(count, 1);

    let config = read_config_text(&context);
    codex_config::parse_document(&config).unwrap();
    // MCP 段按镜像重写，坏行消失
    assert!(!config.contains("BROKEN"), "{config}");
    // 区域外的配置逐字保留，没有被整份重写殃及
    assert!(config.contains("model = \"gpt-5.6\""), "{config}");
    assert!(
        config.contains("[projects.'x']\ntrust_level = \"trusted\""),
        "{config}"
    );
}

#[test]
fn restore_mcp_keeps_managed_entries_when_rebuilding_the_region() {
    const MANAGED: &str = "[mcp_servers.node_repl]\ncommand = \"node_repl.exe\"\n\n[mcp_servers.node_repl.env]\nCODEX_HOME = \"/x\"\n";
    let live = format!("[mcp_servers.github]\nurl = \"https://g/mcp\"\n\n{MANAGED}");
    let (context, _home) = mcp_test_context(&live);
    import_mcp_from_live(&context);

    // 解析不了时重建 MCP 区域：托管条目不在镜像里，必须按 live 原文留在原位
    std::fs::write(
        context.paths.codex_config(),
        format!("model = \"gpt-5.6\"\n\n[mcp_servers.github]\nurl = \"https://g/mcp\"\nbroken = \"oops\n\n{MANAGED}"),
    )
    .unwrap();

    context.restore_mcp_from_database().unwrap();

    let config = read_config_text(&context);
    codex_config::parse_document(&config).unwrap();
    assert!(!config.contains("oops"), "{config}");
    assert!(config.contains("[mcp_servers.node_repl]"), "{config}");
    assert!(config.contains("node_repl.env"), "{config}");
    assert!(config.contains("CODEX_HOME = \"/x\""), "{config}");
}

#[test]
fn restore_mcp_region_stops_at_the_next_section() {
    // MCP 段后面紧跟别的区域时，重建必须止步于下一个表头——尤其数组表头
    // （[[skills.config]]）也要能终止区域，否则邻接区域的条目会被当成 MCP 块吞掉
    const TAIL: &str = "[[skills.config]]\npath = 'C:\\skills\\a\\SKILL.md'\nenabled = false\n\n[plugins.\"browser@openai-bundled\"]\nenabled = true\n";
    let live = format!("[mcp_servers.github]\nurl = \"https://g/mcp\"\n\n{TAIL}");
    let (context, _home) = mcp_test_context(&live);
    import_mcp_from_live(&context);

    std::fs::write(
        context.paths.codex_config(),
        format!("model = \"gpt-5.6\"\n\n[mcp_servers.github]\nurl = \"https://g/mcp\"\nbroken = \"oops\n\n{TAIL}"),
    )
    .unwrap();

    context.restore_mcp_from_database().unwrap();

    let config = read_config_text(&context);
    codex_config::parse_document(&config).unwrap();
    assert!(!config.contains("oops"), "{config}");
    assert!(config.contains("[mcp_servers.github]"), "{config}");
    // 邻接区域逐字保留，一个字节都没被 MCP 区域吞掉
    assert!(config.ends_with(TAIL), "{config}");
    assert!(config.contains("model = \"gpt-5.6\""), "{config}");
}

#[test]
fn restore_mcp_refuses_when_no_mcp_region_can_be_located() {
    let (context, _home) =
        mcp_test_context("[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n");
    import_mcp_from_live(&context);

    // 文件彻底损坏且不含 MCP 段：拒绝整份重写（那会丢掉区域外的全部配置），
    // 原文件一个字节都不动，交由备份恢复处理
    std::fs::write(context.paths.codex_config(), "not [ valid").unwrap();
    assert!(context.restore_mcp_from_database().is_err());
    assert_eq!(read_config_text(&context), "not [ valid");
}

#[test]
fn mcp_list_does_not_absorb_externally_deleted_rows() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );
    import_mcp_from_live(&context);

    // 外部（codex mcp remove）删掉 a：列表只读，a 保留为“仅数据库”差异
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    )
    .unwrap();
    assert_eq!(context.codex_list_mcp_servers().unwrap().len(), 1);
    assert_eq!(context.database.mcp_server_fragments().unwrap().len(), 2);

    // 显式“以配置文件为准”才收敛：a 从数据库清除，预览归零
    let count = import_mcp_from_live(&context);
    assert_eq!(count, 1);
    let fragments = context.database.mcp_server_fragments().unwrap();
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].0, "b");
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
}

/// 引擎开关各自独立：Codex 关只删 config.toml 条目，Claude 侧 live 与开关不动，反之亦然。
#[test]
fn mcp_engine_toggle_only_touches_its_own_live_file() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );
    import_mcp_from_live(&context);
    for name in ["a", "b"] {
        context
            .claude_save_mcp_server(None, name, r#"{"url":"https://claude.example.test/mcp"}"#)
            .unwrap();
    }

    // Codex 关：config.toml 删条目；.claude.json 原样保留，Claude 开关仍是开
    context
        .set_mcp_server_enabled("a", SkillTool::Codex, false)
        .unwrap();
    let live = read_config_text(&context);
    assert!(!live.contains("mcp_servers.a"), "{live}");
    assert!(live.contains("mcp_servers.b"), "{live}");
    let claude = std::fs::read_to_string(context.paths.claude_mcp_config()).unwrap();
    assert!(claude.contains("\"a\""), "{claude}");
    let record = context.database.mcp_server_record("a").unwrap().unwrap();
    assert!(!record.codex_enabled);
    assert!(record.claude_enabled);
    let listed = context.codex_list_mcp_servers().unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|server| server.name == "a")
            .unwrap()
            .enabled,
        Some(false)
    );
    let claude_listed = context.claude_list_mcp_servers().unwrap();
    assert_eq!(
        claude_listed
            .iter()
            .find(|server| server.name == "a")
            .unwrap()
            .enabled,
        None
    );

    // Claude 关：.claude.json 删条目；config.toml 里 b 原样（a 本就不在），Codex 开关不动
    context
        .set_mcp_server_enabled("a", SkillTool::Claude, false)
        .unwrap();
    let claude = std::fs::read_to_string(context.paths.claude_mcp_config()).unwrap();
    assert!(!claude.contains("\"a\""), "{claude}");
    let record = context.database.mcp_server_record("a").unwrap().unwrap();
    assert!(!record.codex_enabled);
    assert!(!record.claude_enabled);

    // Codex 开：条目从镜像片段恢复回 config.toml；Claude 仍是关（.claude.json 不回填）
    context
        .set_mcp_server_enabled("a", SkillTool::Codex, true)
        .unwrap();
    let live = read_config_text(&context);
    assert!(live.contains("mcp_servers.a"), "{live}");
    let claude = std::fs::read_to_string(context.paths.claude_mcp_config()).unwrap();
    assert!(!claude.contains("\"a\""), "{claude}");
    assert!(
        context
            .database
            .mcp_server_record("a")
            .unwrap()
            .unwrap()
            .codex_enabled
    );

    // Claude 开：条目投影回 .claude.json
    context
        .set_mcp_server_enabled("a", SkillTool::Claude, true)
        .unwrap();
    let claude = std::fs::read_to_string(context.paths.claude_mcp_config()).unwrap();
    assert!(claude.contains("\"a\""), "{claude}");
}

/// 镜像整表重写保留关闭行且不吞另一引擎的开关；后续保存不会把已关条目顶回 live。
#[test]
fn mcp_mirror_rewrite_preserves_engine_flags_across_saves() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );
    import_mcp_from_live(&context);
    for name in ["a", "b"] {
        context
            .claude_save_mcp_server(None, name, r#"{"url":"https://claude.example.test/mcp"}"#)
            .unwrap();
    }
    context
        .set_mcp_server_enabled("a", SkillTool::Codex, false)
        .unwrap();
    context
        .set_mcp_server_enabled("b", SkillTool::Claude, false)
        .unwrap();

    // 保存与 a/b 无关的第三台：整表重写后两个关闭行原样保留，开关不被重置
    context
        .codex_save_mcp_server(
            None,
            McpServerSpec {
                name: "c".into(),
                url: Some("https://c/mcp".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let a = context.database.mcp_server_record("a").unwrap().unwrap();
    assert!(!a.codex_enabled, "a 的 Codex 开关被保存冲掉了");
    assert!(a.claude_enabled);
    let b = context.database.mcp_server_record("b").unwrap().unwrap();
    assert!(b.codex_enabled);
    assert!(!b.claude_enabled, "b 的 Claude 开关被保存冲掉了");
    let live = read_config_text(&context);
    assert!(!live.contains("mcp_servers.a"), "{live}");
    assert!(live.contains("mcp_servers.b"), "{live}");
    assert!(live.contains("mcp_servers.c"), "{live}");
    let claude = std::fs::read_to_string(context.paths.claude_mcp_config()).unwrap();
    assert!(claude.contains("\"a\""), "{claude}");
    assert!(!claude.contains("\"b\""), "{claude}");
    assert!(!claude.contains("\"c\""), "{claude}");
}

#[test]
fn mcp_preview_flags_live_only_db_only_and_changed() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);

    // 外部改 a 的 url、新增 b，且不再触发 list：预览应报“内容不同”与“仅配置文件”
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\nurl = \"https://a/v2\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    )
    .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert_eq!(preview.live_count, 2);
    assert_eq!(preview.db_count, 1);
    assert_eq!(preview.entries.len(), 2, "{:?}", preview.entries);

    let changed = preview
        .entries
        .iter()
        .find(|entry| entry.name == "a")
        .unwrap();
    assert_eq!(changed.kind, McpSyncEntryKind::Changed);

    let live_only = preview
        .entries
        .iter()
        .find(|entry| entry.name == "b")
        .unwrap();
    assert_eq!(live_only.kind, McpSyncEntryKind::LiveOnly);

    // live 整段换成 c：a 变成“仅数据库”
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.c]\nurl = \"https://c/mcp\"\n",
    )
    .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    let db_only = preview
        .entries
        .iter()
        .find(|entry| entry.name == "a")
        .unwrap();
    assert_eq!(db_only.kind, McpSyncEntryKind::DbOnly);
}

#[test]
fn mcp_diff_verbs_are_surgical_to_one_entry() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );
    import_mcp_from_live(&context);

    // 外部同时修改 a、b：出现两条 changed 差异
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\nurl = \"https://a/v2\"\n\n[mcp_servers.b]\nurl = \"https://b/v2\"\n",
    )
    .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert_eq!(preview.entries.len(), 2, "{:?}", preview.entries);

    // 只同步 a：镜像仅收 a，b 的差异必须原样保留（回归：单条操作不得全量重写镜像）
    let entry_a = preview
        .entries
        .iter()
        .find(|entry| entry.name == "a")
        .unwrap();
    context
        .set_mcp_mirror_entry("a", Some(entry_a.live_toml.as_deref().unwrap()))
        .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert_eq!(preview.entries.len(), 1, "{:?}", preview.entries);
    assert_eq!(preview.entries[0].name, "b");
    let mirror = context.codex_list_mcp_servers().unwrap();
    let a = mirror.iter().find(|server| server.name == "a").unwrap();
    assert_eq!(a.url.as_deref(), Some("https://a/v2"));

    // 只撤销 b：live 的 b 恢复为数据库旧值，镜像不动
    let db_b = preview.entries[0].db_toml.as_deref().unwrap();
    context.revert_mcp_live_entry("b", Some(db_b)).unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
    let live_text = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live_text.contains("https://b/mcp"), "{live_text}");
    assert!(!live_text.contains("https://b/v2"), "{live_text}");

    // live_only 的撤销 = 从 live 移除单条，镜像中其他条目保留
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\nurl = \"https://a/v2\"\n\n[mcp_servers.c]\nurl = \"https://c/mcp\"\n",
    )
    .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert_eq!(preview.entries.len(), 2, "{:?}", preview.entries);
    context.revert_mcp_live_entry("c", None).unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert_eq!(preview.entries.len(), 1, "{:?}", preview.entries);
    assert_eq!(preview.entries[0].kind, McpSyncEntryKind::DbOnly);
    let live_text = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(!live_text.contains("mcp_servers.c"), "{live_text}");

    // db_only 的同步 = 仅删镜像条目，live 不动
    context.set_mcp_mirror_entry("b", None).unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
    let live_text = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live_text.contains("https://a/v2"), "{live_text}");
}

#[test]
fn mcp_diff_entry_action_accepts_frontend_payload() {
    // 前端批量提交的载荷形状：fragment 为字符串表示写入该条，null 表示删除该条
    let actions: Vec<McpDiffEntryAction> = serde_json::from_value(serde_json::json!([
        { "name": "a", "fragment": "[mcp_servers.a]\nurl = \"https://a/mcp\"\n" },
        { "name": "b", "fragment": null }
    ]))
    .unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(
        actions[0].fragment.as_deref(),
        Some("[mcp_servers.a]\nurl = \"https://a/mcp\"\n")
    );
    assert_eq!(actions[1].fragment, None);
}

#[test]
fn mcp_batch_diff_reverts_in_one_write_keeping_live_layout() {
    let (context, _home) = mcp_test_context(
        "model = \"gpt-5\"\n\n[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n\n[scale]\nkeep = true\n",
    );
    import_mcp_from_live(&context);

    // 外部把两条都改了，且把顺序调换成 b、a
    std::fs::write(
        context.paths.codex_config(),
        "model = \"gpt-5\"\n\n[mcp_servers.b]\nurl = \"https://b/v2\"\n\n[mcp_servers.a]\nurl = \"https://a/v2\"\n\n[scale]\nkeep = true\n",
    )
    .unwrap();

    let actions: Vec<McpDiffEntryAction> = context
        .codex_mcp_sync_preview()
        .unwrap()
        .entries
        .iter()
        .map(|entry| McpDiffEntryAction {
            name: entry.name.clone(),
            fragment: entry.db_toml.clone(),
        })
        .collect();
    let backups = config_backup_count(&context);

    assert_eq!(context.revert_mcp_live_entries(&actions).unwrap(), 2);
    assert_eq!(
        config_backup_count(&context),
        backups + 1,
        "批量撤销只应产生一份备份（逐条调用会把保留池里操作前的备份挤掉）"
    );

    // 原地改写：段内顺序、其他配置段、文件布局全部保留，只把内容改回数据库版本
    let live = read_config_text(&context);
    let pos = |needle: &str| {
        live.find(needle)
            .unwrap_or_else(|| panic!("缺少 {needle}：\n{live}"))
    };
    assert!(pos("[mcp_servers.b]") < pos("[mcp_servers.a]"), "{live}");
    assert!(pos("[mcp_servers.a]") < pos("[scale]"), "{live}");
    assert!(
        live.contains("https://a/mcp") && live.contains("https://b/mcp"),
        "{live}"
    );
    assert!(!live.contains("/v2"), "{live}");
    assert!(context.codex_mcp_sync_preview().unwrap().entries.is_empty());
}

#[test]
fn mcp_batch_diff_live_aborts_without_writing_when_one_entry_fails() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);
    let before = read_config_text(&context);
    let backups = config_backup_count(&context);

    // 第二条的片段里没有 b：整批必须失败，且一条都不能落盘
    let actions = vec![
        McpDiffEntryAction {
            name: "a".to_string(),
            fragment: Some("[mcp_servers.a]\nurl = \"https://a/v2\"\n".to_string()),
        },
        McpDiffEntryAction {
            name: "b".to_string(),
            fragment: Some("[mcp_servers.other]\nurl = \"https://other/mcp\"\n".to_string()),
        },
    ];
    assert!(context.revert_mcp_live_entries(&actions).is_err());
    assert_eq!(read_config_text(&context), before, "整批失败时不应落盘");
    assert_eq!(
        config_backup_count(&context),
        backups,
        "整批失败时不应产生备份"
    );
}

#[test]
fn mcp_batch_mirror_adopt_writes_once_and_keeps_untouched_entries() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n\n[mcp_servers.c]\nurl = \"https://c/mcp\"\n",
    );
    import_mcp_from_live(&context);

    // 外部只改了 a、b；c 保持原样
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\nurl = \"https://a/v2\"\n\n[mcp_servers.b]\nurl = \"https://b/v2\"\n\n[mcp_servers.c]\nurl = \"https://c/mcp\"\n",
    )
    .unwrap();

    let actions: Vec<McpDiffEntryAction> = context
        .codex_mcp_sync_preview()
        .unwrap()
        .entries
        .iter()
        .map(|entry| McpDiffEntryAction {
            name: entry.name.clone(),
            fragment: entry.live_toml.clone(),
        })
        .collect();
    assert_eq!(context.set_mcp_mirror_entries(&actions).unwrap(), 2);

    let mirror = context.codex_list_mcp_servers().unwrap();
    let server = |name: &str| mirror.iter().find(|server| server.name == name).unwrap();
    assert_eq!(server("a").url.as_deref(), Some("https://a/v2"));
    assert_eq!(server("b").url.as_deref(), Some("https://b/v2"));
    assert_eq!(server("c").url.as_deref(), Some("https://c/mcp"));
    assert!(context.codex_mcp_sync_preview().unwrap().entries.is_empty());
}

#[test]
fn mcp_preview_ignores_comment_only_difference() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);

    // 只在条目内加一行注释：建模字段全等 = 语义等价，不算差异
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\n# 手动维护\nurl = \"https://a/mcp\"\n",
    )
    .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
}

#[test]
fn mcp_restore_keeps_live_text_for_semantically_equal_servers() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);

    // live 侧只多了注释（建模字段一致）：恢复时保留 live 原文，不回滚注释
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\n# 手动维护\nurl = \"https://a/mcp\"\n",
    )
    .unwrap();
    context.restore_mcp_from_database().unwrap();
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live.contains("# 手动维护"), "{live}");
    assert!(live.contains("https://a/mcp"), "{live}");

    // live 侧 url 真的不同：恢复才覆盖
    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\nurl = \"https://a/rolled-back\"\n",
    )
    .unwrap();
    context.restore_mcp_from_database().unwrap();
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live.contains("https://a/mcp"), "{live}");
    assert!(!live.contains("rolled-back"), "{live}");
}

#[test]
fn mcp_preview_ignores_blank_line_only_difference() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);

    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\n\n\nurl = \"https://a/mcp\"\n\n",
    )
    .unwrap();
    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
}

#[test]
fn mcp_preview_ignores_legacy_empty_mcp_root_header() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    context
        .database
        .replace_mcp_server_fragments(
            &[(
                "a".into(),
                "[mcp_servers]\n\n[mcp_servers.a]\nurl = \"https://a/mcp\"\n".into(),
            )],
            "1",
        )
        .unwrap();

    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
}

#[test]
fn mcp_preview_empty_when_mirror_matches_live() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);

    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
    assert_eq!(preview.live_count, 1);
    assert_eq!(preview.db_count, 1);
}

/// 首次进入：现场静默成为基线，不产生"全是新增"的差异；另一端基线独立建立。
#[test]
fn mcp_first_entry_imports_live_as_baseline_without_diff() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.b]\nurl = \"https://b/mcp\"\n",
    );
    std::fs::write(
        context.paths.claude_mcp_config(),
        r#"{"mcpServers":{"c":{"type":"http","url":"https://c/mcp"}}}"#,
    )
    .unwrap();

    let preview = context.codex_mcp_sync_preview().unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);
    assert_eq!(preview.live_count, 2);
    assert_eq!(preview.db_count, 2);
    // Claude 端尚未建立基线：镜像里只有 Codex 两行
    let records = context.database.mcp_server_records().unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.codex_installed));
    assert!(records.iter().all(|record| !record.claude_installed));

    // 之后首次进 Claude 端：同样静默建基线，Codex 字段原样保留
    let claude_preview = context.mcp_sync_preview(SkillTool::Claude).unwrap();
    assert!(
        claude_preview.entries.is_empty(),
        "{:?}",
        claude_preview.entries
    );
    let records = context.database.mcp_server_records().unwrap();
    let c = records.iter().find(|record| record.name == "c").unwrap();
    assert!(c.claude_installed);
    assert!(c.claude_json.is_some());
    assert!(!c.codex_installed);
    assert!(records
        .iter()
        .filter(|record| record.name != "c")
        .all(|record| record.codex_installed));
}

/// 基线建立后，外部改动照常报差异：差异语义只属于漂移。
#[test]
fn mcp_drift_after_baseline_still_reports_diff() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    let preview = context.mcp_sync_preview(SkillTool::Codex).unwrap();
    assert!(preview.entries.is_empty(), "{:?}", preview.entries);

    std::fs::write(
        context.paths.codex_config(),
        "[mcp_servers.a]\nurl = \"https://a/v2\"\n",
    )
    .unwrap();
    let preview = context.mcp_sync_preview(SkillTool::Codex).unwrap();
    assert_eq!(preview.entries.len(), 1, "{:?}", preview.entries);
    assert_eq!(preview.entries[0].name, "a");
    assert_eq!(preview.entries[0].kind, McpSyncEntryKind::Changed);
}

/// 残缺条目导不进基线，留在差异页等人工处理。
#[test]
fn mcp_baseline_skips_unparseable_live_entries() {
    let (context, _home) = mcp_test_context(
        "[mcp_servers.a]\nurl = \"https://a/mcp\"\n\n[mcp_servers.broken]\ncustom = \"keep\"\n",
    );
    let preview = context.mcp_sync_preview(SkillTool::Codex).unwrap();
    assert_eq!(preview.entries.len(), 1, "{:?}", preview.entries);
    assert_eq!(preview.entries[0].name, "broken");
    assert_eq!(preview.entries[0].kind, McpSyncEntryKind::LiveOnly);
    let records = context.database.mcp_server_records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, "a");
}

#[test]
fn mcp_preview_fails_when_live_unparseable() {
    let (context, _home) = mcp_test_context("[mcp_servers.a]\nurl = \"https://a/mcp\"\n");
    import_mcp_from_live(&context);

    // live 无法解析：预览报错，前端进入“仅可从数据库恢复”降级模式
    std::fs::write(context.paths.codex_config(), "not [ valid").unwrap();
    assert!(context.codex_mcp_sync_preview().is_err());
}

#[test]
fn mcp_save_consolidates_scattered_live_section() {
    // 复刻 Codex 官方应用写入器留下的散落布局：保存后 mcp 必须收拢为连续块
    let (context, _home) = mcp_test_context(concat!(
        "[plugins.a]\nenabled = true\n\n",
        "[mcp_servers.old]\nurl = \"https://old\"\n\n",
        "[plugins.b]\nenabled = true\n\n",
        "[features]\njs = false\n",
    ));
    context
        .codex_save_mcp_server(
            None,
            McpServerSpec {
                name: "fresh".into(),
                url: Some("https://fresh/mcp".into()),
                ..Default::default()
            },
        )
        .unwrap();

    let config = read_config_text(&context);
    let pos = |needle: &str| {
        config
            .find(needle)
            .unwrap_or_else(|| panic!("缺少 {needle}：\n{config}"))
    };
    // 非 mcp 表保持原相对顺序
    assert!(pos("[plugins.a]") < pos("[plugins.b]"));
    assert!(
        pos("[mcp_servers.old]") < pos("[mcp_servers.fresh]"),
        "{config}"
    );
    // mcp 收拢为连续块：首尾 mcp 之间没有其他表混入
    let mcp_span = &config[pos("[mcp_servers.old]")..pos("[mcp_servers.fresh]")];
    assert!(
        !mcp_span.contains("[plugins.") && !mcp_span.contains("[features]"),
        "{config}"
    );
    // 未建模内容逐字保留
    assert!(config.contains("[plugins.b]"), "{config}");
    assert!(
        codex_config::parse_document(&config).is_ok(),
        "保存并收拢分散 MCP 段后必须保持 config.toml 可解析：\n{config}"
    );
}

#[test]
fn update_builtin_profile_writes_key_back_when_active() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"other\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("deepseek", None, None, Some("sk-old"), None, None)
        .unwrap();
    context
        .codex_set_profile_model(&profile.id, Some("chosen-model"), None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    assert_eq!(
        context
            .get_state()
            .unwrap()
            .active_codex_profile_id
            .as_deref(),
        Some(profile.id.as_str())
    );

    context
        .codex_update_profile(
            &profile.id,
            "DeepSeek 官方",
            None,
            Some("https://api.deepseek.com/"),
            Some("sk-real"),
            None,
        )
        .unwrap();

    // 使用中改密钥：只就地更新供应商段落，模板其余内容保持不变
    let config = String::from_utf8(std::fs::read(context.paths.codex_config()).unwrap()).unwrap();
    assert!(config.contains("model = \"chosen-model\""));
    assert!(config.contains("experimental_bearer_token = \"sk-real\""));
    assert!(!config.contains("sk-old"));
    assert!(!config.contains("<YOUR_API_KEY>"));

    let detail = context.codex_get_profile(&profile.id).unwrap();
    assert_eq!(detail.api_key.as_deref(), Some("sk-real"));
    // 所见即所得：编辑器直接展示真实密钥
    assert!(detail
        .config_fragment
        .contains(r#"experimental_bearer_token = "sk-real""#));
}

#[test]
fn unused_builtin_edit_save_writes_db_only_without_key_prompt() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let original = "model = \"other\"\n";
    std::fs::write(paths.codex_config(), original).unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("deepseek", None, None, Some("sk-test"), None, None)
        .unwrap();

    // 未使用：编辑保存只写库，不要求密钥占位符、不碰 live 配置
    let detail = context.codex_get_profile(&profile.id).unwrap();
    let edited = detail.config_fragment.replace("sk-test", "sk-edited");
    context
        .codex_update_profile(
            &profile.id,
            "DeepSeek",
            None,
            Some("https://api.deepseek.com/"),
            Some("sk-edited"),
            None,
        )
        .unwrap();
    let updated = context
        .codex_update_profile_config(&profile.id, &edited, None, None)
        .unwrap();
    assert!(updated.raw_config.as_deref().unwrap().contains("sk-edited"));
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_config()).unwrap(),
        original
    );
}

#[test]
fn keyless_builtin_saves_to_db_but_apply_requires_key() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"other\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("deepseek", None, None, None, None, None)
        .unwrap();
    assert!(!profile.has_key);
    let detail = context.codex_get_profile(&profile.id).unwrap();
    assert_eq!(detail.api_key.as_deref(), None);
    assert!(detail.config_fragment.contains("<YOUR_API_KEY>"));

    let error = context.codex_apply_profile(&profile.id).unwrap_err();
    assert!(error.0.contains("尚未配置 API Key"));
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_config()).unwrap(),
        "model = \"other\"\n"
    );
}

#[test]
fn active_builtin_save_without_placeholder_keeps_edited_text() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"other\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("deepseek", None, None, Some("sk-test"), None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    // 编辑文本里用户已把占位符改成真实密钥：保存不再报“缺少密钥占位符”
    let edited = r#"
model = "deepseek-flash"
model_provider = "deepseek"
preferred_auth_method = "apikey"
forced_login_method = "api"
model_reasoning_effort = "high"
web_search = "disabled"
model_catalog_json = "~/.codex/models.json"

[model_providers.deepseek]
name = "deepseek"
base_url = "https://api.deepseek.com/"
wire_api = "responses"
experimental_bearer_token = "sk-in-editor"
"#;
    context
        .codex_update_profile_config(&profile.id, edited, None, None)
        .unwrap();
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live.contains("sk-in-editor"));
    assert!(!live.contains("<YOUR_API_KEY>"));
}

#[test]
fn builtin_catalogs_are_not_mixed() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"other\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let deepseek = context
        .codex_add_builtin_profile("deepseek", None, None, Some("sk-d"), None, None)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let zhipu = context
        .codex_add_builtin_profile("zhipu", None, None, Some("sk-z"), None, None)
        .unwrap();

    context.codex_apply_profile(&deepseek.id).unwrap();
    assert_eq!(
        std::fs::read(context.paths.codex_home.join("models.json")).unwrap(),
        crate::codex_builtin::DEEPSEEK_MODELS
    );

    context.codex_apply_profile(&zhipu.id).unwrap();
    assert_eq!(
        std::fs::read(context.paths.codex_home.join("models.json")).unwrap(),
        crate::codex_builtin::ZHIPU_MODELS
    );
}

#[test]
fn apply_minimax_inserts_catalog_line_and_writes_catalog() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"other\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("minimax", None, None, Some("mm-key"), None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    let config = std::fs::read(context.paths.codex_config()).unwrap();
    let rendered = crate::codex_builtin::template("minimax")
        .unwrap()
        .render_config(Some("mm-key"))
        .unwrap();
    // 应用经 toml_edit 往返（MCP 段携带路径）：live 无 MCP 时内容不变、仅补行尾换行
    let live = codex_config::parse_document("model = \"other\"\n").unwrap();
    let expected =
        codex_config::merge_mcp_section(&String::from_utf8_lossy(&rendered), &live).into_bytes();
    assert_eq!(config, expected);
    assert!(String::from_utf8_lossy(&config)
        .contains("model_catalog_json = \"~/.codex/model-catalogs/custom-catalog.json\""));
    assert!(!String::from_utf8_lossy(&config).contains("<YOUR_API_KEY>"));

    let catalog = std::fs::read(
        context
            .paths
            .codex_home
            .join("model-catalogs")
            .join("custom-catalog.json"),
    )
    .unwrap();
    assert_eq!(catalog, crate::codex_builtin::MINIMAX_CATALOG);
}

#[test]
fn apply_chatgpt_writes_official_default_and_clears_empty_desktop_auth() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        "model_provider = \"ZAI\"\nmodel = \"glm-5.3\"\n\n[model_providers.ZAI]\nname = \"ZAI\"\nbase_url = \"https://open.bigmodel.cn/api/v1\"\nexperimental_bearer_token = \"old-key\"\n",
    )
    .unwrap();
    std::fs::write(paths.codex_home.join("auth.json"), b"{\"login\":\"kept\"}").unwrap();

    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, None)
        .unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    assert_eq!(
        std::fs::read(context.paths.codex_config()).unwrap(),
        crate::codex_builtin::CHATGPT_CONFIG
    );
    assert!(!context.paths.codex_home.join("auth.json").exists());
    assert!(!context.paths.codex_home.join("models.json").exists());
}

#[test]
fn builtin_placeholder_key_is_not_exposed_as_api_key() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();

    let context = AppContext::new(paths).unwrap();
    // 占位符密钥不算已填写：codex_get_profile 不应把占位符当成密钥回显
    let payload = CodexProfilePayload {
        builtin: Some("deepseek".into()),
        model_values: [
            ("model".to_string(), "\"deepseek-flash\"".into()),
            ("model_reasoning_effort".to_string(), "\"high\"".into()),
            ("model_catalog_json".to_string(), "\"~/.codex/models.json\"".into()),
        ]
        .into_iter()
        .collect(),
        provider_id: Some("deepseek".into()),
        provider_body: Some(
            "name = \"deepseek\"\nbase_url = \"https://api.deepseek.com/\"\nwire_api = \"responses\"\nexperimental_bearer_token = \"<YOUR_API_KEY>\""
                .into(),
        ),
        ..Default::default()
    };
    let summary = context
        .database
        .codex_insert_profile("DeepSeek 占位符", &payload, &now_ms().to_string())
        .unwrap();

    let detail = context.codex_get_profile(&summary.id).unwrap();
    assert_eq!(detail.api_key, None);
    assert!(detail.config_fragment.contains("<YOUR_API_KEY>"));
    let state = context.get_state().unwrap();
    let stored_summary = state
        .codex_profiles
        .iter()
        .find(|item| item.id == summary.id)
        .unwrap();
    assert!(!stored_summary.has_key);
}

#[test]
fn update_profile_config_saves_captured_fragment_as_structured() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://old.example"
experimental_bearer_token = "secret"
"#,
    )
    .unwrap();
    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("GLM").unwrap();

    let edited = r#"
model = "glm-5.5"
model_provider = "ZAI"
model_reasoning_effort = "medium"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://new.example"
experimental_bearer_token = "new-key"
"#;
    let detail = context
        .codex_update_profile_config(&profile.id, edited, None, None)
        .unwrap();
    assert_eq!(detail.raw_config.as_deref(), Some(edited));
    assert_eq!(
        detail
            .model_values
            .get("model")
            .map(|v| v.trim().trim_matches('"')),
        Some("glm-5.5")
    );
    assert!(detail.config_fragment.contains("https://new.example"));

    context.codex_apply_profile(&profile.id).unwrap();
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live.contains("glm-5.5"));
    assert!(live.contains("https://new.example"));
}

#[test]
fn clearing_catalog_text_untracks_custom_catalog_and_restores_builtin_models_json() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let context = AppContext::new(paths).unwrap();
    let profile = context
        .codex_add_builtin_profile(
            crate::codex_builtin::KIND_DEEPSEEK,
            None,
            None,
            Some("sk-test"),
            None,
            None,
        )
        .unwrap();
    std::fs::write(context.paths.codex_config(), "model = \"other\"\n").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    let catalog_path = context.paths.codex_home.join("models.json");
    assert!(catalog_path.exists());
    std::fs::write(&catalog_path, b"{\"models\": []}").unwrap();

    let config_text = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    let detail = context
        .codex_update_profile_config(&profile.id, &config_text, Some(""), None)
        .unwrap();

    // live 文件立即回写内置资产而不是留着用户改过的内容；激活档 codex_get_profile 再把
    // live 回写进快照，所以编辑器看到的是内置目录原文
    let live = std::fs::read_to_string(&catalog_path).unwrap();
    assert_eq!(live.as_bytes(), crate::codex_builtin::DEEPSEEK_MODELS);
    assert_eq!(
        detail.raw_catalog.as_deref().map(str::as_bytes),
        Some(crate::codex_builtin::DEEPSEEK_MODELS)
    );
}

#[test]
fn updating_active_profile_config_preserves_computer_use_server() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        "model = \"glm-5.3\"\n\n[mcp_servers.computer-use]\ncommand = \"SkyComputerUseClient\"\nenabled = true\n",
    )
    .unwrap();
    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("GLM").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();

    context
        .codex_update_profile_config(&profile.id, "model = \"glm-5.5\"\n", None, None)
        .unwrap();

    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live.contains("[mcp_servers.computer-use]"), "{live}");
    assert!(live.contains("enabled = true"), "{live}");
}

#[test]
fn update_profile_config_follows_edited_provider_and_detaches_builtin() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"

[model_providers.ZAI]
name = "ZAI"
"#,
    )
    .unwrap();
    let context = AppContext::new(paths).unwrap();

    // 捕获配置：model_provider 指向不存在的段 → 宽容保存，段体留空
    let profile = context.codex_capture_profile("GLM").unwrap();
    let saved = context
        .codex_update_profile_config(
            &profile.id,
            "model = \"glm-5.3\"\nmodel_provider = \"ZAI\"\n",
            None,
            None,
        )
        .unwrap();
    assert_eq!(saved.provider.as_deref(), Some("ZAI"));
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .provider_body,
        None
    );

    // 捕获配置：供应商名与段一致改名 → 供应商身份跟随配置
    let updated = context
        .codex_update_profile_config(
            &profile.id,
            "model = \"glm-5.3\"\nmodel_provider = \"OTHER\"\n\n[model_providers.OTHER]\nname = \"OTHER\"\n",
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.provider.as_deref(), Some("OTHER"));
    assert_eq!(
        context
            .database
            .codex_profile(&profile.id)
            .unwrap()
            .payload
            .provider_id
            .as_deref(),
        Some("OTHER")
    );

    // 内置配置：改名后脱离内置模板，按完整配置快照处理
    let builtin = context
        .codex_add_builtin_profile("zhipu", None, None, Some("sk-test"), None, None)
        .unwrap();
    let updated = context
        .codex_update_profile_config(
            &builtin.id,
            "model = \"glm-5.3\"\nmodel_provider = \"OTHER\"\n\n[model_providers.OTHER]\nname = \"OTHER\"\nbase_url = \"https://api.example\"\nexperimental_bearer_token = \"sk-test\"\n",
            None,
            None,
        )
        .unwrap();
    assert_eq!(updated.provider.as_deref(), Some("OTHER"));
    let stored = context.database.codex_profile(&builtin.id).unwrap();
    assert_eq!(stored.payload.provider_id.as_deref(), Some("OTHER"));
    assert_eq!(stored.payload.builtin, None);
}

#[test]
fn update_profile_config_builtin_raw_applies_with_key_and_catalog() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let context = AppContext::new(paths).unwrap();
    std::fs::write(context.paths.codex_config(), "model = \"other\"\n").unwrap();
    let profile = context
        .codex_add_builtin_profile("zhipu", None, None, Some("sk-test"), None, None)
        .unwrap();

    let edited = r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "max"
model_catalog_json = "~/.codex/models.json"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://open.bigmodel.cn/api/v1"
experimental_bearer_token = "sk-test"
extra = "edited"
"#;
    let catalog = r#"{"models":[{"id":"glm-5.3","name":"GLM 5.3"}]}"#;
    let detail = context
        .codex_update_profile_config(&profile.id, edited, Some(catalog), None)
        .unwrap();
    assert_eq!(detail.raw_config.as_deref(), Some(edited));

    context.codex_apply_profile(&profile.id).unwrap();
    let live = std::fs::read_to_string(context.paths.codex_config()).unwrap();
    assert!(live.contains("extra = \"edited\""));
    assert!(live.contains(r#"experimental_bearer_token = "sk-test""#));
    assert_eq!(
        std::fs::read_to_string(context.paths.codex_home.join("models.json")).unwrap(),
        catalog
    );
}

#[test]
fn autosync_preserves_profile_edited_catalog() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    let context = AppContext::new(paths).unwrap();
    let write = |text: &str| std::fs::write(context.paths.codex_config(), text).unwrap();

    write(
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_catalog_json = "zai.json"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
"#,
    );
    let profile_a = context.codex_capture_profile("A").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    write(
        r#"
model = "other-model"
model_provider = "ZAI"
model_catalog_json = "zai.json"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
"#,
    );
    let profile_b = context.codex_capture_profile("B").unwrap();

    let detail_a = context.codex_get_profile(&profile_a.id).unwrap();
    let catalog = r#"{"models":[{"id":"edited"}]}"#;
    context
        .codex_update_profile_config(
            &profile_a.id,
            &detail_a.config_fragment,
            Some(catalog),
            None,
        )
        .unwrap();

    context.codex_apply_profile(&profile_b.id).unwrap();
    let stored_a = context.database.codex_profile(&profile_a.id).unwrap();
    assert_eq!(stored_a.payload.raw_catalog.as_deref(), Some(catalog));
}

#[test]
fn profile_description_survives_live_sync_and_can_be_cleared() {
    let (_home, context) = chatgpt_test_context();
    let profile = context
        .codex_add_builtin_profile("chatgpt", Some("  工作配置  "), None, None, None, None)
        .unwrap();
    assert_eq!(
        context
            .codex_get_profile(&profile.id)
            .unwrap()
            .description
            .as_deref(),
        Some("工作配置")
    );

    context.codex_apply_profile(&profile.id).unwrap();
    let active = context.get_state().unwrap();
    let active_profile = active
        .codex_profiles
        .iter()
        .find(|item| item.id == profile.id)
        .unwrap();
    assert!(serde_json::to_value(active_profile)
        .unwrap()
        .get("description")
        .is_none());
    assert_eq!(
        context
            .codex_get_profile(&profile.id)
            .unwrap()
            .description
            .as_deref(),
        Some("工作配置")
    );

    context
        .codex_update_profile(&profile.id, &profile.name, Some("新描述"), None, None, None)
        .unwrap();
    assert_eq!(
        context
            .codex_get_profile(&profile.id)
            .unwrap()
            .description
            .as_deref(),
        Some("新描述")
    );
    context
        .codex_update_profile(&profile.id, &profile.name, None, None, None, None)
        .unwrap();
    assert_eq!(
        context
            .codex_get_profile(&profile.id)
            .unwrap()
            .description
            .as_deref(),
        Some("新描述")
    );
    context
        .codex_update_profile(&profile.id, &profile.name, Some("  "), None, None, None)
        .unwrap();
    assert_eq!(
        context.codex_get_profile(&profile.id).unwrap().description,
        None
    );
    assert!(context
        .codex_update_profile(
            &profile.id,
            &profile.name,
            Some(&"x".repeat(201)),
            None,
            None,
            None
        )
        .is_err());

    let custom = context
        .codex_add_custom_profile(
            "自定义",
            Some("备注"),
            "model = \"gpt-5.6\"\n",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        context
            .codex_get_profile(&custom.id)
            .unwrap()
            .description
            .as_deref(),
        Some("备注")
    );
}

#[test]
fn update_profile_saves_and_clears_admin_url() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(paths.codex_config(), "model = \"glm-5.3\"\n").unwrap();
    let context = AppContext::new(paths).unwrap();
    let profile = context.codex_capture_profile("GLM").unwrap();

    let summary = context
        .codex_update_profile(
            &profile.id,
            "GLM",
            None,
            None,
            None,
            Some("https://console.example.com"),
        )
        .unwrap();
    assert_eq!(
        summary.admin_url.as_deref(),
        Some("https://console.example.com")
    );

    let invalid = context
        .codex_update_profile(
            &profile.id,
            "GLM",
            None,
            None,
            None,
            Some("console.example.com"),
        )
        .unwrap_err();
    assert!(invalid.0.contains("http"));

    context
        .codex_update_profile(&profile.id, "GLM", None, None, None, Some(""))
        .unwrap();
    let detail = context.codex_get_profile(&profile.id).unwrap();
    assert_eq!(detail.admin_url, None);
}

#[test]
fn duplicate_profile_copies_payload_with_copy_suffix_and_inserts_after_source() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
"#,
    )
    .unwrap();
    let context = AppContext::new(paths).unwrap();
    let before = context.codex_capture_profile("Before").unwrap();
    let profile = context.codex_capture_profile("GLM").unwrap();
    let after = context.codex_capture_profile("After").unwrap();
    context
        .codex_reorder_profiles(&[before.id.clone(), profile.id.clone(), after.id.clone()])
        .unwrap();
    context
        .codex_update_profile(
            &profile.id,
            "GLM",
            None,
            None,
            None,
            Some("https://console.example.com"),
        )
        .unwrap();
    context
        .codex_set_profile_icon(&profile.id, Some("zhipu"))
        .unwrap();

    let dup = context.codex_duplicate_profile(&profile.id).unwrap();
    assert_eq!(dup.name, "GLM copy");
    let ordered_names: Vec<String> = context
        .database
        .codex_profiles()
        .unwrap()
        .into_iter()
        .map(|item| item.name)
        .collect();
    assert_eq!(ordered_names, ["Before", "GLM", "GLM copy", "After"]);
    assert_eq!(
        dup.admin_url.as_deref(),
        Some("https://console.example.com")
    );
    assert_eq!(dup.icon.as_deref(), Some("zhipu"));
    let original = context.database.codex_profile(&profile.id).unwrap();
    let copied = context.database.codex_profile(&dup.id).unwrap();
    assert_eq!(copied.payload, original.payload);

    std::thread::sleep(std::time::Duration::from_millis(2));
    let dup2 = context.codex_duplicate_profile(&profile.id).unwrap();
    assert_eq!(dup2.name, "GLM copy 2");
}

#[test]
fn duplicate_profile_copies_live_auth_and_account_binding() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    std::fs::write(
        paths.codex_config(),
        r#"
model = "glm-5.3"
model_provider = "ZAI"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.example"
"#,
    )
    .unwrap();
    std::fs::write(
        paths.codex_home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-live"}"#,
    )
    .unwrap();
    let context = AppContext::new(paths).unwrap();
    // 捕获的第三方供应商默认未使用；显式应用后复制时应带上当前 live auth.json
    let profile = context.codex_capture_profile("GLM").unwrap();
    context.codex_apply_profile(&profile.id).unwrap();
    assert!(context
        .database
        .codex_profile(&profile.id)
        .unwrap()
        .payload
        .raw_auth
        .is_none());
    let dup = context.codex_duplicate_profile(&profile.id).unwrap();
    assert_eq!(
        context
            .database
            .codex_profile(&dup.id)
            .unwrap()
            .payload
            .raw_auth
            .as_deref(),
        Some(r#"{"OPENAI_API_KEY":"sk-live"}"#)
    );

    // 官方供应商：订阅账号绑定一并复制
    context
        .database
        .upsert_account(&crate::database::StoredAccount {
            id: "acc-1".into(),
            email: Some("a@example.com".into()),
            id_token: None,
            refresh_token: "rt".into(),
            auth_json: None,
            chatgpt_account_id: Some("acc-1".into()),
            user_identity: None,
            plan_type: None,
            authenticated_at: 1,
        })
        .unwrap();
    let official = context
        .codex_add_builtin_profile("chatgpt", None, None, None, None, Some("acc-1"))
        .unwrap();
    let dup2 = context.codex_duplicate_profile(&official.id).unwrap();
    assert_eq!(
        context
            .database
            .codex_profile(&dup2.id)
            .unwrap()
            .account_id
            .as_deref(),
        Some("acc-1")
    );
}

#[test]
fn path_info_includes_log_dir_and_open_path_allows_it() {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    assert!(paths.logs.exists());
    let context = AppContext::new(paths).unwrap();

    let info = context.path_info();
    let logs = info
        .iter()
        .find(|item| item.label == "about.paths.logs")
        .expect("path_info 应包含日志目录条目");
    assert!(logs.path.ends_with("logs"));
    // 白名单由 path_info 驱动：日志目录出现即可被 open_path 打开
    assert!(context.is_managed_path(&logs.path));
}
