//! Claude Desktop（官方桌面应用，3P 第三方推理模式）供应商管理。
//!
//! 桌面端不读 `~/.claude/settings.json`：第三方接入写独立的 3P 配置
//! （macOS `~/Library/Application Support/Claude-3p/`，Windows `%LOCALAPPDATA%` 同构；
//! Store/MSIX 版被文件系统虚拟化到 `Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\`，
//! 见 cc-switch#4842 的根因分析）。生效要求完全退出重启桌面端（无热重载）。
//! 模型名约束：桌面端强校验 `inferenceModels[].name` 必须是 Anthropic 目录真实
//! claude-* 模型名，并把该名原样发给上游——Phase 1 只支持服务端认 claude 名的
//! 直连供应商（如 DeepSeek 的官方映射），其余供应商需要 Phase 2 的本地映射网关。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::claude_desktop_gateway::{self, GatewayConfig};
use super::sync;
use super::AppContext;
use crate::codex::process as codex_process;
use crate::database::StoredClaudeDesktopProfile;
use crate::error::{app_err, AppResult, ProfileApplyError};
use crate::fsutil::{atomic_write, backup_file};
use crate::models::{
    ClaudeDesktopAppStatus, ClaudeDesktopModelSlot, ClaudeDesktopProfileDetail,
    ClaudeDesktopProfileInput, ClaudeDesktopProfileSummary,
};
use crate::paths::{home_dir, now_ms};

const CONFIG_FILE_NAME: &str = "claude_desktop_config.json";
const LIBRARY_DIR_NAME: &str = "configLibrary";
const META_FILE_NAME: &str = "_meta.json";
/// Windows Store/MSIX 版 Claude Desktop 的包族名（配置路径虚拟化）。
#[cfg(windows)]
const WINDOWS_PACKAGE_DIR: &str = "Claude_pzs8sxrjxfjjc";
#[cfg(windows)]
const WINDOWS_CLAUDE_AUMIDS: &[&str] = &["Claude_pzs8sxrjxfjjc!Claude", "Claude_pzs8sxrjxfjjc!App"];
const SLOT_ORDER: [&str; 4] = ["sonnet", "opus", "haiku", "fable"];
/// 路由模式下每个槽位的固定目录名（桌面端只认 Anthropic 目录真名）。
fn canonical_catalog(slot: &str) -> Option<&'static str> {
    match slot {
        "sonnet" => Some("claude-sonnet-5-5"),
        "opus" => Some("claude-opus-5-5"),
        "haiku" => Some("claude-haiku-5-5"),
        "fable" => Some("claude-fable-5-1"),
        _ => None,
    }
}

/// 3P 配置四件套的落点。
#[derive(Debug, Clone)]
pub(crate) struct Desktop3pPaths {
    /// `Claude/claude_desktop_config.json`：写 deploymentMode，保留 mcpServers 等用户键。
    pub config: PathBuf,
    /// `Claude-3p/claude_desktop_config.json`：同上。
    pub p3_config: PathBuf,
    /// `Claude-3p/configLibrary/`：profile 本体与 _meta.json。
    pub library: PathBuf,
}

impl Desktop3pPaths {
    fn from_base(base: &Path) -> Self {
        Self {
            config: base.join("Claude").join(CONFIG_FILE_NAME),
            p3_config: base.join("Claude-3p").join(CONFIG_FILE_NAME),
            library: base.join("Claude-3p").join(LIBRARY_DIR_NAME),
        }
    }

    fn profile(&self, config_uuid: &str) -> PathBuf {
        self.library.join(format!("{config_uuid}.json"))
    }

    fn meta(&self) -> PathBuf {
        self.library.join(META_FILE_NAME)
    }
}

/// 解析 3P 目录。macOS 唯一候选；Windows 优先 MSIX 虚拟化路径
/// （目录存在才用，零注册表依赖），否则按官网安装的 `%LOCALAPPDATA%` 根。
pub(crate) fn resolve_desktop3p(base: &Path) -> Desktop3pPaths {
    #[cfg(windows)]
    {
        let msix = base
            .join("Packages")
            .join(WINDOWS_PACKAGE_DIR)
            .join("LocalCache")
            .join("Local");
        let selected = if msix.join("Claude-3p").is_dir() || msix.join("Claude").is_dir() {
            msix
        } else {
            base.to_path_buf()
        };
        Desktop3pPaths::from_base(&selected)
    }
    #[cfg(not(windows))]
    {
        Desktop3pPaths::from_base(base)
    }
}

/// `configLibrary/_meta.json`：appliedId 指认当前生效的 profile。
#[derive(Debug, Default, Serialize, Deserialize)]
struct MetaFile {
    #[serde(default, rename = "appliedId", skip_serializing_if = "Option::is_none")]
    applied_id: Option<String>,
    #[serde(default)]
    entries: Vec<MetaEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetaEntry {
    id: String,
    name: String,
}

/// profile 本体的托管字段（capture 与漂移回写的解析面）。
#[derive(Debug, Deserialize)]
struct ProfileFile {
    #[serde(default, rename = "inferenceGatewayBaseUrl")]
    base_url: Option<String>,
    #[serde(default, rename = "inferenceGatewayApiKey")]
    api_key: Option<String>,
    #[serde(default, rename = "inferenceGatewayAuthScheme")]
    auth_scheme: Option<String>,
    #[serde(default, rename = "inferenceModels")]
    inference_models: Vec<Value>,
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// 从库行渲染 profile 本体。字段集对齐 cc-switch 逆向与官方 configuration reference；
/// 上游未知键不保留（桌面端 GUI 的自管理键不属于 Agmux 托管域）。
/// 路由模式下 base_url/api_key 换成本地网关地址与令牌（override 一对即生效）。
fn render_profile(
    profile: &StoredClaudeDesktopProfile,
    gateway: Option<(&str, &str)>,
) -> AppResult<Vec<u8>> {
    let (base_url, api_key) = match gateway {
        Some((url, token)) => (url.to_string(), token.to_string()),
        None => (
            profile.base_url.clone().unwrap_or_default(),
            profile.api_key.clone().unwrap_or_default(),
        ),
    };
    let mut document = serde_json::Map::new();
    document.insert("inferenceProvider".into(), json!("gateway"));
    document.insert("inferenceCredentialKind".into(), json!("static"));
    document.insert("inferenceGatewayBaseUrl".into(), json!(base_url));
    document.insert("inferenceGatewayApiKey".into(), json!(api_key));
    document.insert(
        "inferenceGatewayAuthScheme".into(),
        json!(profile.auth_scheme),
    );
    document.insert(
        "inferenceModels".into(),
        Value::Array(render_models(&profile.models)),
    );
    document.insert("coworkEgressAllowedHosts".into(), json!(["*"]));
    document.insert("disableDeploymentModeChooser".into(), json!(true));
    document.insert("modelDiscoveryEnabled".into(), json!(false));
    serde_json::to_vec_pretty(&Value::Object(document))
        .map_err(|error| app_err!("Claude Desktop 配置序列化失败: {error}"))
}

fn parse_fetched_models(text: &Option<String>) -> Vec<String> {
    text.as_deref()
        .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
        .unwrap_or_default()
}

fn new_gateway_token() -> AppResult<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|error| app_err!("无法生成路由网关令牌: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn parse_models(text: &Option<String>) -> Vec<ClaudeDesktopModelSlot> {
    text.as_deref()
        .and_then(|text| serde_json::from_str::<Vec<ClaudeDesktopModelSlot>>(text).ok())
        .unwrap_or_default()
}

fn render_models(models: &Option<String>) -> Vec<Value> {
    parse_models(models)
        .into_iter()
        .filter(|slot| !slot.name.trim().is_empty())
        .map(|slot| {
            let mut entry = serde_json::Map::new();
            entry.insert("name".into(), json!(slot.name));
            if let Some(label) = slot.label_override.filter(|label| !label.trim().is_empty()) {
                entry.insert("labelOverride".into(), json!(label));
            }
            if slot.supports_1m {
                entry.insert("supports1m".into(), json!(true));
            }
            Value::Object(entry)
        })
        .collect()
}

/// live 的 inferenceModels 反解为库内形态：槽位按声明顺序映射 sonnet/opus/haiku/fable。
fn parse_live_models(values: &[Value]) -> Vec<ClaudeDesktopModelSlot> {
    values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let (name, label_override, supports_1m) = match value {
                Value::String(name) => (name.clone(), None, false),
                Value::Object(object) => (
                    object.get("name")?.as_str()?.to_string(),
                    object
                        .get("labelOverride")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    object
                        .get("supports1m")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ),
                _ => return None,
            };
            if name.trim().is_empty() {
                return None;
            }
            Some(ClaudeDesktopModelSlot {
                slot: SLOT_ORDER.get(index)?.to_string(),
                name,
                label_override,
                supports_1m,
                upstream_model: None,
            })
        })
        .collect()
}

/// 读 JSON 对象文件：不存在 → 空对象；存在但非对象 / 解析失败 → 报错（由损坏守卫拦截）。
fn read_json_object(path: &Path, label: &str) -> AppResult<serde_json::Map<String, Value>> {
    match std::fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(serde_json::Map::new()),
        Ok(raw) => match serde_json::from_str::<Value>(&raw) {
            Ok(Value::Object(object)) => Ok(object),
            Ok(_) => Err(app_err!("{label} 不是 JSON 对象")),
            Err(error) => Err(app_err!("{label} 不是有效 JSON: {error}")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::Map::new()),
        Err(error) => Err(app_err!("无法读取 {label}: {error}")),
    }
}

/// 合并写 deploymentMode：保留 mcpServers 等用户键；值未变不重写。
fn merge_deployment_mode(path: &Path, mode: &str) -> AppResult<()> {
    let mut document = read_json_object(path, CONFIG_FILE_NAME)?;
    if document.get("deploymentMode").and_then(Value::as_str) == Some(mode) {
        return Ok(());
    }
    document.insert("deploymentMode".into(), json!(mode));
    let bytes = serde_json::to_vec_pretty(&Value::Object(document))
        .map_err(|error| app_err!("{CONFIG_FILE_NAME} 序列化失败: {error}"))?;
    atomic_write(path, &bytes)
}

fn read_meta(path: &Path) -> AppResult<Option<MetaFile>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<MetaFile>(&raw)
            .map(Some)
            .map_err(|error| app_err!("_meta.json 不是有效 JSON: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(app_err!("无法读取 _meta.json: {error}")),
    }
}

fn write_meta(meta: &MetaFile, path: &Path) -> AppResult<()> {
    let bytes = serde_json::to_vec_pretty(meta)
        .map_err(|error| app_err!("_meta.json 序列化失败: {error}"))?;
    atomic_write(path, &bytes)
}

/// 生成 configLibrary 文件名用的 UUID v4（桌面端样例均为 UUID 形态，保守对齐）。
fn new_config_uuid() -> AppResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| app_err!("无法生成配置标识: {error}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn summary(profile: &StoredClaudeDesktopProfile) -> ClaudeDesktopProfileSummary {
    ClaudeDesktopProfileSummary {
        id: profile.id.clone(),
        name: profile.name.clone(),
        base_url: profile.base_url.clone(),
        has_key: profile
            .api_key
            .as_deref()
            .is_some_and(|key| !key.is_empty()),
        model: parse_models(&profile.models)
            .into_iter()
            .map(|slot| slot.name)
            .next(),
        kind: profile.kind.clone(),
        admin_url: profile.admin_url.clone(),
        icon: profile.icon.clone(),
        show_balance: profile.show_balance,
        mode: profile.mode.clone(),
        updated_at: profile.updated_at.clone(),
    }
}

fn detail(profile: StoredClaudeDesktopProfile) -> ClaudeDesktopProfileDetail {
    ClaudeDesktopProfileDetail {
        id: profile.id,
        name: profile.name,
        kind: profile.kind,
        base_url: profile.base_url,
        api_key: profile.api_key,
        auth_scheme: profile.auth_scheme,
        models: parse_models(&profile.models),
        admin_url: profile.admin_url,
        icon: profile.icon,
        show_balance: profile.show_balance,
        description: profile.description,
        mode: profile.mode,
        upstream_base_url: profile.upstream_base_url,
        upstream_api_key: profile.upstream_api_key,
        fetched_models: parse_fetched_models(&profile.fetched_models),
        updated_at: profile.updated_at,
    }
}

impl AppContext {
    /// 停掉当前路由网关（若有）。拿不到锁说明持锁线程在写，跳过等它自己收尾。
    fn stop_desktop_gateway(&self) {
        if let Ok(mut guard) = self.desktop_gateway.lock() {
            if let Some(handle) = guard.take() {
                handle.stop();
            }
        }
    }

    /// 本机 3P 目录（根候选挂在 AppPaths，随测试注入 home）。
    fn desktop3p(&self) -> Desktop3pPaths {
        resolve_desktop3p(&self.paths.claude_desktop_base)
    }

    /// 每帧刷新的应用状态（get_state 与 claude_desktop_get_status 命令共用）。
    pub fn claude_desktop_status(&self) -> ClaudeDesktopAppStatus {
        claude_desktop_app_status(&self.paths.claude_desktop_base)
    }

    pub fn claude_desktop_list(&self) -> AppResult<Vec<ClaudeDesktopProfileSummary>> {
        let _ = self.sync_active_claude_desktop();
        Ok(self
            .database
            .claude_desktop_profiles()?
            .iter()
            .map(summary)
            .collect())
    }

    pub fn claude_desktop_get(&self, id: &str) -> AppResult<ClaudeDesktopProfileDetail> {
        if self.database.active_claude_desktop_profile()?.as_deref() == Some(id) {
            let _ = sync::registry().harvest(
                self,
                &sync::SyncTrigger::BeforeProfileRead {
                    client: sync::ClientId::ClaudeDesktop,
                    target_id: id.to_string(),
                },
                sync::SyncMaterial::default(),
            );
        }
        Ok(detail(self.database.claude_desktop_profile(id)?))
    }

    /// 捕获现有 3P 配置为新供应商：沿用现文件的 uuid（再应用不换文件名），不改变 live。
    pub fn claude_desktop_capture(&self, name: &str) -> AppResult<ClaudeDesktopProfileDetail> {
        let paths = self.desktop3p();
        let meta = read_meta(&paths.meta())?.ok_or_else(|| {
            app_err!("尚未检测到 Claude Desktop 第三方推理配置，请先在应用内启用或应用一个供应商")
        })?;
        let applied = meta
            .applied_id
            .ok_or_else(|| app_err!("_meta.json 缺少 appliedId，无法捕获"))?;
        let profile_path = paths.profile(&applied);
        let raw = std::fs::read_to_string(&profile_path)
            .map_err(|error| app_err!("读取 Claude Desktop 配置失败: {error}"))?;
        let file: ProfileFile = serde_json::from_str(&raw)
            .map_err(|error| app_err!("Claude Desktop 配置不是有效 JSON: {error}"))?;
        let input = ClaudeDesktopProfileInput {
            name: name.to_string(),
            base_url: file.base_url,
            api_key: file.api_key,
            auth_scheme: file.auth_scheme,
            models: Some(parse_live_models(&file.inference_models)),
            ..Default::default()
        };
        let created = self.claude_desktop_insert(&input, &applied)?;
        sync::registry().harvest(
            self,
            &sync::SyncTrigger::AfterCapture {
                client: sync::ClientId::ClaudeDesktop,
            },
            sync::SyncMaterial::default(),
        )?;
        Ok(detail(created))
    }

    /// 新增或更新；更新激活中的配置时立即重写 profile 文件（对齐 claude_save 语义）。
    pub fn claude_desktop_save(
        &self,
        id: Option<&str>,
        mut input: ClaudeDesktopProfileInput,
    ) -> AppResult<ClaudeDesktopProfileDetail> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        input.name = super::validated_name(&input.name)?;
        input.base_url = clean(input.base_url);
        input.api_key = clean(input.api_key);
        input.kind = clean(input.kind);
        input.admin_url = clean(input.admin_url);
        input.icon = clean(input.icon);
        let scheme = match input.auth_scheme.as_deref() {
            None => "bearer".to_string(),
            Some(scheme @ ("bearer" | "x-api-key")) => scheme.to_string(),
            Some(other) => return Err(app_err!("不支持的鉴权方式：{other}")),
        };
        input.auth_scheme = Some(scheme);
        let route = input.mode.as_deref() == Some("route");
        if let Some(models) = &mut input.models {
            for slot in models {
                if !SLOT_ORDER.contains(&slot.slot.as_str()) {
                    return Err(app_err!("无效的模型槽位：{}", slot.slot));
                }
                if route {
                    // 路由槽位名由后端定为目录名：前端填的是 upstream_model，名字不信任客户端
                    if let Some(catalog) = canonical_catalog(&slot.slot) {
                        slot.name = catalog.to_string();
                    }
                }
                if !slot.name.starts_with("claude-") {
                    return Err(app_err!(
                        "模型名 {} 不是 claude-* 形态：桌面端会拒绝非 Anthropic 目录模型名",
                        slot.name
                    ));
                }
            }
        }
        let timestamp = now_ms().to_string();
        let stored = match id {
            Some(id) => {
                let is_active =
                    self.database.active_claude_desktop_profile()?.as_deref() == Some(id);
                let profile_path = self
                    .database
                    .claude_desktop_profile(id)
                    .ok()
                    .filter(|_| is_active)
                    .map(|profile| self.desktop3p().profile(&profile.config_uuid));
                let rollback_paths = profile_path.iter().cloned().collect::<Vec<_>>();
                crate::fsutil::with_file_rollback(&rollback_paths, || {
                    self.database
                        .update_claude_desktop_profile(id, &input, &timestamp, |updated| {
                            if let Some(path) = &profile_path {
                                backup_file(
                                    path,
                                    &self.paths.config_backup,
                                    "claude-desktop-profile",
                                )?;
                                atomic_write(path, &render_profile(updated, None)?)?;
                            }
                            Ok(())
                        })
                })?
            }
            None => {
                let config_uuid = new_config_uuid()?;
                self.database
                    .insert_claude_desktop_profile(&input, &config_uuid, &timestamp)?
            }
        };
        Ok(detail(stored))
    }

    fn claude_desktop_insert(
        &self,
        input: &ClaudeDesktopProfileInput,
        config_uuid: &str,
    ) -> AppResult<StoredClaudeDesktopProfile> {
        let timestamp = now_ms().to_string();
        self.database
            .insert_claude_desktop_profile(input, config_uuid, &timestamp)
    }

    pub fn claude_desktop_set_icon(&self, id: &str, icon: Option<String>) -> AppResult<()> {
        let icon = super::validated_icon(icon.as_deref())?;
        self.database
            .set_claude_desktop_profile_icon(id, icon.as_deref(), &now_ms().to_string())
    }

    pub fn claude_desktop_reorder(&self, ids: &[String]) -> AppResult<()> {
        self.database
            .reorder_claude_desktop_profiles(ids, &now_ms().to_string())
    }

    pub fn claude_desktop_duplicate(&self, id: &str) -> AppResult<ClaudeDesktopProfileDetail> {
        if self.database.active_claude_desktop_profile()?.as_deref() == Some(id) {
            let _ = sync::registry().harvest(
                self,
                &sync::SyncTrigger::BeforeProfileClone {
                    client: sync::ClientId::ClaudeDesktop,
                    target_id: id.to_string(),
                },
                sync::SyncMaterial::default(),
            );
        }
        let stored = self.database.claude_desktop_profile(id)?;
        let profiles = self.database.claude_desktop_profiles()?;
        let (candidate, copy_index) = super::profile_copy_plan(
            id,
            profiles
                .iter()
                .map(|profile| (profile.id.as_str(), profile.name.as_str())),
        )?;
        let timestamp = now_ms().to_string();
        // 副本用新 uuid：configLibrary 文件名按 profile 终身绑定，副本是独立配置。
        let created = self.database.insert_claude_desktop_profile(
            &ClaudeDesktopProfileInput {
                name: candidate,
                kind: stored.kind.clone(),
                base_url: stored.base_url.clone(),
                api_key: stored.api_key.clone(),
                auth_scheme: Some(stored.auth_scheme.clone()),
                models: parse_models(&stored.models).into(),
                admin_url: stored.admin_url.clone(),
                icon: stored.icon.clone(),
                show_balance: stored.show_balance,
                description: stored.description.clone(),
                mode: stored.mode.clone(),
                upstream_base_url: stored.upstream_base_url.clone(),
                upstream_api_key: stored.upstream_api_key.clone(),
                fetched_models: Some(parse_fetched_models(&stored.fetched_models)),
            },
            &new_config_uuid()?,
            &timestamp,
        )?;
        let mut ordered_ids: Vec<String> = profiles.into_iter().map(|profile| profile.id).collect();
        ordered_ids.insert(copy_index, created.id.clone());
        self.database
            .reorder_claude_desktop_profiles(&ordered_ids, &timestamp)?;
        Ok(detail(created))
    }

    pub fn claude_desktop_delete(&self, id: &str) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        if self.database.active_claude_desktop_profile()?.as_deref() == Some(id) {
            return Err(app_err!(
                "无法删除使用中的 Claude Desktop 供应商配置，请先切换到其他配置"
            ));
        }
        self.database.delete_claude_desktop_profile(id)?;
        tauri_plugin_log::log::info!(
            "[provider.claude-desktop.delete] profile_id={id} outcome=success msg=\"已删除 Claude Desktop 供应商配置\""
        );
        Ok(())
    }

    pub fn claude_desktop_apply(&self, id: &str) -> AppResult<()> {
        self.claude_desktop_apply_confirmed(id, false)
            .map_err(|error| app_err!("{error}"))
    }

    /// 写 3P 四件套：两个 claude_desktop_config.json 置 deploymentMode=3p、
    /// profile 本体、_meta.json（entries 只保留当前激活项与外来条目）。
    /// 上一家激活配置的 profile 文件一并撤下，configLibrary 不残留半管理条目。
    pub fn claude_desktop_apply_confirmed(
        &self,
        id: &str,
        overwrite: bool,
    ) -> Result<(), ProfileApplyError> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let paths = self.desktop3p();
        let stored = self.database.claude_desktop_profile(id)?;
        // 路由模式：先起网关（渲染要用端口），直连沿用库里的端点与密钥
        let route = stored.mode.as_deref() == Some("route");
        let mut gateway: Option<(String, String)> = None;
        if route {
            let upstream_base = stored
                .upstream_base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| app_err!("路由模式请先填写上游 API 地址"))?;
            let upstream_key = stored
                .upstream_api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| app_err!("路由模式请先填写上游 API Key"))?
                .to_string();
            let mapping: std::collections::BTreeMap<String, String> = parse_models(&stored.models)
                .into_iter()
                .filter_map(|slot| {
                    let upstream = slot.upstream_model?.trim().to_string();
                    if upstream.is_empty() {
                        return None;
                    }
                    Some((slot.name.trim().to_string(), upstream))
                })
                .collect();
            if mapping.is_empty() {
                return Err(app_err!("路由模式至少要填一个上游模型映射").into());
            }
            let token = match stored
                .gateway_token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(token) => token.to_string(),
                None => {
                    let token = new_gateway_token()?;
                    self.database
                        .update_claude_desktop_gateway_token(id, &token)?;
                    token
                }
            };
            self.stop_desktop_gateway();
            let (client, _) = tauri::async_runtime::block_on(super::connections::http_client())
                .map_err(|error| app_err!("{error}"))?;
            let handle = claude_desktop_gateway::start_blocking(
                GatewayConfig {
                    token: token.clone(),
                    upstream_base: upstream_base.to_string(),
                    upstream_key,
                    upstream_scheme: stored.auth_scheme.clone(),
                    mapping,
                },
                client,
            )
            .map_err(|error| app_err!("{error}"))?;
            let url = format!("http://127.0.0.1:{}", handle.port);
            *self
                .desktop_gateway
                .lock()
                .map_err(|_| app_err!("路由网关状态锁已损坏"))? = Some(handle);
            gateway = Some((url, token));
        } else {
            self.stop_desktop_gateway();
        }
        if !route
            && stored
                .base_url
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(app_err!("请先填写 API 地址").into());
        }
        if !route
            && stored
                .api_key
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(app_err!("请先填写 API Key").into());
        }
        let profile_path = paths.profile(&stored.config_uuid);
        let meta_path = paths.meta();
        let previous_path = self
            .database
            .active_claude_desktop_profile()
            .ok()
            .flatten()
            .filter(|previous| previous != id)
            .and_then(|previous| self.database.claude_desktop_profile(&previous).ok())
            .map(|profile| paths.profile(&profile.config_uuid))
            .filter(|path| path != &profile_path);
        let mut rollback_paths = vec![
            paths.config.clone(),
            paths.p3_config.clone(),
            profile_path.clone(),
            meta_path.clone(),
        ];
        if let Some(previous) = &previous_path {
            rollback_paths.push(previous.clone());
        }
        // 损坏守卫：存在的文件非空且不是 JSON 对象 → 需用户确认才覆盖
        let damaged = rollback_paths.iter().any(|path| {
            std::fs::read_to_string(path)
                .is_ok_and(|raw| !raw.trim().is_empty() && read_json_object(path, "").is_err())
        });
        if damaged && !overwrite {
            return Err(ProfileApplyError::overwrite_required());
        }
        if !damaged {
            sync::registry().harvest(
                self,
                &sync::SyncTrigger::BeforeLiveOverwrite {
                    client: sync::ClientId::ClaudeDesktop,
                    target_id: id.to_string(),
                },
                sync::SyncMaterial::default(),
            )?;
        }
        let our_uuids: HashSet<String> = self
            .database
            .claude_desktop_config_uuids()?
            .into_iter()
            .collect();
        let result = crate::fsutil::with_file_rollback(&rollback_paths, || {
            merge_deployment_mode(&paths.config, "3p")?;
            merge_deployment_mode(&paths.p3_config, "3p")?;
            if let Some(previous) = &previous_path {
                backup_file(
                    previous,
                    &self.paths.config_backup,
                    "claude-desktop-profile",
                )?;
                let _ = std::fs::remove_file(previous);
            }
            backup_file(
                &profile_path,
                &self.paths.config_backup,
                "claude-desktop-profile",
            )?;
            atomic_write(
                &profile_path,
                &render_profile(
                    &stored,
                    gateway
                        .as_ref()
                        .map(|(url, token)| (url.as_str(), token.as_str())),
                )?,
            )?;
            let mut meta = read_meta(&meta_path)?.unwrap_or_default();
            meta.entries.retain(|entry| !our_uuids.contains(&entry.id));
            meta.entries.push(MetaEntry {
                id: stored.config_uuid.clone(),
                name: stored.name.clone(),
            });
            meta.applied_id = Some(stored.config_uuid.clone());
            write_meta(&meta, &meta_path)?;
            self.database.commit_claude_desktop_apply(
                id,
                &format!("profile_id={id} profile_name={:?}", stored.name),
                &now_ms().to_string(),
            )
        });
        // 写盘失败时停下刚起的网关，避免网关指向新映射而桌面端还是旧配置
        if result.is_err() {
            self.stop_desktop_gateway();
        }
        result?;
        tauri_plugin_log::log::info!(
            "[apply.claude-desktop] profile_id={id} outcome=success msg=\"已写入 Claude Desktop 3P 配置，需完全重启桌面端生效\""
        );
        Ok(())
    }

    /// 恢复官方模式：deploymentMode 写回 1p、撤下 Agmux 的 profile 与 meta 条目
    /// （外来条目保留）。无激活且 meta 无我们的条目时幂等成功。
    pub fn claude_desktop_restore_official(&self) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        self.stop_desktop_gateway();
        let paths = self.desktop3p();
        let meta_path = paths.meta();
        let meta = read_meta(&meta_path)?.unwrap_or_default();
        let our_uuids: HashSet<String> = self
            .database
            .claude_desktop_config_uuids()?
            .into_iter()
            .collect();
        let mut owned_files: Vec<PathBuf> = meta
            .entries
            .iter()
            .filter(|entry| our_uuids.contains(&entry.id))
            .map(|entry| paths.profile(&entry.id))
            .collect();
        if let Some(active) = self
            .database
            .active_claude_desktop_profile()?
            .and_then(|id| self.database.claude_desktop_profile(&id).ok())
        {
            let path = paths.profile(&active.config_uuid);
            if !owned_files.contains(&path) {
                owned_files.push(path);
            }
        }
        let has_ours =
            !owned_files.is_empty() || self.database.active_claude_desktop_profile()?.is_some();
        if !has_ours {
            return Ok(());
        }
        let mut rollback_paths = vec![
            paths.config.clone(),
            paths.p3_config.clone(),
            meta_path.clone(),
        ];
        rollback_paths.extend(owned_files.iter().cloned());
        let mut next_meta = meta;
        crate::fsutil::with_file_rollback(&rollback_paths, || {
            merge_deployment_mode(&paths.config, "1p")?;
            merge_deployment_mode(&paths.p3_config, "1p")?;
            for file in &owned_files {
                backup_file(file, &self.paths.config_backup, "claude-desktop-profile")?;
                let _ = std::fs::remove_file(file);
            }
            next_meta
                .entries
                .retain(|entry| !our_uuids.contains(&entry.id));
            // appliedId 指认"当前生效的 3P 配置"；我们撤走后它就是空位——
            // 直接删键，而不是谎称某个从未被应用的外来条目已生效
            if next_meta
                .applied_id
                .as_deref()
                .is_some_and(|id| our_uuids.contains(id))
            {
                next_meta.applied_id = None;
            }
            write_meta(&next_meta, &meta_path)?;
            self.database.set_active_claude_desktop_profile(None)?;
            self.database.record_event(
                None,
                "restore_claude_desktop",
                "success",
                None,
                &now_ms().to_string(),
            )
        })?;
        tauri_plugin_log::log::info!(
            "[restore.claude-desktop] outcome=success msg=\"已恢复 Claude Desktop 官方模式，需完全重启桌面端生效\""
        );
        Ok(())
    }

    /// 外部改过 3P 配置时，把 live 托管字段回写进激活快照（四道守卫对齐
    /// sync_active_claude_settings）。appliedId 不指向我们的 uuid 说明用户在
    /// 桌面端 GUI 切走或恢复官方：不回写、不动库，状态栏显示真实态。
    pub(super) fn sync_active_claude_desktop(&self) -> AppResult<sync::SyncOutcome> {
        let Some(id) = self.database.active_claude_desktop_profile()? else {
            return Ok(sync::SyncOutcome::bare(sync::SyncKind::NoActiveProfile));
        };
        let Ok(stored) = self.database.claude_desktop_profile(&id) else {
            return Ok(sync::SyncOutcome::bare(sync::SyncKind::NoActiveProfile));
        };
        let paths = self.desktop3p();
        let meta = match read_meta(&paths.meta()) {
            Ok(meta) => meta,
            Err(error) => {
                // serde 报错只含行列；外部动态文本按 {:?} 编码并截断
                let brief: String = error.0.chars().take(120).collect();
                tauri_plugin_log::log::warn!(
                    "[sync.claude-desktop.parse] profile={:?} outcome=failure failure_kind=io_error error={brief:?} msg=\"_meta.json 读取失败\"",
                    stored.name
                );
                return Ok(sync::SyncOutcome::of(
                    sync::SyncKind::LiveUnreadable,
                    &stored.name,
                ));
            }
        };
        let Some(applied) = meta.and_then(|meta| meta.applied_id) else {
            return Ok(sync::SyncOutcome::of(
                sync::SyncKind::LiveAbsent,
                &stored.name,
            ));
        };
        if applied != stored.config_uuid {
            return Ok(sync::SyncOutcome::bare(sync::SyncKind::TargetMismatch));
        }
        let raw = match std::fs::read_to_string(paths.profile(&stored.config_uuid)) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(sync::SyncOutcome::of(
                    sync::SyncKind::LiveAbsent,
                    &stored.name,
                ));
            }
            Err(_) => {
                return Ok(sync::SyncOutcome::of(
                    sync::SyncKind::LiveUnreadable,
                    &stored.name,
                ));
            }
        };
        let live: ProfileFile = match serde_json::from_str(&raw) {
            Ok(live) => live,
            Err(error) => {
                tauri_plugin_log::log::warn!(
                    "[sync.claude-desktop.parse] profile={:?} outcome=failure failure_kind=parse_error msg=\"profile 文件不是有效 JSON\"",
                    stored.name
                );
                return Ok(
                    sync::SyncOutcome::of(sync::SyncKind::LiveParseError, &stored.name)
                        .with_position(sync::parse_error_position(&error.to_string())),
                );
            }
        };
        let live_models = parse_live_models(&live.inference_models);
        let live_scheme = live.auth_scheme.unwrap_or_else(|| "bearer".into());
        if live.base_url == stored.base_url
            && live.api_key == stored.api_key
            && live_scheme == stored.auth_scheme
            && live_models == parse_models(&stored.models)
        {
            return Ok(sync::SyncOutcome::of(
                sync::SyncKind::Unchanged,
                &stored.name,
            ));
        }
        self.database.update_claude_desktop_profile(
            &id,
            &ClaudeDesktopProfileInput {
                name: stored.name.clone(),
                kind: stored.kind.clone(),
                base_url: live.base_url,
                api_key: live.api_key,
                auth_scheme: Some(live_scheme),
                models: Some(live_models),
                admin_url: stored.admin_url.clone(),
                icon: stored.icon.clone(),
                // 捕获不碰用户在库里的展示偏好
                show_balance: stored.show_balance,
                description: stored.description.clone(),
                mode: None,
                upstream_base_url: None,
                upstream_api_key: None,
                fetched_models: None,
            },
            &now_ms().to_string(),
            |_| Ok(()),
        )?;
        Ok(sync::SyncOutcome::of(sync::SyncKind::Wrote, &stored.name))
    }

    /// 卡片测试连通：复用 Claude 侧的 /v1/models 探测（/anthropic 后缀剥回同根）。
    pub async fn claude_desktop_test_profile(
        &self,
        id: &str,
    ) -> AppResult<super::CodexProfileConnectionResult> {
        let stored = self.database.claude_desktop_profile(id)?;
        let base_url = stored
            .base_url
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| app_err!("请先填写 API 地址"))?;
        let api_key = stored
            .api_key
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| app_err!("请先填写 API Key"))?;
        super::test_claude_connection(&base_url, &api_key).await
    }
}

// ---------------------------------------------------------------------------
// 进程检测 / 启动 / 重启 / 状态
// ---------------------------------------------------------------------------

fn collect_claude_process_ids(filter: impl Fn(&sysinfo::Process) -> bool) -> Vec<u32> {
    claude_process_system()
        .processes()
        .iter()
        .filter(|(_, process)| filter(process))
        .map(|(pid, _)| pid.as_u32())
        .collect()
}

/// 只刷新进程可执行路径（不扫 CPU/内存），对齐 codex::process 的开销纪律。
fn claude_process_system() -> sysinfo::System {
    sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_processes(
            sysinfo::ProcessRefreshKind::nothing()
                .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet)
                .with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
        ),
    )
}

fn is_claude_desktop_process(process: &sysinfo::Process) -> bool {
    process.exe().is_some_and(is_claude_desktop_executable)
}

/// 主可执行 .../Claude.app/Contents/MacOS/Claude；helper 包是
/// "Claude Helper.app/Contents/MacOS/Claude Helper (…)"，天然不匹配。
#[cfg(target_os = "macos")]
pub(crate) fn is_claude_desktop_executable(exe: &Path) -> bool {
    exe.to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".app/contents/macos/claude")
}

#[cfg(windows)]
fn is_claude_desktop_executable(exe: &Path) -> bool {
    let exe_text = exe
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if !exe_text.ends_with("\\claude.exe") {
        return false;
    }
    exe_text.contains("\\anthropicclaude\\")
        || exe_text.contains("\\packages\\claude_pzs8sxrjxfjjc\\")
        || exe_text.ends_with("\\claude\\claude.exe")
}

/// Windows 的「已拉起」判据：只认主进程，Electron helper（--type=）不算。
#[cfg(windows)]
fn find_claude_main_ids() -> Vec<u32> {
    collect_claude_process_ids(|process| {
        is_claude_desktop_process(process)
            && !process
                .cmd()
                .iter()
                .skip(1)
                .any(|argument| argument.to_string_lossy().starts_with("--type="))
    })
}

#[cfg(not(windows))]
fn find_claude_main_ids() -> Vec<u32> {
    collect_claude_process_ids(is_claude_desktop_process)
}

/// 应用安装位置（固定 + 自动识别，不支持手动覆盖，对齐 Codex）。
fn claude_desktop_install() -> Option<(PathBuf, &'static str)> {
    #[cfg(target_os = "macos")]
    {
        let names = ["Claude.app"];
        let roots = [
            PathBuf::from("/Applications"),
            home_dir().join("Applications"),
        ];
        roots
            .iter()
            .flat_map(|root| names.iter().map(move |name| root.join(name)))
            .find(|path| path.is_dir())
            .map(|path| (path, "auto"))
    }
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        let msix = local
            .join("Packages")
            .join(WINDOWS_PACKAGE_DIR)
            .join("LocalCache")
            .join("Local");
        if msix.join("Claude-3p").is_dir() || msix.join("Claude").is_dir() {
            return Some((msix, "local_msix"));
        }
        let exe = ["AnthropicClaude", "Claude"]
            .iter()
            .map(|name| local.join(name).join("claude.exe"))
            .find(|path| path.is_file());
        // shortcut: 官网 exe 的真实目录名需真机核对；AnthropicClaude 是历史 Squirrel 布局
        exe.map(|path| (path, "local_installer"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

/// 每帧 get_state 刷新的应用状态（对齐 CodexAppStatus 语义）。
pub fn claude_desktop_app_status(base: &Path) -> ClaudeDesktopAppStatus {
    let install = claude_desktop_install();
    let running = !find_claude_main_ids().is_empty();
    let deployment_mode = read_json_object(&resolve_desktop3p(base).p3_config, CONFIG_FILE_NAME)
        .ok()
        .and_then(|object| {
            object
                .get("deploymentMode")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    match install {
        Some((path, source)) => ClaudeDesktopAppStatus {
            installed: true,
            running,
            display_path: path.display().to_string(),
            source: source.to_string(),
            deployment_mode,
        },
        None => ClaudeDesktopAppStatus {
            installed: false,
            running,
            display_path: "未识别".into(),
            source: "not_found".into(),
            deployment_mode,
        },
    }
}

fn launch_claude_desktop() -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let app = claude_desktop_install()
            .map(|(path, _)| path)
            .ok_or_else(|| app_err!("未找到可启动的 Claude Desktop 应用"))?;
        // 先常规 open -a；强杀后 LaunchServices 状态未同步时改 open -n 兜底（对齐 Codex）
        if codex_process::macos_open_app(&app, false).is_ok() {
            return Ok(());
        }
        codex_process::macos_open_app(&app, true)
    }
    #[cfg(windows)]
    {
        if let Some((path, source)) = claude_desktop_install() {
            if source == "local_installer" {
                use std::os::windows::process::CommandExt;
                use std::process::{Command, Stdio};
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                Command::new(&path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|error| app_err!("无法启动 Claude Desktop: {error}"))?;
                return Ok(());
            }
        }
        codex_process::activate_windows_package_by_aumid(WINDOWS_CLAUDE_AUMIDS)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Err(app_err!("当前系统暂不支持启动 Claude Desktop 应用"))
    }
}

impl AppContext {
    /// 完全退出并重启 Claude Desktop（配置无热重载，这是生效的唯一途径）。
    pub fn restart_claude_desktop(&self) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        sync::registry().harvest(
            self,
            &sync::SyncTrigger::BeforeProcessStart {
                client: sync::ClientId::ClaudeDesktop,
            },
            sync::SyncMaterial::default(),
        )?;
        let process_ids = collect_claude_process_ids(is_claude_desktop_process);
        let action = if process_ids.is_empty() {
            "start"
        } else {
            "restart"
        };
        let mut force_killed = false;
        if !process_ids.is_empty() {
            // PID 复用防御：击杀前/退出判定都重新取进程表复核身份（对齐 Codex 纪律）
            let refresh_claude_processes = |ids: &[u32]| -> Vec<u32> {
                let system = claude_process_system();
                ids.iter()
                    .copied()
                    .filter(|id| {
                        system
                            .process(sysinfo::Pid::from_u32(*id))
                            .is_some_and(is_claude_desktop_process)
                    })
                    .collect()
            };
            let outcome = codex_process::shutdown_process_ids_with(
                &process_ids,
                codex_process::GRACEFUL_EXIT_TIMEOUT_MS,
                5_000,
                codex_process::request_graceful_quit,
                |ids: &[u32]| {
                    let system = claude_process_system();
                    for id in ids {
                        if let Some(process) = system.process(sysinfo::Pid::from_u32(*id)) {
                            if is_claude_desktop_process(process) {
                                let _ = process.kill();
                            }
                        }
                    }
                },
                refresh_claude_processes,
                std::thread::sleep,
            );
            if outcome == codex_process::ShutdownOutcome::Timeout {
                let message = "Claude Desktop 未在超时时间内退出，已取消重新启动";
                self.database.record_event(
                    None,
                    "restart_claude_desktop",
                    "timeout",
                    Some(message),
                    &now_ms().to_string(),
                )?;
                tauri_plugin_log::log::warn!(
                    "[settings.restart-claude-desktop] outcome=failure failure_kind=timeout msg=\"{message}\""
                );
                return Err(app_err!("{message}"));
            }
            force_killed = outcome == codex_process::ShutdownOutcome::Forced;
        }
        let result = (|| {
            launch_claude_desktop()?;
            if codex_process::wait_for_running_with(
                10_000,
                100,
                || !find_claude_main_ids().is_empty(),
                std::thread::sleep,
            ) {
                Ok(())
            } else {
                Err(app_err!(
                    "Claude Desktop 启动超时，请检查应用是否可以正常打开"
                ))
            }
        })();
        let status = if result.is_ok() { "success" } else { "failed" };
        let message = result
            .as_ref()
            .err()
            .map(|error| error.0.clone())
            .or_else(|| force_killed.then(|| "claude desktop exited via force kill".to_string()));
        self.database.record_event(
            None,
            "restart_claude_desktop",
            status,
            message.as_deref(),
            &now_ms().to_string(),
        )?;
        match &result {
            Ok(()) => tauri_plugin_log::log::info!(
                "[settings.restart-claude-desktop] action={action} outcome=success msg=\"Claude Desktop 进程已拉起\""
            ),
            Err(error) => tauri_plugin_log::log::warn!(
                "[settings.restart-claude-desktop] action={action} outcome=failure failure_kind=internal error={error:?} msg=\"{}\"",
                if action == "start" { "Claude Desktop 启动失败" } else { "Claude Desktop 重启失败" }
            ),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::AppContext;

    fn test_context() -> (tempfile::TempDir, AppContext) {
        let home = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(home.path()).unwrap();
        paths.ensure().unwrap();
        let context = AppContext::new(paths.clone()).unwrap();
        (home, context)
    }

    fn deepseek_input() -> ClaudeDesktopProfileInput {
        ClaudeDesktopProfileInput {
            name: "DeepSeek".into(),
            kind: Some("deepseek".into()),
            base_url: Some("https://api.deepseek.com/anthropic".into()),
            api_key: Some("sk-fixture".into()),
            auth_scheme: Some("bearer".into()),
            models: Some(vec![
                ClaudeDesktopModelSlot {
                    slot: "sonnet".into(),
                    name: "claude-sonnet-5-5".into(),
                    label_override: Some("DeepSeek V4 Flash".into()),
                    supports_1m: false,
                    upstream_model: None,
                },
                ClaudeDesktopModelSlot {
                    slot: "opus".into(),
                    name: "claude-opus-5-5".into(),
                    label_override: Some("DeepSeek V4 Pro".into()),
                    supports_1m: false,
                    upstream_model: None,
                },
            ]),
            admin_url: None,
            icon: None,
            show_balance: true,
            description: None,
            mode: None,
            upstream_base_url: None,
            upstream_api_key: None,
            fetched_models: None,
        }
    }

    #[test]
    fn show_balance_persists_through_save_and_update() {
        let (_home, context) = test_context();
        let created = context.claude_desktop_save(None, deepseek_input()).unwrap();
        assert!(created.show_balance);
        let stored = context
            .database
            .claude_desktop_profile(&created.id)
            .unwrap();
        assert!(stored.show_balance);
        // 关掉开关再保存：库里与详情都要跟着变
        let mut updated_input = deepseek_input();
        updated_input.show_balance = false;
        let updated = context
            .claude_desktop_save(Some(&created.id), updated_input)
            .unwrap();
        assert!(!updated.show_balance);
        assert!(
            !context
                .database
                .claude_desktop_profile(&created.id)
                .unwrap()
                .show_balance
        );
    }

    #[test]
    fn route_apply_starts_gateway_and_writes_local_endpoint() {
        let (_home, context) = test_context();
        let created = context
            .claude_desktop_save(
                None,
                ClaudeDesktopProfileInput {
                    name: "GLM 路由".into(),
                    kind: Some("zhipu".into()),
                    mode: Some("route".into()),
                    upstream_base_url: Some("https://open.bigmodel.cn/api/anthropic".into()),
                    upstream_api_key: Some("sk-upstream".into()),
                    models: Some(vec![ClaudeDesktopModelSlot {
                        slot: "sonnet".into(),
                        name: "claude-sonnet-5-5".into(),
                        label_override: None,
                        supports_1m: false,
                        upstream_model: Some("glm-4.7".into()),
                    }]),
                    ..Default::default()
                },
            )
            .unwrap();
        context.claude_desktop_apply(&created.id).unwrap();
        // profile 本体写的是本地网关地址与令牌，不是上游
        let stored = context
            .database
            .claude_desktop_profile(&created.id)
            .unwrap();
        let paths = context.desktop3p();
        let body: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.profile(&stored.config_uuid)).unwrap(),
        )
        .unwrap();
        let base = body["inferenceGatewayBaseUrl"].as_str().unwrap();
        assert!(base.starts_with("http://127.0.0.1:"));
        let token = stored.gateway_token.clone().unwrap();
        assert_eq!(body["inferenceGatewayApiKey"], token.as_str());
        let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
        tauri::async_runtime::block_on(async {
            // 本地回环不走系统代理；目录名可发现、令牌不对要 401
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let catalog: Value = client
                .get(format!("http://127.0.0.1:{port}/v1/models"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(catalog["data"][0]["id"], "claude-sonnet-5-5");
            let denied = client
                .get(format!("http://127.0.0.1:{port}/v1/models"))
                .bearer_auth("wrong")
                .send()
                .await
                .unwrap();
            assert_eq!(denied.status().as_u16(), 401);
        });
        // 回官方即停网关
        context.claude_desktop_restore_official().unwrap();
    }

    fn applied_profile(context: &AppContext) -> StoredClaudeDesktopProfile {
        let created = context.claude_desktop_save(None, deepseek_input()).unwrap();
        context.claude_desktop_apply(&created.id).unwrap();
        context
            .database
            .claude_desktop_profile(&created.id)
            .unwrap()
    }

    #[test]
    fn apply_writes_all_four_files_and_keeps_foreign_entries_and_user_keys() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        // 用户已有 mcpServers 配置（普通 config 的托管外键必须保留）
        std::fs::create_dir_all(paths.config.parent().unwrap()).unwrap();
        std::fs::write(
            &paths.config,
            r#"{"mcpServers": {"fetch": {"command": "uvx"}}}"#,
        )
        .unwrap();
        // 外部工具（cc-switch 等）留下的 meta 条目必须保留
        std::fs::create_dir_all(&paths.library).unwrap();
        std::fs::write(
            paths.meta(),
            r#"{"appliedId": "00000000-0000-4000-8000-000000157210", "entries": [{"id": "00000000-0000-4000-8000-000000157210", "name": "CC Switch"}]}"#,
        )
        .unwrap();

        let stored = applied_profile(&context);

        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        let p3: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.p3_config).unwrap()).unwrap();
        assert_eq!(config["deploymentMode"], "3p");
        assert_eq!(p3["deploymentMode"], "3p");
        assert_eq!(config["mcpServers"]["fetch"]["command"], "uvx");

        let profile: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.profile(&stored.config_uuid)).unwrap(),
        )
        .unwrap();
        assert_eq!(profile["inferenceProvider"], "gateway");
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            profile["inferenceModels"][0]["labelOverride"],
            "DeepSeek V4 Flash"
        );
        assert_eq!(profile["modelDiscoveryEnabled"], false);

        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(paths.meta()).unwrap()).unwrap();
        assert_eq!(meta["appliedId"], stored.config_uuid.as_str());
        let ids: Vec<&str> = meta["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        assert!(ids.contains(&"00000000-0000-4000-8000-000000157210"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn switching_profiles_removes_previous_file_and_uuid_stays_stable() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let first = applied_profile(&context);
        let second = context
            .claude_desktop_save(
                None,
                ClaudeDesktopProfileInput {
                    name: "Relay".into(),
                    base_url: Some("https://relay.example.com".into()),
                    api_key: Some("sk-relay".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        context.claude_desktop_apply(&second.id).unwrap();

        assert!(!paths.profile(&first.config_uuid).exists());
        assert!(paths
            .profile(
                &context
                    .database
                    .claude_desktop_profile(&second.id)
                    .unwrap()
                    .config_uuid
            )
            .exists());
        // 二次应用同一配置：文件名不变（uuid 终身绑定）
        context.claude_desktop_apply(&second.id).unwrap();
        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(paths.meta()).unwrap()).unwrap();
        let entries = meta["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]["id"],
            context
                .database
                .claude_desktop_profile(&second.id)
                .unwrap()
                .config_uuid
        );
    }

    #[test]
    fn apply_failure_midway_restores_all_four_files() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let stored = applied_profile(&context);
        // 让 profile 目标位置变成目录：atomic_write 的 rename 必然失败，触发回滚
        std::fs::remove_file(paths.profile(&stored.config_uuid)).unwrap();
        std::fs::create_dir_all(paths.profile(&stored.config_uuid)).unwrap();
        let before_meta = std::fs::read_to_string(paths.meta()).unwrap();
        let before_config = std::fs::read_to_string(&paths.config).unwrap();

        let result = context.claude_desktop_apply(&stored.id);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(&paths.config).unwrap(),
            before_config
        );
        assert_eq!(std::fs::read_to_string(paths.meta()).unwrap(), before_meta);
    }

    #[test]
    fn corrupted_live_reports_parse_error_and_keeps_last_valid_snapshot() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let stored = applied_profile(&context);

        // 破坏 profile 本体：守卫必须报 LiveParseError 且快照不被覆盖
        std::fs::write(paths.profile(&stored.config_uuid), "{ not json").unwrap();
        let outcome = context.sync_active_claude_desktop().unwrap();
        assert_eq!(outcome.kind, sync::SyncKind::LiveParseError);
        assert_eq!(outcome.profile.as_deref(), Some("DeepSeek"));
        let unchanged = context.database.claude_desktop_profile(&stored.id).unwrap();
        assert_eq!(
            unchanged.base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );

        // 外部修复后自愈：漂移收敛入库
        std::fs::write(
            paths.profile(&stored.config_uuid),
            serde_json::to_vec_pretty(&json!({
                "inferenceProvider": "gateway",
                "inferenceGatewayBaseUrl": "https://api.deepseek.com/anthropic",
                "inferenceGatewayApiKey": "sk-fixture",
                "inferenceGatewayAuthScheme": "bearer",
                "inferenceModels": [{"name": "claude-sonnet-4-6", "labelOverride": "DeepSeek V4 Flash"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let outcome = context.sync_active_claude_desktop().unwrap();
        assert_eq!(outcome.kind, sync::SyncKind::Wrote);
        let updated = context.database.claude_desktop_profile(&stored.id).unwrap();
        let models = parse_models(&updated.models);
        assert_eq!(models[0].name, "claude-sonnet-4-6");
        // 再收割一次收敛为 Unchanged
        let outcome = context.sync_active_claude_desktop().unwrap();
        assert_eq!(outcome.kind, sync::SyncKind::Unchanged);
    }

    #[test]
    fn foreign_applied_id_reports_target_mismatch_without_touching_database() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let stored = applied_profile(&context);
        // 用户在桌面端 GUI 切到别家配置：appliedId 不再指向我们
        let meta = json!({"appliedId": "foreign-id", "entries": [
            {"id": "foreign-id", "name": "Other"},
            {"id": stored.config_uuid, "name": "DeepSeek"}
        ]});
        std::fs::write(paths.meta(), serde_json::to_vec_pretty(&meta).unwrap()).unwrap();
        let outcome = context.sync_active_claude_desktop().unwrap();
        assert_eq!(outcome.kind, sync::SyncKind::TargetMismatch);
        let unchanged = context.database.claude_desktop_profile(&stored.id).unwrap();
        assert_eq!(
            unchanged.base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
    }

    #[test]
    fn restore_official_reverts_files_keeps_foreign_entries_and_is_idempotent() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let stored = applied_profile(&context);
        std::fs::write(
            paths.meta(),
            serde_json::to_vec_pretty(&json!({
                "appliedId": stored.config_uuid,
                "entries": [
                    {"id": stored.config_uuid, "name": "DeepSeek"},
                    {"id": "00000000-0000-4000-8000-000000157210", "name": "CC Switch"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        context.claude_desktop_restore_official().unwrap();

        let config: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert_eq!(config["deploymentMode"], "1p");
        assert!(!paths.profile(&stored.config_uuid).exists());
        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(paths.meta()).unwrap()).unwrap();
        assert!(meta.get("appliedId").is_none());
        assert_eq!(meta["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            meta["entries"][0]["id"],
            "00000000-0000-4000-8000-000000157210"
        );
        assert!(context
            .database
            .active_claude_desktop_profile()
            .unwrap()
            .is_none());
        // 幂等
        context.claude_desktop_restore_official().unwrap();
    }

    #[test]
    fn capture_reuses_the_applied_config_uuid() {
        let (_home, context) = test_context();
        let stored = applied_profile(&context);
        let captured = context.claude_desktop_capture("Imported").unwrap();
        assert_eq!(
            captured.base_url.as_deref(),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(captured.models.len(), 2);
        let row = context
            .database
            .claude_desktop_profile(&captured.id)
            .unwrap();
        assert_eq!(row.config_uuid, stored.config_uuid);
        assert_eq!(row.name, "Imported");
    }

    #[test]
    fn damaged_live_requires_confirmation_before_overwrite() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let stored = applied_profile(&context);
        std::fs::write(paths.profile(&stored.config_uuid), "{ broken").unwrap();
        let error = context
            .claude_desktop_apply_confirmed(&stored.id, false)
            .unwrap_err();
        assert!(matches!(error, ProfileApplyError::OverwriteRequired { .. }));
        // 确认后可覆盖自愈
        context
            .claude_desktop_apply_confirmed(&stored.id, true)
            .unwrap();
        let profile: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.profile(&stored.config_uuid)).unwrap(),
        )
        .unwrap();
        assert_eq!(profile["inferenceProvider"], "gateway");
    }

    #[test]
    fn windows_paths_prefer_msix_when_its_config_exists() {
        let home = tempfile::tempdir().unwrap();
        let local = home.path().join("AppData").join("Local");
        // 非 Windows 下 resolve 直接落根；Windows 候选逻辑单独验证
        #[cfg(not(windows))]
        {
            let roots = resolve_desktop3p(&local);
            assert_eq!(
                roots.p3_config,
                local.join("Claude-3p").join(CONFIG_FILE_NAME)
            );
            assert_eq!(roots.library, local.join("Claude-3p").join("configLibrary"));
        }
        #[cfg(windows)]
        {
            let roots = resolve_desktop3p(&local);
            assert_eq!(
                roots.p3_config,
                local.join("Claude-3p").join(CONFIG_FILE_NAME)
            );
            // MSIX 目录存在 → 虚拟化路径优先
            let msix = local
                .join("Packages")
                .join(WINDOWS_PACKAGE_DIR)
                .join("LocalCache")
                .join("Local");
            std::fs::create_dir_all(msix.join("Claude")).unwrap();
            let roots = resolve_desktop3p(&local);
            assert_eq!(
                roots.p3_config,
                msix.join("Claude-3p").join(CONFIG_FILE_NAME)
            );
        }
    }

    #[test]
    fn render_and_parse_models_round_trip_with_camel_case_file_keys() {
        let models = serde_json::to_string(&vec![
            ClaudeDesktopModelSlot {
                slot: "sonnet".into(),
                name: "claude-sonnet-5-5".into(),
                label_override: Some("DeepSeek V4 Flash".into()),
                supports_1m: true,
                upstream_model: None,
            },
            ClaudeDesktopModelSlot {
                slot: "haiku".into(),
                name: "claude-haiku-5-5".into(),
                label_override: None,
                supports_1m: false,
                upstream_model: None,
            },
        ])
        .unwrap();
        let rendered = render_models(&Some(models));
        assert_eq!(
            rendered[0],
            json!({"name": "claude-sonnet-5-5", "labelOverride": "DeepSeek V4 Flash", "supports1m": true})
        );
        assert_eq!(rendered[1], json!({"name": "claude-haiku-5-5"}));
        let parsed = parse_live_models(&rendered);
        assert_eq!(parsed[0].slot, "sonnet");
        assert!(parsed[0].supports_1m);
        assert_eq!(
            parsed[0].label_override.as_deref(),
            Some("DeepSeek V4 Flash")
        );
        assert_eq!(parsed[1].slot, "opus");
        assert_eq!(parsed[1].name, "claude-haiku-5-5");
        // 字符串条目按槽位顺序解析
        let parsed = parse_live_models(&[json!("claude-sonnet-5-5")]);
        assert_eq!(parsed[0].slot, "sonnet");
    }

    #[test]
    fn meta_round_trip_drops_applied_id_when_none() {
        let meta = MetaFile::default();
        let bytes = serde_json::to_vec(&meta).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("appliedId"));
        let meta = MetaFile {
            applied_id: Some("u".into()),
            entries: vec![MetaEntry {
                id: "u".into(),
                name: "n".into(),
            }],
        };
        let bytes = serde_json::to_vec(&meta).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("appliedId"));
    }

    #[test]
    fn merge_deployment_mode_preserves_foreign_keys_and_skips_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude_desktop_config.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({"mcpServers": {"fetch": {"command": "uvx"}}}))
                .unwrap(),
        )
        .unwrap();
        merge_deployment_mode(&path, "3p").unwrap();
        let merged: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(merged["deploymentMode"], "3p");
        assert_eq!(merged["mcpServers"]["fetch"]["command"], "uvx");
        let before = std::fs::read_to_string(&path).unwrap();
        merge_deployment_mode(&path, "3p").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn config_uuid_is_v4_shaped() {
        let uuid = new_config_uuid().unwrap();
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.as_bytes()[14], b'4');
        assert!(uuid.as_bytes()[19].is_ascii_hexdigit());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_filter_matches_only_claude_bundle() {
        assert!(is_claude_desktop_executable(Path::new(
            "/Applications/Claude.app/Contents/MacOS/Claude"
        )));
        assert!(!is_claude_desktop_executable(Path::new(
            "/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper (GPU)"
        )));
        assert!(!is_claude_desktop_executable(Path::new(
            "/usr/local/bin/claude"
        )));
    }
}
