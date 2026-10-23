//! 使用 Codex 官方 stdio 协议读取安装清单与 MCP 状态，不读取凭证或修改账号。
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{app_err, AppResult};

use super::cli::{drain_pipe, find_codex_cli, CODEX_RUNTIME_NOT_FOUND};
use super::{CodexAppSummary, MarketplacePlugin, PluginSummary};

pub(crate) struct AppServer {
    child: Child,
    responses: Receiver<Result<Value, serde_json::Error>>,
    next_id: u64,
    deadline: Instant,
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AppServer {
    fn start(home: &Path) -> AppResult<Self> {
        Self::start_with_codex_home(home, &home.join(".codex"))
    }

    pub(crate) fn start_with_codex_home(home: &Path, codex_home: &Path) -> AppResult<Self> {
        let executable =
            find_codex_cli(home).ok_or_else(|| app_err!("{CODEX_RUNTIME_NOT_FOUND}"))?;
        let mut command = crate::services::cli::command(&executable);
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(home)
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::network::Network::detect()
            .map_err(|error| app_err!("网络设置无效: {error}"))?
            .apply(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| app_err!("无法启动 Codex app-server: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| app_err!("Codex 输出管道不可用"))?;
        let stderr = child.stderr.take();
        std::thread::spawn(move || drain_pipe(stderr));
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(serde_json::from_str(&line)).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            responses,
            next_id: 0,
            deadline: Instant::now() + Duration::from_secs(60),
        };
        server.request(
            "initialize",
            json!({
                "clientInfo": {"name": "agmux", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true}
            }),
        )?;
        server.send(json!({"method": "initialized"}))?;
        Ok(server)
    }

    fn send(&mut self, message: Value) -> AppResult<()> {
        let input = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| app_err!("Codex 输入管道不可用"))?;
        writeln!(input, "{message}")
            .and_then(|_| input.flush())
            .map_err(|error| app_err!("Codex 请求发送失败: {error}"))
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> AppResult<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id": id, "method": method, "params": params}))?;
        loop {
            let response = self
                .responses
                .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| app_err!("Codex {method} 未返回结果: {error}"))?
                .map_err(|error| app_err!("Codex {method} 返回无效 JSON: {error}"))?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                // 服务端文本可能含配置原文，只显示协议错误码。
                return Err(app_err!(
                    "Codex 不支持或无法完成 {method}（错误码 {}），请更新 Codex 后重试",
                    error["code"]
                ));
            }
            return response
                .get("result")
                .cloned()
                .ok_or_else(|| app_err!("Codex {method} 响应缺少 result"));
        }
    }
}

#[derive(Deserialize)]
pub(super) struct InstalledPlugins {
    pub marketplaces: Vec<InstalledMarketplace>,
}

#[derive(Deserialize)]
pub(super) struct InstalledMarketplace {
    pub name: String,
    pub plugins: Vec<InstalledPlugin>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub install_policy: String,
    pub install_policy_source: Option<String>,
    #[serde(default)]
    pub source: Value,
    pub version: Option<String>,
    pub local_version: Option<String>,
    pub interface: Option<Value>,
    #[serde(default)]
    pub installed: bool,
    pub remote_plugin_id: Option<String>,
    pub share_context: Option<Value>,
    #[serde(default)]
    pub availability: String,
    #[serde(default)]
    pub auth_policy: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    marketplaces: Vec<InstalledMarketplace>,
    marketplace_load_errors: Vec<Value>,
}

pub(super) fn plugin_catalog(home: &Path) -> AppResult<Vec<MarketplacePlugin>> {
    let catalog: Catalog = serde_json::from_value(
        AppServer::start(home)?.request("plugin/list", json!({"cwds": [home]}))?,
    )
    .map_err(|error| app_err!("Codex 插件目录无效: {error}"))?;
    if !catalog.marketplace_load_errors.is_empty() {
        // 失败详情可能内嵌仓库地址或配置原文，只提取市场标识进文案与日志，指名后用户才知道该移除哪个市场。
        let failed: Vec<&str> = catalog
            .marketplace_load_errors
            .iter()
            .filter_map(|error| {
                ["marketplace", "marketplaceName", "name"]
                    .iter()
                    .find_map(|key| error.get(*key).and_then(Value::as_str))
            })
            .collect();
        tauri_plugin_log::log::warn!(
            "[plugin.catalog] outcome=failure failure_kind=protocol_error marketplaces={:?} msg=\"Codex 部分插件市场加载失败\"",
            failed
        );
        return Err(if failed.is_empty() {
            app_err!("Codex 部分插件市场加载失败，请检查账号和网络后重试")
        } else {
            app_err!(
                "Codex 插件市场「{}」加载失败，请检查账号和网络，或移除失败的市场后重试",
                failed.join("、")
            )
        });
    }
    let has_remote = catalog
        .marketplaces
        .iter()
        .any(|market| market.name == "openai-curated-remote");
    let mut seen = std::collections::HashMap::<String, usize>::new();
    let mut plugins: Vec<MarketplacePlugin> = Vec::new();
    for market in catalog.marketplaces {
        for entry in market.plugins {
            if entry.availability.is_empty() || entry.install_policy.is_empty() {
                return Err(app_err!(
                    "Codex 插件目录缺少可安装状态，请更新 Codex 后重试"
                ));
            }
            if !desktop_plugin(&entry, &market.name)
                || (has_remote && market.name == "openai-curated")
                || entry.availability != "AVAILABLE"
                || entry.install_policy != "AVAILABLE"
            {
                continue;
            }
            let canonical = entry
                .remote_plugin_id
                .as_deref()
                .or_else(|| {
                    entry
                        .share_context
                        .as_ref()
                        .and_then(|context| context.get("remotePluginId"))
                        .and_then(Value::as_str)
                })
                .unwrap_or(&entry.id);
            if let Some(&index) = seen.get(canonical) {
                if entry.installed {
                    plugins[index].installed = true;
                }
                continue;
            }
            seen.insert(canonical.to_owned(), plugins.len());
            let (name, marketplace) = entry
                .id
                .rsplit_once('@')
                .ok_or_else(|| app_err!("Codex 返回无效插件标识"))?;
            super::catalog::validate_plugin_name(name)?;
            super::catalog::validate_plugin_name(marketplace)?;
            if name != entry.name || marketplace != market.name {
                return Err(app_err!("Codex 插件目录的安装身份不一致"));
            }
            let local = entry
                .source
                .get("path")
                .and_then(Value::as_str)
                .map(Path::new);
            let manifest = local.and_then(super::catalog::read_manifest);
            let icon_path = local
                .and_then(|path| super::store::plugin_icon_from(manifest.as_ref(), path))
                .map(|path| path.display().to_string());
            let mut summary = PluginSummary {
                icon_path,
                ..Default::default()
            };
            apply_interface(&mut summary, entry.interface.as_ref());
            plugins.push(MarketplacePlugin {
                plugin_id: entry.id,
                name: entry.name,
                version: entry.version,
                installed: entry.installed,
                auth_policy: entry.auth_policy,
                source: super::catalog::plugin_source_label(Some(&entry.source)),
                display_name: summary.display_name,
                description: summary.description,
                icon_path: summary.icon_path,
            });
        }
    }
    Ok(plugins)
}

pub(super) fn installed_plugins(home: &Path) -> AppResult<InstalledPlugins> {
    serde_json::from_value(
        AppServer::start(home)?.request("plugin/installed", json!({"cwds": [home]}))?,
    )
    .map_err(|error| app_err!("Codex 插件安装清单无效: {error}"))
}

#[derive(Deserialize)]
struct InstalledApps {
    apps: Vec<InstalledApp>,
}

#[derive(Deserialize)]
struct InstalledApp {
    id: String,
    enabled: bool,
    callable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppMetadata {
    id: String,
    name: String,
    description: Option<String>,
    icon_url: Option<String>,
}

#[derive(Deserialize)]
struct ReadApps {
    apps: Vec<AppMetadata>,
    #[serde(rename = "missingAppIds")]
    missing: Vec<String>,
}

pub(super) fn installed_apps(home: &Path) -> AppResult<Vec<CodexAppSummary>> {
    let mut server = AppServer::start(home)?;
    let installed: InstalledApps =
        serde_json::from_value(server.request("app/installed", json!({}))?)
            .map_err(|error| app_err!("Codex 应用安装清单无效: {error}"))?;
    let mut metadata = Vec::new();
    for chunk in installed.apps.chunks(100) {
        let response: ReadApps = serde_json::from_value(server.request(
            "app/read",
            json!({"appIds": chunk.iter().map(|app| &app.id).collect::<Vec<_>>()}),
        )?)
        .map_err(|error| app_err!("Codex 应用信息无效: {error}"))?;
        if !response.missing.is_empty() {
            return Err(app_err!("Codex 未返回部分已安装应用的信息，请刷新重试"));
        }
        metadata.extend(response.apps);
    }
    installed
        .apps
        .into_iter()
        .filter(|app| app.callable)
        .map(|app| {
            let index = metadata
                .iter()
                .position(|item| item.id == app.id)
                .ok_or_else(|| app_err!("Codex 应用 {} 缺少名称等信息", app.id))?;
            let item = metadata.swap_remove(index);
            Ok(CodexAppSummary {
                id: app.id,
                name: item.name,
                description: item.description,
                enabled: app.enabled,
                icon_path: item.icon_url,
            })
        })
        .collect()
}

/// Codex 自管的系统市场：随运行时分发、不归用户安装卸载。可见性与可卸载共用此名单。
pub(super) fn system_marketplace(marketplace: &str) -> bool {
    matches!(marketplace, "openai-bundled" | "openai-primary-runtime")
}

pub(super) fn desktop_plugin(plugin: &InstalledPlugin, marketplace: &str) -> bool {
    // shortcut: 账号隐藏名单未公开，官方提供页面可见性字段后替换此处的来源分类。
    !system_marketplace(marketplace)
        && plugin.install_policy_source.as_deref() != Some("IMPLICIT_CANONICAL_APP")
}

// 清单展示与操作共用 PluginSummary，保持身份为 name@marketplace。
pub(super) fn apply_interface(summary: &mut PluginSummary, interface: Option<&Value>) {
    let Some(interface) = interface else { return };
    for (target, key) in [
        (&mut summary.display_name, "displayName"),
        (&mut summary.description, "shortDescription"),
    ] {
        if let Some(value) = interface.get(key).and_then(Value::as_str) {
            *target = Some(value.to_owned());
        }
    }
    if summary.icon_path.is_none() {
        summary.icon_path = interface
            .get("logoUrl")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_metadata_preserves_local_icons_and_fills_missing_icons() {
        let interface = json!({"displayName":"Sample", "logoUrl":"https://example.test/logo.svg"});
        for path in [
            "/plugins/sample/assets/logo.svg",
            "/plugins/sample/assets/icon.png",
        ] {
            let mut summary = PluginSummary {
                icon_path: Some(path.into()),
                ..Default::default()
            };
            apply_interface(&mut summary, Some(&interface));
            assert_eq!(summary.icon_path.as_deref(), Some(path));
            assert_eq!(summary.display_name.as_deref(), Some("Sample"));
        }
        let mut summary = PluginSummary::default();
        apply_interface(&mut summary, Some(&interface));
        assert_eq!(
            summary.icon_path.as_deref(),
            Some("https://example.test/logo.svg")
        );
    }
}
