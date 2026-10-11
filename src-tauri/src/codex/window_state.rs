//! Codex 桌面端主窗口状态。Agofra 在 Windows 上的重启走「WM_CLOSE 只会关窗到
//! 托盘 → 超时强杀」阶梯，强杀绕过 Electron 的优雅退出落盘，窗口尺寸停留在
//! 上次优雅退出时的旧值（实测 2026-09-17 常驻 1280x820 出厂默认）。这里在
//! 重启前抓取实时窗口矩形，强杀后写回 Codex 自己的状态文件，让新实例按原
//! 尺寸建窗；macOS 走 AppleEvent 真优雅退出，Codex 自己落盘，无需干预。

use std::path::Path;

use serde::Serialize;

use crate::fsutil::atomic_write;

/// Codex 桌面端全局状态文件与主窗口边界键名（OpenAI 内部格式，无兼容承诺；
/// 键名变更时回写无人读取，静默退回「重启后小窗口」的现状，无副作用）。
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const WINDOW_BOUNDS_KEY: &str = "electron-main-window-bounds";

/// 主窗口边界，DIP 逻辑坐标——Electron getBounds 的坐标系，与状态文件存储一致。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub is_maximized: bool,
}

/// 物理像素 → DIP，按窗口所在显示器 DPI 换算（与 Electron 每显示器语义一致）。
/// 混合 DPI 多屏下副屏定位可能有亚像素误差，尺寸不受影响，可接受。
fn to_dip(value: i32, dpi: u32) -> i32 {
    if dpi == 0 {
        return value;
    }
    (f64::from(value) * 96.0 / f64::from(dpi)).round() as i32
}

/// 把窗口边界写进状态文件文本：只替换一个键，其余内容（statsig、overlay 等
/// 桌面端自己的状态）原样保留。解析失败返回 None，调用方放弃回写、原文件不动。
fn apply_to_state_text(text: &str, bounds: &CodexWindowBounds) -> Option<String> {
    let mut state: serde_json::Value = serde_json::from_str(text).ok()?;
    state
        .as_object_mut()?
        .insert(WINDOW_BOUNDS_KEY.into(), serde_json::to_value(bounds).ok()?);
    serde_json::to_string(&state).ok()
}

/// 强杀完成后、新实例拉起前调用：此时 Codex 进程已退出，无并发写者。
/// 任一步失败（无文件 / 解析失败 / 写失败）返回 false，重启主流程照常继续。
pub fn persist_window_bounds(codex_home: &Path, bounds: &CodexWindowBounds) -> bool {
    let state_path = codex_home.join(GLOBAL_STATE_FILE);
    let Ok(text) = std::fs::read_to_string(&state_path) else {
        return false;
    };
    let Some(updated) = apply_to_state_text(&text, bounds) else {
        return false;
    };
    atomic_write(&state_path, updated.as_bytes()).is_ok()
}

/// 抓取 Codex 主窗口的恢复态矩形与最大化标记。窗口隐藏（托盘驻留）时
/// GetWindowPlacement 仍返回有效的恢复态矩形，因此不要求窗口可见。
#[cfg(windows)]
pub fn capture_main_window_bounds(ids: &[u32]) -> Option<CodexWindowBounds> {
    use std::collections::HashSet;

    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowPlacement, GetWindowThreadProcessId, IsWindowVisible,
        SW_SHOWMAXIMIZED, WINDOWPLACEMENT,
    };

    struct Capture {
        pids: HashSet<u32>,
        best: Option<CodexWindowBounds>,
        best_visible: bool,
        best_area: i64,
    }

    // 候选评分：可见窗口优先，其次恢复态矩形面积最大（主窗口必然最大）。
    // 不按窗口类名过滤，避免耦合 Electron 内部类名。
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam 指向本线程栈上的 Capture，EnumWindows 返回前不会再被使用
        let capture = unsafe { &mut *(lparam.0 as *mut Capture) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 || !capture.pids.contains(&pid) {
            return true.into();
        }
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
            return true.into();
        }
        let rect = placement.rcNormalPosition;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return true.into();
        }
        let area = i64::from(width) * i64::from(height);
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        if !(visible && (!capture.best_visible || area > capture.best_area)) {
            return true.into();
        }
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        capture.best = Some(CodexWindowBounds {
            x: to_dip(rect.left, dpi),
            y: to_dip(rect.top, dpi),
            width: to_dip(width, dpi),
            height: to_dip(height, dpi),
            is_maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32,
        });
        capture.best_visible = visible;
        capture.best_area = area;
        true.into()
    }

    let mut capture = Capture {
        pids: ids.iter().copied().collect(),
        best: None,
        best_visible: false,
        best_area: 0,
    };
    let enumerated = unsafe {
        EnumWindows(
            Some(enum_proc),
            LPARAM(&mut capture as *mut Capture as isize),
        )
    };
    if enumerated.is_err() {
        return None;
    }
    capture.best
}

#[cfg(not(windows))]
pub fn capture_main_window_bounds(_ids: &[u32]) -> Option<CodexWindowBounds> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: i32, y: i32, width: i32, height: i32, is_maximized: bool) -> CodexWindowBounds {
        CodexWindowBounds {
            x,
            y,
            width,
            height,
            is_maximized,
        }
    }

    const SAMPLE: &str = r#"{"electron-main-window-bounds":{"x":640,"y":286,"width":1280,"height":820,"isMaximized":false},"statsig":{"stable":true}}"#;

    #[test]
    fn apply_overwrites_bounds_and_preserves_other_keys() {
        let updated = apply_to_state_text(SAMPLE, &bounds(100, 50, 2000, 1200, true)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&updated).unwrap();

        assert_eq!(
            parsed["electron-main-window-bounds"],
            serde_json::json!({ "x": 100, "y": 50, "width": 2000, "height": 1200, "isMaximized": true })
        );
        // 桌面端自己的其余状态不能丢
        assert_eq!(parsed["statsig"]["stable"], serde_json::json!(true));
    }

    #[test]
    fn apply_rejects_text_it_cannot_merge() {
        // 不是 JSON、或顶层不是对象（如数组）：一律放弃回写
        assert!(apply_to_state_text("not json", &bounds(0, 0, 1, 1, false)).is_none());
        assert!(apply_to_state_text("[1,2]", &bounds(0, 0, 1, 1, false)).is_none());
    }

    #[test]
    fn bounds_serialize_with_electron_key_names() {
        let value = serde_json::to_value(bounds(1, 2, 3, 4, true)).unwrap();
        assert_eq!(value["isMaximized"], serde_json::json!(true));
        assert!(value.get("is_maximized").is_none());
    }

    #[test]
    fn to_dip_scales_by_monitor_dpi() {
        assert_eq!(to_dip(1920, 96), 1920); // 100%：恒等
        assert_eq!(to_dip(1920, 144), 1280); // 150%
        assert_eq!(to_dip(1920, 128), 1440); // 133%
        assert_eq!(to_dip(100, 0), 100); // 异常 DPI 兜底
    }
}
