//! 插件生命周期操作：列表、市场管理、安装/卸载/升级。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::codex::config as codex_config;
use crate::error::{app_err, AppResult};
use crate::fsutil::{atomic_write, backup_file};
use crate::paths::now_ms;
use crate::services::AppContext;

use super::catalog::{
    enrich_marketplace_metadata, enrich_plugin_sources, find_plugin_updates, manifest_description,
    marketplace_sources, parse_marketplace_identities, parse_marketplace_list_output,
    parse_marketplace_name, parse_marketplace_plugins_output, parse_marketplace_source,
    plugin_source_label, read_manifest, resolve_install_marketplace, sort_marketplaces,
    validate_plugin_name,
};
use super::cli::run_codex_plugin;
use super::skills::is_registered_skill;
use super::store::{
    installed_icon_fallback, plugin_icon_from, plugin_store_path, store_skills,
    trusted_plugin_store_path,
};
use super::{MarketplacePlugin, PluginMarketplace, PluginSkill, PluginSummary, PluginUpdate};

// ==================== AppContext 服务 ====================

impl AppContext {
    pub async fn list_plugin_catalog(&self) -> AppResult<Vec<MarketplacePlugin>> {
        let home = self
            .paths
            .codex_home
            .parent()
            .ok_or_else(|| app_err!("Codex 配置目录无父目录"))?
            .to_path_buf();
        tauri::async_runtime::spawn_blocking(move || super::appserver::plugin_catalog(&home))
            .await
            .map_err(|error| app_err!("插件目录任务失败: {error}"))?
    }
    pub async fn list_codex_apps(&self) -> AppResult<Vec<super::CodexAppSummary>> {
        let home = self
            .paths
            .codex_home
            .parent()
            .ok_or_else(|| app_err!("Codex 配置目录无父目录"))?
            .to_path_buf();
        tauri::async_runtime::spawn_blocking(move || super::appserver::installed_apps(&home))
            .await
            .map_err(|error| app_err!("应用列表任务失败: {error}"))?
    }
    /// 用户级插件开关由 live 保管，切供应商时沿用既有全局段保留逻辑。
    pub async fn set_plugin_enabled(
        &self,
        plugin_id: &str,
        enabled: bool,
    ) -> AppResult<Vec<PluginSummary>> {
        let (name, marketplace) = plugin_id
            .split_once('@')
            .ok_or_else(|| app_err!("插件标识必须为「插件名@市场名」"))?;
        validate_plugin_name(name)?;
        validate_plugin_name(marketplace)?;
        let plugins = self.list_plugins().await?;
        let plugin = plugins
            .iter()
            .find(|item| item.name == name && item.marketplace.as_deref() == Some(marketplace))
            .ok_or_else(|| app_err!("没有找到插件「{plugin_id}」"))?;
        if !plugin.can_toggle {
            return Err(app_err!(
                "「{plugin_id}」的启停由 Codex 管理，无法通过本地插件配置修改"
            ));
        }
        {
            let _guard = self
                .operation
                .lock()
                .map_err(|_| app_err!("操作锁已损坏"))?;
            let mut document = codex_config::parse_document(&self.read_live_config()?)?;
            let plugins = document
                .entry("plugins")
                .or_insert(toml_edit::table())
                .as_table_like_mut()
                .ok_or_else(|| app_err!("plugins 必须为配置表"))?;
            let plugin = plugins
                .entry(plugin_id)
                .or_insert(toml_edit::table())
                .as_table_like_mut()
                .ok_or_else(|| app_err!("插件配置必须为配置表"))?;
            plugin.insert("enabled", toml_edit::value(enabled));
            let updated = codex_config::normalize_global_section_order(&document.to_string());
            let path = self.paths.codex_config();
            backup_file(&path, &self.paths.config_backup, "config")?;
            atomic_write(&path, updated.as_bytes())?;
        }
        let plugins = self.list_plugins().await?;
        if !plugins.iter().any(|item| {
            item.name == name
                && item.marketplace.as_deref() == Some(marketplace)
                && item.enabled == enabled
        }) {
            return Err(app_err!(
                "插件配置已保存，但 Codex 返回的启用状态未改变，请检查项目或管理员设置"
            ));
        }
        tauri_plugin_log::log::info!(
            "[plugin.toggle] client=\"Codex\" plugin={plugin_id:?} enabled={enabled} outcome=success msg=\"Codex 插件用户级开关已保存\""
        );
        Ok(plugins)
    }

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

    /// 只读取 Codex 确认的安装与启停状态，失败交给上层保留上一份快照。
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
        if requested_store_path == Some("") {
            return Ok(Vec::new());
        }
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
    ) -> AppResult<Vec<PluginSummary>> {
        validate_plugin_name(marketplace)?;
        validate_plugin_name(name)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        let marketplace = marketplace.to_string();
        let name = name.to_string();
        let target = tauri::async_runtime::spawn_blocking({
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
                output.map(|_| target)
            }
        })
        .await
        .map_err(|error| app_err!("安装插件任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;
        let plugins = self.list_plugins().await?;
        if !plugins.iter().any(|plugin| {
            plugin.name == name && plugin.marketplace.as_deref() == Some(target.as_str())
        }) {
            return Err(app_err!(
                "插件「{name}@{target}」已执行安装，但列表中尚未出现"
            ));
        }
        tauri_plugin_log::log::info!(
            "[plugin.install] plugin={name:?} target={target:?} outcome=success msg=\"Codex 插件已安装\""
        );

        let _ = self.database.record_event(
            None,
            "plugin",
            "install",
            Some(&format!("{name}@{marketplace}")),
            &now_ms().to_string(),
        );

        Ok(plugins)
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

    /// 通过官方 CLI 刷新单个外部市场，不升级其中已安装的插件。
    pub async fn refresh_plugin_marketplace(&self, name: &str) -> AppResult<()> {
        validate_plugin_name(name)?;
        if !self
            .list_plugin_marketplaces()
            .await?
            .iter()
            .any(|item| item.name == name && item.kind == "third-party")
        {
            return Err(app_err!("只能刷新已添加的外部插件市场"));
        }
        let home = self
            .paths
            .codex_home
            .parent()
            .ok_or_else(|| app_err!("无法定位用户主目录"))?
            .to_path_buf();
        let name = name.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            run_codex_plugin(&home, &["marketplace", "upgrade", &name]).map(|_| ())
        })
        .await
        .map_err(|error| app_err!("刷新插件市场任务失败: {error}"))?
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

    /// 本地与远程插件均由 Codex CLI 卸载，并核验安装记录。
    pub async fn uninstall_plugin(
        &self,
        name: &str,
        marketplace: &str,
    ) -> AppResult<Vec<PluginSummary>> {
        validate_plugin_name(name)?;
        validate_plugin_name(marketplace)?;
        let Some(home) = self.paths.codex_home.parent().map(Path::to_path_buf) else {
            return Err(app_err!("无法定位用户主目录"));
        };
        if is_registered_skill(&home, name) {
            return Err(app_err!(
                "「{name}」属于 Codex Skill 注册表，请在 Codex 内管理它"
            ));
        }
        let plugins = self.list_plugins().await?;
        let Some(plugin) = plugins
            .iter()
            .find(|item| item.name == name && item.marketplace.as_deref() == Some(marketplace))
        else {
            return Err(app_err!("没有找到名为「{name}」的插件"));
        };
        if !plugin.can_uninstall {
            return Err(app_err!("「{name}」不支持在这里卸载，请在 Codex 内管理它"));
        }
        let selector = format!("{name}@{marketplace}");
        tauri::async_runtime::spawn_blocking({
            let home = home.clone();
            let selector = selector.clone();
            move || run_codex_plugin(&home, &["remove", &selector])
        })
        .await
        .map_err(|error| app_err!("卸载任务失败: {error}"))??;
        self.normalize_plugin_config_order()?;
        let plugins = self.list_plugins().await?;
        if plugins
            .iter()
            .any(|item| item.name == name && item.marketplace.as_deref() == Some(marketplace))
        {
            return Err(app_err!(
                "Codex 执行卸载后仍返回插件「{selector}」，请在 Codex 中确认状态后刷新"
            ));
        }
        tauri_plugin_log::log::info!(
            "[plugin.uninstall] selector={selector:?} outcome=success msg=\"Codex 插件已卸载\""
        );
        let _ = self.database.record_event(
            None,
            "plugin",
            "uninstall",
            Some(name),
            &now_ms().to_string(),
        );
        Ok(plugins)
    }
}

/// 图标直传的大小上限；超过则现场瘦身（解码→缩放→重编码）而不是拒绝。
const PLUGIN_ICON_MAX_BYTES: usize = 512 * 1024;
/// 卡片图标实际所需的最大边长（2.125rem 色块 × 高分屏余量）。
const PLUGIN_ICON_MAX_DIMENSION: u32 = 256;
/// 远程图标下载上限与允许的主机（桌面端远程目录缓存里的 CDN）。
const REMOTE_ICON_MAX_BYTES: usize = 8 * 1024 * 1024;
const REMOTE_ICON_HOSTS: [&str; 4] = ["openai.com", "oaistatic.com", "openai.org", "chatgpt.com"];

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

    let listing = super::appserver::installed_plugins(home)?;
    let has_remote = listing.marketplaces.iter().any(|market| {
        market.name == "openai-curated-remote"
            || market
                .plugins
                .iter()
                .any(|plugin| plugin.id.ends_with("@openai-curated-remote"))
    });
    let mut seen = std::collections::HashSet::new();
    for entry in listing
        .marketplaces
        .into_iter()
        .flat_map(|market| market.plugins)
    {
        let (_, marketplace) = entry
            .id
            .rsplit_once('@')
            .ok_or_else(|| app_err!("Codex 返回无效插件标识"))?;
        let marketplace = marketplace.to_owned();
        if !seen.insert(entry.id.clone()) {
            continue;
        }
        let desktop_visible = super::appserver::desktop_plugin(&entry, &marketplace)
            && !(marketplace == "openai-curated" && has_remote);
        let interface = entry.interface;
        let name = entry.name;
        validate_plugin_name(&name)?;
        validate_plugin_name(&marketplace)?;
        let source_type = entry.source.get("type").and_then(|value| value.as_str());
        let local = matches!(source_type, Some("local" | "git" | "git-subdir" | "npm"));
        let remote = source_type == Some("remote");
        let source = plugin_source_label(Some(&entry.source)).unwrap_or_default();
        let version = entry.local_version.or(entry.version);
        let origin = if marketplace.starts_with("openai") {
            "official"
        } else {
            "codex"
        };
        let plugin_path =
            plugin_store_path(codex_home, &marketplace, &name, version.as_deref(), &source);
        let manifest = read_manifest(&plugin_path);
        // 图标：manifest logo → 约定名 → 物化目录回退链（git URL/连接器 ID 报告的路径找不到真身时）
        let icon_path = plugin_icon_from(manifest.as_ref(), &plugin_path)
            .or_else(|| installed_icon_fallback(codex_home, &marketplace, &name))
            .map(|path| path.display().to_string());
        summaries.push(PluginSummary {
            version: version.or_else(|| manifest.as_ref().and_then(|item| item.version.clone())),
            display_name: manifest
                .as_ref()
                .and_then(|item| item.interface.as_ref())
                .and_then(|item| item.display_name.clone()),
            description: manifest.as_ref().and_then(manifest_description),
            enabled: entry.enabled,
            desktop_visible,
            can_toggle: local,
            // 可卸载=策略允许且来源物化可清；只排除系统市场（bundled/runtime）。
            // 不能用 origin 代理：openai-api-curated 官方出身但用户自装，必须可卸。
            can_uninstall: entry.install_policy == "AVAILABLE"
                && (remote || local)
                && !super::appserver::system_marketplace(&marketplace),
            origin: origin.to_string(),
            marketplace: Some(marketplace),
            store_path: if plugin_path.is_dir() {
                plugin_path.display().to_string()
            } else {
                String::new()
            },
            source_url: None,
            icon_path,
            name,
        });
        super::appserver::apply_interface(summaries.last_mut().unwrap(), interface.as_ref());
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

    fn install_cli_fixture(home: &Path, listing: serde_json::Value) {
        let root = home.join(".codex");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(root.join("listing.json"), listing.to_string()).unwrap();
        let source = home.join("plugin_cli_fixture.rs");
        std::fs::write(&source, r#"fn main() {
            use std::io::{BufRead, Write};
            let root = std::path::PathBuf::from(std::env::var_os("CODEX_HOME").unwrap());
            let args: Vec<String> = std::env::args().collect();
            if args[1] == "app-server" {
                for line in std::io::stdin().lock().lines() {
                    let line = line.unwrap();
                    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(root.join("requests.jsonl")).unwrap();
                    writeln!(log, "{line}").unwrap();
                    let Some(value) = line.split("\"id\":").nth(1) else { continue };
                    let id = value.split([',', '}']).next().unwrap().trim();
                    if root.join("rpc-error").exists() && !line.contains("\"initialize\"") {
                        println!("{{\"id\":{id},\"error\":{{\"code\":-32601,\"message\":\"private configuration\"}}}}");
                        std::io::stdout().flush().unwrap();
                        continue;
                    }
                    let result = if line.contains("\"initialize\"") { "{}".to_owned() }
                    else if line.contains("\"plugin/list\"") {
                        std::fs::read_to_string(root.join("catalog.json")).unwrap()
                    }
                    else if line.contains("\"plugin/installed\"") {
                        let mut listing = std::fs::read_to_string(root.join("listing.json")).unwrap();
                        let config = std::fs::read_to_string(root.join("config.toml")).unwrap_or_default();
                        if !listing.contains("\"source\":\"remote\"") && config.contains("enabled = false") {
                            listing = listing.replace("\"enabled\":true", "\"enabled\":false");
                        }
                        if let (Some(start), Some(end)) = (listing.find('['), listing.rfind(']')) {
                            let plugins = listing[start..=end].replace("\"pluginId\":", "\"id\":").replace("\"source\":\"", "\"type\":\"");
                            format!("{{\"marketplaces\":[{{\"name\":\"fixture\",\"plugins\":{plugins}}}]}}")
                        } else { "{}".to_owned() }
                    } else if line.contains("\"app/installed\"") {
                        std::fs::read_to_string(root.join("apps.json")).unwrap()
                    } else {
                        std::fs::read_to_string(root.join(format!("app-metadata-{id}.json")))
                            .or_else(|_| std::fs::read_to_string(root.join("app-metadata.json"))).unwrap()
                    };
                    println!("{{\"method\":\"fixture/notification\"}}");
                    println!("{{\"id\":{id},\"result\":{result}}}");
                    std::io::stdout().flush().unwrap();
                }
                return;
            }
            if args[2] == "remove" || args[2] == "add" {
                let (file, selector) = if args[2] == "remove" {
                    ("removed-selector", args[3].clone())
                } else {
                    ("added-selector", format!("{}@{}", args[3], args[5]))
                };
                std::fs::write(root.join(file), selector).unwrap();
                if root.join("after.json").exists() {
                    std::fs::copy(root.join("after.json"), root.join("listing.json")).unwrap();
                }
            }
            if args[2] == "list" {
                if !args.iter().any(|arg| arg == "--json") {
                    println!("Marketplace `openai-curated-remote`");
                    return;
                }
                let mut listing = std::fs::read_to_string(root.join("listing.json")).unwrap();
                let config = std::fs::read_to_string(root.join("config.toml")).unwrap_or_default();
                if !listing.contains("\"source\":\"remote\"") && config.contains("enabled = false") {
                    listing = listing.replace("\"enabled\":true", "\"enabled\":false");
                }
                println!("{listing}");
            }
        }"#).unwrap();
        let output = std::process::Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(bin.join(file_name()))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn installed_inventory_separates_runtime_implicit_and_mirror_entries() {
        let (home, context) = test_context();
        let entry = |name: &str, market: &str| {
            serde_json::json!({
                "pluginId": format!("{name}@{market}"), "name": name, "enabled": false,
                "source": {"source":"remote"}, "installPolicy":"AVAILABLE"
            })
        };
        let user = entry("sample", "openai-curated-remote");
        let mut implicit = entry("implicit", "openai-curated-remote");
        implicit["installPolicySource"] = "IMPLICIT_CANONICAL_APP".into();
        let mut admin = entry("admin", "team");
        admin["installPolicy"] = "INSTALLED_BY_DEFAULT".into();
        install_cli_fixture(
            home.path(),
            serde_json::json!({"installed":[
                user.clone(), user, implicit, admin, entry("runtime", "openai-primary-runtime"),
                entry("builtin", "openai-bundled"), entry("mirror-only", "openai-curated"),
                entry("codex-app-tools", "custom")
            ]}),
        );
        let plugins = context.list_plugins().await.unwrap();
        assert_eq!(plugins.len(), 7);
        let visible: Vec<_> = plugins
            .iter()
            .filter(|plugin| plugin.desktop_visible)
            .collect();
        assert_eq!(
            visible
                .iter()
                .map(|plugin| plugin.name.as_str())
                .collect::<Vec<_>>(),
            ["admin", "codex-app-tools", "sample"]
        );
        assert!(visible.iter().all(|plugin| !plugin.enabled));
        assert!(
            !visible
                .iter()
                .find(|plugin| plugin.name == "admin")
                .unwrap()
                .can_uninstall
        );
    }

    #[tokio::test]
    async fn catalog_uses_remote_identity_permissions_and_reports_incomplete_results() {
        let (home, context) = test_context();
        install_cli_fixture(home.path(), serde_json::json!({"installed":[]}));
        let entry = |name: &str, market: &str| {
            serde_json::json!({
                "id":format!("{name}@{market}"), "name":name, "enabled":true, "installed":false,
                "installPolicy":"AVAILABLE", "availability":"AVAILABLE", "authPolicy":"ON_USE",
                "source":{"type":"remote"}, "interface":{"displayName":"Sample","shortDescription":"Description","logoUrl":"https://example.test/icon.png"}
            })
        };
        let market = "openai-curated-remote";
        let mut sample = entry("sample", market);
        sample["remotePluginId"] = "remote-one".into();
        let mut duplicate = entry("duplicate", market);
        duplicate["shareContext"] = serde_json::json!({"remotePluginId":"remote-one"});
        duplicate["installed"] = true.into();
        let mut installed = entry("installed", market);
        installed["installed"] = true.into();
        let mut denied = entry("denied", market);
        denied["availability"] = "DISABLED_BY_ADMIN".into();
        let mut unavailable = entry("unavailable", market);
        unavailable["installPolicy"] = "NOT_AVAILABLE".into();
        let mut implicit = entry("implicit", market);
        implicit["installPolicySource"] = "IMPLICIT_CANONICAL_APP".into();
        let mut catalog = serde_json::json!({"marketplaceLoadErrors":[],"marketplaces":[
            {"name":"openai-bundled","plugins":[entry("runtime","openai-bundled")]},
            {"name":"openai-curated","plugins":[entry("mirror-only","openai-curated")]},
            {"name":market,"plugins":[sample,duplicate,installed,denied,unavailable,implicit]},
            {"name":"custom","plugins":[entry("custom","custom")]}
        ]});
        let path = context.paths.codex_home.join("catalog.json");
        std::fs::write(&path, catalog.to_string()).unwrap();
        let result = context.list_plugin_catalog().await.unwrap();
        assert_eq!(
            result
                .iter()
                .map(|plugin| plugin.plugin_id.as_str())
                .collect::<Vec<_>>(),
            [
                "sample@openai-curated-remote",
                "installed@openai-curated-remote",
                "custom@custom"
            ]
        );
        assert_eq!(result[0].display_name.as_deref(), Some("Sample"));
        assert_eq!(result[0].description.as_deref(), Some("Description"));
        assert!(result[0].installed);
        assert!(result[1].installed);
        catalog["marketplaceLoadErrors"] =
            serde_json::json!([{"marketplace":"fixture","message":"private error"}]);
        std::fs::write(&path, catalog.to_string()).unwrap();
        let error = context.list_plugin_catalog().await.unwrap_err().0;
        assert!(error.contains("fixture")); // 指名失败市场，用户才知道该移除哪个
        assert!(!error.contains("private")); // 失败详情原文不进文案
        catalog["marketplaceLoadErrors"] = serde_json::json!([]);
        catalog["marketplaces"][0]["plugins"][0]
            .as_object_mut()
            .unwrap()
            .remove("availability");
        std::fs::write(&path, catalog.to_string()).unwrap();
        assert!(context
            .list_plugin_catalog()
            .await
            .unwrap_err()
            .0
            .contains("缺少可安装状态"));
    }

    #[tokio::test]
    async fn installed_apps_read_all_metadata_chunks_and_reject_partial_or_unsupported_results() {
        let (home, context) = test_context();
        install_cli_fixture(home.path(), serde_json::json!({"installed":[]}));
        let root = &context.paths.codex_home;
        let apps: Vec<_> = (0..101)
            .map(|index| {
                serde_json::json!({
                    "id":format!("app-{index}"), "enabled": index != 0, "callable": index != 1
                })
            })
            .collect();
        let metadata: Vec<_> = (0..101)
            .map(|index| {
                serde_json::json!({
                    "id":format!("app-{index}"), "name":format!("App {index}"),
                    "description":"fixture", "pluginDisplayNames":["Sample"]
                })
            })
            .collect();
        std::fs::write(
            root.join("apps.json"),
            serde_json::json!({"apps":apps}).to_string(),
        )
        .unwrap();
        for (index, chunk) in metadata.chunks(100).enumerate() {
            std::fs::write(
                root.join(format!("app-metadata-{}.json", index + 3)),
                serde_json::json!({"apps":chunk,"missingAppIds":[]}).to_string(),
            )
            .unwrap();
        }
        let result = context.list_codex_apps().await.unwrap();
        assert_eq!(result.len(), 100);
        assert_eq!(result[0].name, "App 0");
        assert!(!result[0].enabled);
        assert_eq!(result.last().unwrap().id, "app-100");
        let requests: Vec<serde_json::Value> = std::fs::read_to_string(root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let chunks: Vec<_> = requests
            .iter()
            .filter(|request| request["method"] == "app/read")
            .map(|request| request["params"]["appIds"].as_array().unwrap().len())
            .collect();
        assert_eq!(chunks, [100, 1]);
        std::fs::write(
            root.join("app-metadata-3.json"),
            r#"{"apps":[],"missingAppIds":["app-0"]}"#,
        )
        .unwrap();
        assert!(context
            .list_codex_apps()
            .await
            .unwrap_err()
            .0
            .contains("未返回部分"));
        std::fs::write(root.join("rpc-error"), "").unwrap();
        let error = context.list_codex_apps().await.unwrap_err().0;
        assert!(error.contains("-32601"));
        assert!(!error.contains("private configuration"));
    }

    #[tokio::test]
    async fn plugin_list_keeps_cli_disabled_state_when_local_config_says_enabled() {
        let (home, context) = test_context();
        install_cli_fixture(
            home.path(),
            serde_json::json!({"installed": [
                {"pluginId":"sample@openai-curated-remote", "name":"sample", "marketplaceName":"openai-curated-remote", "enabled":false, "installPolicy":"AVAILABLE", "source":{"source":"remote", "id":"plugin_fixture"}},
                {"pluginId":"managed@team", "name":"managed", "marketplaceName":"team", "enabled":true, "installPolicy":"INSTALLED_BY_DEFAULT", "source":{"source":"remote", "id":"plugin_managed"}},
                {"pluginId":"unknown@market", "name":"unknown", "marketplaceName":"market", "enabled":true, "installPolicy":"AVAILABLE"},
                {"pluginId":"notion@openai-api-curated", "name":"notion", "marketplaceName":"openai-api-curated", "enabled":true, "installPolicy":"AVAILABLE", "source":{"source":"local", "path":"C:\\tmp\\notion"}},
                {"pluginId":"browser@openai-bundled", "name":"browser", "marketplaceName":"openai-bundled", "enabled":true, "installPolicy":"AVAILABLE", "source":{"source":"local", "path":"C:\\tmp\\browser"}}
            ]}),
        );
        let config = "[plugins.\"sample@openai-curated-remote\"]\nenabled = true\n";
        std::fs::write(context.paths.codex_config(), config).unwrap();
        let plugins = context.list_plugins().await.unwrap();
        assert_eq!(plugins.len(), 5);
        let sample = plugins
            .iter()
            .find(|plugin| plugin.name == "sample")
            .unwrap();
        assert!(!sample.enabled);
        assert!(!sample.can_toggle);
        assert!(sample.can_uninstall);
        // api-curated 是官方市场但用户自装的本地物化，必须可卸载；系统市场（bundled/runtime）才不可卸。
        let notion = plugins
            .iter()
            .find(|plugin| plugin.name == "notion")
            .unwrap();
        assert!(notion.can_toggle);
        assert!(notion.can_uninstall);
        let browser = plugins
            .iter()
            .find(|plugin| plugin.name == "browser")
            .unwrap();
        assert!(browser.can_toggle);
        assert!(!browser.can_uninstall);
        assert!(plugins
            .iter()
            .filter(|plugin| plugin.name == "managed" || plugin.name == "unknown")
            .all(|plugin| !plugin.can_toggle && !plugin.can_uninstall));
        assert!(context
            .set_plugin_enabled("sample@openai-curated-remote", true)
            .await
            .is_err());
        assert!(context.uninstall_plugin("managed", "team").await.is_err());
        assert!(!context.paths.codex_home.join("removed-selector").exists());
        assert_eq!(
            std::fs::read_to_string(context.paths.codex_config()).unwrap(),
            config
        );
        for invalid in [
            "{}",
            r#"{"installed":[{"pluginId":"sample@market","name":"sample","marketplaceName":"market"}]}"#,
        ] {
            std::fs::write(context.paths.codex_home.join("listing.json"), invalid).unwrap();
            assert!(context.list_plugins().await.is_err());
        }
    }

    #[tokio::test]
    async fn plugin_toggle_preserves_other_settings_and_rejects_invalid_documents() {
        let (home, context) = test_context();
        install_cli_fixture(
            home.path(),
            serde_json::json!({"installed": [
                {"pluginId":"sample@market", "name":"sample", "marketplaceName":"market", "enabled":true, "installPolicy":"AVAILABLE", "source":{"source":"local", "path":"/fixture"}}
            ]}),
        );
        let path = context.paths.codex_config();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = concat!(
            "model = 'fixture'\n",
            "[plugins.\"sample@market\"]\nenabled = true\ncustom = 'keep'\n",
            "[plugins.\"other@market\"]\nenabled = true\n",
            "[mcp_servers.sample]\nurl = 'https://example.test/mcp'\n"
        );
        std::fs::write(&path, original).unwrap();
        for enabled in [false, true] {
            context
                .set_plugin_enabled("sample@market", enabled)
                .await
                .unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let document = codex_config::parse_document(&text).unwrap();
            assert_eq!(
                document["plugins"]["sample@market"]["enabled"].as_bool(),
                Some(enabled)
            );
            assert_eq!(
                document["plugins"]["sample@market"]["custom"].as_str(),
                Some("keep")
            );
            assert_eq!(
                document["plugins"]["other@market"]["enabled"].as_bool(),
                Some(true)
            );
            assert_eq!(document["model"].as_str(), Some("fixture"));
            assert_eq!(
                document["mcp_servers"]["sample"]["url"].as_str(),
                Some("https://example.test/mcp")
            );
        }
        assert!(std::fs::read_dir(&context.paths.config_backup)
            .unwrap()
            .next()
            .is_some());
        for invalid in [
            "plugins = false\n",
            "[plugins]\n\"sample@market\" = false\n",
            "model = ",
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(context
                .set_plugin_enabled("sample@market", false)
                .await
                .is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
        }
        for invalid in [
            "sample",
            "@market",
            "sample@",
            "sample@../market",
            "sample@a@b",
        ] {
            assert!(context.set_plugin_enabled(invalid, true).await.is_err());
        }
        std::fs::write(&path, "model = 'fixture'\n").unwrap();
        let actual = context
            .set_plugin_enabled("sample@market", false)
            .await
            .unwrap();
        assert!(!actual[0].enabled);
        let document =
            codex_config::parse_document(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            document["plugins"]["sample@market"]["enabled"].as_bool(),
            Some(false)
        );
        std::fs::write(context.paths.codex_home.join("listing.json"), r#"{"installed":[{"pluginId":"sample@market","name":"sample","marketplaceName":"market","enabled":false,"source":{"source":"local"}}]}"#).unwrap();
        let error = context
            .set_plugin_enabled("sample@market", true)
            .await
            .unwrap_err();
        assert!(error.0.contains("启用状态未改变"));
    }

    #[tokio::test]
    async fn remote_install_and_uninstall_use_exact_marketplace_and_check_result() {
        let (home, context) = test_context();
        let local = serde_json::json!({"pluginId":"sample@market", "name":"sample", "marketplaceName":"market", "enabled":true, "installPolicy":"AVAILABLE", "source":{"source":"git", "url":"https://example.test/plugins.git"}});
        install_cli_fixture(
            home.path(),
            serde_json::json!({"installed": [local.clone()]}),
        );
        let root = &context.paths.codex_home;
        let error = context
            .install_marketplace_plugin("openai-curated", "sample")
            .await
            .unwrap_err();
        assert!(error.0.contains("尚未出现"));
        std::fs::write(root.join("after.json"), serde_json::json!({"installed": [local.clone(),
            {"pluginId":"sample@openai-curated-remote", "name":"sample", "marketplaceName":"openai-curated-remote", "enabled":false, "installPolicy":"AVAILABLE", "source":{"source":"remote", "id":"plugin_fixture"}}
        ]}).to_string()).unwrap();
        let installed = context
            .install_marketplace_plugin("openai-curated", "sample")
            .await
            .unwrap();
        assert_eq!(installed.len(), 2);
        let skills_dir = root.join("plugins/cache/market/sample/1.0.0/skills/example");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: example\ndescription: Fixture\n---\nFixture",
        )
        .unwrap();
        assert_eq!(
            context
                .list_plugin_skills("sample", None)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(context
            .list_plugin_skills("sample", Some(""))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("added-selector")).unwrap(),
            "sample@openai-curated-remote"
        );
        std::fs::write(
            root.join("after.json"),
            serde_json::json!({"installed":[local]}).to_string(),
        )
        .unwrap();
        let remaining = context
            .uninstall_plugin("sample", "openai-curated-remote")
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].marketplace.as_deref(), Some("market"));
        assert_eq!(
            std::fs::read_to_string(root.join("removed-selector")).unwrap(),
            "sample@openai-curated-remote"
        );
        std::fs::remove_file(root.join("after.json")).unwrap();
        let error = context
            .uninstall_plugin("sample", "market")
            .await
            .unwrap_err();
        assert!(error.0.contains("仍返回插件"));
    }

    #[tokio::test]
    async fn marketplace_refresh_rejects_invalid_selectors_before_running_cli() {
        let (_home, context) = test_context();
        for name in ["../market", ""] {
            let error = context.refresh_plugin_marketplace(name).await.unwrap_err();
            assert!(error.0.contains("非法字符"));
        }
    }

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

    #[tokio::test]
    async fn readonly_plugins_reject_uninstall() {
        let (home, context) = test_context();
        std::fs::create_dir_all(home.path().join(".agents/skills")).unwrap();
        std::fs::write(
            home.path().join(".agents/.skill-lock.json"),
            r#"{"version":3,"skills":{"lark-base":{"source":"larksuite/cli","sourceType":"github"}}}"#,
        )
        .unwrap();
        let error = context
            .uninstall_plugin("lark-base", "market")
            .await
            .unwrap_err();
        assert!(error.0.contains("Skill 注册表"));
    }
}
