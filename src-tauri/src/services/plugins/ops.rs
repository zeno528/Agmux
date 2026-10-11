//! 插件生命周期操作：列表、市场管理、安装/卸载/升级与 GitHub 预览。

use std::path::{Path, PathBuf};

use crate::codex::config as codex_config;
use crate::error::{app_err, AppResult};
use crate::fsutil::{atomic_write, backup_file};
use crate::paths::now_ms;
use crate::services::plugin_net::{
    fetch_raw_file, fetch_repo_tree, parse_github_url, preview_file_limit,
};
use crate::services::AppContext;

use super::catalog::{
    derive_contains, enrich_marketplace_metadata, enrich_plugin_sources, files_under_root,
    find_plugin_updates, manifest_description, marketplace_sources, parse_manifest_text,
    parse_marketplace_identities, parse_marketplace_list_output, parse_marketplace_name,
    parse_marketplace_plugins_output, parse_marketplace_source, parse_plugin_list_output,
    plugin_roots, read_manifest, resolve_install_marketplace, root_within, sort_marketplaces,
    validate_plugin_name,
};
use super::cli::{find_codex_cli, run_codex_plugin};
use super::skills::is_registered_skill;
use super::store::{
    plugin_store_path, scan_codex_plugin_cache, store_contains, store_skills,
    trusted_plugin_store_path,
};
use super::MANIFEST_RELATIVE_PATH;
use super::{
    MarketplacePlugin, PluginCandidate, PluginMarketplace, PluginPreview, PluginSkill,
    PluginSummary, PluginUpdate,
};

// ==================== AppContext 服务 ====================

impl AppContext {
    /// Codex CLI 直接写 config.toml 后，收拢 Agofra 管理的全局段；无变化不落盘。
    fn normalize_plugin_config_order(&self) -> AppResult<()> {
        let config_path = self.paths.codex_config();
        let text = self.read_live_config()?;
        let normalized = codex_config::normalize_global_section_order(&text);
        if normalized != text {
            backup_file(&config_path, &self.paths.config_backup, "config")?;
            atomic_write(&config_path, normalized.as_bytes())?;
        }
        Ok(())
    }

    /// 已安装插件列表：`codex plugin list`（主源，含启停）→ 插件缓存（CLI 缺席回退）。
    pub async fn list_plugins(&self) -> AppResult<Vec<PluginSummary>> {
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Ok(Vec::new());
        };
        let codex_home = self.paths.codex_home.clone();
        let summaries =
            tauri::async_runtime::spawn_blocking(move || list_plugins_sync(&home, &codex_home))
                .await
                .map_err(|error| app_err!("插件列表任务失败: {error}"))??;
        Ok(summaries)
    }

    /// 进入插件详情时再读取该插件内的 Skill 明细，避免列表阶段扫描所有插件目录。
    pub async fn list_plugin_skills(
        &self,
        name: &str,
        requested_store_path: Option<&str>,
    ) -> AppResult<Vec<PluginSkill>> {
        validate_plugin_name(name)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let codex_home = self.paths.codex_home.clone();
        let name = name.to_string();
        let requested_store_path = requested_store_path.map(str::to_owned);
        tauri::async_runtime::spawn_blocking(move || {
            let mut store_path = requested_store_path
                .as_deref()
                .and_then(|path| trusted_plugin_store_path(&home, &codex_home, &name, path));
            if store_path.is_none() {
                store_path = list_plugins_sync(&home, &codex_home)?
                    .into_iter()
                    .find(|item| item.name == name)
                    .map(|item| PathBuf::from(item.store_path));
            }
            let store_path = store_path.ok_or_else(|| app_err!("没有找到名为「{name}」的插件"))?;
            if !store_path.is_dir() {
                return Ok(Vec::new());
            }
            Ok(store_skills(&store_path))
        })
        .await
        .map_err(|error| app_err!("插件 Skill 读取任务失败: {error}"))?
    }

    /// 读取 Codex 当前配置中的插件市场，包含官方与外部市场。
    pub async fn list_plugin_marketplaces(&self) -> AppResult<Vec<PluginMarketplace>> {
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Ok(Vec::new());
        };
        tauri::async_runtime::spawn_blocking(move || {
            let output = run_codex_plugin(&home, &["marketplace", "list"])?;
            let mut items = parse_marketplace_list_output(&output);
            enrich_marketplace_metadata(&mut items, &marketplace_sources(&home));
            sort_marketplaces(&mut items);
            Ok(items)
        })
        .await
        .map_err(|error| app_err!("插件市场列表任务失败: {error}"))?
    }

    /// 读取 Codex 已配置市场的官方目录快照；available 同时包含已装与未装条目。
    pub async fn list_marketplace_plugins(
        &self,
        marketplace: &str,
        root: Option<&str>,
    ) -> AppResult<Vec<MarketplacePlugin>> {
        validate_plugin_name(marketplace)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Ok(Vec::new());
        };
        let marketplace = marketplace.to_string();
        let root = root
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        tauri::async_runtime::spawn_blocking(move || {
            let output = run_codex_plugin(
                &home,
                &[
                    "list",
                    "--marketplace",
                    &marketplace,
                    "--available",
                    "--json",
                ],
            )?;
            // 别名身份（openai-curated ↔ openai-curated-remote）导致的 installed 标记
            // 偏差由前端对照已安装名单纠正（数据现成，避免此处每次多跑一轮 CLI）。
            parse_marketplace_plugins_output(&output, &marketplace, root.as_deref())
        })
        .await
        .map_err(|error| app_err!("插件市场目录任务失败: {error}"))?
    }

    /// 按 Codex 官方选择器安装已配置市场中的插件，并继承当前 live config.toml 的供应商配置。
    pub async fn install_marketplace_plugin(
        &self,
        marketplace: &str,
        name: &str,
    ) -> AppResult<PluginSummary> {
        validate_plugin_name(marketplace)?;
        validate_plugin_name(name)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let marketplace = marketplace.to_string();
        let name = name.to_string();
        tauri::async_runtime::spawn_blocking({
            let home = home.clone();
            let marketplace = marketplace.clone();
            let name = name.clone();
            move || {
                // Codex 给官方 curated 目录挂镜像/远程双身份（openai-curated ↔
                // openai-curated-remote）：桌面端安装注册表只落在远程身份下，
                // 镜像名安装对桌面端不可见。目标市场存在 `-remote` 原体时重定向，
                // 其余市场不受影响。
                let mut target = marketplace.clone();
                if let Ok(listing) = run_codex_plugin(&home, &["list"]) {
                    target = resolve_install_marketplace(
                        &marketplace,
                        &parse_marketplace_identities(&listing),
                    );
                }
                if target != marketplace {
                    tauri_plugin_log::log::debug!(
                        "[plugin.market.resolve] plugin={marketplace:?} target={target:?} outcome=success msg=\"安装市场解析\""
                    );
                }
                let output = run_codex_plugin(&home, &["add", &name, "--marketplace", &target]);
                // 用户触发的里程碑动作，Info 级在 release 也留痕（失败已由 CLI 咽喉 Warn）
                if output.is_ok() {
                    tauri_plugin_log::log::info!(
                        "[plugin.install] plugin={name:?} target={target:?} outcome=success msg=\"插件安装成功\""
                    );
                }
                output
            }
        })
        .await
        .map_err(|error| app_err!("安装插件任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;

        let _ = self.database.record_event(
            None,
            "plugin",
            "install",
            Some(&format!("{name}@{marketplace}")),
            &now_ms().to_string(),
        );

        self.list_plugins()
            .await?
            .into_iter()
            .find(|plugin| plugin.name == name)
            .ok_or_else(|| app_err!("插件「{name}」已执行安装，但列表中尚未出现"))
    }

    /// 刷新第三方 Git 市场快照，并找出版本变化的已安装插件。
    pub async fn check_plugin_updates(&self) -> AppResult<Vec<PluginUpdate>> {
        let marketplaces = self.list_plugin_marketplaces().await?;
        let third_party: Vec<_> = marketplaces
            .into_iter()
            .filter(|marketplace| marketplace.kind == "third-party")
            .collect();
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Ok(Vec::new());
        };
        let names: Vec<_> = third_party
            .iter()
            .map(|marketplace| marketplace.name.clone())
            .collect();
        tauri::async_runtime::spawn_blocking(move || {
            for name in names {
                run_codex_plugin(&home, &["marketplace", "upgrade", &name])?;
            }
            Ok(())
        })
        .await
        .map_err(|error| app_err!("检查插件更新任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;

        let installed = self.list_plugins().await?;
        let mut updates = Vec::new();
        for marketplace in third_party {
            let available = self
                .list_marketplace_plugins(&marketplace.name, Some(&marketplace.root))
                .await?;
            updates.extend(find_plugin_updates(&installed, &available));
        }
        Ok(updates)
    }

    /// 用已刷新的外部市场快照重新安装指定插件。
    pub async fn upgrade_marketplace_plugin(&self, marketplace: &str, name: &str) -> AppResult<()> {
        validate_plugin_name(marketplace)?;
        validate_plugin_name(name)?;
        let marketplaces = self.list_plugin_marketplaces().await?;
        if !marketplaces
            .iter()
            .any(|item| item.name == marketplace && item.kind == "third-party")
        {
            return Err(app_err!("只能升级外部插件市场中的插件"));
        }
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let selector = format!("{name}@{marketplace}");
        tauri::async_runtime::spawn_blocking(move || run_codex_plugin(&home, &["add", &selector]))
            .await
            .map_err(|error| app_err!("升级插件任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;
        Ok(())
    }

    /// 添加 Git 插件市场，并返回 Codex 识别到的市场名。
    pub async fn add_plugin_marketplace(&self, url: &str) -> AppResult<PluginMarketplace> {
        let (source_arg, fallback_name) = parse_marketplace_source(url)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let marketplace = tauri::async_runtime::spawn_blocking(move || {
            let output = match run_codex_plugin(&home, &["marketplace", "add", &source_arg]) {
                Ok(output) => output,
                Err(error)
                    if error
                        .to_string()
                        .to_ascii_lowercase()
                        .contains("reserved and cannot be added") =>
                {
                    // Codex 保留的官方市场（openai-curated/bundled 等）由官方自动提供，
                    // 拒绝用户来源添加——属预期行为，翻译成人话而不是透传 CLI 报错
                    return Err(app_err!(
                        "「{fallback_name}」属于 Codex 官方保留市场，已由 Codex 自动提供（列表中显示为已安装），无需手动添加"
                    ));
                }
                Err(error)
                    if error
                        .to_string()
                        .to_ascii_lowercase()
                        .contains("already added from a different source") =>
                {
                    let list_output = run_codex_plugin(&home, &["marketplace", "list"])?;
                    let mut marketplaces = parse_marketplace_list_output(&list_output);
                    enrich_marketplace_metadata(&mut marketplaces, &marketplace_sources(&home));
                    return marketplaces
                        .into_iter()
                        .find(|marketplace| marketplace.name == fallback_name)
                        .ok_or(error);
                }
                Err(error) => return Err(error),
            };
            let name = parse_marketplace_name(&output, &fallback_name);
            let list_output = run_codex_plugin(&home, &["marketplace", "list"])?;
            let mut marketplaces = parse_marketplace_list_output(&list_output);
            enrich_marketplace_metadata(&mut marketplaces, &marketplace_sources(&home));
            Ok(marketplaces
                .into_iter()
                .find(|marketplace| marketplace.name == name)
                .unwrap_or(PluginMarketplace {
                    name,
                    root: String::new(),
                    kind: "third-party".into(),
                    source_url: None,
                    display_name: None,
                    description: None,
                }))
        })
        .await
        .map_err(|error| app_err!("添加插件市场任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;
        Ok(marketplace)
    }

    /// 移除第三方插件市场来源；遵循 Codex CLI 语义，不自动删除该市场下已安装的插件。
    pub async fn remove_plugin_marketplace(&self, name: &str) -> AppResult<()> {
        validate_plugin_name(name)?;
        let marketplaces = self.list_plugin_marketplaces().await?;
        let Some(marketplace) = marketplaces.iter().find(|item| item.name == name) else {
            return Err(app_err!("没有找到插件市场「{name}」"));
        };
        if marketplace.kind == "official" {
            return Err(app_err!("「{name}」属于 Codex 官方市场，不能在这里移除"));
        }
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let name = name.to_string();
        let command_name = name.clone();
        tauri::async_runtime::spawn_blocking(move || {
            run_codex_plugin(&home, &["marketplace", "remove", &command_name])
        })
        .await
        .map_err(|error| app_err!("移除插件市场任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;
        let _ = self.database.record_event(
            None,
            "plugin",
            "marketplace-remove",
            Some(&name),
            &now_ms().to_string(),
        );
        Ok(())
    }

    /// 预览：仓库元数据 + 每个插件根的清单与文件列表（不落盘）。
    pub async fn preview_plugin(&self, url: &str) -> AppResult<PluginPreview> {
        let source = parse_github_url(url)?;
        let tree = fetch_repo_tree(&source).await?;
        let roots: Vec<String> = plugin_roots(&tree.entries)
            .into_iter()
            .filter(|root| root_within(root, source.sub_path.as_deref().unwrap_or_default()))
            .collect();
        if roots.is_empty() {
            return Err(app_err!(
                "仓库里没有找到 {MANIFEST_RELATIVE_PATH}，请确认这是一个 Codex 插件仓库"
            ));
        }

        let mut candidates = Vec::new();
        for root in roots {
            let files = files_under_root(&tree.entries, &root);
            let relative_manifest = if root.is_empty() {
                MANIFEST_RELATIVE_PATH.to_string()
            } else {
                format!("{root}/{MANIFEST_RELATIVE_PATH}")
            };
            let manifest_bytes =
                fetch_raw_file(&source, &tree.reference, &relative_manifest).await?;
            let manifest = parse_manifest_text(&String::from_utf8_lossy(&manifest_bytes))?;
            validate_plugin_name(&manifest.name)?;
            let contains = derive_contains(&files);
            candidates.push(PluginCandidate {
                sub_path: root,
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                display_name: manifest
                    .interface
                    .as_ref()
                    .and_then(|item| item.display_name.clone()),
                description: manifest_description(&manifest),
                capabilities: manifest
                    .interface
                    .as_ref()
                    .map(|item| item.capabilities.clone())
                    .unwrap_or_default(),
                contains,
                files: files
                    .iter()
                    .map(|path| path.to_string())
                    .take(preview_file_limit())
                    .collect(),
            });
        }
        Ok(PluginPreview {
            repo: format!("{}/{}", source.owner, source.repo),
            reference: tree.reference,
            default_branch: tree.default_branch,
            candidates,
        })
    }

    /// 安装：预览拿到插件名 → `marketplace add <owner/repo[@ref]>` → `plugin add <名>@<市场>`。
    /// 全程官方 CLI 路径，落盘与状态归 Codex 管。
    pub async fn install_plugin(
        &self,
        url: &str,
        sub_path: Option<&str>,
    ) -> AppResult<PluginSummary> {
        let source = parse_github_url(url)?;
        let tree = fetch_repo_tree(&source).await?;
        let target_sub = sub_path.or(source.sub_path.as_deref());
        let roots: Vec<String> = plugin_roots(&tree.entries)
            .into_iter()
            .filter(|root| root_within(root, target_sub.unwrap_or_default()))
            .collect();
        let root = match roots.as_slice() {
            [root] => root.clone(),
            [] => {
                return Err(app_err!(
                    "仓库里没有找到 {MANIFEST_RELATIVE_PATH}，请确认地址指向 Codex 插件目录"
                ))
            }
            _ => {
                return Err(app_err!(
                    "该地址下有 {} 个插件，请先预览并选择具体插件",
                    roots.len()
                ))
            }
        };
        let relative_manifest = if root.is_empty() {
            MANIFEST_RELATIVE_PATH.to_string()
        } else {
            format!("{root}/{MANIFEST_RELATIVE_PATH}")
        };
        let manifest_bytes = fetch_raw_file(&source, &tree.reference, &relative_manifest).await?;
        let manifest = parse_manifest_text(&String::from_utf8_lossy(&manifest_bytes))?;
        validate_plugin_name(&manifest.name)?;

        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let source_arg = match &source.ref_name {
            Some(reference) => format!("{}/{}@{reference}", source.owner, source.repo),
            None => format!("{}/{}", source.owner, source.repo),
        };
        let selector_name = manifest.name.clone();
        // CLI 调用跑在 blocking 线程池（git 同步可能数十秒）
        let marketplace_name = tauri::async_runtime::spawn_blocking({
            let home = home.clone();
            let source_arg = source_arg.clone();
            move || {
                let output = run_codex_plugin(&home, &["marketplace", "add", &source_arg]);
                match output {
                    Ok(text) => parse_marketplace_name(&text, ""),
                    Err(error) => {
                        // 源可能已添加过（重复安装/升级）：忽略 add 错误，让 plugin add 兜底
                        let _ = error;
                        String::new()
                    }
                }
            }
        })
        .await
        .map_err(|error| app_err!("安装任务失败: {error}"))?;
        let marketplace_name = if marketplace_name.is_empty() {
            source.repo.clone()
        } else {
            marketplace_name
        };

        let selector = format!("{selector_name}@{marketplace_name}");
        tauri::async_runtime::spawn_blocking({
            let home = home.clone();
            let selector = selector.clone();
            move || run_codex_plugin(&home, &["add", &selector])
        })
        .await
        .map_err(|error| app_err!("安装任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;

        let _ = self.database.record_event(
            None,
            "plugin",
            "install",
            Some(&format!("{selector}@{}", tree.reference)),
            &now_ms().to_string(),
        );

        // 从列表里取回安装后的真实状态（版本/路径由 Codex 维护）
        let plugins = self.list_plugins().await?;
        plugins
            .into_iter()
            .find(|item| item.name == manifest.name)
            .ok_or_else(|| {
                app_err!(
                    "安装命令已执行，但列表里没找到「{}」，请刷新查看",
                    manifest.name
                )
            })
    }

    /// 卸载：由 Codex CLI 管理的第三方插件走 `codex plugin remove`。
    pub async fn uninstall_plugin(&self, name: &str) -> AppResult<()> {
        validate_plugin_name(name)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        if is_registered_skill(&home, name) {
            return Err(app_err!(
                "「{name}」属于 Codex Skill 注册表，请在 Codex 内管理它"
            ));
        }
        let plugins = self.list_plugins().await?;
        let Some(plugin) = plugins.iter().find(|item| item.name == name) else {
            return Err(app_err!("没有找到名为「{name}」的插件"));
        };
        if plugin.origin == "official" {
            return Err(app_err!(
                "「{name}」属于 Codex 官方市场，请在 Codex 内管理它"
            ));
        }
        if plugin.origin == "codex" {
            let marketplace = plugin
                .marketplace
                .clone()
                .ok_or_else(|| app_err!("缺少「{name}」的市场信息，无法调用卸载"))?;
            let selector = format!("{name}@{marketplace}");
            tauri::async_runtime::spawn_blocking({
                let home = home.clone();
                let selector = selector.clone();
                move || {
                    let output = run_codex_plugin(&home, &["remove", &selector]);
                    // 与安装同口径：用户触发的里程碑动作，Info 级 release 也留痕
                    if output.is_ok() {
                        tauri_plugin_log::log::info!(
                            "[plugin.uninstall] selector={selector:?} outcome=success msg=\"插件卸载成功\""
                        );
                    }
                    output
                }
            })
            .await
            .map_err(|error| app_err!("卸载任务失败: {error}"))??;
            self.normalize_plugin_config_order()?;
            let _ = self.database.record_event(
                None,
                "plugin",
                "uninstall",
                Some(name),
                &now_ms().to_string(),
            );
            return Ok(());
        }
        Err(app_err!("不支持的插件来源"))
    }
}

/// Codex 桌面端自身内置的实现层插件（浏览器、计算机操作、桌面工具通道）：
/// 桌面 UI 不计入已安装、用户不可管理，这里同样过滤以保持两侧一致。
pub(super) fn is_desktop_builtin(name: &str) -> bool {
    matches!(
        name,
        "browser" | "chrome" | "unified-computer-use" | "codex-app-tools"
    )
}

/// list_plugins 的同步实现（跑在 blocking 线程池）。
pub(super) fn list_plugins_sync(home: &Path, codex_home: &Path) -> AppResult<Vec<PluginSummary>> {
    let sources = marketplace_sources(home);
    let mut summaries: Vec<PluginSummary> = Vec::new();

    // 1) `codex plugin list`（主源）：覆盖运行时、捆绑和外部市场，含启停状态。
    //    CLI 在但 list 失败（超时/报错）时把错误交给上层，让前端保住上一份真实缓存；
    //    禁止回退缓存扫描——plugins/cache 是全量市场目录（含未装条目、桌面内置、
    //    镜像重复、origin 全错），静默顶替会把它伪装成"已安装列表"。
    if find_codex_cli(home).is_some() {
        let output = run_codex_plugin(home, &["list"])?;
        for (name, marketplace, enabled, version, path) in parse_plugin_list_output(&output) {
            if is_desktop_builtin(&name) {
                continue;
            }
            let origin = if marketplace.starts_with("openai") {
                "official"
            } else {
                "codex"
            };
            let plugin_path =
                plugin_store_path(codex_home, &marketplace, &name, version.as_deref(), &path);
            let manifest = read_manifest(&plugin_path);
            summaries.push(PluginSummary {
                // 版本以 manifest 为准（根因见 store.rs），读不到才回落 CLI 版本。
                version: manifest
                    .as_ref()
                    .and_then(|item| item.version.clone())
                    .or(version),
                display_name: manifest
                    .as_ref()
                    .and_then(|item| item.interface.as_ref())
                    .and_then(|item| item.display_name.clone()),
                description: manifest.as_ref().and_then(manifest_description),
                category: manifest
                    .as_ref()
                    .and_then(|item| item.interface.as_ref())
                    .and_then(|item| item.category.clone()),
                capabilities: manifest
                    .as_ref()
                    .and_then(|item| item.interface.as_ref())
                    .map(|item| item.capabilities.clone())
                    .unwrap_or_default(),
                contains: if plugin_path.is_dir() {
                    store_contains(&plugin_path)
                } else {
                    Vec::new()
                },
                enabled,
                origin: origin.to_string(),
                marketplace: Some(marketplace),
                store_path: plugin_path.display().to_string(),
                source_url: None,
                name,
            });
        }
    } else {
        summaries.extend(scan_codex_plugin_cache(codex_home));
    }
    enrich_plugin_sources(&mut summaries, &sources);
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::codex_cli::file_name;
    use crate::services::plugins::test_context;

    /// CLI 在而 list 失败时必须报错，不能回退缓存扫描：plugins/cache 是全量市场目录
    /// （含未装条目、桌面内置、镜像重复、origin 全错），静默顶替会把它伪装成已安装列表。
    #[test]
    fn cli_list_failure_surfaces_error_not_cache_scan() {
        let home = tempfile::tempdir().unwrap();
        // 探测链首位（~/.codex/bin）放一个"存在但不可执行"的假 CLI：find 命中、
        // spawn/执行必败，让分支稳定落在"CLI 在但 list 失败"，与本机真实 CLI 解耦
        let bin = home.path().join(".codex").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(file_name()), "not an executable").unwrap();
        // 缓存里放可扫条目：若错误被缓存扫描吞掉，这里会被冒充成已安装列表返回
        let manifest_dir = home
            .path()
            .join(".codex/plugins/cache/sample-marketplace/sample-plugin/v1.0.0/.codex-plugin");
        std::fs::create_dir_all(&manifest_dir).unwrap();
        std::fs::write(
            manifest_dir.join("plugin.json"),
            r#"{"name":"sample-plugin"}"#,
        )
        .unwrap();

        let result = list_plugins_sync(home.path(), &home.path().join(".codex"));
        assert!(
            result.is_err(),
            "CLI 在而 list 失败应报错，不能拿缓存目录冒充已安装列表：{:?}",
            result
        );
    }

    #[test]
    fn desktop_builtin_plugins_are_identified() {
        for name in [
            "browser",
            "chrome",
            "unified-computer-use",
            "codex-app-tools",
        ] {
            assert!(is_desktop_builtin(name), "{name} 应识别为桌面内置");
        }
        assert!(!is_desktop_builtin("computer-use"));
        assert!(!is_desktop_builtin("ponytail"));
    }

    #[tokio::test]
    async fn readonly_plugins_reject_uninstall() {
        let (home, context) = test_context();
        std::fs::create_dir_all(home.path().join(".agents/skills")).unwrap();
        std::fs::write(
            home.path().join(".agents/.skill-lock.json"),
            r#"{"version":3,"skills":{"lark-base":{"source":"larksuite/cli","sourceType":"github"}}}"#,
        )
        .unwrap();
        let error = context.uninstall_plugin("lark-base").await.unwrap_err();
        assert!(error.0.contains("Skill 注册表"));
    }
}
