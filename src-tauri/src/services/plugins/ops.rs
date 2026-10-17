//! 插件生命周期操作：列表、市场管理、安装/卸载/升级与 GitHub 预览。

use std::collections::HashMap;
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
    installed_icon_fallback, plugin_icon_from, plugin_store_path, scan_codex_plugin_cache,
    store_contains, store_skills, trusted_plugin_store_path,
};
use super::MANIFEST_RELATIVE_PATH;
use super::{
    MarketplacePlugin, PluginCandidate, PluginMarketplace, PluginPreview, PluginSkill,
    PluginSummary, PluginUpdate,
};

// ==================== AppContext 服务 ====================

impl AppContext {
    /// 插件卡片图标 → data URL。`raw_path` 是本地受信根
    /// （codex/claude 插件缓存与市场物化目录）内的图片文件，或远程目录给出的 https logo URL。
    /// 超重位图（>512KB，实测 ponytail logo 554KB/5422px）现场缩到卡片尺寸并落盘缓存。
    pub async fn plugin_icon_data_url(&self, raw_path: String) -> AppResult<Option<String>> {
        if raw_path.starts_with("https://") {
            return self.remote_icon_data_url(&raw_path).await;
        }
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let codex_home = self.paths.codex_home.clone();
        let claude_plugins = self.paths.claude_home.join("plugins");
        let icon_cache = self.paths.plugin_icons.clone();
        tauri::async_runtime::spawn_blocking(move || {
            read_plugin_icon(&home, &codex_home, &claude_plugins, &icon_cache, &raw_path)
        })
        .await
        .map_err(|error| app_err!("插件图标读取任务失败: {error}"))?
    }

    /// 远程图标：桌面端目录缓存里的 CDN logo（curated-remote 条目本地无物化）。
    /// 只认已知主机、10 秒超时、8MB 上限；下载后走与本地图标同一条瘦身/缓存管线。
    async fn remote_icon_data_url(&self, raw_url: &str) -> AppResult<Option<String>> {
        if !remote_icon_allowed(raw_url) {
            return Err(app_err!("插件图标地址不在允许的来源内"));
        }
        // /content 的图片身份在 id 参数中；签名变化不改变缓存，图片变化必须区分。
        let mut stable = url::Url::parse(raw_url).map_err(|error| app_err!("{error}"))?;
        let image_id = stable
            .query_pairs()
            .find(|(name, _)| name == "id")
            .map(|(_, value)| value.into_owned());
        stable.set_query(None);
        if let Some(image_id) = image_id {
            stable.query_pairs_mut().append_pair("id", &image_id);
        }
        stable.set_fragment(None);
        let key = hash_key(stable.as_str());
        let cache_file = self.paths.plugin_icons.join(format!("{key}.png"));
        if let Ok(cached) = std::fs::read(&cache_file) {
            use base64::Engine as _;
            return Ok(Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(cached)
            )));
        }
        let client = crate::network::Network::current()
            .await
            .map_err(|error| app_err!("{error}"))?
            .builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|error| app_err!("创建网络客户端失败: {error}"))?;
        let response = client
            .get(raw_url)
            .send()
            .await
            .map_err(|error| app_err!("插件图标下载失败：{error}"))?;
        if !response.status().is_success() {
            return Err(app_err!(
                "插件图标下载失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        if response.content_length().unwrap_or(0) as usize > REMOTE_ICON_MAX_BYTES {
            return Err(app_err!(
                "插件图标超过 {}MB 上限",
                REMOTE_ICON_MAX_BYTES / 1024 / 1024
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| app_err!("插件图标下载失败：{error}"))?;
        if bytes.len() > REMOTE_ICON_MAX_BYTES {
            return Err(app_err!(
                "插件图标超过 {}MB 上限",
                REMOTE_ICON_MAX_BYTES / 1024 / 1024
            ));
        }
        let icon_cache = self.paths.plugin_icons.clone();
        let bytes = bytes.to_vec();
        tauri::async_runtime::spawn_blocking(move || {
            use base64::Engine as _;
            let Some(resized) = shrink_icon(&icon_cache, &key, &bytes) else {
                return Ok(None);
            };
            Ok(Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(resized)
            )))
        })
        .await
        .map_err(|error| app_err!("插件图标处理任务失败: {error}"))?
    }

    /// Codex CLI 直接写 config.toml 后，收拢 Agmux 管理的全局段；无变化不落盘。
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
        let codex_home = self.paths.codex_home.clone();
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
            let mut items =
                parse_marketplace_plugins_output(&output, &marketplace, root.as_deref())?;
            // 未物化的远程条目（curated-remote）同样从桌面端目录缓存补 logo URL
            if items.iter().any(|item| item.icon_path.is_none()) {
                let map = remote_logo_urls(&codex_home);
                for item in &mut items {
                    if item.icon_path.is_none() {
                        if let Some(url) = map.get(&item.name) {
                            item.icon_path = Some(url.clone());
                        }
                    }
                }
            }
            Ok(items)
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

/// 图标直传的大小上限；超过则现场瘦身（解码→缩放→重编码）而不是拒绝。
const PLUGIN_ICON_MAX_BYTES: usize = 512 * 1024;
/// 卡片图标实际所需的最大边长（2.125rem 色块 × 高分屏余量）。
const PLUGIN_ICON_MAX_DIMENSION: u32 = 256;
/// 远程图标下载上限与允许的主机（桌面端远程目录缓存里的 CDN）。
const REMOTE_ICON_MAX_BYTES: usize = 8 * 1024 * 1024;
const REMOTE_ICON_HOSTS: [&str; 3] = ["openai.com", "oaistatic.com", "openai.org"];

/// 超重位图瘦身：解码 → 缩到卡片尺寸 → PNG 重编码，按 key 落盘缓存。
/// 缓存键不含签名参数/时间戳，同一 logo 只处理一次。
fn shrink_icon(icon_cache: &Path, key: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let cache_file = icon_cache.join(format!("{key}.png"));
    if let Ok(cached) = std::fs::read(&cache_file) {
        return Some(cached);
    }
    let thumb = image::load_from_memory(bytes)
        .ok()?
        .thumbnail(PLUGIN_ICON_MAX_DIMENSION, PLUGIN_ICON_MAX_DIMENSION);
    let mut out = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    // 缓存写失败不阻塞本次返回；旧缓存文件不自动清理（单个几十 KB）
    let _ = atomic_write(&cache_file, &out);
    Some(out)
}

fn hash_key(parts: impl std::hash::Hash) -> String {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 远程图标地址白名单：只接受 https 且主机属于已知 CDN。
fn remote_icon_allowed(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    REMOTE_ICON_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

#[derive(serde::Deserialize)]
struct RemoteCatalog {
    #[serde(default)]
    plugins: Vec<RemoteCatalogPlugin>,
}

#[derive(serde::Deserialize)]
struct RemoteCatalogPlugin {
    name: String,
    #[serde(default)]
    release: Option<RemoteCatalogRelease>,
}

#[derive(serde::Deserialize)]
struct RemoteCatalogRelease {
    #[serde(default)]
    interface: Option<RemoteCatalogInterface>,
}

#[derive(serde::Deserialize)]
struct RemoteCatalogInterface {
    #[serde(default)]
    logo_url: Option<String>,
}

/// 已装插件名 → 远程 logo URL。数据源是桌面端自己写的远程目录缓存
/// (`~/.codex/cache/remote_plugin_catalog/*.json`，单文件 35MB级)：
/// curated-remote 插件本地没有物化，只有这里带 https 签名 logo URL。
/// 按目录最新 mtime 记忆化，一次会话只解析一次。
fn remote_logo_urls(codex_home: &Path) -> std::sync::Arc<HashMap<String, String>> {
    type Memo = Option<(u64, std::sync::Arc<HashMap<String, String>>)>;
    static MEMO: std::sync::Mutex<Memo> = std::sync::Mutex::new(None);
    let dir = codex_home.join("cache").join("remote_plugin_catalog");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Default::default();
    };
    let files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect();
    let stamp = files
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok()?.modified().ok())
        .filter_map(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .max()
        .unwrap_or(0);
    let mut memo = MEMO.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((cached, map)) = memo.as_ref() {
        if *cached == stamp {
            return map.clone();
        }
    }
    let mut map = HashMap::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(catalog) = serde_json::from_str::<RemoteCatalog>(&text) else {
            continue;
        };
        for plugin in catalog.plugins {
            let Some(url) = plugin
                .release
                .and_then(|release| release.interface)
                .and_then(|interface| interface.logo_url)
            else {
                continue;
            };
            map.entry(plugin.name).or_insert(url);
        }
    }
    let map = std::sync::Arc::new(map);
    *memo = Some((stamp, map.clone()));
    map
}

/// 本地解析不到图标的条目补远程 URL（exa 等 curated-remote 条目本地无物化）。
fn fill_remote_icons(codex_home: &Path, summaries: &mut [PluginSummary]) {
    let missing: Vec<usize> = (0..summaries.len())
        .filter(|index| summaries[*index].icon_path.is_none())
        .collect();
    if missing.is_empty() {
        return;
    }
    let map = remote_logo_urls(codex_home);
    for index in missing {
        if let Some(url) = map.get(&summaries[index].name) {
            summaries[index].icon_path = Some(url.clone());
        }
    }
}

/// 图标读取的白名单根：与 `trusted_plugin_store_path` 同源的受信区，
/// 再补 Claude 插件缓存与市场物化目录（claudeHome/plugins 含 cache 与 marketplaces）。
fn read_plugin_icon(
    home: &Path,
    codex_home: &Path,
    claude_plugins: &Path,
    icon_cache: &Path,
    raw_path: &str,
) -> AppResult<Option<String>> {
    use base64::Engine as _;
    let data_url = |mime: &str, bytes: &[u8]| {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    };
    let path =
        std::fs::canonicalize(raw_path).map_err(|error| app_err!("插件图标文件不可读: {error}"))?;
    let under_root = |root: &Path| {
        std::fs::canonicalize(root)
            .map(|root| path.starts_with(root))
            .unwrap_or(false)
    };
    let trusted = [
        codex_home.join("plugins"),
        // 第三方市场的物化 checkout（ponytail 等实测在 .tmp/marketplaces）
        codex_home.join(".tmp"),
        home.join(".cache/codex-runtimes/plugins"),
        home.join(".cache/codex/plugins"),
        claude_plugins.to_path_buf(),
    ];
    if !trusted.iter().any(|root| under_root(root)) {
        return Err(app_err!("插件图标路径不在受信的插件目录内"));
    }
    let mime = match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => return Ok(None),
    };
    let metadata =
        std::fs::metadata(&path).map_err(|error| app_err!("插件图标文件不可读: {error}"))?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|error| app_err!("插件图标文件不可读: {error}"))?;
    if bytes.len() <= PLUGIN_ICON_MAX_BYTES {
        return Ok(Some(data_url(mime, &bytes)));
    }
    // 超重图标：不是拒绝，而是现场瘦身——解码缩到卡片实际所需尺寸后重编码，
    // 处理结果按（路径+大小+mtime）落盘缓存，第二次起直接读小文件。
    // SVG 无法廉价缩身且极少超限，维持放弃。解码失败（损坏/伪装扩展名）回落字母。
    if mime == "image/svg+xml" {
        return Ok(None);
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let Some(resized) = shrink_icon(
        icon_cache,
        &hash_key((&path, bytes.len(), modified)),
        &bytes,
    ) else {
        return Ok(None);
    };
    Ok(Some(data_url("image/png", &resized)))
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
            // 图标：manifest logo → 约定名 → 物化目录回退链（git URL/连接器 ID 报告的路径找不到真身时）
            let icon_path = plugin_icon_from(manifest.as_ref(), &plugin_path)
                .or_else(|| installed_icon_fallback(codex_home, &marketplace, &name))
                .map(|path| path.display().to_string());
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
                icon_path,
                name,
            });
        }
    } else {
        summaries.extend(scan_codex_plugin_cache(codex_home));
    }
    enrich_plugin_sources(&mut summaries, &sources);
    // curated-remote 等未物化条目：本地解析不到图标时补远程目录里的 CDN logo
    fill_remote_icons(codex_home, &mut summaries);
    summaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::codex_cli::file_name;
    use crate::services::plugins::test_context;

    #[tokio::test]
    async fn remote_icon_cache_keeps_image_ids_and_ignores_signatures() {
        let (_home, context) = test_context();
        let first = "https://files.openai.com/content?id=fixture-one";
        let second = "https://files.openai.com/content?id=fixture-two";
        for (url, bytes) in [
            ("https://files.openai.com/content", b"old".as_slice()),
            (first, b"first".as_slice()),
            (second, b"second".as_slice()),
        ] {
            std::fs::write(
                context
                    .paths
                    .plugin_icons
                    .join(format!("{}.png", hash_key(url))),
                bytes,
            )
            .unwrap();
        }
        let read = |url: &str| format!("{url}&sig=fixture");
        let first_icon = context.remote_icon_data_url(&read(first)).await.unwrap();
        let second_icon = context.remote_icon_data_url(&read(second)).await.unwrap();
        assert_ne!(first_icon, second_icon);
        assert_eq!(
            first_icon,
            context
                .remote_icon_data_url(&format!("{first}&sig=rotated#fragment"))
                .await
                .unwrap()
        );
    }

    #[test]
    fn remote_logo_urls_map_plugin_names_from_desktop_catalog() {
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        let dir = codex_home.join("cache/remote_plugin_catalog");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("fixture.json"),
            r#"{"plugins":[
                {"name":"app-fixture","release":{"interface":{"logo_url":"https://files.openai.com/content?id=file_fixture&sig=abc"}}},
                {"name":"app-no-logo","release":{"interface":{}}}
            ]}"#,
        )
        .unwrap();
        let map = remote_logo_urls(&codex_home);
        assert_eq!(
            map.get("app-fixture").map(String::as_str),
            Some("https://files.openai.com/content?id=file_fixture&sig=abc")
        );
        assert!(!map.contains_key("app-no-logo"));
    }

    #[test]
    fn remote_icon_hosts_are_allowlisted() {
        assert!(remote_icon_allowed("https://files.openai.com/content?id=x"));
        assert!(remote_icon_allowed("https://cdn.oaistatic.com/a.png"));
        // 非白名单主机、http、私有协议一律拒绝
        assert!(!remote_icon_allowed("https://evil.example.com/a.png"));
        assert!(!remote_icon_allowed("http://files.openai.com/a.png"));
        assert!(!remote_icon_allowed("connectors://app_x/logo"));
        assert!(!remote_icon_allowed("file:///etc/passwd"));
    }

    #[test]
    fn plugin_icon_read_whitelists_roots_and_extensions() {
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        let claude_plugins = home.path().join(".claude/plugins");
        let icon_cache = home.path().join("plugin-icons");
        std::fs::create_dir_all(&icon_cache).unwrap();
        let icon = codex_home.join("plugins/cache/market/sample/1.0.0/assets/icon.png");
        std::fs::create_dir_all(icon.parent().unwrap()).unwrap();
        std::fs::write(&icon, b"not-really-png").unwrap();
        // 受信根内：返回 data URL
        let data_url = read_plugin_icon(
            home.path(),
            &codex_home,
            &claude_plugins,
            &icon_cache,
            &icon.display().to_string(),
        )
        .unwrap()
        .expect("受信根内的 png 应产出 data URL");
        assert!(data_url.starts_with("data:image/png;base64,"));
        // 受信根外：拒绝（不能当任意文件读取通道）
        let outside = home.path().join("secret.png");
        std::fs::write(&outside, b"x").unwrap();
        assert!(read_plugin_icon(
            home.path(),
            &codex_home,
            &claude_plugins,
            &icon_cache,
            &outside.display().to_string()
        )
        .is_err());
        // 受信根内但非图片扩展：返回 None 而非报错
        let text = codex_home.join("plugins/cache/market/sample/1.0.0/README.md");
        std::fs::write(&text, b"readme").unwrap();
        assert_eq!(
            read_plugin_icon(
                home.path(),
                &codex_home,
                &claude_plugins,
                &icon_cache,
                &text.display().to_string()
            )
            .unwrap(),
            None
        );
    }

    /// 超重位图（实测 ponytail logo 554KB/5422px）不拒绝：现场缩到卡片尺寸重编码，
    /// 结果落盘缓存，第二次调用直接命中缓存。
    #[test]
    fn oversized_raster_icon_is_resized_and_cached() {
        use base64::Engine as _;
        let home = tempfile::tempdir().unwrap();
        let codex_home = home.path().join(".codex");
        let claude_plugins = home.path().join(".claude/plugins");
        let icon_cache = home.path().join("plugin-icons");
        std::fs::create_dir_all(&icon_cache).unwrap();
        // 1500×1500 随机噪声位图：PNG 压不动，确保远超 512KB 直传上限
        let mut state = 0x2545_F491_u32;
        let bitmap = image::RgbaImage::from_fn(1500, 1500, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            image::Rgba([state as u8, (state >> 8) as u8, (state >> 16) as u8, 255])
        });
        let icon = codex_home.join("plugins/cache/market/sample/1.0.0/assets/icon.png");
        std::fs::create_dir_all(icon.parent().unwrap()).unwrap();
        image::DynamicImage::ImageRgba8(bitmap).save(&icon).unwrap();
        assert!(
            std::fs::metadata(&icon).unwrap().len() as usize > PLUGIN_ICON_MAX_BYTES,
            "fixture 必须超过直传上限才有意义"
        );

        let first = read_plugin_icon(
            home.path(),
            &codex_home,
            &claude_plugins,
            &icon_cache,
            &icon.display().to_string(),
        )
        .unwrap()
        .expect("超重位图应缩身后返回");
        let payload = first.strip_prefix("data:image/png;base64,").unwrap();
        let decoded = image::load_from_memory(
            &base64::engine::general_purpose::STANDARD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        assert!(decoded.width().max(decoded.height()) <= PLUGIN_ICON_MAX_DIMENSION);
        // 处理结果已落盘：缓存目录出现一个 png，第二次调用返回同一份内容
        assert_eq!(std::fs::read_dir(&icon_cache).unwrap().count(), 1);
        let second = read_plugin_icon(
            home.path(),
            &codex_home,
            &claude_plugins,
            &icon_cache,
            &icon.display().to_string(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(first, second);
    }

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
