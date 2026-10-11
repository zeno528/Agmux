pub mod auth;
pub mod codex;
pub mod codex_builtin;
pub mod commands;
pub mod database;
pub mod error;
pub mod fsutil;
pub mod models;
mod network;
pub mod paths;
pub mod services;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_log::{
    log, FileOpenStrategy, RotationStrategy, Target, TargetKind, TimezoneStrategy,
};
use tauri_plugin_window_state::{Builder as WindowStateBuilder, StateFlags};

use crate::{models::TrayClickAction, services::AppContext};

#[cfg(target_os = "macos")]
fn should_restore_main_window_on_reopen(has_visible_windows: bool) -> bool {
    !has_visible_windows
}

#[derive(serde::Deserialize)]
pub struct TrayProfile {
    pub id: String,
    pub name: String,
}

/// 启动时钟：run() 入口的 Instant 交给命令层，前端里程碑折算到同一进程起点。
pub struct StartupClock(std::time::Instant);

pub struct TrayClickMode(pub AtomicBool);

/// 托盘菜单文案。语言由前端解析后传入（Rust 侧无法得知 "system" 对应哪种系统语言）。
fn tray_labels(
    language: &str,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    if language == "en-US" {
        (
            "Open main window",
            "Switch provider",
            "Accounts",
            "Settings...",
            "Quit",
        )
    } else {
        ("打开主界面", "切换供应商", "账号", "设置…", "退出")
    }
}

pub fn tray_menu(
    app: &tauri::AppHandle,
    language: &str,
    profiles: &[TrayProfile],
    active_profile_id: Option<&str>,
) -> tauri::Result<Menu<tauri::Wry>> {
    let (show, switch, accounts, settings, quit) = tray_labels(language);
    let show_item = MenuItem::with_id(app, "show", show, true, None::<&str>)?;
    let active = profiles
        .iter()
        .find(|profile| Some(profile.id.as_str()) == active_profile_id);
    let switch_title = active
        .map(|profile| profile.name.replace('&', "&&"))
        .unwrap_or_else(|| switch.to_string());
    #[cfg(target_os = "macos")]
    let switch_menu = Submenu::with_id_and_native_icon(
        app,
        "switch",
        switch_title,
        !profiles.is_empty(),
        active.map(|_| tauri::menu::NativeIcon::MenuOnState),
    )?;
    #[cfg(not(target_os = "macos"))]
    let switch_menu = Submenu::with_id(app, "switch", switch_title, !profiles.is_empty())?;
    for profile in profiles {
        let selected = Some(profile.id.as_str()) == active_profile_id;
        switch_menu.append(&CheckMenuItem::with_id(
            app,
            format!("profile:{}", profile.id),
            profile.name.replace('&', "&&"),
            true,
            selected,
            None::<&str>,
        )?)?;
    }
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let accounts_item = MenuItem::with_id(app, "accounts", accounts, true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", settings, true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", quit, true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show_item,
            &separator_one,
            &switch_menu,
            &separator_two,
            &accounts_item,
            &settings_item,
            &quit_item,
        ],
    )?;
    #[cfg(windows)]
    if active.is_some() {
        use tauri::menu::ContextMenu;
        use windows::Win32::UI::WindowsAndMessaging::{
            CheckMenuItem, HMENU, MF_BYPOSITION, MF_CHECKED,
        };

        // Tauri 的 Submenu 没有 checked 属性；直接设置原生菜单的勾选栏，保留文字缩进和子菜单箭头。
        let result = unsafe {
            CheckMenuItem(
                HMENU(menu.hpopupmenu()? as _),
                2,
                MF_BYPOSITION.0 | MF_CHECKED.0,
            )
        };
        if result == u32::MAX {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(menu)
}

/// 唤出并聚焦主窗口，托盘左键与菜单多个入口共用。
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 启动埋点：进程起点时钟。native / webview / 前端三段里程碑都折算到它，
    // Info 级（一场启动各一行），供性能基线与回归对比
    let startup_started = std::time::Instant::now();
    // panic 钩子最先装：日志插件就绪后的崩溃写入文件，再交还默认处理器；
    // 这是全项目唯一的 error! 调用（日志规约：error 留给崩溃）
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| message.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "非字符串 panic".to_string());
        let location = info
            .location()
            .map(|location| location.to_string())
            .unwrap_or_else(|| "未知位置".to_string());
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("<unnamed>").to_string();
        log::error!(
            "[panic.crash] thread={thread:?} outcome=failure failure_kind=internal error={:?} msg=\"Agofra 崩溃\"",
            format!("{message} @{location}")
        );
        default_hook(info);
    }));

    let paths = paths::app_paths().expect("无法定位用户数据目录");
    let database = Arc::new(database::Database::open(&paths).expect("无法初始化 Agofra 数据库"));
    let context = AppContext::new_with_database(paths.clone(), database.clone());
    let oauth_state = auth::CodexOAuthState(Arc::new(auth::codex_oauth::CodexOAuthManager::new(
        database,
    )));

    tauri::Builder::default()
        // WebView 文档加载完成点：webview 导航结束的里程碑，折算到进程起点。
        // 只记 Finished——Started 在文档开始拉取时就触发，对启动诊断没有增量信息。
        // SPA 无二次导航；dev 热重载各多一行，可接受
        .on_page_load(move |_webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                log::info!(
                    "[app.startup] stage=page_load_finished since_start_ms={} msg=\"WebView 文档加载完成\"",
                    startup_started.elapsed().as_millis()
                );
            }
        })
        // 日志插件放链条首位：其 setup 最先挂全局 logger，后续插件的日志也能被捕获
        .plugin(
            tauri_plugin_log::Builder::new()
                // 磁盘封顶：单文件攒到 1MB 才轮转出备份，最多留 10 个旧文件。
                // 实测单会话日志仅几 KB~几十 KB，平时只会看到一个 agofra.log，
                // 会话边界靠每次启动的「Agofra v 启动」横幅行分隔
                .rotation_strategy(RotationStrategy::KeepSome(10))
                .max_file_size(1_000_000)
                // 单文件追加（插件默认行为）：不做按启动分文件，避免日志目录文件堆积
                .file_open_strategy(FileOpenStrategy::Append)
                .timezone_strategy(TimezoneStrategy::UseLocal)
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                // updater 每次检查更新会把完整响应（含三平台签名）打成 DEBUG，
                // 淹没真正有用的日志行，压到 Info；release 本就是 Info，此行只影响 dev
                .level_for("tauri_plugin_updater", log::LevelFilter::Info)
                // reqwest 每次建连都记一行 DEBUG（一场会话数百条），压到 Info 留出
                // 自有 debug 日志的可读性；连接失败仍会以 WARN 冒出，不受影响
                .level_for("reqwest", log::LevelFilter::Info)
                // tao（窗口库）在 Windows 上偶发成对 event_loop DEBUG，与业务无关
                .level_for("tao", log::LevelFilter::Info)
                // 行格式自定义：时间 + 级别 + 消息。默认格式会注入模块路径
                // （agofra_lib::services::…），与消息里的域前缀双重定位纯属冗余，
                // 去掉后每行短 40 字符；导航只认域前缀。
                .format(|out, message, record| {
                    let now = chrono::Local::now();
                    out.finish(format_args!(
                        "[{}][{}] [{}] {}",
                        now.format("%Y-%m-%d"),
                        now.format("%H:%M:%S"),
                        record.level(),
                        message
                    ))
                })
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::Folder {
                        path: paths.logs.clone(),
                        file_name: Some("agofra".into()),
                    }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            WindowStateBuilder::default()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .manage(context)
        .manage(oauth_state)
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::report_startup_mark,
            commands::get_codex_status,
            commands::codex_capture_profile,
            commands::codex_add_builtin_profile,
            commands::codex_add_custom_profile,
            commands::codex_get_builtin_catalog,
            commands::codex_get_builtin_config,
            commands::codex_test_profile_connection,
            commands::codex_test_provider_connection,
            commands::codex_fetch_provider_models,
            commands::codex_fetch_chatgpt_models,
            commands::codex_get_profile_balance,
            commands::claude_get_profile_balance,
            commands::export_database,
            commands::export_database_to,
            commands::import_database,
            commands::list_database_backups,
            commands::restore_database,
            commands::delete_database_backup,
            commands::rename_database_backup,
            commands::rename_profile,
            commands::codex_reorder_profiles,
            commands::codex_set_profile_icon,
            commands::codex_set_profile_show_balance,
            commands::codex_set_profile_fetched_models,
            commands::set_profile_balance,
            commands::codex_set_profile_account,
            commands::codex_duplicate_profile,
            commands::codex_get_profile,
            commands::codex_update_profile,
            commands::codex_update_profile_config,
            commands::codex_set_profile_model,
            commands::codex_patch_chatgpt_context_config,
            commands::codex_patch_system_proxy_config,
            commands::codex_patch_context_management_config,
            commands::validate_toml,
            commands::format_toml,
            commands::codex_delete_profile,
            commands::codex_apply_profile,
            commands::codex_list_mcp_servers,
            commands::claude_list_mcp_servers,
            commands::claude_get_mcp_server_json,
            commands::claude_save_mcp_server,
            commands::claude_delete_mcp_server,
            commands::probe_mcp_server,
            commands::codex_save_mcp_server,
            commands::codex_delete_mcp_server,
            commands::set_mcp_server_enabled,
            commands::set_mcp_mirror,
            commands::revert_mcp_live,
            commands::set_mcp_mirror_entries,
            commands::revert_mcp_live_entries,
            commands::codex_get_mcp_section_toml,
            commands::restore_mcp_from_database,
            commands::codex_mcp_sync_preview,
            commands::claude_mcp_sync_preview,
            commands::claude_resolve_mcp_entries,
            commands::codex_get_mcp_server_toml,
            commands::patch_mcp_fragment,
            commands::parse_mcp_fragment,
            commands::restart_codex,
            commands::set_window_theme,
            commands::set_tray_menu,
            commands::auth_start_browser_login,
            commands::auth_poll_browser_login,
            commands::auth_cancel_browser_login,
            commands::auth_get_status,
            commands::auth_get_quota,
            commands::auth_warmup,
            commands::auth_preview,
            commands::auth_remove_account,
            commands::open_url,
            commands::get_settings,
            commands::get_proxy_status,
            commands::save_settings,
            commands::log_update_event,
            commands::report_cli_update_tick,
            commands::check_app_update,
            commands::set_update_marker,
            commands::take_update_marker,
            commands::list_plugins,
            commands::list_skills,
            commands::get_skill_content,
            commands::get_import_skill_content,
            commands::scan_unmanaged_skills,
            commands::import_skill,
            commands::enable_skill,
            commands::disable_skill,
            commands::claude_list_profiles,
            commands::claude_get_common_settings,
            commands::claude_save_common_settings,
            commands::claude_open_terminal,
            commands::claude_get_profile,
            commands::claude_capture_profile,
            commands::claude_save_profile,
            commands::claude_delete_profile,
            commands::claude_apply_profile,
            commands::claude_set_profile_icon,
            commands::claude_set_profile_show_balance,
            commands::claude_reorder_profiles,
            commands::claude_duplicate_profile,
            commands::claude_test_profile,
            commands::claude_test_connection,
            commands::claude_fetch_models,
            commands::claude_get_cli_status,
            commands::claude_check_cli_update,
            commands::claude_install_cli,
            commands::claude_update_cli,
            commands::codex_get_cli_status,
            commands::codex_check_cli_update,
            commands::codex_install_cli,
            commands::codex_update_cli,
            commands::delete_skill,
            commands::list_plugin_skills,
            commands::list_plugin_marketplaces,
            commands::list_marketplace_plugins,
            commands::add_plugin_marketplace,
            commands::remove_plugin_marketplace,
            commands::install_marketplace_plugin,
            commands::check_plugin_updates,
            commands::upgrade_marketplace_plugin,
            commands::preview_plugin,
            commands::install_plugin,
            commands::uninstall_plugin,
            commands::open_path,
        ])
        .setup(move |app| {
            log::info!(
                "[app.startup] stage=native_ready since_start_ms={} msg=\"进入 Tauri setup\"",
                startup_started.elapsed().as_millis()
            );
            log::info!(
                "[app.start] version=\"{}\" outcome=success msg=\"Agofra 启动\"",
                env!("CARGO_PKG_VERSION")
            );
            // macOS 上窗口配置 visible:false 不生效（创建后实际处于可见状态），
            // 统一先隐藏一次；非静默启动时由前端在 settings 加载后 show()。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            use tauri_plugin_autostart::ManagerExt;

            #[cfg(desktop)]
            app.handle()
                .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                    show_main_window(app);
                }))?;

            // 设置加载失败若静默回退默认值，用户配置「消失」且无迹可寻，必须留痕
            let settings = match app.state::<AppContext>().settings() {
                Ok(settings) => settings,
                Err(error) => {
                    log::warn!(
                        "[settings.load] outcome=failure failure_kind=internal error={error:?} msg=\"加载设置失败，本次启动使用默认值\""
                    );
                    Default::default()
                }
            };
            network::set_proxy(settings.proxy_mode.clone(), settings.proxy_url.clone());
            // reg.exe / scutil 属于阻塞平台调用，只在后台记录一次实际网络走向。
            tauri::async_runtime::spawn_blocking(|| {
                match network::Network::detect() {
                    Ok(network) if network.proxy.is_some() => {
                        log::info!("[net.proxy] outcome=success proxy={} msg=\"应用请求经代理连接\"", network.display.as_deref().unwrap_or("-"))
                    }
                    Ok(_) => log::debug!("[net.proxy] outcome=success msg=\"应用请求直连\""),
                    Err(error) => log::warn!("[net.proxy] outcome=failure failure_kind={} msg={:?}", error.kind, error.message),
                }
            });
            let show_tray_menu_on_left_click = settings.tray_click_action == TrayClickAction::ShowMenu;
            app.manage(TrayClickMode(AtomicBool::new(show_tray_menu_on_left_click)));
            // dev 构建与安装版共用 identifier，自启注册表值名同为 productName，
            // dev 若照常同步会把开机自启改写成 target/debug 下的二进制
            if settings.autostart_enabled && !tauri::is_dev() {
                match app.autolaunch().enable() {
                    Ok(()) => {
                        log::debug!("[autostart.sync] outcome=success msg=\"开机自启已启用\"")
                    }
                    Err(error) => log::warn!(
                        "[autostart.sync] outcome=failure failure_kind=internal error={error:?} msg=\"同步开机自启设置失败\""
                    ),
                }
            }

            let scheduler_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if let Err(error) = scheduler_handle.state::<AppContext>().auto_backup_if_due()
                    {
                        log::warn!(
                            "[backup.auto] outcome=failure failure_kind=internal error={error:?} msg=\"自动备份调度失败\""
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });

            // 启动时钟交给命令层：前端里程碑（state_ready / window_shown）折算到进程起点
            app.manage(StartupClock(startup_started));
            let menu = tray_menu(app.handle(), &settings.language, &[], None)?;
            TrayIconBuilder::with_id("main")
                .icon(
                    // Windows 上 default_window_icon 取 ICO 第一帧（16x16），托盘按 DPI 放大后发糊；
                    // 固定用 32x32 PNG（macOS 的 default_window_icon 本就取自该文件）。
                    tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
                        .expect("托盘图标解码失败"),
                )
                .menu(&menu)
                .show_menu_on_left_click(show_tray_menu_on_left_click)
                .on_tray_icon_event(|tray, event| {
                    // 左键行为由设置控制；菜单模式下不再同时唤出主窗口。
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if tray.app_handle().state::<TrayClickMode>().0.load(Ordering::Relaxed) {
                            return;
                        }
                        show_main_window(tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "settings" => {
                        show_main_window(app);
                        let _ = app.emit("tray-open-settings", ());
                    }
                    "accounts" => {
                        show_main_window(app);
                        let _ = app.emit("tray-open-accounts", ());
                    }
                    "quit" => app.exit(0),
                    id => {
                        if let Some(profile_id) = id.strip_prefix("profile:") {
                            let _ = app.emit("tray-switch-profile", profile_id);
                        }
                    }
                })
                .build(app)?;

            // Windows 任务栏优先取窗口 Small 槽图标，而 Tauri 默认塞给窗口的 default_window_icon
            // 是 ICO 第一帧（16x16），高 DPI 下放大发糊；显式覆盖为 32x32（150% 缩放任务栏为 36px）。
            #[cfg(target_os = "windows")]
            if let Some(main) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
                    .expect("窗口图标解码失败");
                let _ = main.set_icon(icon);
            }

            log::info!(
                "[app.startup] stage=setup_end since_start_ms={} msg=\"Tauri setup 完成\"",
                startup_started.elapsed().as_millis()
            );
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = window
                    .app_handle()
                    .state::<AppContext>()
                    .settings()
                    .map(|settings| settings.minimize_to_tray)
                    .unwrap_or(false);
                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Agofra")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            {
                if let tauri::RunEvent::Reopen {
                    has_visible_windows,
                    ..
                } = event
                {
                    if should_restore_main_window_on_reopen(has_visible_windows) {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                }
            }

            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppResult;

    #[test]
    fn tray_labels_include_accounts_in_both_languages() {
        assert_eq!(tray_labels("en-US").2, "Accounts");
        assert_eq!(tray_labels("zh-CN").2, "账号");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reopen_restores_only_when_no_window_is_visible() {
        assert!(should_restore_main_window_on_reopen(false));
        assert!(!should_restore_main_window_on_reopen(true));
    }

    #[test]
    fn service_context_initializes_empty_database() -> AppResult<()> {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths::from_home(dir.path())?;
        let context = AppContext::new(paths)?;
        let state = context.get_state()?;
        assert!(state.codex_profiles.is_empty());
        assert!(state.active_codex_profile_id.is_none());
        Ok(())
    }
}
