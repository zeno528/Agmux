//! 插件存储：CLI 缺席时的缓存扫描、安装路径解析与 contains/skills 读取。

use std::fs;
use std::path::{Path, PathBuf};

use super::catalog::{derive_contains, read_manifest};
use super::skills::{read_skill_description, walk_files};
use super::{PluginSkill, PluginSummary, PLUGIN_CACHE_RELATIVE_PATH};

pub(super) fn store_contains(plugin_dir: &Path) -> Vec<String> {
    let files = walk_files(plugin_dir);
    let relative: Vec<&str> = files.iter().map(|path| path.as_str()).collect();
    derive_contains(&relative)
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
    if let Some(version) = version {
        let version_dir = cache_root.join(version);
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
