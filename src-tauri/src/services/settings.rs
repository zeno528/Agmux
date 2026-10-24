use super::storage::{backup_keep_count, DATABASE_BACKUP_PREFIX};
use super::sync;
use super::{
    app_err, atomic_write, codex_process, codex_window_state, now_ms, prune_backups, AppContext,
    AppResult, Path, PathInfo, Settings,
};

fn open_in_file_explorer(path: &Path) -> AppResult<()> {
    #[cfg(windows)]
    {
        // 目录直接打开自身；SHOpenFolderAndSelectItems 的语义是「打开父目录并
        // 选中目标」，只适合文件，目录走它会落在上一层还得再点一次进去
        if path.is_dir() {
            return std::process::Command::new("explorer")
                .arg(path)
                .spawn()
                .map(|_| ())
                .map_err(|error| app_err!("无法打开资源管理器：{error}"));
        }

        use windows::{
            core::HSTRING,
            Win32::{
                System::Com::CoInitialize,
                UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems},
            },
        };

        let _ = unsafe { CoInitialize(None) };
        // 走到这里必然是文件：打开其父目录并选中该文件
        let folder = path.parent().unwrap_or(path);
        let folder_text = HSTRING::from(folder);
        let folder_id = unsafe { ILCreateFromPathW(&folder_text) };
        if folder_id.is_null() {
            return Err(app_err!("无法定位资源管理器路径：{}", folder.display()));
        }
        let item_text = HSTRING::from(path);
        let item_id = path
            .is_file()
            .then(|| unsafe { ILCreateFromPathW(&item_text) });
        let selection = item_id
            .filter(|item| !item.is_null())
            .map(|item| [item.cast_const()]);
        let result = unsafe {
            SHOpenFolderAndSelectItems(
                folder_id.cast_const(),
                selection.as_ref().map(|items| &items[..]),
                0,
            )
        };
        unsafe {
            ILFree(Some(folder_id.cast_const()));
            if let Some(item) = item_id.filter(|item| !item.is_null()) {
                ILFree(Some(item.cast_const()));
            }
        }
        result.map_err(|error| app_err!("无法打开资源管理器：{error}"))
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| app_err!("无法打开文件管理器：{error}"))
    }
}

// 用 serde 序列化后 diff，避免手写字段列表与 Settings 结构体漂移
fn changed_settings_fields(before: &Settings, after: &Settings) -> String {
    let diff = match (
        serde_json::to_value(before).ok(),
        serde_json::to_value(after).ok(),
    ) {
        (Some(a), Some(b)) => a
            .as_object()
            .zip(b.as_object())
            .map(|(a, b)| {
                b.iter()
                    .filter(|(key, value)| a.get(*key) != Some(value))
                    .map(|(key, _)| key.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|| "unknown".into()),
        _ => "unknown".into(),
    };
    if diff.is_empty() {
        "none".into()
    } else {
        diff
    }
}

impl AppContext {
    pub fn restart_codex(&self) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        // 拉起前回写快照：严格读 live、读不了只记日志不拦
        // （语义与日志设计见 sync 执行体的 BeforeProcessStart 分支）。
        sync::registry().harvest(
            self,
            &sync::SyncTrigger::BeforeProcessStart {
                client: sync::ClientId::Codex,
            },
            sync::SyncMaterial::default(),
        )?;
        let process_ids = codex_process::find_process_ids(None);
        // 本次动作由 Codex 是否在跑决定：在跑＝先停后启的重启，没跑＝纯启动。两者共用本函数，
        // 收尾日志必须自己区分，否则纯启动也会记成“已重新启动”。
        let action = if process_ids.is_empty() {
            "start"
        } else {
            "restart"
        };
        // 强杀会绕过 Electron 的优雅退出落盘，先抓下实时窗口矩形，强杀后写回状态
        // 文件（Windows）；Codex 未运行时无窗口可抓，返回 None，回写自然跳过
        let window_bounds = codex_window_state::capture_main_window_bounds(&process_ids);
        let running_app_path = codex_process::running_app_path(&process_ids)
            .map(|path| path.to_string_lossy().into_owned());
        let mut force_killed = false;
        if !process_ids.is_empty() {
            // 先礼后兵：优雅退出请求（WM_CLOSE / AppleEvent quit）→ 超时强杀兜底；
            // Codex 未运行时本段整体跳过，直接走下方启动流程
            let outcome = codex_process::shutdown_process_ids(&process_ids);
            if outcome == codex_process::ShutdownOutcome::Timeout {
                let message = "Codex 未在超时时间内退出，已取消重新启动";
                self.database.record_event(
                    None,
                    "restart",
                    "timeout",
                    Some(message),
                    &now_ms().to_string(),
                )?;
                tauri_plugin_log::log::warn!(
                    "[settings.restart] outcome=failure failure_kind=timeout msg=\"Codex 未在超时时间内退出，已取消重新启动\""
                );
                return Err(app_err!("{message}"));
            }
            // 优雅路径静默失效（权限被拒/托盘拦截/headless）时唯一可观测的痕迹
            force_killed = outcome == codex_process::ShutdownOutcome::Forced;
        }
        let result = (|| {
            // 强杀完成后、新实例拉起前回写：Codex 进程已退出，无并发写者，
            // 新实例按回写尺寸建窗。失败只留日志，不影响重启主流程。
            if let Some(bounds) = &window_bounds {
                if codex_window_state::persist_window_bounds(&self.paths.codex_home, bounds) {
                    tauri_plugin_log::log::debug!(
                        "[settings.restart.window_bounds] outcome=success width={} height={} maximized={} msg=\"回写窗口尺寸\"",
                        bounds.width,
                        bounds.height,
                        bounds.is_maximized
                    );
                } else {
                    tauri_plugin_log::log::warn!(
                        "[settings.restart.window_bounds] outcome=failure failure_kind=io_error msg=\"回写窗口尺寸失败，跳过还原\""
                    );
                }
            }
            codex_process::launch_codex(running_app_path.as_deref())?;
            if codex_process::wait_for_running(10_000, 100) {
                Ok(())
            } else {
                Err(app_err!("Codex 启动超时，请检查应用是否可以正常打开"))
            }
        })();
        let status = if result.is_ok() { "success" } else { "failed" };
        let message = result
            .as_ref()
            .err()
            .map(|error| error.0.clone())
            .or_else(|| force_killed.then(|| "codex exited via force kill".to_string()));
        self.database.record_event(
            None,
            "restart",
            status,
            message.as_deref(),
            &now_ms().to_string(),
        )?;
        match &result {
            // 只断言验证过的事实：进程拉起来了。start/restart 的区分已经在 action= 字段里，
            // 而"应用是否真的可用"Agmux 无从验证——配置坏掉时进程照样会起来并弹错误框，
            // 那时候记"已启动"就是撒谎（19:40 那次就是这么记的）。
            Ok(()) => tauri_plugin_log::log::info!(
                "[settings.restart] action={action} outcome=success msg=\"Codex 进程已拉起\""
            ),
            Err(error) => tauri_plugin_log::log::warn!(
                "[settings.restart] action={action} outcome=failure failure_kind=internal error={error:?} msg=\"{}\"",
                if action == "start" { "Codex 启动失败" } else { "Codex 重启失败" }
            ),
        }
        result
    }

    pub fn settings(&self) -> AppResult<Settings> {
        let path = &self.paths.settings;
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // 首次运行：写出默认设置文件，方便直接手动编辑
                let defaults = Settings::default();
                let text = serde_json::to_string_pretty(&defaults)
                    .map_err(|_| app_err!("默认设置序列化失败"))?;
                atomic_write(path, text.as_bytes())?;
                return Ok(defaults);
            }
            Err(error) => return Err(app_err!("无法读取设置文件 {}: {error}", path.display())),
        };
        serde_json::from_str(&text)
            .map_err(|error| app_err!("设置文件 {} 无效: {error}", path.display()))
    }

    pub fn save_settings(&self, settings: &Settings) -> AppResult<Settings> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let previous = self.settings().ok();
        let mut settings = settings.clone();
        settings.theme = settings.theme.trim().to_lowercase();
        if !["system", "light", "dark"].contains(&settings.theme.as_str()) {
            return Err(app_err!("不支持的主题设置"));
        }
        // 语言值大小写不敏感地归一到规范形式（容忍手动编辑过 settings.json）
        settings.language = match settings.language.trim().to_ascii_lowercase().as_str() {
            "zh-cn" | "zh" => "zh-CN".into(),
            "en-us" | "en" => "en-US".into(),
            "system" => "system".into(),
            _ => return Err(app_err!("不支持的界面语言设置")),
        };
        settings.proxy_url = settings.proxy_url.trim().to_owned();
        if settings.proxy_mode == crate::models::ProxyMode::Custom {
            crate::network::Network::new(Some(settings.proxy_url.clone()))
                .map_err(|error| app_err!("{error}"))?;
        }
        let text =
            serde_json::to_string_pretty(&settings).map_err(|_| app_err!("设置序列化失败"))?;
        atomic_write(&self.paths.settings, text.as_bytes())?;
        // 与落盘共用操作锁，保证并发保存时运行中的选择与最后一份文件一致。
        crate::network::set_proxy(settings.proxy_mode.clone(), settings.proxy_url.clone());
        prune_backups(
            &self.paths.database_backup,
            DATABASE_BACKUP_PREFIX,
            ".db",
            backup_keep_count(settings.database_backup_keep_count),
        );
        let changed = previous
            .as_ref()
            .map(|before| changed_settings_fields(before, &settings))
            .unwrap_or_else(|| "unknown".into());
        tauri_plugin_log::log::info!(
            "[settings.save] changed={changed:?} outcome=success msg=\"设置已保存\""
        );
        Ok(settings)
    }

    pub fn set_update_marker(&self, version: &str) -> AppResult<()> {
        if let Err(error) = atomic_write(&self.paths.update_marker, version.as_bytes()) {
            tauri_plugin_log::log::warn!(
                "[update.install] version={version:?} outcome=failure failure_kind=io_error error={error:?} msg=\"写入更新标记失败\""
            );
            return Err(error);
        }
        // 安装器启动（Windows 下随即杀进程）前最后一条日志，升级排障以此为界
        tauri_plugin_log::log::info!(
            "[update.install] version={version:?} outcome=success msg=\"已写入更新标记，即将启动安装器\""
        );
        Ok(())
    }

    /// 读取并清除「已更新到 vX」标记（一次性消费）；无标记返回 None。
    /// rollback=true 表示安装失败后的取回：同样是有标记，语义从「更新成功」变「回滚」，日志分级不同。
    pub fn take_update_marker(&self, rollback: bool) -> AppResult<Option<String>> {
        match std::fs::read_to_string(&self.paths.update_marker) {
            Ok(text) => {
                let _ = std::fs::remove_file(&self.paths.update_marker);
                let version = text.trim().to_string();
                if rollback {
                    tauri_plugin_log::log::warn!(
                        "[update.install] version={version:?} outcome=failure failure_kind=internal msg=\"安装失败，回滚更新标记\""
                    );
                } else {
                    tauri_plugin_log::log::info!(
                        "[update.success] version={version:?} outcome=success msg=\"已更新到 {version}\""
                    );
                }
                Ok(Some(version))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                tauri_plugin_log::log::warn!(
                    "[update.success] outcome=failure failure_kind=io_error error={error:?} msg=\"读取更新结果标记失败\""
                );
                Err(app_err!("无法读取更新标记: {error}"))
            }
        }
    }

    pub fn open_path(&self, path: &str) -> AppResult<()> {
        if !self.is_managed_path(path) {
            return Err(app_err!("不能打开未列出的本机路径"));
        }
        open_in_file_explorer(Path::new(path))
    }

    pub(super) fn path_info(&self) -> Vec<PathInfo> {
        // label 是 i18n key，由前端翻译展示；对应 src/i18n/locales/*/settings.ts 的 about.paths.*
        vec![
            PathInfo {
                label: "about.paths.appData".into(),
                path: self.paths.root.display().to_string(),
            },
            PathInfo {
                label: "about.paths.backups".into(),
                path: self.paths.database_backup.display().to_string(),
            },
            PathInfo {
                label: "about.paths.logs".into(),
                path: self.paths.logs.display().to_string(),
            },
        ]
    }

    pub(super) fn is_managed_path(&self, path: &str) -> bool {
        self.path_info().iter().any(|item| item.path == path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_custom_proxy_never_overwrites_last_saved_settings() {
        let home = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(home.path()).unwrap();
        paths.ensure().unwrap();
        let context = AppContext::new(paths).unwrap();
        let before = context.settings().unwrap();
        let original = std::fs::read(&context.paths.settings).unwrap();
        for address in [
            "",
            "not-a-url",
            "socks5://127.0.0.1:1",
            "http://user:fixture-secret@",
        ] {
            let mut invalid = before.clone();
            invalid.proxy_mode = crate::models::ProxyMode::Custom;
            invalid.proxy_url = address.into();
            let error = context.save_settings(&invalid).unwrap_err().to_string();
            assert!(!error.contains("fixture-secret"));
            assert_eq!(context.settings().unwrap(), before);
            assert_eq!(std::fs::read(&context.paths.settings).unwrap(), original);
        }
    }

    #[test]
    fn settings_log_lists_changed_fields_without_values() {
        let before = Settings::default();
        let mut after = before.clone();
        after.auto_check_update = false;
        after.database_backup_keep_count = 10;

        assert_eq!(
            changed_settings_fields(&before, &after),
            "auto_check_update,database_backup_keep_count"
        );
        assert_eq!(changed_settings_fields(&before, &before), "none");
    }
}
