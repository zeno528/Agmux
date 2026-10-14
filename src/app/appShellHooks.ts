import { useCallback, useEffect, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import { api, isTauri } from "../api";
import type { AppView, AppState, AuthStatus, CodexAppStatus, Settings } from "../types";

export type { AppView };

/** 设置里的 startup_view 是透传字符串，手改坏值在此回落默认页。 */
export function asAppView(value: string | null | undefined): AppView {
  const appViews: AppView[] = ["codexProfiles", "claudeProfiles", "plugins", "accounts", "mcp", "skills", "agentTools", "settings"];
  return appViews.includes(value as AppView) ? value as AppView : "codexProfiles";
}

// 启动期预发 get_state：IPC 与脚本求值、React 挂载并行，首个 refresh 直接吃这份在途结果，
// 省掉"挂载完 effect 才发请求"的一轮串行等待。只消费一次；激活/聚焦后的 refresh 照常发新请求。
// 失败后清空占位，下一次 refresh 可以重新读取。
let pendingStartupState: Promise<AppState> | null = api.getState();
// 首个 refresh 必然消费它；万一没消费到也不抛 unhandled rejection，错误处理在 refresh 里
pendingStartupState.catch(() => undefined);

export function useAppState() {
  const [state, setState] = useState<AppState | null>(null);
  const [authStatusReady, setAuthStatusReady] = useState(false);
  const [loadError, setLoadError] = useState("");
  const stateRef = useRef<AppState | null>(null);

  useEffect(() => {
    stateRef.current = state;
  }, [state]);

  const refresh = useCallback(async () => {
    const request = pendingStartupState;
    pendingStartupState = null;
    try {
      const nextState = await (request ?? api.getState());
      const merged = stateRef.current ? { ...nextState, auth_status: stateRef.current.auth_status } : nextState;
      stateRef.current = merged;
      setState(merged);
      setLoadError("");
    } catch (error) {
      setLoadError(String(error));
    }
  }, []);

  const updateAuthStatus = useCallback((auth_status: AuthStatus) => {
    const previous = stateRef.current;
    if (!previous) return;
    const next = { ...previous, auth_status };
    stateRef.current = next;
    setState(next);
    setAuthStatusReady(true);
  }, []);

  const refreshAuthStatus = useCallback(async () => {
    try {
      updateAuthStatus(await api.authGetStatus());
    } catch {
      // 首屏已经显示时，后台认证刷新失败保留旧快照。
      setAuthStatusReady(true);
    }
  }, [updateAuthStatus]);

  const updateCodex = useCallback((codex: CodexAppStatus) => {
    const previous = stateRef.current;
    if (!previous) return;
    const next = { ...previous, codex };
    stateRef.current = next;
    setState(next);
  }, []);

  const updateSettings = useCallback((settings: Settings) => {
    const previous = stateRef.current;
    if (!previous) return;
    const next = { ...previous, settings };
    stateRef.current = next;
    setState(next);
  }, []);

  const previewTheme = useCallback((theme: Settings["theme"]) => {
    const previous = stateRef.current;
    if (!previous) return;
    const next = { ...previous, settings: { ...previous.settings, theme } };
    stateRef.current = next;
    setState(next);
  }, []);

  return { state, stateRef, loadError, authStatusReady, refresh, refreshAuthStatus, updateAuthStatus, updateCodex, updateSettings, previewTheme };
}

export function useThemeMode(theme: Settings["theme"] | undefined) {
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  const isDark = theme === "dark" || ((theme ?? "system") === "system" && systemDark);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.add("theme-switching");
    root.classList.toggle("dark", isDark);
    root.style.colorScheme = isDark ? "dark" : "light";
    const frame = requestAnimationFrame(() => root.classList.remove("theme-switching"));
    if (isTauri) void api.setWindowTheme(isDark).catch(() => undefined);
    return () => cancelAnimationFrame(frame);
  }, [isDark]);

}

export function useCodexPolling(
  stateRef: MutableRefObject<AppState | null>,
  updateCodex: (codex: CodexAppStatus) => void,
) {
  const codexPollTimer = useRef<number | undefined>(undefined);
  const codexPolling = useRef(false);

  const pollCodexStatus = useCallback(async () => {
    if (codexPolling.current || !stateRef.current) return;
    codexPolling.current = true;
    try {
      updateCodex(await api.getCodexStatus());
    } catch {
      // 轮询失败保留上次状态。
    } finally {
      codexPolling.current = false;
    }
  }, [stateRef, updateCodex]);

  const stop = useCallback(() => {
    if (codexPollTimer.current !== undefined) {
      window.clearInterval(codexPollTimer.current);
      codexPollTimer.current = undefined;
    }
  }, []);

  const start = useCallback(() => {
    if (document.hidden || !stateRef.current) {
      stop();
      return;
    }
    if (codexPollTimer.current === undefined) {
      codexPollTimer.current = window.setInterval(() => void pollCodexStatus(), 3000);
    }
  }, [pollCodexStatus, stateRef, stop]);

  return { start, stop };
}

export function useActivationRefresh() {
  const [activationEpoch, setActivationEpoch] = useState(0);
  const activeRef = useRef(!document.hidden);
  const activate = useCallback(() => {
    if (activeRef.current) return false;
    activeRef.current = true;
    setActivationEpoch((value) => value + 1);
    return true;
  }, []);
  const deactivate = useCallback(() => {
    if (!activeRef.current) return false;
    activeRef.current = false;
    return true;
  }, []);
  return { activationEpoch, activate, deactivate };
}

export function useSidebar() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(
    () => localStorage.getItem("agmux.sidebar-collapsed") === "1",
  );
  const [sidebarFlyoutArmed, setSidebarFlyoutArmed] = useState(true);

  const toggleSidebar = () => {
    setSidebarFlyoutArmed(false);
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      localStorage.setItem("agmux.sidebar-collapsed", next ? "1" : "0");
      return next;
    });
  };

  return {
    sidebarCollapsed,
    sidebarFlyoutArmed,
    setSidebarFlyoutArmed,
    toggleSidebar,
  };
}
