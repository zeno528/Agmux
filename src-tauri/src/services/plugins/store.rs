//! 插件存储：CLI 缺席时的缓存扫描、安装路径解析与 contains/skills 读取。

use std::fs;
use std::path::{Path, PathBuf};

use super::catalog::{derive_contains, read_manifest, resolve_local_path};
use super::skills::{read_skill_description, walk_files};
use super::{PluginSkill, PluginSummary, PLUGIN_CACHE_RELATIVE_PATH};

pub(super) fn store_contains(plugin_dir: &Path) -> Vec<String> {
    let files = walk_files(plugin_dir);
    let relative: Vec<&str> = files.iter().map(|path| path.as_str()).collect();
    derive_contains(&relative)
}

/// 图标文件扩展名（与后端图标读取 command 的白名单一致）。
const ICON_EXTENSIONS: [&str; 5] = ["png", "svg", "jpg", "jpeg", "webp"];

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| ICON_EXTENSIONS.contains(&value.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 插件目录内的本地图标：先按约定名直探（codex `assets/icon.png` 实测 4/5 命中），
/// 再模式扫 `assets/`（`icon*` / `logo*` / `app-icon*` 前缀，覆盖 superpowers 的
/// app-icon.png、ponytail 的 logo-dark.png、superpowers 清单引用的 antigravity-logo.png）；
/// 文档截图（example-*、benchmark-* 等）不匹配前缀，不会误当图标。
pub(super) fn plugin_icon_path(plugin_dir: &Path) -> Option<PathBuf> {
    let direct = ["assets/icon.png", "assets/logo.png", "icon.png", "logo.png"]
        .iter()
        .map(|relative| plugin_dir.join(relative))
        .find(|path| path.is_file());
    if direct.is_some() {
        return direct;
    }
    let assets = plugin_dir.join("assets");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&assets)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && is_image_file(path)
                && path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|stem| {
                        let stem = stem.to_ascii_lowercase();
                        ["icon", "logo", "app-icon"]
                            .iter()
                            .any(|prefix| stem.starts_with(prefix))
                    })
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// 已装插件图标解析的目录回退链：`codex plugin list` 报的路径可能是 git URL 或
/// 连接器 ID（实测 ponytail 报 git URL、curated-remote 报 plugin_connector_1p_*），
/// 真身落在市场物化目录或缓存的其他市场别名下，逐个补探。
pub(super) fn installed_icon_fallback(
    codex_home: &Path,
    marketplace: &str,
    name: &str,
) -> Option<PathBuf> {
    // 每个候选目录先读 manifest 的 interface.logo（canva 的图标叫 composer-icon.png，
    // 只有 manifest 指得准），再按约定名/模式扫。
    let find = |dir: &Path| {
        let manifest = read_manifest(dir);
        plugin_icon_from(manifest.as_ref(), dir)
    };
    let candidates = [
        codex_home
            .join(".tmp")
            .join("marketplaces")
            .join(marketplace)
            .join("plugins")
            .join(name),
        // 仓库根即插件的市场（ponytail 单插件仓库）
        codex_home
            .join(".tmp")
            .join("marketplaces")
            .join(marketplace),
    ];
    if let Some(hit) = candidates.iter().find_map(|dir| find(dir)) {
        return Some(hit);
    }
    // 缓存市场别名（openai-curated-remote 的条目物化在 openai-api-curated 下）：
    // 扫缓存根下任意市场目录里的同名插件，取最新版本。
    let cache_root = codex_home.join(PLUGIN_CACHE_RELATIVE_PATH);
    std::fs::read_dir(&cache_root)
        .ok()?
        .flatten()
        .filter_map(|market| {
            let versions = std::fs::read_dir(market.path().join(name)).ok()?;
            versions
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .max_by_key(|entry| entry.file_name())
                .map(|entry| entry.path())
        })
        .find_map(|dir| find(&dir))
}

/// 图标解析统一口：Codex 的 `interface.logo`/`logoDark`、Claude 的顶层 `logo` 是相对路径
/// （实测 github → ./assets/github.png，文件名各插件自定义，猜不中）；
/// manifest 缺字段或文件不在时按约定名探测。只认仓库内相对路径，外链与穿越拒绝。
pub(super) fn plugin_icon_from(
    manifest: Option<&super::PluginManifest>,
    plugin_dir: &Path,
) -> Option<PathBuf> {
    manifest
        .and_then(|item| item.interface.as_ref())
        .and_then(|interface| {
            interface
                .logo
                .clone()
                .or_else(|| interface.logo_dark.clone())
        })
        .or_else(|| manifest.and_then(|item| item.logo.clone()))
        .filter(|raw| !raw.contains("://") && !raw.contains('\\'))
        .and_then(|raw| resolve_local_path(plugin_dir, &raw))
        .filter(|path| path.is_file())
        .or_else(|| plugin_icon_path(plugin_dir))
}

/// 读取插件内的 Skill 清单；Codex 插件约定为 `skills/<name>/SKILL.md`。
pub(super) fn store_skills(plugin_dir: &Path) -> Vec<PluginSkill> {
    walk_files(plugin_dir)
        .into_iter()
        .filter_map(|path| {
            let name = if path == "SKILL.md" {
                plugin_dir
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("SKILL")
                    .to_string()
            } else {
                let skill_path = path.strip_prefix("skills/")?;
                let name = skill_path.strip_suffix("/SKILL.md")?;
                if name.is_empty() {
                    return None;
                }
                name.to_string()
            };
            let description = read_skill_description(&plugin_dir.join(&path));
            Some(PluginSkill {
                name,
                path,
                description,
            })
        })
        .collect()
}

/// CLI 缺席时的回退：扫 `~/.codex/plugins/cache/<marketplace>/<plugin>/<version>/`。
pub(super) fn scan_codex_plugin_cache(codex_home: &Path) -> Vec<PluginSummary> {
    let cache_root = codex_home.join(PLUGIN_CACHE_RELATIVE_PATH);
    let Ok(marketplaces) = std::fs::read_dir(&cache_root) else {
        return Vec::new();
    };
    let mut summaries = Vec::new();
    for marketplace in marketplaces.flatten() {
        let marketplace_name = marketplace.file_name().to_string_lossy().to_string();
        let Ok(plugins) = std::fs::read_dir(marketplace.path()) else {
            continue;
        };
        for plugin in plugins.flatten() {
            let Ok(versions) = std::fs::read_dir(plugin.path()) else {
                continue;
            };
            let Some((version_dir_name, version_dir)) = versions
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().to_string(),
                        entry.path(),
                    )
                })
                .max_by(|(left, _), (right, _)| left.cmp(right))
            else {
                continue;
            };
            let manifest = read_manifest(&version_dir);
            let plugin_dir_name = plugin.file_name().to_string_lossy().to_string();
            let mut summary = PluginSummary::from_parts(
                &plugin_dir_name,
                manifest.as_ref(),
                store_contains(&version_dir),
                "codex",
                &version_dir,
            );
            // 版本以 manifest 为准：目录名是安装时定死的兜底值，升级后僵死
            // （实测 ponytail 目录名 1.0.0、manifest 4.10.3）；manifest 缺才回落。
            summary.version = summary.version.or(Some(version_dir_name));
            summary.marketplace = Some(marketplace_name.clone());
            summary.enabled = true;
            summaries.push(summary);
        }
    }
    summaries
}

/// Codex 有时在列表中返回 Git 源而非已物化的插件目录；此时读取实际缓存。
pub(super) fn plugin_store_path(
    codex_home: &Path,
    marketplace: &str,
    name: &str,
    version: Option<&str>,
    reported_path: &str,
) -> PathBuf {
    let reported = Path::new(reported_path);
    if reported.is_dir() {
        return reported.to_path_buf();
    }
    let cache_root = codex_home
        .join(PLUGIN_CACHE_RELATIVE_PATH)
        .join(marketplace)
        .join(name);
    if let Some(version_dir) = version.and_then(|version| resolve_local_path(&cache_root, version))
    {
        if version_dir.is_dir() {
            return version_dir;
        }
    }
    std::fs::read_dir(&cache_root)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .max_by_key(|entry| entry.file_name())
        })
        .map(|entry| entry.path())
        .unwrap_or(cache_root)
}

pub(super) fn trusted_plugin_store_path(
    home: &Path,
    codex_home: &Path,
    name: &str,
    raw_path: &str,
) -> Option<PathBuf> {
    let path = fs::canonicalize(raw_path).ok()?;
    let roots = [
        codex_home.join("plugins"),
        home.join(".cache/codex-runtimes/plugins"),
        home.join(".cache/codex/plugins"),
    ];
    if !roots.iter().any(|root| {
        fs::canonicalize(root)
            .map(|root| path.starts_with(root))
            .unwrap_or(false)
    }) {
        return None;
    }
    (read_manifest(&path)
        .as_ref()
        .is_some_and(|manifest| manifest.name == name))
    .then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_icon_path_pattern_scans_assets_without_picking_screenshots() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("superpowers");
        let assets = plugin.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        // 文档截图与基准图：不以 icon/logo/app-icon 开头，不得命中
        for name in [
            "benchmark-3model.svg",
            "claude-md-improver-example.png",
            "readme.jpg",
        ] {
            std::fs::write(assets.join(name), b"x").unwrap();
        }
        assert_eq!(plugin_icon_path(&plugin), None);
        std::fs::write(assets.join("app-icon.png"), b"x").unwrap();
        std::fs::write(assets.join("logo-dark.png"), b"x").unwrap();
        assert_eq!(
            plugin_icon_path(&plugin),
            Some(assets.join("app-icon.png")),
            "多个候选按文件名排序取稳定结果"
        );
    }

    #[test]
    fn installed_icon_fallback_probes_market_checkout_and_cache_alias() {
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        // 仓库根即插件的市场（ponytail 形态）：.tmp/marketplaces/<market>/assets/logo.png
        let repo_root = codex_home.join(".tmp/marketplaces/sample-market");
        std::fs::create_dir_all(repo_root.join("assets")).unwrap();
        std::fs::write(repo_root.join("assets/logo.png"), b"x").unwrap();
        assert_eq!(
            installed_icon_fallback(&codex_home, "sample-market", "sample-market"),
            Some(repo_root.join("assets/logo.png"))
        );
        // 缓存别名市场：curated-remote 条目物化在别的市场目录名下
        let cached = codex_home
            .join(PLUGIN_CACHE_RELATIVE_PATH)
            .join("alias-market")
            .join("gmail")
            .join("0.1.22")
            .join("assets");
        std::fs::create_dir_all(&cached).unwrap();
        std::fs::write(cached.join("icon.png"), b"x").unwrap();
        assert_eq!(
            installed_icon_fallback(&codex_home, "curated-remote", "gmail"),
            Some(cached.join("icon.png"))
        );
    }

    #[test]
    fn plugin_icon_from_prefers_manifest_logo_and_rejects_links() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("github");
        std::fs::create_dir_all(plugin.join("assets")).unwrap();
        std::fs::write(plugin.join("assets/icon.png"), b"x").unwrap();
        std::fs::write(plugin.join("assets/real-logo.png"), b"x").unwrap();
        // 官方字段形态：相对路径带 ./ 前缀，文件名各插件自定义（github → assets/github.png）
        for text in [
            r#"{"name":"github","interface":{"logo":"./assets/real-logo.png"}}"#,
            r#"{"name":"github","logo":"./assets/real-logo.png"}"#,
        ] {
            let manifest = crate::services::plugins::catalog::parse_manifest_text(text).unwrap();
            assert_eq!(
                plugin_icon_from(Some(&manifest), &plugin),
                Some(plugin.join("assets/real-logo.png")),
                "manifest logo 优先于约定名"
            );
        }
        // 外链与路径穿越不是本地图标，回落约定名
        let remote = crate::services::plugins::catalog::parse_manifest_text(
            r#"{"name":"x","interface":{"logo":"https://cdn.example.com/logo.png"}}"#,
        )
        .unwrap();
        assert_eq!(
            plugin_icon_from(Some(&remote), &plugin),
            Some(plugin.join("assets/icon.png"))
        );
        assert_eq!(
            plugin_icon_from(None, &plugin),
            Some(plugin.join("assets/icon.png"))
        );
    }

    #[test]
    fn plugin_icon_path_prefers_assets_convention() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join("sample");
        assert_eq!(plugin_icon_path(&plugin), None, "空目录无图标");
        std::fs::create_dir_all(plugin.join("assets")).unwrap();
        std::fs::write(plugin.join("assets/logo.png"), b"png").unwrap();
        assert_eq!(
            plugin_icon_path(&plugin),
            Some(plugin.join("assets/logo.png"))
        );
        std::fs::write(plugin.join("assets/icon.png"), b"png").unwrap();
        assert_eq!(
            plugin_icon_path(&plugin),
            Some(plugin.join("assets/icon.png")),
            "icon.png 优先于 logo.png"
        );
    }

    #[test]
    fn store_skills_reads_names_and_descriptions() {
        let root = tempfile::tempdir().unwrap();
        let skill_dir = root.path().join("skills").join("session-summary");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Summarize sessions\n---\n# Session summary\n",
        )
        .unwrap();

        let skills = store_skills(root.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "session-summary");
        assert_eq!(skills[0].path, "skills/session-summary/SKILL.md");
        assert_eq!(skills[0].description.as_deref(), Some("Summarize sessions"));
    }

    #[test]
    fn plugin_store_path_falls_back_to_cached_version_for_git_source() {
        let codex_home = tempfile::tempdir().unwrap();
        let cached = codex_home
            .path()
            .join(PLUGIN_CACHE_RELATIVE_PATH)
            .join("ponytail")
            .join("ponytail")
            .join("4.9.0");
        std::fs::create_dir_all(&cached).unwrap();

        assert_eq!(
            plugin_store_path(
                codex_home.path(),
                "ponytail",
                "ponytail",
                Some("4.9.0"),
                "https://github.com/DietrichGebert/ponytail.git, ref `main`",
            ),
            cached
        );
        for version in ["..", ".", "../.."] {
            assert_eq!(
                plugin_store_path(
                    codex_home.path(),
                    "ponytail",
                    "ponytail",
                    Some(version),
                    "missing"
                ),
                cached
            );
        }
        #[cfg(windows)]
        {
            let drive = codex_home
                .path()
                .components()
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy();
            let version = format!("{drive}.");
            assert_eq!(
                plugin_store_path(
                    codex_home.path(),
                    "ponytail",
                    "ponytail",
                    Some(&version),
                    "missing"
                ),
                cached
            );
        }
    }

    #[tokio::test]
    async fn scan_codex_plugin_cache_reads_manifest_and_version() {
        // 直接测底层 cache 扫描函数，避开 list_plugins 的 CLI 探测链。
        // 本机 PATH 装了真 codex CLI 会劫持 list_plugins 走 CLI 路径，与 cache fixture 无关；
        // 这里直调 scan_codex_plugin_cache，本机环境跟它零耦合。
        // fixture 用抽象名（sample-marketplace / sample-plugin / v1.0.0），不撞现实插件。
        // 目录名是安装时定死的兜底值，升级后可能僵死：sample-plugin 的 manifest
        // 缺 version 回落目录名，stale-plugin 的 manifest version 优先于目录名。
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        let write_plugin = |name: &str, dir_version: &str, manifest: &str| {
            let dir = codex_home
                .join("plugins")
                .join("cache")
                .join("sample-marketplace")
                .join(name)
                .join(dir_version)
                .join(".codex-plugin");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("plugin.json"), manifest).unwrap();
        };
        write_plugin("sample-plugin", "v1.0.0", r#"{"name":"sample-plugin"}"#);
        write_plugin(
            "stale-plugin",
            "0.9.0",
            r#"{"name":"stale-plugin","version":"1.2.0"}"#,
        );

        let plugins = scan_codex_plugin_cache(&codex_home);
        assert_eq!(plugins.len(), 2);
        let plugin = plugins
            .iter()
            .find(|item| item.name == "sample-plugin")
            .unwrap();
        assert_eq!(plugin.marketplace.as_deref(), Some("sample-marketplace"));
        assert_eq!(plugin.version.as_deref(), Some("v1.0.0"));
        assert_eq!(plugin.origin, "codex");
        assert!(plugin.enabled);
        let stale = plugins
            .iter()
            .find(|item| item.name == "stale-plugin")
            .unwrap();
        assert_eq!(stale.version.as_deref(), Some("1.2.0"));
    }
}
