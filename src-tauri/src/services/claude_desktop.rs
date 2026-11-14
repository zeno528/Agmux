//! Claude Desktop（官方桌面应用，3P 第三方推理模式）供应商管理。
//!
//! 桌面端不读 `~/.claude/settings.json`：第三方接入写独立的 3P 配置
//! （macOS `~/Library/Application Support/Claude-3p/`，Windows `%LOCALAPPDATA%` 同构；
//! Store/MSIX 版被文件系统虚拟化到 `Packages\Claude_pzs8sxrjxfjjc\LocalCache\Local\`，
//! 见 cc-switch#4842 的根因分析）。生效要求完全退出重启桌面端（无热重载）。
//! 直连写上游端点；路由和聚合共用一个本地网关，按 Desktop 角色 ID 分发。
//! 供应商保存只写库，应用后的快照负责网关重启恢复和直连漂移检测。

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::claude_desktop_gateway::{self, GatewayConfig, GatewayRoute};
use super::sync;
use super::AppContext;
use crate::codex::process as codex_process;
use crate::database::StoredClaudeDesktopProfile;
use crate::error::{app_err, AppResult, ProfileApplyError};
use crate::fsutil::{atomic_write, backup_file};
use crate::models::{
    ClaudeDesktopAppStatus, ClaudeDesktopDiagnostics, ClaudeDesktopMode, ClaudeDesktopModelSlot,
    ClaudeDesktopProfileDetail, ClaudeDesktopProfileInput, ClaudeDesktopProfileSummary,
    ClaudeDesktopRouting,
};
use crate::paths::now_ms;

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

// Desktop 按角色前缀校验整组模型；不是任意 claude-* 都能接入。
fn is_desktop_model_id(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    let name = name.strip_prefix("anthropic/").unwrap_or(&name);
    !name.contains("[1m]")
        && SLOT_ORDER.iter().any(|slot| {
            name.strip_prefix(&format!("claude-{slot}-"))
                .is_some_and(|suffix| !suffix.is_empty())
        })
}

// Desktop 4.29+ 用 behavesAs 为自定义 ID 显示原生 Effort，发给上游的 name 不变。
pub(super) fn alias_behavior(name: &str) -> Option<&'static str> {
    SLOT_ORDER
        .iter()
        .find(|slot| name.starts_with(&format!("claude-{slot}-agmux-")))
        .and_then(|slot| canonical_catalog(slot))
}

// DeepSeek 官方端点按 Claude 前缀映射模型；Flash 不能用 Opus 前缀（会调用 Pro）。
fn direct_model_id(
    profile: &StoredClaudeDesktopProfile,
    model: &ClaudeDesktopModelSlot,
) -> Option<String> {
    if is_desktop_model_id(&model.name) {
        return Some(model.name.clone());
    }
    let url = url::Url::parse(profile.base_url.as_deref()?).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.deepseek.com")
        || url.path().trim_end_matches('/') != "/anthropic"
    {
        return None;
    }
    let role = match model.name.as_str() {
        "deepseek-flash" | "deepseek-v4-flash" | "deepseek-v4-flash-vision-exp" => {
            if model.slot == "haiku" {
                "haiku"
            } else {
                "sonnet"
            }
        }
        "deepseek-v4-pro" => "opus",
        _ => return None,
    };
    Some(format!("claude-{role}-agmux-{}", model.slot))
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

// Desktop 4.4212+ 会将 configLibrary 里的明文密钥换成 OS 绑定密文，不能作为上游凭证。
fn is_desktop_encrypted_key(value: &str) -> bool {
    let value = value.trim_start();
    value.starts_with("__sealed__:") || value.starts_with("__encrypted__:")
}

fn validate_desktop_api_key(key: Option<&str>) -> AppResult<()> {
    if key.is_some_and(is_desktop_encrypted_key) {
        return Err(app_err!("这是 Claude Desktop 的本机加密凭证，不能作为 API Key。请在供应商编辑页填写原始 API Key"));
    }
    Ok(())
}

/// 从库行渲染 profile 本体。字段集对齐 cc-switch 逆向与官方 configuration reference；
/// 上游未知键不保留（桌面端 GUI 的自管理键不属于 Agmux 托管域）。
/// 路由模式下 base_url/api_key 换成本地网关地址与令牌（override 一对即生效）。
fn render_profile(
    profile: &StoredClaudeDesktopProfile,
    gateway: Option<(&str, &str, &BTreeMap<String, GatewayRoute>)>,
) -> AppResult<Vec<u8>> {
    let (base_url, api_key) = match gateway {
        Some((url, token, _)) => (url.to_string(), token.to_string()),
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
        json!(if gateway.is_some() {
            "bearer"
        } else {
            &profile.auth_scheme
        }),
    );
    let mut models: Vec<Value> = match gateway {
        Some((_, _, routes)) => {
            let mut entries: Vec<_> = routes.iter().collect();
            entries.sort_by_key(|(id, _)| {
                SLOT_ORDER
                    .iter()
                    .position(|slot| canonical_catalog(slot) == Some(id.as_str()))
                    .unwrap_or(SLOT_ORDER.len())
            });
            entries
                .into_iter()
                .map(|(id, route)| {
                    json!({
                        "name":id, "labelOverride":route.label, "supports1m":route.supports_1m
                    })
                })
                .collect()
        }
        None => {
            let mut rendered = render_models(&profile.models);
            for (entry, model) in rendered.iter_mut().zip(parse_models(&profile.models)) {
                if let Some(id) = direct_model_id(profile, &model).filter(|id| id != &model.name) {
                    entry["name"] = json!(id);
                    if entry.get("labelOverride").is_none() {
                        entry["labelOverride"] = json!(model.name);
                    }
                }
            }
            rendered
        }
    };
    for model in &mut models {
        if let Some(behavior) = model
            .get("name")
            .and_then(Value::as_str)
            .and_then(alias_behavior)
        {
            model["behavesAs"] = json!(behavior);
        }
    }
    if !models.is_empty() {
        document.insert("inferenceModels".into(), Value::Array(models));
    }
    document.insert("coworkEgressAllowedHosts".into(), json!(["*"]));
    document.insert("disableDeploymentModeChooser".into(), json!(true));
    document.insert(
        "modelDiscoveryEnabled".into(),
        json!(gateway.is_some() || parse_models(&profile.models).is_empty()),
    );
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
    let mut models = text
        .as_deref()
        .and_then(|text| serde_json::from_str::<Vec<ClaudeDesktopModelSlot>>(text).ok())
        .unwrap_or_default();
    for model in &mut models {
        if let Some(upstream) = model
            .upstream_model
            .take()
            .filter(|value| !value.trim().is_empty())
        {
            model.name = upstream;
        }
    }
    models
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
    let models = parse_models(&profile.models);
    let requires_routing = models
        .iter()
        .any(|model| direct_model_id(profile, model).is_none());
    ClaudeDesktopProfileSummary {
        id: profile.id.clone(),
        name: profile.name.clone(),
        base_url: profile.base_url.clone(),
        has_key: profile
            .api_key
            .as_deref()
            .is_some_and(|key| !key.is_empty() && !is_desktop_encrypted_key(key)),
        model: models.into_iter().map(|model| model.name).next(),
        requires_routing,
        kind: profile.kind.clone(),
        admin_url: profile.admin_url.clone(),
        icon: profile.icon.clone(),
        show_balance: profile.show_balance,
        updated_at: profile.updated_at.clone(),
    }
}

fn detail(profile: StoredClaudeDesktopProfile) -> ClaudeDesktopProfileDetail {
    ClaudeDesktopProfileDetail {
        id: profile.id,
        name: profile.name,
        kind: profile.kind,
        base_url: profile.base_url,
        api_key: profile.api_key.filter(|key| !is_desktop_encrypted_key(key)),
        auth_scheme: profile.auth_scheme,
        models: parse_models(&profile.models),
        admin_url: profile.admin_url,
        icon: profile.icon,
        show_balance: profile.show_balance,
        description: profile.description,
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
    pub fn claude_desktop_status(&self) -> AppResult<ClaudeDesktopAppStatus> {
        let routing = self.database.claude_desktop_routing()?;
        let mut status = claude_desktop_app_status(&self.paths.claude_desktop_base);
        status.mode = routing.mode;
        status.members = routing.members;
        status.selected_members = routing.selected_members;
        status.gateway_running = self
            .desktop_gateway
            .lock()
            .map_err(|_| app_err!("路由网关状态锁已损坏"))?
            .as_ref()
            .is_some_and(|gateway| gateway.is_running());
        Ok(status)
    }

    pub fn claude_desktop_diagnostics(&self) -> AppResult<ClaudeDesktopDiagnostics> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let routing = self.database.claude_desktop_routing()?;
        let active = self
            .database
            .active_claude_desktop_profile()?
            .map(|id| self.database.claude_desktop_profile(&id))
            .transpose()?;
        let gateway_url = (routing.mode != ClaudeDesktopMode::Direct && routing.port != 0)
            .then(|| format!("http://127.0.0.1:{}", routing.port));
        let applied_routes: BTreeMap<String, GatewayRoute> = routing
            .gateway_routes
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| app_err!("网关快照无效: {error}"))?
            .unwrap_or_default();
        let expected = if let (Some(profile), Some(url)) = (&active, &gateway_url) {
            Some(render_profile(
                profile,
                Some((url, &routing.token, &applied_routes)),
            )?)
        } else {
            routing
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.as_bytes().to_vec())
        };
        let paths = self.desktop3p();
        let live_matches = if let (Some(profile), Some(expected)) = (&active, expected) {
            let mode = read_json_object(&paths.config, CONFIG_FILE_NAME)?;
            let ours = read_meta(&paths.meta())?
                .and_then(|meta| meta.applied_id)
                .as_deref()
                == Some(profile.config_uuid.as_str());
            let live = std::fs::read(paths.profile(&profile.config_uuid))
                .ok()
                .and_then(|raw| serde_json::from_slice::<ProfileFile>(&raw).ok());
            let expected: ProfileFile = serde_json::from_slice(&expected)
                .map_err(|error| app_err!("已应用快照无效: {error}"))?;
            ours && mode.get("deploymentMode").and_then(Value::as_str) == Some("3p")
                && live.is_some_and(|live| {
                    live.base_url == expected.base_url
                        && !expected
                            .api_key
                            .as_deref()
                            .is_some_and(is_desktop_encrypted_key)
                        && (live.api_key == expected.api_key
                            || (live
                                .api_key
                                .as_deref()
                                .is_some_and(is_desktop_encrypted_key)
                                && expected.api_key.as_deref().is_some_and(|key| {
                                    !key.is_empty() && !is_desktop_encrypted_key(key)
                                })))
                        && live.auth_scheme == expected.auth_scheme
                        && live.inference_models == expected.inference_models
                })
        } else {
            false
        };
        let handle = self
            .desktop_gateway
            .lock()
            .map_err(|_| app_err!("路由状态锁已损坏"))?;
        let gateway_running = handle.as_ref().is_some_and(|gateway| gateway.is_running());
        let mut data = ClaudeDesktopDiagnostics {
            mode: routing.mode,
            active_name: active.map(|profile| profile.name),
            gateway_running,
            gateway_url,
            live_matches,
            routes: applied_routes
                .iter()
                .map(|(id, route)| claude_desktop_gateway::route_info(id, route))
                .collect(),
            activity: Default::default(),
        };
        if let Some(gateway) = handle.as_ref() {
            gateway.diagnostics(&mut data)?;
        }
        Ok(data)
    }

    pub fn claude_desktop_list(&self) -> AppResult<Vec<ClaudeDesktopProfileSummary>> {
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

    /// 保存供应商；映射模式需重新应用，避免修改运行中网关的投影。
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
        validate_desktop_api_key(input.api_key.as_deref())?;
        input.kind = clean(input.kind);
        input.admin_url = clean(input.admin_url);
        input.icon = clean(input.icon);
        let scheme = match input.auth_scheme.as_deref() {
            None => "bearer".to_string(),
            Some(scheme @ ("bearer" | "x-api-key")) => scheme.to_string(),
            Some(other) => return Err(app_err!("不支持的鉴权方式：{other}")),
        };
        input.auth_scheme = Some(scheme);
        if let Some(models) = &mut input.models {
            let mut roles = HashSet::new();
            for model in models {
                model.name = model.name.trim().to_string();
                if !SLOT_ORDER.contains(&model.slot.as_str()) || !roles.insert(model.slot.clone()) {
                    return Err(app_err!("模型角色无效或重复：{}", model.slot));
                }
                if model.name.is_empty() {
                    return Err(app_err!("请填写实际请求模型"));
                }
                model.upstream_model = None;
            }
        }
        let timestamp = now_ms().to_string();
        let stored = match id {
            Some(id) => self
                .database
                .update_claude_desktop_profile(id, &input, &timestamp, None)?,
            None => {
                let config_uuid = new_config_uuid()?;
                self.database
                    .insert_claude_desktop_profile(&input, &config_uuid, &timestamp)?
            }
        };
        Ok(detail(stored))
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
        validate_desktop_api_key(stored.api_key.as_deref())?;
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
        let routing = self.database.claude_desktop_routing()?;
        if self.database.active_claude_desktop_profile()?.is_some()
            && routing.mode == ClaudeDesktopMode::Aggregate
            && routing.members.iter().any(|member| member == id)
        {
            return Err(app_err!("请先从聚合成员中移除并重新应用，再删除供应商"));
        }
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

    fn desktop_gateway_config(
        &self,
        selected: &StoredClaudeDesktopProfile,
        routing: &ClaudeDesktopRouting,
    ) -> AppResult<BTreeMap<String, GatewayRoute>> {
        let mut routes = BTreeMap::new();
        let mut ids = vec![selected.id.clone()];
        if routing.mode == ClaudeDesktopMode::Aggregate {
            for id in &routing.members {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
        }
        for id in ids {
            let profile = self.database.claude_desktop_profile(&id)?;
            let base = profile
                .base_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| app_err!("供应商 {} 缺少 API 地址", profile.name))?;
            let key = profile
                .api_key
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| app_err!("供应商 {} 缺少 API Key", profile.name))?;
            validate_desktop_api_key(Some(key))?;
            let models = parse_models(&profile.models);
            if routing.mode == ClaudeDesktopMode::Direct {
                if models
                    .iter()
                    .any(|model| direct_model_id(&profile, model).is_none())
                {
                    return Err(app_err!("直连需要 Claude 角色模型 ID，请改用路由或聚合"));
                }
                continue;
            }
            if models.is_empty() {
                return Err(app_err!("供应商 {} 至少需要一个模型", profile.name));
            }
            for model in models {
                let route_id = if id == selected.id {
                    canonical_catalog(&model.slot)
                        .ok_or_else(|| app_err!("模型角色无效"))?
                        .to_string()
                } else {
                    let digest = Sha256::digest(format!("{id}\0{}", model.name).as_bytes());
                    format!(
                        "claude-sonnet-agmux-{}",
                        digest[..8]
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>()
                    )
                };
                let label = model
                    .label_override
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or_else(|| model.name.clone());
                routes.insert(
                    route_id,
                    GatewayRoute {
                        base: base.to_string(),
                        key: key.to_string(),
                        scheme: profile.auth_scheme.clone(),
                        model: model.name,
                        label: format!("{} / {label}", profile.name),
                        supports_1m: model.supports_1m,
                    },
                );
            }
        }
        if routes.len() > 200 {
            return Err(app_err!("Claude Desktop 最多支持 200 个模型"));
        }
        Ok(routes)
    }

    pub fn restore_desktop_gateway(&self) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let routing = self.database.claude_desktop_routing()?;
        if routing.mode == ClaudeDesktopMode::Direct
            || routing.port == 0
            || routing.token.is_empty()
            || self
                .desktop_gateway
                .lock()
                .map_err(|_| app_err!("路由状态锁已损坏"))?
                .as_ref()
                .is_some_and(|gateway| gateway.is_running())
        {
            return Ok(());
        }
        let Some(id) = self.database.active_claude_desktop_profile()? else {
            return Ok(());
        };
        let _ = self.database.claude_desktop_profile(&id)?;
        let routes = serde_json::from_str(
            routing
                .gateway_routes
                .as_deref()
                .ok_or_else(|| app_err!("缺少已应用的网关快照，请重新应用"))?,
        )
        .map_err(|error| app_err!("网关快照无效: {error}"))?;
        let client = tauri::async_runtime::block_on(crate::network::Network::current())
            .map_err(|error| app_err!("{error}"))?
            .client()
            .map_err(|error| app_err!("{error}"))?;
        let handle = claude_desktop_gateway::start_blocking(
            GatewayConfig {
                token: routing.token,
                routes,
            },
            client,
            routing.port,
        )?;
        *self
            .desktop_gateway
            .lock()
            .map_err(|_| app_err!("路由状态锁已损坏"))? = Some(handle);
        Ok(())
    }

    /// 聚合开关只记住选择；不改 live、当前网关或激活供应商。
    pub fn claude_desktop_set_members(&self, members: Vec<String>) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let mut selected = Vec::new();
        for id in members {
            self.database.claude_desktop_profile(&id)?;
            if !selected.contains(&id) {
                selected.push(id);
            }
        }
        let mut routing = self.database.claude_desktop_routing()?;
        routing.selected_members = Some(selected);
        self.database.save_claude_desktop_routing(&routing)
    }

    pub fn claude_desktop_apply(&self, id: &str) -> AppResult<()> {
        self.claude_desktop_apply_confirmed(id, false, None, None)
            .map_err(|error| app_err!("{error}"))
    }

    /// 写 3P 四件套：两个 claude_desktop_config.json 置 deploymentMode=3p、
    /// profile 本体、_meta.json（entries 只保留当前激活项与外来条目）。
    /// 上一家激活配置的 profile 文件一并撤下，configLibrary 不残留半管理条目。
    pub fn claude_desktop_apply_confirmed(
        &self,
        id: &str,
        overwrite: bool,
        mode: Option<ClaudeDesktopMode>,
        members: Option<Vec<String>>,
    ) -> Result<(), ProfileApplyError> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let paths = self.desktop3p();
        let stored = self.database.claude_desktop_profile(id)?;
        let mut routing = self.database.claude_desktop_routing()?;
        if let Some(mode) = mode {
            routing.mode = mode;
        }
        if routing.mode == ClaudeDesktopMode::Aggregate {
            let selected = members
                .or_else(|| routing.selected_members.clone())
                .unwrap_or_else(|| routing.members.clone());
            routing.members = vec![id.to_string()];
            for member in selected {
                if !routing.members.contains(&member) {
                    routing.members.push(member);
                }
            }
            routing.selected_members = Some(routing.members.clone());
        }
        let route = routing.mode != ClaudeDesktopMode::Direct;
        let config = self.desktop_gateway_config(&stored, &routing)?;
        routing.gateway_routes = if route {
            Some(
                serde_json::to_string(&config)
                    .map_err(|error| app_err!("网关快照序列化失败: {error}"))?,
            )
        } else {
            None
        };
        let mut pending_gateway = None;
        let mut gateway = None;
        if route {
            if routing.token.is_empty() {
                routing.token = new_gateway_token()?;
            }
            let client = tauri::async_runtime::block_on(crate::network::Network::current())
                .map_err(|error| app_err!("{error}"))?
                .client()
                .map_err(|error| app_err!("{error}"))?;
            // 切换使用新端口，旧网关保留到落盘成功；重启恢复才重用已保存端口。
            let handle = claude_desktop_gateway::start_blocking(
                GatewayConfig {
                    token: routing.token.clone(),
                    routes: config.clone(),
                },
                client,
                0,
            )?;
            routing.port = handle.port;
            gateway = Some((
                format!("http://127.0.0.1:{}", handle.port),
                routing.token.clone(),
            ));
            pending_gateway = Some(handle);
        }
        routing.snapshot = Some(
            String::from_utf8(render_profile(&stored, None)?)
                .map_err(|error| app_err!("配置编码失败: {error}"))?,
        );
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
                        .map(|(url, token)| (url.as_str(), token.as_str(), &config)),
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
                Some(id),
                &routing,
                &format!("profile_id={id} profile_name={:?}", stored.name),
                &now_ms().to_string(),
            )
        });
        result?;
        let mut current = self
            .desktop_gateway
            .lock()
            .map_err(|_| app_err!("路由网关状态锁已损坏"))?;
        if let Some(previous) = std::mem::replace(&mut *current, pending_gateway) {
            previous.stop();
        }
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
            self.database.commit_claude_desktop_apply(
                None,
                &ClaudeDesktopRouting {
                    selected_members: {
                        let routing = self.database.claude_desktop_routing()?;
                        routing.selected_members.or(Some(routing.members))
                    },
                    ..Default::default()
                },
                "official",
                &now_ms().to_string(),
            )
        })?;
        self.stop_desktop_gateway();
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
        let mut live: ProfileFile = match serde_json::from_str(&raw) {
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
        let mut routing = self.database.claude_desktop_routing()?;
        let snapshot = String::from_utf8(render_profile(&stored, None)?)
            .map_err(|error| app_err!("配置编码失败: {error}"))?;
        if routing.mode != ClaudeDesktopMode::Direct
            || routing
                .snapshot
                .as_ref()
                .is_some_and(|applied| applied != &snapshot)
        {
            // 本地地址与令牌是应用产物，不是供应商凭证或上游模型映射。
            return Ok(sync::SyncOutcome::of(
                sync::SyncKind::Unchanged,
                &stored.name,
            ));
        }
        if !routing.token.is_empty() && live.api_key.as_deref() == Some(routing.token.as_str()) {
            return Ok(sync::SyncOutcome::of(
                sync::SyncKind::TargetMismatch,
                &stored.name,
            ));
        }
        if live
            .api_key
            .as_deref()
            .is_none_or(|key| key.trim().is_empty() || is_desktop_encrypted_key(key))
        {
            if live.base_url != stored.base_url {
                tauri_plugin_log::log::warn!("[sync.claude-desktop] outcome=skipped failure_kind=unreadable_credential_target_changed msg=\"桌面端端点已变且凭证不可导入，保留供应商配置\"");
                return Ok(sync::SyncOutcome::of(
                    sync::SyncKind::TargetMismatch,
                    &stored.name,
                ));
            }
            live.api_key = stored.api_key.clone();
        }
        let mut live_models = parse_live_models(&live.inference_models);
        if live.base_url == stored.base_url {
            for (live, expected) in live_models.iter_mut().zip(parse_models(&stored.models)) {
                if direct_model_id(&stored, &expected).as_deref() == Some(live.name.as_str()) {
                    if live.name != expected.name
                        && expected.label_override.is_none()
                        && live.label_override.as_deref() == Some(expected.name.as_str())
                    {
                        live.label_override = None;
                    }
                    live.name = expected.name;
                    live.slot = expected.slot;
                }
            }
        }
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
        let input = ClaudeDesktopProfileInput {
            name: stored.name.clone(),
            kind: stored.kind.clone(),
            base_url: live.base_url,
            api_key: live.api_key,
            auth_scheme: Some(live_scheme),
            models: Some(live_models),
            admin_url: stored.admin_url.clone(),
            icon: stored.icon.clone(),
            show_balance: stored.show_balance,
            description: stored.description.clone(),
            fetched_models: Some(parse_fetched_models(&stored.fetched_models)),
        };
        let mut synced = stored.clone();
        synced.base_url = input.base_url.clone();
        synced.api_key = input.api_key.clone();
        synced.auth_scheme = input.auth_scheme.clone().unwrap_or_else(|| "bearer".into());
        synced.models = Some(
            serde_json::to_string(&input.models)
                .map_err(|error| app_err!("模型序列化失败: {error}"))?,
        );
        routing.snapshot = Some(
            String::from_utf8(render_profile(&synced, None)?)
                .map_err(|error| app_err!("配置编码失败: {error}"))?,
        );
        self.database.update_claude_desktop_profile(
            &id,
            &input,
            &now_ms().to_string(),
            Some(&routing),
        )?;
        Ok(sync::SyncOutcome::of(sync::SyncKind::Wrote, &stored.name))
    }

    /// 卡片测试连通：复用 Claude 侧的 /v1/models 探测（/anthropic 后缀剥回同根）。
    pub async fn claude_desktop_test_profile(
        &self,
        id: &str,
    ) -> AppResult<super::CodexProfileConnectionResult> {
        let stored = self.database.claude_desktop_profile(id)?;
        let (base_url, api_key) = (stored.base_url.as_deref(), stored.api_key.as_deref());
        let base_url = base_url
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| app_err!("请先填写 API 地址"))?;
        let api_key = api_key
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| app_err!("请先填写 API Key"))?;
        validate_desktop_api_key(Some(api_key))?;
        super::test_claude_connection(base_url, api_key).await
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
            crate::paths::home_dir().join("Applications"),
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
            mode: ClaudeDesktopMode::Direct,
            members: Vec::new(),
            selected_members: None,
            gateway_running: false,
        },
        None => ClaudeDesktopAppStatus {
            installed: false,
            running,
            display_path: "未识别".into(),
            source: "not_found".into(),
            deployment_mode,
            mode: ClaudeDesktopMode::Direct,
            members: Vec::new(),
            selected_members: None,
            gateway_running: false,
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
            fetched_models: None,
        }
    }

    #[test]
    fn desktop_model_ids_require_a_supported_role_and_version() {
        for name in [
            "claude-sonnet-5",
            "anthropic/claude-opus-5",
            "claude-fable-5-1",
        ] {
            assert!(is_desktop_model_id(name), "{name}");
        }
        for name in [
            "claude-custom",
            "claude-sonnet-",
            "claude-3-5-sonnet",
            "claude-opus-5[1m]",
        ] {
            assert!(!is_desktop_model_id(name), "{name}");
        }
    }

    #[test]
    fn request_model_alone_supports_mapping_and_display_name_defaults_to_it() {
        let (_home, context) = test_context();
        let mut input = deepseek_input();
        input.auth_scheme = None;
        input.base_url = Some("https://relay.example/anthropic".into());
        input.models = Some(vec![ClaudeDesktopModelSlot {
            slot: "sonnet".into(),
            name: "deepseek-flash".into(),
            label_override: None,
            supports_1m: false,
            upstream_model: None,
        }]);
        let created = context.claude_desktop_save(None, input).unwrap();
        let stored = context
            .database
            .claude_desktop_profile(&created.id)
            .unwrap();
        assert!(summary(&stored).requires_routing);
        assert_eq!(created.auth_scheme, "bearer");
        assert_eq!(created.models[0].name, "deepseek-flash");
        assert!(created.models[0].label_override.is_none());
        assert!(context
            .claude_desktop_apply_confirmed(
                &created.id,
                false,
                Some(ClaudeDesktopMode::Direct),
                None
            )
            .is_err());
        context
            .claude_desktop_apply_confirmed(
                &created.id,
                false,
                Some(ClaudeDesktopMode::Route),
                None,
            )
            .unwrap();
        let live: Value = serde_json::from_slice(
            &std::fs::read(context.desktop3p().profile(&stored.config_uuid)).unwrap(),
        )
        .unwrap();
        assert_eq!(live["inferenceModels"][0]["name"], "claude-sonnet-5-5");
        assert_eq!(
            live["inferenceModels"][0]["labelOverride"],
            "DeepSeek / deepseek-flash"
        );
        assert_eq!(
            context.claude_desktop_get(&created.id).unwrap().models[0].name,
            "deepseek-flash"
        );
        context.claude_desktop_restore_official().unwrap();
    }

    #[test]
    fn aggregate_switches_persist_without_applying_and_allow_remembering_none() {
        let (_home, context) = test_context();
        let active = applied_profile(&context);
        let second = context
            .claude_desktop_save(
                None,
                ClaudeDesktopProfileInput {
                    name: "Second".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let path = context.desktop3p().profile(&active.config_uuid);
        let before = std::fs::read(&path).unwrap();
        let applied = context.database.claude_desktop_routing().unwrap();
        context
            .claude_desktop_set_members(vec![second.id.clone(), second.id.clone()])
            .unwrap();
        let routing = context.database.claude_desktop_routing().unwrap();
        assert_eq!(routing.selected_members, Some(vec![second.id.clone()]));
        assert_eq!(routing.members, applied.members);
        assert_eq!(routing.mode, applied.mode);
        assert_eq!(routing.snapshot, applied.snapshot);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            context
                .database
                .active_claude_desktop_profile()
                .unwrap()
                .as_deref(),
            Some(active.id.as_str())
        );
        let reopened = crate::database::Database::open(&context.paths).unwrap();
        assert_eq!(
            reopened.claude_desktop_routing().unwrap().selected_members,
            Some(vec![second.id.clone()])
        );
        assert!(context
            .claude_desktop_set_members(vec!["missing-id".into()])
            .is_err());
        assert_eq!(
            context
                .database
                .claude_desktop_routing()
                .unwrap()
                .selected_members,
            routing.selected_members
        );
        context.claude_desktop_set_members(vec![]).unwrap();
        assert_eq!(
            context.claude_desktop_status().unwrap().selected_members,
            Some(vec![])
        );
        context.claude_desktop_apply(&active.id).unwrap();
        assert_eq!(
            context
                .database
                .claude_desktop_routing()
                .unwrap()
                .selected_members,
            Some(vec![])
        );
        context
            .claude_desktop_set_members(vec![second.id.clone()])
            .unwrap();
        context.claude_desktop_restore_official().unwrap();
        assert_eq!(
            context
                .database
                .claude_desktop_routing()
                .unwrap()
                .selected_members,
            Some(vec![second.id.clone()])
        );
        context.claude_desktop_delete(&second.id).unwrap();
        assert_eq!(
            context
                .database
                .claude_desktop_routing()
                .unwrap()
                .selected_members,
            Some(vec![])
        );
    }

    #[test]
    fn generated_aliases_declare_behavior_without_rewriting_routing_or_real_models() {
        let (_home, context) = test_context();
        let profile = context.claude_desktop_save(None, deepseek_input()).unwrap();
        let stored = context
            .database
            .claude_desktop_profile(&profile.id)
            .unwrap();
        let routes = BTreeMap::from([(
            "claude-sonnet-agmux-012345abcdef".into(),
            GatewayRoute {
                base: "https://upstream.example".into(),
                key: "private-key".into(),
                scheme: "bearer".into(),
                model: "actual-model".into(),
                label: "Provider / Model".into(),
                supports_1m: false,
            },
        )]);
        let profile: Value = serde_json::from_slice(
            &render_profile(
                &stored,
                Some(("http://127.0.0.1:12345", "local-token", &routes)),
            )
            .unwrap(),
        )
        .unwrap();
        let model = &profile["inferenceModels"][0];
        assert_eq!(model["name"], "claude-sonnet-agmux-012345abcdef");
        assert_eq!(model["behavesAs"], "claude-sonnet-5-5");
        assert_eq!(
            routes["claude-sonnet-agmux-012345abcdef"].model,
            "actual-model"
        );
        assert_eq!(
            alias_behavior("claude-opus-agmux-sonnet"),
            Some("claude-opus-5-5")
        );
        for name in [
            "claude-sonnet-4-5",
            "claude-opus-5-5",
            "deepseek-flash",
            "custom-alias",
        ] {
            assert_eq!(alias_behavior(name), None);
        }
    }

    #[test]
    fn official_deepseek_requests_stay_direct_and_flash_never_calls_pro() {
        let (_home, context) = test_context();
        let mut input = deepseek_input();
        input.models = Some(
            SLOT_ORDER
                .iter()
                .map(|slot| ClaudeDesktopModelSlot {
                    slot: (*slot).into(),
                    name: "deepseek-flash".into(),
                    label_override: None,
                    supports_1m: false,
                    upstream_model: None,
                })
                .collect(),
        );
        let created = context.claude_desktop_save(None, input).unwrap();
        let stored = context
            .database
            .claude_desktop_profile(&created.id)
            .unwrap();
        assert!(!summary(&stored).requires_routing);
        context
            .claude_desktop_apply_confirmed(
                &created.id,
                false,
                Some(ClaudeDesktopMode::Direct),
                None,
            )
            .unwrap();
        let live: Value = serde_json::from_slice(
            &std::fs::read(context.desktop3p().profile(&stored.config_uuid)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            live["inferenceGatewayBaseUrl"],
            "https://api.deepseek.com/anthropic"
        );
        for model in live["inferenceModels"].as_array().unwrap() {
            let id = model["name"].as_str().unwrap();
            assert!(id.starts_with("claude-sonnet-") || id.starts_with("claude-haiku-"));
            assert_eq!(model["behavesAs"].as_str(), alias_behavior(id));
            assert!(
                model["behavesAs"]
                    .as_str()
                    .unwrap()
                    .starts_with("claude-sonnet-")
                    || model["behavesAs"]
                        .as_str()
                        .unwrap()
                        .starts_with("claude-haiku-")
            );
            assert_eq!(model["labelOverride"], "deepseek-flash");
        }
        assert!(!context.claude_desktop_status().unwrap().gateway_running);
        assert_eq!(
            context.sync_active_claude_desktop().unwrap().kind,
            sync::SyncKind::Unchanged
        );
        assert!(context
            .claude_desktop_get(&created.id)
            .unwrap()
            .models
            .iter()
            .all(|model| model.name == "deepseek-flash"));
        let mut pro = created.models[0].clone();
        pro.name = "deepseek-v4-pro".into();
        assert!(direct_model_id(&stored, &pro)
            .unwrap()
            .starts_with("claude-opus-"));
        pro.name = "unknown-model".into();
        assert!(direct_model_id(&stored, &pro).is_none());
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
    fn direct_without_manual_models_uses_discovery_and_route_requires_explicit_model() {
        let (_home, context) = test_context();
        let mut input = deepseek_input();
        input.models = Some(Vec::new());
        let created = context.claude_desktop_save(None, input).unwrap();
        context.claude_desktop_apply(&created.id).unwrap();
        let stored = context
            .database
            .claude_desktop_profile(&created.id)
            .unwrap();
        let path = context.desktop3p().profile(&stored.config_uuid);
        let before = std::fs::read(&path).unwrap();
        let live: Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(live["modelDiscoveryEnabled"], true);
        assert!(live.get("inferenceModels").is_none());
        assert!(context
            .claude_desktop_apply_confirmed(
                &created.id,
                false,
                Some(ClaudeDesktopMode::Route),
                None
            )
            .is_err());
        let diagnostics = context.claude_desktop_diagnostics().unwrap();
        assert!(diagnostics.live_matches);
        assert!(!diagnostics.gateway_running);
        assert!(diagnostics.gateway_url.is_none());
        assert!(diagnostics.activity.started_at_ms.is_none());
        assert!(diagnostics.activity.models.is_empty());
        let mut modified = live;
        modified["inferenceGatewayBaseUrl"] = json!("https://changed.example");
        std::fs::write(&path, serde_json::to_vec(&modified).unwrap()).unwrap();
        assert!(!context.claude_desktop_diagnostics().unwrap().live_matches);
        std::fs::write(&path, &before).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert_eq!(
            context.database.claude_desktop_routing().unwrap().mode,
            ClaudeDesktopMode::Direct
        );
    }

    #[test]
    fn save_keeps_live_and_saved_draft_survives_passive_sync() {
        let (_home, context) = test_context();
        let stored = applied_profile(&context);
        let path = context.desktop3p().profile(&stored.config_uuid);
        let before = std::fs::read(&path).unwrap();
        let mut input = deepseek_input();
        input.api_key = Some("edited-key".into());
        context
            .claude_desktop_save(Some(&stored.id), input)
            .unwrap();
        context.get_state().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            context
                .claude_desktop_get(&stored.id)
                .unwrap()
                .api_key
                .as_deref(),
            Some("edited-key")
        );
        context.claude_desktop_apply(&stored.id).unwrap();
        let live: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(live["inferenceGatewayApiKey"], "edited-key");
    }

    #[test]
    fn aggregation_routes_credentials_and_models_and_restores_only_applied_snapshot() {
        use axum::{body::Body, http::Request, response::IntoResponse, routing::any};
        tauri::async_runtime::block_on(async {
            async fn echo(request: Request<Body>) -> axum::response::Response {
                let key = request
                    .headers()
                    .get("authorization")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                let body = axum::body::to_bytes(request.into_body(), 4096)
                    .await
                    .unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap();
                if body["fixture"] == "stream" || body["fixture"] == "late_error" {
                    let tail = if body["fixture"] == "stream" {
                        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\ndata: {\"type\":\"message_stop\"}\n\n"
                    } else {
                        "data: {\"type\":\"error\",\"error\":{\"message\":\"private-error\"}}\n\n"
                    };
                    return ([("content-type", "text/event-stream")], format!("data: {{\"type\":\"message_start\",\"message\":{{\"usage\":{{\"input_tokens\":10,\"output_tokens\":1}}}}}}\n\n{tail}")).into_response();
                }
                if body["fixture"] == "http_error" {
                    return (axum::http::StatusCode::TOO_MANY_REQUESTS, "private-error")
                        .into_response();
                }
                axum::Json(json!({"key":key, "model":body["model"], "usage":{"input_tokens":10,"output_tokens":4}, "thinking": body["thinking"], "output_config": body["output_config"]})).into_response()
            }
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tauri::async_runtime::spawn(async move {
                axum::serve(listener, axum::Router::new().fallback(any(echo)))
                    .await
                    .unwrap();
            });
            let (_home, context) = test_context();
            let create = |name: &str, model: &str, key: &str| {
                context
                    .claude_desktop_save(
                        None,
                        ClaudeDesktopProfileInput {
                            name: name.into(),
                            base_url: Some(base.clone()),
                            api_key: Some(key.into()),
                            models: Some(vec![ClaudeDesktopModelSlot {
                                slot: "sonnet".into(),
                                name: model.into(),
                                label_override: None,
                                supports_1m: false,
                                upstream_model: None,
                            }]),
                            ..Default::default()
                        },
                    )
                    .unwrap()
            };
            let first = create("First", "glm-fixture", "first-key");
            let second = create("Second", "kimi-fixture", "second-key");
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            // apply uses a blocking runtime bridge, just as the IPC command does.
            let applying = context.clone();
            let id = first.id.clone();
            let member = second.id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                applying.claude_desktop_apply_confirmed(
                    &id,
                    false,
                    Some(ClaudeDesktopMode::Aggregate),
                    Some(vec![member]),
                )
            })
            .await
            .unwrap()
            .unwrap();
            let routing = context.database.claude_desktop_routing().unwrap();
            let url = format!("http://127.0.0.1:{}", routing.port);
            let catalog: Value = client
                .get(format!("{url}/v1/models"))
                .bearer_auth(&routing.token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(catalog["data"].as_array().unwrap().len(), 2);
            for model in catalog["data"].as_array().unwrap() {
                let response: Value = client
                    .post(format!("{url}/v1/messages"))
                    .bearer_auth(&routing.token)
                    .json(&json!({"model":model["id"],"messages":[],"thinking":{"type":"adaptive"},"output_config":{"effort":"max"}}))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                assert_eq!(response["thinking"], json!({"type":"adaptive"}));
                assert_eq!(response["output_config"], json!({"effort":"max"}));
                if model["id"]
                    .as_str()
                    .unwrap()
                    .starts_with("claude-sonnet-agmux-")
                {
                    assert_eq!(model["behaves_as"], "claude-sonnet-5-5");
                }
                if model["display_name"]
                    .as_str()
                    .unwrap()
                    .starts_with("First /")
                {
                    assert_eq!(response["key"], "Bearer first-key");
                    assert_eq!(response["model"], "glm-fixture");
                } else {
                    assert_eq!(response["key"], "Bearer second-key");
                    assert_eq!(response["model"], "kimi-fixture");
                }
            }
            let model = &catalog["data"][0]["id"];
            for fixture in ["stream", "late_error", "http_error"] {
                let response = client
                    .post(format!("{url}/v1/messages"))
                    .bearer_auth(&routing.token)
                    .json(&json!({"model":model,"fixture":fixture}))
                    .send()
                    .await
                    .unwrap();
                let status = response.status();
                let raw = response.text().await.unwrap();
                if fixture == "http_error" {
                    assert_eq!(status.as_u16(), 429);
                } else {
                    assert_eq!(status.as_u16(), 200);
                    assert!(raw.starts_with("data: {\"type\":\"message_start\""));
                }
            }
            let diagnostics = context.claude_desktop_diagnostics().unwrap();
            assert!(diagnostics.live_matches);
            assert!(diagnostics.gateway_running);
            assert_eq!(diagnostics.routes.len(), 2);
            assert_eq!(
                diagnostics
                    .activity
                    .models
                    .iter()
                    .map(|model| model.requests)
                    .sum::<u64>(),
                5
            );
            assert_eq!(
                diagnostics
                    .activity
                    .models
                    .iter()
                    .map(|model| model.completed)
                    .sum::<u64>(),
                3
            );
            assert_eq!(
                diagnostics
                    .activity
                    .models
                    .iter()
                    .map(|model| model.failed)
                    .sum::<u64>(),
                2
            );
            assert_eq!(
                diagnostics
                    .activity
                    .models
                    .iter()
                    .filter_map(|model| model.tokens.input_tokens)
                    .sum::<u64>(),
                40
            );
            assert_eq!(
                diagnostics
                    .activity
                    .models
                    .iter()
                    .filter_map(|model| model.tokens.output_tokens)
                    .sum::<u64>(),
                13
            );
            assert!(diagnostics
                .activity
                .requests
                .iter()
                .any(|event| event.status == Some(200)
                    && event.outcome == crate::models::ClaudeDesktopRequestOutcome::StreamError));
            let serialized = serde_json::to_string(&diagnostics).unwrap();
            for secret in [
                "first-key",
                "second-key",
                routing.token.as_str(),
                "private-error",
            ] {
                assert!(!serialized.contains(secret));
            }
            assert_eq!(
                client
                    .get(format!("{url}/v1/models"))
                    .bearer_auth("wrong")
                    .send()
                    .await
                    .unwrap()
                    .status(),
                401
            );
            assert_eq!(
                client
                    .post(format!("{url}/v1/messages"))
                    .bearer_auth(&routing.token)
                    .json(&json!({"model":"unknown"}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                400
            );
            assert_eq!(routing.members, vec![first.id.clone(), second.id.clone()]);
            assert_eq!(routing.selected_members.as_ref(), Some(&routing.members));
            context
                .claude_desktop_set_members(vec![second.id.clone()])
                .unwrap();
            let selected = context.database.claude_desktop_routing().unwrap();
            assert_eq!(selected.selected_members, Some(vec![second.id.clone()]));
            assert_eq!(selected.members, routing.members);
            assert_eq!(
                context.claude_desktop_diagnostics().unwrap().routes.len(),
                2
            );
            // A failed write must keep the existing gateway, active pointer and files.
            let invalid = create("Invalid target", "other-model", "other-key");
            let invalid_stored = context
                .database
                .claude_desktop_profile(&invalid.id)
                .unwrap();
            std::fs::create_dir_all(context.desktop3p().profile(&invalid_stored.config_uuid))
                .unwrap();
            let applying = context.clone();
            tauri::async_runtime::spawn_blocking(move || {
                assert!(applying
                    .claude_desktop_apply_confirmed(
                        &invalid.id,
                        false,
                        Some(ClaudeDesktopMode::Route),
                        None
                    )
                    .is_err());
            })
            .await
            .unwrap();
            assert_eq!(
                context
                    .database
                    .active_claude_desktop_profile()
                    .unwrap()
                    .as_deref(),
                Some(first.id.as_str())
            );
            assert_eq!(
                context.database.claude_desktop_routing().unwrap().port,
                routing.port
            );
            assert_eq!(
                context
                    .database
                    .claude_desktop_routing()
                    .unwrap()
                    .selected_members,
                selected.selected_members
            );
            // Saving doesn't mutate the applied snapshot; process recovery uses its old credential.
            context
                .claude_desktop_save(
                    Some(&second.id),
                    ClaudeDesktopProfileInput {
                        name: second.name.clone(),
                        base_url: Some(base),
                        api_key: Some("unapplied-edit".into()),
                        models: Some(second.models),
                        ..Default::default()
                    },
                )
                .unwrap();
            context.stop_desktop_gateway();
            // graceful shutdown releases the listener asynchronously.
            for _ in 0..100 {
                if tokio::net::TcpListener::bind(("127.0.0.1", routing.port))
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let recovering = context.clone();
            tauri::async_runtime::spawn_blocking(move || recovering.restore_desktop_gateway())
                .await
                .unwrap()
                .unwrap();
            let member_model = catalog["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| {
                    model["display_name"]
                        .as_str()
                        .unwrap()
                        .starts_with("Second /")
                })
                .unwrap();
            let restored: Value = client
                .post(format!("{url}/v1/messages"))
                .bearer_auth(&routing.token)
                .json(&json!({"model":member_model["id"]}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(restored["key"], "Bearer second-key");
            let applying = context.clone();
            tauri::async_runtime::spawn_blocking(move || {
                applying.claude_desktop_apply_confirmed(
                    &first.id,
                    false,
                    Some(ClaudeDesktopMode::Route),
                    None,
                )
            })
            .await
            .unwrap()
            .unwrap();
            let routing = context.database.claude_desktop_routing().unwrap();
            let catalog: Value = client
                .get(format!("http://127.0.0.1:{}/v1/models", routing.port))
                .bearer_auth(&routing.token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(catalog["data"].as_array().unwrap().len(), 1);
            context.claude_desktop_restore_official().unwrap();
            assert!(!context.claude_desktop_status().unwrap().gateway_running);
            assert_eq!(
                context.database.claude_desktop_routing().unwrap().mode,
                ClaudeDesktopMode::Direct
            );
            server.abort();
        });
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
    fn desktop_encrypted_or_missing_key_never_replaces_provider_key() {
        for encrypted in [
            Some("__sealed__:v1:opaque-desktop-blob"),
            Some("__encrypted__:opaque-desktop-blob"),
            None,
            Some("  "),
        ] {
            let (_home, context) = test_context();
            let stored = applied_profile(&context);
            let path = context.desktop3p().profile(&stored.config_uuid);
            let mut live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            live["inferenceGatewayApiKey"] = json!(encrypted);
            let bytes = serde_json::to_vec(&live).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            for _ in 0..2 {
                assert_eq!(
                    context.sync_active_claude_desktop().unwrap().kind,
                    sync::SyncKind::Unchanged
                );
                assert_eq!(
                    context.claude_desktop_get(&stored.id).unwrap().api_key,
                    stored.api_key
                );
            }
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            if encrypted.is_some_and(is_desktop_encrypted_key) {
                assert!(context.claude_desktop_diagnostics().unwrap().live_matches);
            }
            let copy = context.claude_desktop_duplicate(&stored.id).unwrap();
            assert_eq!(copy.api_key, stored.api_key);
            context.claude_desktop_apply(&stored.id).unwrap();
            let live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                live["inferenceGatewayApiKey"].as_str(),
                stored.api_key.as_deref()
            );
        }
    }

    #[test]
    fn encrypted_key_keeps_safe_model_edits_but_rejects_a_different_endpoint() {
        let (_home, context) = test_context();
        let stored = applied_profile(&context);
        let path = context.desktop3p().profile(&stored.config_uuid);
        let mut live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        live["inferenceGatewayApiKey"] = json!("__sealed__:v1:opaque-desktop-blob");
        live["inferenceModels"][0]["labelOverride"] = json!("Edited display name");
        std::fs::write(&path, serde_json::to_vec(&live).unwrap()).unwrap();
        assert_eq!(
            context.sync_active_claude_desktop().unwrap().kind,
            sync::SyncKind::Wrote
        );
        let updated = context.database.claude_desktop_profile(&stored.id).unwrap();
        assert_eq!(updated.api_key, stored.api_key);
        assert_eq!(
            parse_models(&updated.models)[0].label_override.as_deref(),
            Some("Edited display name")
        );
        assert_eq!(
            context.sync_active_claude_desktop().unwrap().kind,
            sync::SyncKind::Unchanged
        );
        live["inferenceGatewayBaseUrl"] = json!("https://different.example/anthropic");
        std::fs::write(&path, serde_json::to_vec(&live).unwrap()).unwrap();
        assert_eq!(
            context.sync_active_claude_desktop().unwrap().kind,
            sync::SyncKind::TargetMismatch
        );
        let unchanged = context.database.claude_desktop_profile(&stored.id).unwrap();
        assert_eq!(unchanged.base_url, stored.base_url);
        assert_eq!(unchanged.api_key, stored.api_key);
    }

    #[test]
    fn opaque_credentials_cannot_be_saved_duplicated_or_applied() {
        let (_home, context) = test_context();
        let stored = applied_profile(&context);
        let path = context.desktop3p().profile(&stored.config_uuid);
        let before = std::fs::read(&path).unwrap();
        let mut input = deepseek_input();
        input.api_key = Some("__sealed__:v1:private-ciphertext".into());
        let error = context
            .claude_desktop_save(Some(&stored.id), input.clone())
            .unwrap_err();
        assert!(!error.to_string().contains("private-ciphertext"));
        assert!(context.claude_desktop_save(None, input.clone()).is_err());
        assert_eq!(
            context
                .database
                .claude_desktop_profile(&stored.id)
                .unwrap()
                .api_key,
            stored.api_key
        );
        let mut live: Value = serde_json::from_slice(&before).unwrap();
        live["inferenceGatewayApiKey"] = json!("__sealed__:v1:private-ciphertext");
        std::fs::write(&path, serde_json::to_vec(&live).unwrap()).unwrap();
        // 旧版已经污染的库行也不能显示为可用凭证或被再次应用。
        context
            .database
            .update_claude_desktop_profile(&stored.id, &input, "fixture", None)
            .unwrap();
        let contaminated = context.database.claude_desktop_profile(&stored.id).unwrap();
        assert!(!summary(&contaminated).has_key);
        assert!(detail(contaminated).api_key.is_none());
        assert!(context.claude_desktop_duplicate(&stored.id).is_err());
        let encrypted_live = std::fs::read(&path).unwrap();
        assert!(context.claude_desktop_apply(&stored.id).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), encrypted_live);
        assert_eq!(context.database.claude_desktop_profiles().unwrap().len(), 1);
    }

    #[test]
    fn direct_sync_never_imports_a_stale_gateway_token() {
        let (_home, context) = test_context();
        let stored = applied_profile(&context);
        let path = context.desktop3p().profile(&stored.config_uuid);
        let mut routing = context.database.claude_desktop_routing().unwrap();
        routing.token = "local-gateway-secret".into();
        context
            .database
            .commit_claude_desktop_apply(Some(&stored.id), &routing, "fixture", "fixture")
            .unwrap();
        let mut live: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        live["inferenceGatewayBaseUrl"] = json!("http://127.0.0.1:12345");
        live["inferenceGatewayApiKey"] = json!(routing.token);
        std::fs::write(path, serde_json::to_vec(&live).unwrap()).unwrap();
        assert_eq!(
            context.sync_active_claude_desktop().unwrap().kind,
            sync::SyncKind::TargetMismatch
        );
        let unchanged = context.database.claude_desktop_profile(&stored.id).unwrap();
        assert_eq!(unchanged.api_key, stored.api_key);
        assert_eq!(unchanged.base_url, stored.base_url);
        assert_eq!(context.database.claude_desktop_profiles().unwrap().len(), 1);
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
    fn damaged_live_requires_confirmation_before_overwrite() {
        let (_home, context) = test_context();
        let paths = context.desktop3p();
        let stored = applied_profile(&context);
        std::fs::write(paths.profile(&stored.config_uuid), "{ broken").unwrap();
        let error = context
            .claude_desktop_apply_confirmed(&stored.id, false, None, None)
            .unwrap_err();
        assert!(matches!(error, ProfileApplyError::OverwriteRequired { .. }));
        // 确认后可覆盖自愈
        context
            .claude_desktop_apply_confirmed(&stored.id, true, None, None)
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
