//! Claude Code 插件服务：全部经 `claude plugin` 官方子命令（--json）驱动，
//! 不读写 CLI 的内部文件（installed_plugins.json / catalog 缓存只归 CLI 管）。
//!
//! 数据形状以 2026-10-17 对 claude 2.1.302 的隔离实测为准：
//! - `list --json --available` → `{installed: [...], available: [...]}`；
//!   未配置任何市场时退回纯已装数组形态（解析两种都收）。
//! - 同一插件 id 可按 scope 和 projectPath 多行并存。
//! - `list` 的 `enabled` 是「user 闸 && cwd 项目闸」合并后的有效值（同插件所有行
//!   同值），不是每行 scope 的开关；逐行真源由 `overlay_scoped_enabled` 从各
//!   scope 的 settings 文件补齐。
//! - 操作结果行 `{command, outcome, message, ...}`，outcome=ok 即成功；
//!   update 额外带 updateOutcome（updated / up_to_date）与新旧版本号。
//! - marketplace list 的 source 有两形：github 带 repo，directory 带 path。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::cli::run_claude_plugin;
use crate::error::{app_err, AppResult};
use crate::paths::now_ms;
use crate::services::AppContext;

/// 已安装插件条目（`claude plugin list --json`，按 scope 一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginEntry {
    /// 形如 `name@marketplace`。
    pub id: String,
    #[serde(default)]
    pub version: Option<String>,
    /// user / project / local。
    pub scope: String,
    pub enabled: bool,
    #[serde(default)]
    pub install_path: Option<String>,
    #[serde(default)]
    pub installed_at: Option<String>,
    #[serde(default)]
    pub last_updated: Option<String>,
    /// project/local scope 所属目录。
    #[serde(default)]
    pub project_path: Option<String>,
}

/// 已配置的插件市场（`claude plugin marketplace list --json`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeMarketplace {
    pub name: String,
    /// github / directory。
    pub source: String,
    /// github 源的 owner/repo。
    #[serde(default)]
    pub repo: Option<String>,
    /// directory 源的本地路径。
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
}

/// 市场目录中的可安装插件（`list --json --available`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAvailablePlugin {
    /// 形如 `name@marketplace`。
    pub plugin_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub marketplace_name: String,
}

/// 已装 + 可用目录全景（进页一次拉全）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginInventory {
    #[serde(default)]
    pub installed: Vec<ClaudePluginEntry>,
    #[serde(default)]
    pub available: Vec<ClaudeAvailablePlugin>,
}

/// `claude plugin update` 的结果（updateOutcome 区分 upgraded / already-latest）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginUpdateResult {
    /// updated / up_to_date。
    pub update_outcome: String,
    #[serde(default)]
    pub old_version: String,
    #[serde(default)]
    pub new_version: String,
}

/// CLI 操作结果行（install/uninstall/enable/disable/marketplace 系列的 --json 输出）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeCliOpOutcome {
    outcome: String,
    #[serde(default)]
    message: Option<String>,
    /// marketplace add 成功行携带新增市场名。
    #[serde(default)]
    marketplace: Option<String>,
    #[serde(default)]
    update_outcome: Option<String>,
    #[serde(default)]
    old_version: Option<String>,
    #[serde(default)]
    new_version: Option<String>,
}

/// 插件选择器（name@marketplace）直接做 argv 参数：拒绝路径分隔、冒号、空白与
/// 开关前缀，防止用户输入被当成 CLI 旗标；长度上限 160 覆盖 name+marketplace。
fn validate_claude_selector(selector: &str) -> AppResult<()> {
    let valid = !selector.is_empty()
        && selector.len() <= 160
        && !selector.starts_with('-')
        && !selector
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | ' ' | '\t' | '\n' | '\r' | '\0'));
    if valid {
        Ok(())
    } else {
        Err(app_err!("插件标识「{selector}」包含非法字符"))
    }
}

/// 市场名单个名称做 argv 参数（无 @，其余规则同选择器）。
fn validate_claude_marketplace_name(name: &str) -> AppResult<()> {
    let valid = !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '@' | ' ' | '\t' | '\n' | '\r' | '\0'));
    if valid {
        Ok(())
    } else {
        Err(app_err!("市场名「{name}」包含非法字符"))
    }
}

/// 市场源（owner/repo、git/https URL 或本地路径）：允许的字符面最宽，
/// 只拒绝开关前缀与控制字符，长度上限 300。
fn validate_claude_marketplace_source(source: &str) -> AppResult<()> {
    let valid = !source.is_empty()
        && source.len() <= 300
        && !source.starts_with('-')
        && !source.chars().any(|c| c.is_control());
    if valid {
        Ok(())
    } else {
        Err(app_err!("市场源地址「{source}」包含非法字符"))
    }
}

/// 安装 scope 必须是 CLI 认可的三选一。
fn validate_claude_scope(scope: &str) -> AppResult<()> {
    if matches!(scope, "user" | "project" | "local") {
        Ok(())
    } else {
        Err(app_err!("无效的安装范围「{scope}」"))
    }
}

fn claude_plugin_cwd(scope: &str, project_path: Option<&str>) -> AppResult<Option<PathBuf>> {
    validate_claude_scope(scope)?;
    if scope == "user" {
        return Ok(None);
    }
    let path = project_path
        .map(PathBuf::from)
        .ok_or_else(|| app_err!("项目级插件缺少项目目录"))?;
    if !path.is_absolute() || !path.is_dir() {
        return Err(app_err!("插件项目目录无效或已不存在: {}", path.display()));
    }
    Ok(Some(path))
}

/// 逐行开关的真源：各 scope settings 文件里的 `enabledPlugins.<pluginId>` 布尔——
/// user → <config>/settings.json、project → <project>/.claude/settings.json、
/// local → <project>/.claude/settings.local.json（claude 2.1.302 实测 enable/disable 落盘位置）。
/// 文件缺失或解析失败返回 None（调用方保留 CLI 值兜底），key 缺失视为关。
fn scoped_enabled_flag(
    scope: &str,
    project_path: Option<&str>,
    config_dir: &Path,
    plugin_id: &str,
) -> Option<bool> {
    let file = match scope {
        "user" => config_dir.join("settings.json"),
        "project" | "local" => {
            let name = if scope == "local" {
                "settings.local.json"
            } else {
                "settings.json"
            };
            Path::new(project_path?).join(".claude").join(name)
        }
        _ => return None,
    };
    let text = std::fs::read_to_string(&file).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    Some(
        value
            .get("enabledPlugins")
            .and_then(|plugins| plugins.get(plugin_id))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    )
}

/// `list` 的 `enabled` 是 cwd 合并后的有效值（user 闸关死时项目行也显示关，
/// 反之中性目录下项目行跟着 user 值走），用它当逐行开关会谎报两个方向。
/// 用各 scope settings 文件的真实布尔覆盖；读不到的行保留 CLI 值。
fn overlay_scoped_enabled(inventory: &mut ClaudePluginInventory, config_dir: &Path) {
    for entry in &mut inventory.installed {
        if let Some(enabled) = scoped_enabled_flag(
            &entry.scope,
            entry.project_path.as_deref(),
            config_dir,
            &entry.id,
        ) {
            entry.enabled = enabled;
        }
    }
}

fn parse_claude_inventory(text: &str) -> AppResult<ClaudePluginInventory> {
    let text = text.trim();
    let parse_error = |error: serde_json::Error| app_err!("Claude 插件列表解析失败: {error}");
    // 未配置任何市场时 CLI 退回纯已装数组形态
    if text.starts_with('[') {
        let installed =
            serde_json::from_str::<Vec<ClaudePluginEntry>>(text).map_err(parse_error)?;
        return Ok(ClaudePluginInventory {
            installed,
            available: Vec::new(),
        });
    }
    serde_json::from_str::<ClaudePluginInventory>(text).map_err(parse_error)
}

fn parse_claude_marketplaces(text: &str) -> AppResult<Vec<ClaudeMarketplace>> {
    serde_json::from_str::<Vec<ClaudeMarketplace>>(text.trim())
        .map_err(|error| app_err!("Claude 插件市场列表解析失败: {error}"))
}

/// 取输出的最后一个非空行解析：CLI 声明结果行是“一行 JSON”
/// （marketplace add 是“最后一行”），前面可能夹带人类可读的进度行。
fn parse_claude_op(text: &str) -> AppResult<ClaudeCliOpOutcome> {
    let last_line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let parsed: ClaudeCliOpOutcome = serde_json::from_str(last_line)
        .map_err(|error| app_err!("Claude 插件命令结果解析失败: {error}"))?;
    if parsed.outcome == "ok" {
        Ok(parsed)
    } else {
        // 退出码为 0 但 outcome 非 ok 的兜底；message 缺失时给通用文案
        Err(app_err!(
            "Claude Code CLI 报错：{}",
            parsed.message.unwrap_or_else(|| "操作未成功".to_string())
        ))
    }
}

impl AppContext {
    fn claude_home(&self) -> AppResult<std::path::PathBuf> {
        self.paths
            .claude_home
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| app_err!("无法定位用户主目录"))
    }

    fn claude_plugin_event(&self, action: &str, plugin_id: &str) {
        let _ = self.database.record_event(
            None,
            "plugin",
            action,
            Some(&format!("claude:{plugin_id}")),
            &now_ms().to_string(),
        );
    }

    /// 一级页只取已安装列表；市场页才请求可安装目录。
    pub async fn claude_list_plugins(
        &self,
        include_available: bool,
    ) -> AppResult<ClaudePluginInventory> {
        let home = self.claude_home()?;
        let config_dir = self.paths.claude_home.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let args: &[&str] = if include_available {
                &["list", "--json", "--available"]
            } else {
                &["list", "--json"]
            };
            let output = run_claude_plugin(&home, args, None)?;
            let mut inventory = parse_claude_inventory(&output)?;
            overlay_scoped_enabled(&mut inventory, &config_dir);
            Ok(inventory)
        })
        .await
        .map_err(|error| app_err!("Claude 插件列表任务失败: {error}"))?
    }

    /// 读取 Claude Code 已配置的插件市场。
    pub async fn claude_list_plugin_marketplaces(&self) -> AppResult<Vec<ClaudeMarketplace>> {
        let home = self.claude_home()?;
        tauri::async_runtime::spawn_blocking(move || {
            let output = run_claude_plugin(&home, &["marketplace", "list", "--json"], None)?;
            parse_claude_marketplaces(&output)
        })
        .await
        .map_err(|error| app_err!("Claude 插件市场列表任务失败: {error}"))?
    }

    /// 添加插件市场；成功后重查列表按名找回完整条目（结果行没有 installLocation 等字段）。
    pub async fn claude_add_plugin_marketplace(
        &self,
        source: &str,
    ) -> AppResult<ClaudeMarketplace> {
        validate_claude_marketplace_source(source)?;
        let home = self.claude_home()?;
        let source = source.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let output =
                run_claude_plugin(&home, &["marketplace", "add", &source, "--json"], None)?;
            let added = parse_claude_op(&output)?;
            let name = added
                .marketplace
                .clone()
                .ok_or_else(|| app_err!("Claude Code CLI 未返回新增市场名，请刷新市场列表确认"))?;
            let listing = run_claude_plugin(&home, &["marketplace", "list", "--json"], None)?;
            parse_claude_marketplaces(&listing)?
                .into_iter()
                .find(|item| item.name == name)
                .ok_or_else(|| app_err!("市场「{name}」已添加但未出现在列表中，请刷新重试"))
        })
        .await
        .map_err(|error| app_err!("添加 Claude 插件市场任务失败: {error}"))?
    }

    /// 移除插件市场（该市场安装的插件一并移除，由 CLI 维护一致性）。
    pub async fn claude_remove_plugin_marketplace(&self, name: &str) -> AppResult<()> {
        validate_claude_marketplace_name(name)?;
        let home = self.claude_home()?;
        let name = name.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let output =
                run_claude_plugin(&home, &["marketplace", "remove", &name, "--json"], None)?;
            parse_claude_op(&output).map(|_| ())
        })
        .await
        .map_err(|error| app_err!("移除 Claude 插件市场任务失败: {error}"))?
    }

    /// 按名单个刷新市场目录（前端逐市场调用；全部串行刷新容易超出单次限时）。
    pub async fn claude_update_plugin_marketplace(&self, name: &str) -> AppResult<()> {
        validate_claude_marketplace_name(name)?;
        let home = self.claude_home()?;
        let name = name.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let output =
                run_claude_plugin(&home, &["marketplace", "update", &name, "--json"], None)?;
            parse_claude_op(&output).map(|_| ())
        })
        .await
        .map_err(|error| app_err!("刷新 Claude 插件市场任务失败: {error}"))?
    }

    /// 从已配置市场安装插件（scope 走 CLI 默认 user）。
    pub async fn claude_install_plugin(&self, plugin_id: &str) -> AppResult<()> {
        validate_claude_selector(plugin_id)?;
        let home = self.claude_home()?;
        let plugin_id = plugin_id.to_string();
        tauri::async_runtime::spawn_blocking({
            let plugin_id = plugin_id.clone();
            move || {
                let output = run_claude_plugin(&home, &["install", &plugin_id, "--json"], None)?;
                parse_claude_op(&output).map(|_| ())
            }
        })
        .await
        .map_err(|error| app_err!("安装 Claude 插件任务失败: {error}"))??;
        tauri_plugin_log::log::info!(
            "[plugin.install] client=\"Claude Code\" plugin={plugin_id:?} outcome=success msg=\"Claude Code 插件安装成功\""
        );
        self.claude_plugin_event("install", &plugin_id);
        Ok(())
    }

    /// 按行所属 scope 卸载插件。
    pub async fn claude_uninstall_plugin(
        &self,
        plugin_id: &str,
        scope: &str,
        project_path: Option<&str>,
    ) -> AppResult<()> {
        validate_claude_selector(plugin_id)?;
        let cwd = claude_plugin_cwd(scope, project_path)?;
        let home = self.claude_home()?;
        let plugin_id = plugin_id.to_string();
        let scope = scope.to_string();
        let config_dir = self.paths.claude_home.clone();
        tauri::async_runtime::spawn_blocking({
            let plugin_id = plugin_id.clone();
            let scope = scope.clone();
            move || {
                // 主目录也是项目时，两种 scope 共用 settings.json，卸载项目项不能关闭全局项。
                // user 开关读 settings 文件——list 的 enabled 是 cwd 合并值，此处不可信。
                let user_installed =
                    parse_claude_inventory(&run_claude_plugin(&home, &["list", "--json"], None)?)?
                        .installed
                        .iter()
                        .any(|entry| entry.id == plugin_id && entry.scope == "user");
                let preserve_user = scope == "project"
                    && cwd
                        .as_ref()
                        .is_some_and(|path| path.canonicalize().ok() == home.canonicalize().ok())
                    && user_installed
                    && scoped_enabled_flag("user", None, &config_dir, &plugin_id).unwrap_or(false);
                let output = run_claude_plugin(
                    &home,
                    &["uninstall", &plugin_id, "-s", &scope, "--json"],
                    cwd.as_deref(),
                )?;
                parse_claude_op(&output)?;
                if preserve_user {
                    let output = run_claude_plugin(
                        &home,
                        &["enable", &plugin_id, "-s", "user", "--json"],
                        None,
                    )?;
                    parse_claude_op(&output)?;
                }
                Ok(())
            }
        })
        .await
        .map_err(|error| app_err!("卸载 Claude 插件任务失败: {error}"))??;
        tauri_plugin_log::log::info!(
            "[plugin.uninstall] client=\"Claude Code\" plugin={plugin_id:?} scope={scope:?} outcome=success msg=\"Claude Code 插件已卸载\""
        );
        self.claude_plugin_event("uninstall", &plugin_id);
        Ok(())
    }

    /// 启用/禁用插件（只翻 settings 的开关，不动安装）。
    pub async fn claude_set_plugin_enabled(
        &self,
        plugin_id: &str,
        scope: &str,
        enabled: bool,
        project_path: Option<&str>,
    ) -> AppResult<()> {
        validate_claude_selector(plugin_id)?;
        let cwd = claude_plugin_cwd(scope, project_path)?;
        let home = self.claude_home()?;
        let plugin_id = plugin_id.to_string();
        let scope = scope.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            let verb = if enabled { "enable" } else { "disable" };
            let output = run_claude_plugin(
                &home,
                &[verb, &plugin_id, "-s", &scope, "--json"],
                cwd.as_deref(),
            )?;
            parse_claude_op(&output).map(|_| ())
        })
        .await
        .map_err(|error| app_err!("切换 Claude 插件启停任务失败: {error}"))?
    }

    /// 升级插件到市场最新版；不做预检，updateOutcome 区分 updated / up_to_date。
    /// CLI 声明升级需重启会话才生效，前端文案负责传达。
    pub async fn claude_update_plugin(
        &self,
        plugin_id: &str,
        scope: &str,
        project_path: Option<&str>,
    ) -> AppResult<ClaudePluginUpdateResult> {
        validate_claude_selector(plugin_id)?;
        let cwd = claude_plugin_cwd(scope, project_path)?;
        let home = self.claude_home()?;
        let plugin_id = plugin_id.to_string();
        let scope = scope.to_string();
        let result = tauri::async_runtime::spawn_blocking({
            let plugin_id = plugin_id.clone();
            move || {
                let output = run_claude_plugin(
                    &home,
                    &["update", &plugin_id, "-s", &scope, "--json"],
                    cwd.as_deref(),
                )?;
                let parsed = parse_claude_op(&output)?;
                Ok(ClaudePluginUpdateResult {
                    update_outcome: parsed
                        .update_outcome
                        .unwrap_or_else(|| "updated".to_string()),
                    old_version: parsed.old_version.unwrap_or_default(),
                    new_version: parsed.new_version.unwrap_or_default(),
                })
            }
        })
        .await
        .map_err(|error| app_err!("升级 Claude 插件任务失败: {error}"))??;
        tauri_plugin_log::log::info!(
            "[plugin.update] client=\"Claude Code\" plugin={plugin_id:?} outcome=success update_outcome={} msg=\"Claude Code 插件升级完成\"",
            result.update_outcome
        );
        self.claude_plugin_event("update", &plugin_id);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_operations_require_the_selected_project_directory() {
        let project = tempfile::tempdir().unwrap();
        for scope in ["project", "local"] {
            assert_eq!(
                claude_plugin_cwd(scope, project.path().to_str())
                    .unwrap()
                    .as_deref(),
                Some(project.path())
            );
            assert!(claude_plugin_cwd(scope, None).is_err());
            assert!(claude_plugin_cwd(scope, Some("relative-project")).is_err());
        }
        assert!(claude_plugin_cwd("user", None).unwrap().is_none());
    }

    /// CLI 的 enabled 是 cwd 合并值（同插件所有行同值）；逐行真源在各 scope 的
    /// settings 文件，缺 key 视为关，读不到文件的行保留 CLI 值。
    #[test]
    fn overlay_scoped_enabled_reads_each_scope_flag_file() {
        fn entry(scope: &str, id: &str, project_path: Option<&str>) -> ClaudePluginEntry {
            ClaudePluginEntry {
                id: id.to_string(),
                version: None,
                scope: scope.to_string(),
                enabled: true,
                install_path: None,
                installed_at: None,
                last_updated: None,
                project_path: project_path.map(str::to_string),
            }
        }
        let config = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(
            config.path().join("settings.json"),
            r#"{"enabledPlugins":{"demo@mkt":true}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(project.path().join(".claude")).unwrap();
        std::fs::write(
            project.path().join(".claude").join("settings.json"),
            r#"{"enabledPlugins":{"demo@mkt":false}}"#,
        )
        .unwrap();
        std::fs::write(
            project.path().join(".claude").join("settings.local.json"),
            r#"{"enabledPlugins":{"other@mkt":true}}"#,
        )
        .unwrap();
        let project_dir = project.path().to_string_lossy().into_owned();
        let mut inventory = ClaudePluginInventory {
            installed: vec![
                entry("user", "demo@mkt", None),
                entry("project", "demo@mkt", Some(&project_dir)),
                // local 的 settings.local.json 没有 demo@mkt 的 key → 关
                entry("local", "demo@mkt", Some(&project_dir)),
                // user settings 没有 ghost@mkt 的 key → 关
                entry("user", "ghost@mkt", None),
                // 项目目录已不存在，文件读不到 → 保留 CLI 值
                entry("project", "gone@mkt", Some(r"C:\definitely\missing")),
            ],
            available: Vec::new(),
        };
        overlay_scoped_enabled(&mut inventory, config.path());
        let enabled: Vec<bool> = inventory.installed.iter().map(|e| e.enabled).collect();
        assert_eq!(enabled, vec![true, false, false, false, true]);
    }

    /// 实测 fixture（claude 2.1.302 真机输出，路径改抽象名）。
    const INSTALLED_USER: &str = r#"{"id":"code-review@claude-plugins-official","version":"1fad13414a44","scope":"user","enabled":true,"installPath":"C:\\Users\\<user>\\.claude\\plugins\\cache\\claude-plugins-official\\code-review\\1fad13414a44","installedAt":"2026-07-27T17:57:52.773Z","lastUpdated":"2026-07-27T17:57:52.773Z","projectEnabled":false}"#;
    const INSTALLED_PROJECT: &str = r#"{"id":"code-review@claude-plugins-official","version":"1fad13414a44","scope":"project","enabled":false,"installPath":"C:\\Users\\<user>\\.claude\\plugins\\cache\\claude-plugins-official\\code-review\\1fad13414a44","installedAt":"2026-10-14T17:06:53.598Z","lastUpdated":"2026-10-14T17:06:53.598Z","projectPath":"C:\\Users\\<user>","projectEnabled":false,"readFromFolder":"C:\\demo","folderVersion":"1fad13414a44"}"#;

    #[test]
    fn parse_installed_entries_covers_both_scopes() {
        let entry: ClaudePluginEntry = serde_json::from_str(INSTALLED_USER).unwrap();
        assert_eq!(entry.id, "code-review@claude-plugins-official");
        assert_eq!(entry.scope, "user");
        assert!(entry.enabled);
        assert_eq!(entry.version.as_deref(), Some("1fad13414a44"));
        assert!(entry.project_path.is_none());

        let entry: ClaudePluginEntry = serde_json::from_str(INSTALLED_PROJECT).unwrap();
        assert_eq!(entry.scope, "project");
        assert!(!entry.enabled);
        assert_eq!(entry.project_path.as_deref(), Some(r"C:\Users\<user>"));
        // 未知字段（readFromFolder/folderVersion/projectEnabled）忽略不报错
    }

    #[test]
    fn parse_inventory_accepts_composite_and_bare_array() {
        let composite = format!(
            r#"{{"installed":[{INSTALLED_USER}],"available":[{{"pluginId":"ponytail@ponytail","name":"ponytail","description":"Forces the laziest solution that works.","marketplaceName":"ponytail","source":"./"}}]}}"#
        );
        let inventory = parse_claude_inventory(&composite).unwrap();
        assert_eq!(inventory.installed.len(), 1);
        assert_eq!(inventory.available.len(), 1);
        assert_eq!(inventory.available[0].plugin_id, "ponytail@ponytail");
        assert_eq!(inventory.available[0].marketplace_name, "ponytail");

        // 未配置市场时 CLI 退回纯已装数组
        let bare = format!("[{INSTALLED_USER},{INSTALLED_PROJECT}]");
        let inventory = parse_claude_inventory(&bare).unwrap();
        assert_eq!(inventory.installed.len(), 2);
        assert!(inventory.available.is_empty());
    }

    #[test]
    fn parse_marketplaces_covers_github_and_directory_shapes() {
        let text = r#"[
            {"name":"ponytail","source":"github","repo":"DietrichGebert/ponytail","installLocation":"C:\\Users\\<user>\\.claude\\plugins\\marketplaces\\ponytail"},
            {"name":"demo","source":"directory","path":"C:\\demo\\market","installLocation":"C:\\demo\\market"}
        ]"#;
        let items = parse_claude_marketplaces(text).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].source, "github");
        assert_eq!(items[0].repo.as_deref(), Some("DietrichGebert/ponytail"));
        assert!(items[0].path.is_none());
        assert_eq!(items[1].source, "directory");
        assert_eq!(items[1].path.as_deref(), Some(r"C:\demo\market"));
        assert!(items[1].repo.is_none());
    }

    #[test]
    fn parse_op_takes_last_json_line_and_checks_outcome() {
        let updated = r#"前情人类可读行
{"command":"update","outcome":"ok","plugin":"ponytail@ponytail","updateOutcome":"updated","oldVersion":"4.10.0","newVersion":"4.11.0","message":"Plugin updated"}"#;
        let parsed = parse_claude_op(updated).unwrap();
        assert_eq!(parsed.update_outcome.as_deref(), Some("updated"));
        assert_eq!(parsed.old_version.as_deref(), Some("4.10.0"));

        let up_to_date = r#"{"command":"update","outcome":"ok","updateOutcome":"up_to_date","oldVersion":"4.11.0","newVersion":"4.11.0"}"#;
        assert_eq!(
            parse_claude_op(up_to_date)
                .unwrap()
                .update_outcome
                .as_deref(),
            Some("up_to_date")
        );

        let failed =
            r#"{"command":"install","outcome":"failed","message":"marketplace not found"}"#;
        let error = parse_claude_op(failed).unwrap_err();
        assert!(error.0.contains("marketplace not found"));
    }

    #[test]
    fn selectors_and_sources_reject_flag_and_path_injection() {
        assert!(validate_claude_selector("ponytail@ponytail").is_ok());
        assert!(validate_claude_selector("--json").is_err());
        assert!(validate_claude_selector("a/b@c").is_err());
        assert!(validate_claude_selector("").is_err());
        assert!(validate_claude_selector("a b@c").is_err());

        assert!(validate_claude_marketplace_name("ponytail").is_ok());
        assert!(validate_claude_marketplace_name("a@b").is_err());
        assert!(validate_claude_marketplace_name("../x").is_err());

        assert!(validate_claude_marketplace_source("DietrichGebert/ponytail").is_ok());
        assert!(validate_claude_marketplace_source("https://example.com/market.json").is_ok());
        assert!(validate_claude_marketplace_source("--claudeai").is_err());

        assert!(validate_claude_scope("user").is_ok());
        assert!(validate_claude_scope("project").is_ok());
        assert!(validate_claude_scope("local").is_ok());
        assert!(validate_claude_scope("--json").is_err());
    }
}
