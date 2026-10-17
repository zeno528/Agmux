//! 插件与市场目录解析：CLI 表格输出、市场元数据富化、manifest 读取与 contains 推导。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::codex::config::parse_document;
use crate::services::plugin_net::{parse_github_url, TreeEntry};

use super::store::store_contains;
use super::{
    MarketplacePlugin, PluginManifest, PluginMarketplace, PluginSummary, PluginUpdate,
    CLAUDE_MANIFEST_RELATIVE_PATH, MANIFEST_RELATIVE_PATH,
};
use crate::error::{app_err, AppResult};
/// 解析 `codex plugin list` 的表格输出：
/// `<插件>@<市场>  installed, enabled|disabled  <版本>  <路径>`；`not installed` 跳过。
pub(super) fn parse_plugin_list_output(
    text: &str,
) -> Vec<(String, String, bool, Option<String>, String)> {
    let mut items = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.is_empty()
            || trimmed.starts_with("PLUGIN ")
            || trimmed.starts_with("Marketplace `")
        {
            continue;
        }
        let Some((selector, rest)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let Some((plugin, marketplace)) = selector.split_once('@') else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.starts_with("not installed") {
            continue;
        }
        if !rest.starts_with("installed,") {
            continue;
        }
        let enabled = rest.starts_with("installed, enabled");
        let after_status = rest
            .strip_prefix("installed, enabled")
            .or_else(|| rest.strip_prefix("installed, disabled"))
            .unwrap_or(rest)
            .trim_start();
        let mut parts = after_status.split_whitespace();
        let version = parts
            .next()
            .filter(|token| !token.contains('\\') && !token.contains('/'));
        let path = parts.collect::<Vec<_>>().join(" ");
        items.push((
            plugin.to_string(),
            marketplace.to_string(),
            enabled,
            version.map(str::to_string),
            path,
        ));
    }
    items
}

pub(super) fn parse_marketplace_list_output(text: &str) -> Vec<PluginMarketplace> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("MARKETPLACE") {
                return None;
            }
            let (name, root) = trimmed.split_once(char::is_whitespace)?;
            let name = name.trim();
            let root = root.trim();
            if name.is_empty() || root.is_empty() {
                return None;
            }
            Some(PluginMarketplace {
                name: name.to_string(),
                root: root.to_string(),
                kind: if name.starts_with("openai") {
                    "official".into()
                } else {
                    "third-party".into()
                },
                source_url: None,
                display_name: None,
                description: None,
            })
        })
        .collect()
}

pub(super) fn sort_marketplaces(items: &mut [PluginMarketplace]) {
    items.sort_by(|left, right| {
        (left.kind != "official")
            .cmp(&(right.kind != "official"))
            .then_with(|| left.name.cmp(&right.name))
    });
}

pub(super) fn marketplace_sources(home: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(home.join(".codex/config.toml")) else {
        return BTreeMap::new();
    };
    let Ok(document) = parse_document(&text) else {
        return BTreeMap::new();
    };
    let Some(marketplaces) = document
        .as_table()
        .get("marketplaces")
        .and_then(|item| item.as_table())
    else {
        return BTreeMap::new();
    };
    marketplaces
        .iter()
        .filter_map(|(name, item)| {
            let source = item.as_table()?.get("source")?.as_str()?;
            (source.starts_with("http://")
                || source.starts_with("https://")
                || source.starts_with("ssh://")
                || source.starts_with("git@"))
            .then(|| (name.to_string(), source.to_string()))
        })
        .collect()
}

pub(super) fn enrich_plugin_sources(
    items: &mut [PluginSummary],
    sources: &BTreeMap<String, String>,
) {
    for item in items {
        item.source_url = item
            .marketplace
            .as_ref()
            .and_then(|name| sources.get(name).cloned());
    }
}

pub(super) fn read_marketplace_document(root: &Path) -> Option<Value> {
    [
        root.join(".agents/plugins/marketplace.json"),
        root.join(".claude-plugin/marketplace.json"),
        root.join("marketplace.json"),
    ]
    .into_iter()
    .find_map(|path| std::fs::read_to_string(path).ok())
    .and_then(|text| serde_json::from_str(&text).ok())
}

pub(super) fn marketplace_metadata(root: &Path) -> (Option<String>, Option<String>) {
    let document = read_marketplace_document(root);
    let interface = document.as_ref().and_then(|item| item.get("interface"));
    let display_name = interface
        .and_then(|item| item.get("displayName"))
        .and_then(Value::as_str)
        .map(String::from);
    let description = document
        .as_ref()
        .and_then(|item| item.get("metadata"))
        .and_then(|item| item.get("description"))
        .and_then(Value::as_str)
        .or_else(|| {
            interface
                .and_then(|item| item.get("longDescription"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            interface
                .and_then(|item| item.get("shortDescription"))
                .and_then(Value::as_str)
        })
        .map(String::from)
        .or_else(|| read_manifest(root).as_ref().and_then(manifest_description));
    (display_name, description)
}

pub(super) fn enrich_marketplace_metadata(
    items: &mut [PluginMarketplace],
    sources: &BTreeMap<String, String>,
) {
    for item in items {
        let (display_name, description) = marketplace_metadata(Path::new(&item.root));
        item.source_url = sources.get(&item.name).cloned();
        item.display_name = display_name;
        item.description = description;
    }
}

#[derive(Default)]
pub(super) struct MarketplaceEntryMetadata {
    display_name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    capabilities: Vec<String>,
    contains: Vec<String>,
    version: Option<String>,
    icon_path: Option<String>,
}

pub(super) fn marketplace_entry_metadata(
    root: Option<&Path>,
    name: &str,
) -> MarketplaceEntryMetadata {
    let Some(root) = root else {
        return MarketplaceEntryMetadata::default();
    };
    let Some(document) = read_marketplace_document(root) else {
        return MarketplaceEntryMetadata::default();
    };
    let Some(entry) = document
        .get("plugins")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
        })
    else {
        return MarketplaceEntryMetadata::default();
    };
    let interface = entry.get("interface");
    let local_path = entry_local_path(entry).and_then(|raw| resolve_local_path(root, &raw));
    let local_manifest = local_path.as_deref().and_then(read_manifest);
    let manifest_interface = local_manifest
        .as_ref()
        .and_then(|item| item.interface.as_ref());
    // 图标先看清单条目指向的本地目录，再按市场物化约定探 `plugins/<name>`（openai 运行时市场走后者）。
    let icon_dir = local_path
        .clone()
        .or_else(|| Some(root.join("plugins").join(name)));
    let icon_path = icon_dir
        .as_deref()
        .and_then(|dir| {
            let manifest = if Some(dir) == local_path.as_deref() {
                local_manifest.clone()
            } else {
                read_manifest(dir)
            };
            super::store::plugin_icon_from(manifest.as_ref(), dir)
        })
        .map(|path| path.display().to_string());
    MarketplaceEntryMetadata {
        display_name: interface
            .and_then(|item| item.get("displayName"))
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| manifest_interface.and_then(|item| item.display_name.clone())),
        description: entry
            .get("description")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| local_manifest.as_ref().and_then(manifest_description)),
        category: entry
            .get("category")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| manifest_interface.and_then(|item| item.category.clone())),
        capabilities: manifest_interface
            .map(|item| item.capabilities.clone())
            .unwrap_or_default(),
        contains: local_path
            .as_deref()
            .map(store_contains)
            .unwrap_or_default(),
        version: local_manifest
            .as_ref()
            .and_then(|item| item.version.clone()),
        icon_path,
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginCatalogEntry {
    plugin_id: String,
    name: String,
    marketplace_name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    installed: bool,
    #[serde(default)]
    auth_policy: String,
    #[serde(default)]
    source: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PluginCatalogOutput {
    #[serde(default)]
    installed: Vec<PluginCatalogEntry>,
    #[serde(default)]
    available: Vec<PluginCatalogEntry>,
}

pub(super) fn plugin_source_label(source: Option<&Value>) -> Option<String> {
    let object = source?.as_object()?;
    ["url", "repo", "path"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str).map(String::from))
}

pub(super) fn parse_marketplace_plugins_output(
    text: &str,
    marketplace: &str,
    root: Option<&Path>,
) -> AppResult<Vec<MarketplacePlugin>> {
    let output: PluginCatalogOutput = serde_json::from_str(text)
        .map_err(|error| app_err!("codex plugin list JSON 无效: {error}"))?;
    let mut seen = Vec::new();
    Ok(output
        .installed
        .into_iter()
        .chain(output.available)
        .filter(|item| item.marketplace_name == marketplace)
        .filter_map(|item| {
            if seen.iter().any(|id| id == &item.plugin_id) {
                return None;
            }
            seen.push(item.plugin_id.clone());
            let metadata = marketplace_entry_metadata(root, &item.name);
            Some(MarketplacePlugin {
                plugin_id: item.plugin_id,
                name: item.name,
                version: item.version.or(metadata.version),
                installed: item.installed,
                auth_policy: item.auth_policy,
                source: plugin_source_label(item.source.as_ref()),
                display_name: metadata.display_name,
                description: metadata.description,
                category: metadata.category,
                capabilities: metadata.capabilities,
                contains: metadata.contains,
                icon_path: metadata.icon_path,
            })
        })
        .collect())
}

/// 解析 `codex plugin list` 输出的市场身份集合（"Marketplace `X`" 头部行）。
pub(super) fn parse_marketplace_identities(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("Marketplace `"))
        .filter_map(|rest| {
            rest.strip_suffix('`')
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .collect()
}

/// 安装目标市场：镜像存在 `-remote` 原体时优先远程身份（桌面端注册表口径），否则原样。
pub(super) fn resolve_install_marketplace(marketplace: &str, identities: &[String]) -> String {
    let remote = format!("{marketplace}-remote");
    if identities.contains(&remote) {
        remote
    } else {
        marketplace.to_string()
    }
}

pub(super) fn find_plugin_updates(
    installed: &[PluginSummary],
    available: &[MarketplacePlugin],
) -> Vec<PluginUpdate> {
    available
        .iter()
        .filter(|plugin| plugin.installed)
        .filter_map(|plugin| {
            let (name, marketplace) = plugin.plugin_id.rsplit_once('@')?;
            let version = plugin.version.as_deref()?;
            installed
                .iter()
                .find(|item| {
                    item.origin == "codex"
                        && item.name == name
                        && item.marketplace.as_deref() == Some(marketplace)
                })
                .filter(|item| item.version.as_deref() != Some(version))
                .map(|_| PluginUpdate {
                    name: name.to_string(),
                    marketplace: marketplace.to_string(),
                    version: version.to_string(),
                })
        })
        .collect()
}

/// 从 `marketplace add` 的输出里解析市场名（如 “Marketplace `ponytail`”），失败回退仓库名。
pub(super) fn parse_marketplace_name(output: &str, fallback: &str) -> String {
    if let Some(start) = output.find('`') {
        if let Some(length) = output[start + 1..].find('`') {
            let name = &output[start + 1..start + 1 + length];
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Codex 官方支持的 marketplace add 来源：GitHub 简写、Git URL、SSH URL 或本地目录。
pub(super) fn parse_marketplace_source(input: &str) -> AppResult<(String, String)> {
    let text = input.trim();
    if text.is_empty() || text.starts_with('-') {
        return Err(app_err!("插件市场地址不能为空，且不能以 - 开头"));
    }
    if let Ok(source) = parse_github_url(text) {
        let argument = match source.ref_name {
            Some(reference) => format!("{}/{}@{reference}", source.owner, source.repo),
            None => format!("{}/{}", source.owner, source.repo),
        };
        return Ok((argument, source.repo));
    }
    let accepted = text.starts_with("http://")
        || text.starts_with("https://")
        || text.starts_with("ssh://")
        || text.starts_with("git@")
        || Path::new(text).is_absolute()
        || text.starts_with("./")
        || text.starts_with("../");
    if !accepted {
        return Err(app_err!(
            "无法识别插件市场来源，请使用 owner/repo、Git URL、SSH URL 或本地目录"
        ));
    }
    let last = text
        .trim_matches(['/', '\\'])
        .rsplit(['/', '\\', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or("third-party-marketplace");
    let fallback = last.strip_suffix(".git").unwrap_or(last).to_string();
    Ok((text.to_string(), fallback))
}

/// 市场条目的 local 源路径（兼容两种写法）。
pub(super) fn entry_local_path(entry: &Value) -> Option<String> {
    match entry.get("source")? {
        Value::String(path) => Some(path.clone()),
        Value::Object(object) => {
            if object.get("source").and_then(Value::as_str) == Some("local") {
                object.get("path")?.as_str().map(String::from)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// local 路径相对 marketplace 根（= 用户主目录）解析；拒绝绝对路径与 `..` 穿越。
/// `.cursor-plugin` 布局允许无 `./` 前缀（与 Codex 行为一致）。
pub(super) fn resolve_local_path(home: &Path, raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    let relative = trimmed.strip_prefix("./").unwrap_or(trimmed);
    let relative_path = Path::new(relative);
    if !relative_path.is_relative() {
        return None;
    }
    let mut full = home.to_path_buf();
    for component in relative_path.components() {
        match component {
            std::path::Component::Normal(part) => full.push(part),
            _ => return None,
        }
    }
    Some(full)
}

// ==================== 共享工具 ====================

pub(super) fn parse_manifest_text(text: &str) -> AppResult<PluginManifest> {
    serde_json::from_str(text).map_err(|error| app_err!("plugin.json 不是有效 JSON: {error}"))
}

pub(super) fn manifest_description(manifest: &PluginManifest) -> Option<String> {
    manifest
        .description
        .clone()
        .or_else(|| manifest.interface.as_ref()?.long_description.clone())
        .or_else(|| manifest.interface.as_ref()?.short_description.clone())
}

pub(super) fn read_manifest(plugin_root: &Path) -> Option<PluginManifest> {
    let text = std::fs::read_to_string(plugin_root.join(MANIFEST_RELATIVE_PATH))
        .or_else(|_| std::fs::read_to_string(plugin_root.join(CLAUDE_MANIFEST_RELATIVE_PATH)))
        .ok()?;
    parse_manifest_text(&text).ok()
}

/// 插件名做选择器/目录名：拒绝路径分隔与目录穿越，其余保留官方命名。
pub(super) fn validate_plugin_name(name: &str) -> AppResult<()> {
    let valid = !name.is_empty()
        && name.len() <= 80
        && !name.contains(['/', '\\', ':', '@'])
        && name != "."
        && name != "..";
    if valid {
        Ok(())
    } else {
        Err(app_err!("插件名「{name}」包含非法字符"))
    }
}

/// 从文件树里找插件根目录（.codex-plugin/plugin.json 的父目录），仓库根插件表示为空串。
pub(super) fn plugin_roots(entries: &[TreeEntry]) -> Vec<String> {
    let mut roots: Vec<String> = entries
        .iter()
        .filter(|entry| entry.kind == "blob" && entry.path.ends_with(MANIFEST_RELATIVE_PATH))
        .map(|entry| {
            entry
                .path
                .strip_suffix(&format!("/{MANIFEST_RELATIVE_PATH}"))
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// 判断 root 是否等于指定子路径或位于其下（子目录过滤用）。
pub(super) fn root_within(root: &str, sub_path: &str) -> bool {
    let sub = sub_path.trim_matches('/');
    if sub.is_empty() {
        return true;
    }
    root == sub || root.starts_with(&format!("{sub}/"))
}

pub(super) fn files_under_root<'a>(entries: &'a [TreeEntry], root: &str) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|entry| entry.kind == "blob" && root_within(&entry.path, root))
        .map(|entry| entry.path.as_str())
        .collect()
}

/// 从插件文件清单推导内容类型（白皮书的「包含内容」维度）。
pub(super) fn derive_contains(files: &[&str]) -> Vec<String> {
    let mut contains = Vec::new();
    let has = |predicate: &dyn Fn(&str) -> bool| files.iter().any(|path| predicate(path));
    if has(&|path| path.starts_with("skills/") || path == "skills") {
        contains.push("skills".into());
    }
    if has(&|path| path.ends_with("/.mcp.json") || path == ".mcp.json") {
        contains.push("mcp".into());
    }
    if has(&|path| path.ends_with("/.app.json") || path == ".app.json") {
        contains.push("app".into());
    }
    if has(&|path| path.ends_with("/hooks.json") || path == "hooks.json") {
        contains.push("hooks".into());
    }
    if has(&|path| path.starts_with("agents/")) {
        contains.push("agents".into());
    }
    if has(&|path| path.starts_with("commands/")) {
        contains.push("commands".into());
    }
    contains
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::plugin_net::TreeEntry;
    use crate::services::plugins::*;
    use serde_json::json;

    fn entry(path: &str) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            kind: "blob".into(),
        }
    }

    #[test]
    fn plugin_roots_find_nested_and_root_plugins() {
        let entries = vec![
            entry(".codex-plugin/plugin.json"),
            entry("skills/a/SKILL.md"),
            entry("plugins/foo/.codex-plugin/plugin.json"),
            entry("plugins/foo/skills/b/SKILL.md"),
            entry("plugins/foo/node_modules/x/.codex-plugin/plugin.json"),
        ];
        let roots = plugin_roots(&entries);
        assert_eq!(
            roots,
            vec![
                "".to_string(),
                "plugins/foo".to_string(),
                "plugins/foo/node_modules/x".to_string()
            ]
        );
    }

    #[test]
    fn root_within_matches_self_and_children_only() {
        assert!(root_within("plugins/foo", "plugins"));
        assert!(root_within("plugins/foo", "plugins/foo"));
        assert!(!root_within("plugins/foobar", "plugins/foo"));
        assert!(root_within("anything", ""));
    }

    #[test]
    fn derive_contains_labels_content_types() {
        let files = vec!["skills/a/SKILL.md", ".mcp.json", "hooks.json"];
        assert_eq!(derive_contains(&files), vec!["skills", "mcp", "hooks"]);
    }

    #[test]
    fn resolve_local_path_accepts_prefix_and_cursor_style() {
        let home = Path::new("/home/user");
        assert_eq!(
            resolve_local_path(home, "./plugins/foo"),
            Some(home.join("plugins/foo"))
        );
        assert_eq!(
            resolve_local_path(home, "plugins/foo"),
            Some(home.join("plugins/foo"))
        );
        assert_eq!(resolve_local_path(home, "../escape"), None);
        assert_eq!(resolve_local_path(home, "/absolute"), None);
    }

    #[test]
    fn entry_local_path_supports_string_and_object_forms() {
        let string_form = json!({ "name": "a", "source": "./plugins/a" });
        assert_eq!(
            entry_local_path(&string_form).as_deref(),
            Some("./plugins/a")
        );
        let object_form =
            json!({ "name": "b", "source": { "source": "local", "path": "./plugins/b" } });
        assert_eq!(
            entry_local_path(&object_form).as_deref(),
            Some("./plugins/b")
        );
        let url_form = json!({ "name": "c", "source": { "source": "url", "url": "https://github.com/x/y.git" } });
        assert_eq!(entry_local_path(&url_form), None);
    }

    #[test]
    fn manifest_parses_official_sample() {
        let text = r#"{
            "name": "memory-bank",
            "version": "1.0.0",
            "description": "Memory management",
            "interface": { "displayName": "Memory Bank", "category": "memory", "capabilities": ["read", "write"] }
        }"#;
        let manifest = parse_manifest_text(text).unwrap();
        assert_eq!(manifest.name, "memory-bank");
        assert_eq!(manifest.version.as_deref(), Some("1.0.0"));
        assert_eq!(
            manifest.interface.unwrap().display_name.as_deref(),
            Some("Memory Bank")
        );
    }

    #[test]
    fn plugin_list_output_parses_status_and_versions() {
        let output = "\
Marketplace `openai-bundled`
C:\\users\\\\.codex\\.tmp\\bundled-marketplaces\\openai-bundled\\.agents\\plugins\\marketplace.json

PLUGIN                           STATUS              VERSION       PATH
codex-app-tools@openai-bundled  installed, enabled  0.1.0         C:\\bundle\\plugins\\codex-app-tools
latex@openai-bundled            not installed                     C:\\bundle\\plugins\\latex
ponytail@ponytail               installed, disabled 4.9.0         C:\\cache\\ponytail\\ponytail\\4.9.0
";
        let items = parse_plugin_list_output(output);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            (
                "codex-app-tools".into(),
                "openai-bundled".into(),
                true,
                Some("0.1.0".into()),
                "C:\\bundle\\plugins\\codex-app-tools".into()
            )
        );
        assert!(!items[1].2);
        assert_eq!(items[1].1, "ponytail");
    }

    #[test]
    fn marketplace_name_falls_back_to_repo() {
        assert_eq!(
            parse_marketplace_name("added Marketplace `ponytail` ok", "fallback"),
            "ponytail"
        );
        assert_eq!(
            parse_marketplace_name("no backticks here", "my-repo"),
            "my-repo"
        );
    }

    #[test]
    fn marketplace_list_output_parses_third_party_marketplaces() {
        let output = "MARKETPLACE             ROOT\nopenai-bundled          C:\\bundled\nponytail                C:\\marketplaces\\ponytail\n";
        let items = parse_marketplace_list_output(output);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "openai-bundled");
        assert_eq!(items[0].kind, "official");
        assert_eq!(items[1].name, "ponytail");
        assert_eq!(items[1].kind, "third-party");
        assert_eq!(items[1].root, "C:\\marketplaces\\ponytail");
    }

    #[test]
    fn marketplaces_sort_official_before_third_party_then_name() {
        let mut items = vec![
            PluginMarketplace {
                name: "zeta".into(),
                kind: "third-party".into(),
                ..Default::default()
            },
            PluginMarketplace {
                name: "beta".into(),
                kind: "official".into(),
                ..Default::default()
            },
            PluginMarketplace {
                name: "alpha".into(),
                kind: "official".into(),
                ..Default::default()
            },
            PluginMarketplace {
                name: "agent".into(),
                kind: "third-party".into(),
                ..Default::default()
            },
        ];

        sort_marketplaces(&mut items);

        assert_eq!(
            items.into_iter().map(|item| item.name).collect::<Vec<_>>(),
            ["alpha", "beta", "agent", "zeta"]
        );
    }

    #[test]
    fn plugin_updates_ignore_official_and_match_newer_third_party_versions() {
        let updates = find_plugin_updates(
            &[
                PluginSummary {
                    name: "official-plugin".into(),
                    version: Some("1.0.0".into()),
                    origin: "official".into(),
                    marketplace: Some("openai-bundled".into()),
                    ..Default::default()
                },
                PluginSummary {
                    name: "ponytail".into(),
                    version: Some("4.9.0".into()),
                    origin: "codex".into(),
                    marketplace: Some("ponytail".into()),
                    ..Default::default()
                },
            ],
            &[MarketplacePlugin {
                plugin_id: "ponytail@ponytail".into(),
                name: "ponytail".into(),
                version: Some("5.0.0".into()),
                installed: true,
                ..Default::default()
            }],
        );

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].name, "ponytail");
        assert_eq!(updates[0].marketplace, "ponytail");
        assert_eq!(updates[0].version, "5.0.0");
    }

    #[test]
    fn marketplace_sources_read_git_source_from_codex_config() {
        let temp = tempfile::tempdir().unwrap();
        let codex = temp.path().join(".codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(
            codex.join("config.toml"),
            "[marketplaces.youmind]\nsource_type = \"git\"\nsource = \"https://github.com/YouMind-OpenLab/plugin-marketplace.git\"\n\n[marketplaces.local]\nsource_type = \"local\"\nsource = \"C:/plugins/local\"\n",
        )
        .unwrap();

        let sources = marketplace_sources(temp.path());
        assert_eq!(
            sources.get("youmind").map(String::as_str),
            Some("https://github.com/YouMind-OpenLab/plugin-marketplace.git")
        );
        assert!(!sources.contains_key("local"));
    }

    #[test]
    fn install_marketplace_redirects_to_remote_identity() {
        let identities = vec![
            "openai-primary-runtime".to_string(),
            "openai-bundled".to_string(),
            "ponytail".to_string(),
            "openai-curated-remote".to_string(),
        ];
        // 镜像名 → 远程原体（桌面端注册表口径）
        assert_eq!(
            resolve_install_marketplace("openai-curated", &identities),
            "openai-curated-remote"
        );
        // 无 -remote 原体的市场保持原样
        assert_eq!(
            resolve_install_marketplace("ponytail", &identities),
            "ponytail"
        );
        // 解析器只认 "Marketplace `X`" 头部行
        let listing = "Marketplace `a`\n\nPLUGIN  STATUS\nx@a  installed, enabled  1.0  C:\\x\nMarketplace `b`\n";
        assert_eq!(parse_marketplace_identities(listing), vec!["a", "b"]);
    }

    #[test]
    fn marketplace_plugin_json_keeps_installed_and_available_entries() {
        let output = r#"{
          "installed": [{
            "pluginId": "ponytail@ponytail",
            "name": "ponytail",
            "marketplaceName": "ponytail",
            "version": "4.9.0",
            "installed": true,
            "enabled": true,
            "installPolicy": "AVAILABLE",
            "authPolicy": "ON_INSTALL",
            "source": {"source": "git", "url": "https://github.com/DietrichGebert/ponytail.git"}
          }],
          "available": [{
            "pluginId": "grill@other",
            "name": "grill",
            "marketplaceName": "other",
            "version": "1.3.0",
            "installed": false,
            "enabled": false,
            "installPolicy": "AVAILABLE",
            "authPolicy": "ON_USE",
            "source": {"source": "url", "url": "https://example.com/grill.git"}
          }]
        }"#;
        let items = parse_marketplace_plugins_output(output, "ponytail", None).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].plugin_id, "ponytail@ponytail");
        assert!(items[0].installed);
        assert_eq!(
            items[0].source.as_deref(),
            Some("https://github.com/DietrichGebert/ponytail.git")
        );
    }

    #[test]
    fn marketplace_plugin_metadata_reads_catalog_and_local_manifest() {
        let root = tempfile::tempdir().unwrap();
        let marketplace_dir = root.path().join(".agents/plugins");
        let plugin_dir = root.path().join("plugins/local-tool/.codex-plugin");
        std::fs::create_dir_all(&marketplace_dir).unwrap();
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::create_dir_all(root.path().join("plugins/local-tool/skills/example")).unwrap();
        std::fs::write(
            root.path()
                .join("plugins/local-tool/skills/example/SKILL.md"),
            "# Example",
        )
        .unwrap();
        std::fs::write(root.path().join("plugins/local-tool/.mcp.json"), "{}").unwrap();
        std::fs::write(
            marketplace_dir.join("marketplace.json"),
            r#"{
              "name": "fixture",
              "interface": {"displayName": "Fixture Market"},
              "metadata": {"description": "Fixture market description"},
              "plugins": [{
                "name": "local-tool",
                "description": "Catalog description",
                "category": "Productivity",
                "keywords": ["local", "fixture"],
                "source": "./plugins/local-tool"
              }]
            }"#,
        )
        .unwrap();
        std::fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
              "name": "local-tool",
              "version": "2.0.0",
              "interface": {
                "displayName": "Local Tool",
                "capabilities": ["Read", "Write"]
              }
            }"#,
        )
        .unwrap();
        let output = r#"{
          "available": [{
            "pluginId": "local-tool@fixture",
            "name": "local-tool",
            "marketplaceName": "fixture",
            "installed": false,
            "enabled": false,
            "installPolicy": "AVAILABLE",
            "authPolicy": "ON_USE",
            "source": "./plugins/local-tool"
          }]
        }"#;

        let items = parse_marketplace_plugins_output(output, "fixture", Some(root.path())).unwrap();
        assert_eq!(items[0].display_name.as_deref(), Some("Local Tool"));
        assert_eq!(items[0].description.as_deref(), Some("Catalog description"));
        assert_eq!(items[0].category.as_deref(), Some("Productivity"));
        assert_eq!(
            items[0].capabilities,
            vec!["Read".to_string(), "Write".to_string()]
        );
        assert_eq!(items[0].version.as_deref(), Some("2.0.0"));
        assert_eq!(items[0].contains, vec!["skills", "mcp"]);
    }

    #[test]
    fn plugin_name_rejects_selector_characters() {
        assert!(validate_plugin_name("memory-bank").is_ok());
        assert!(validate_plugin_name("a@b").is_err());
        assert!(validate_plugin_name("../x").is_err());
    }

    #[test]
    fn marketplace_source_accepts_codex_supported_forms() {
        assert_eq!(
            parse_marketplace_source("owner/repo@main").unwrap().0,
            "owner/repo@main"
        );
        assert_eq!(
            parse_marketplace_source("https://git.example.com/plugins.git").unwrap(),
            (
                "https://git.example.com/plugins.git".into(),
                "plugins".into()
            )
        );
        assert!(parse_marketplace_source("not-a-marketplace").is_err());
    }
}
