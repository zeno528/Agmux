import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Download } from "lucide-react";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { checkForAppUpdate, UPDATED_VERSION_KEY, type AppUpdate } from "./appUpdate";
import { updateFailureMessage } from "./updateText";
import { preloadUpdateNotesRenderer, UpdateNotesDialog } from "./UpdateNotesDialog";
import type { Settings } from "../../types";

/** 更新日志入口：直接打开对应版本的 GitHub Release 页，避免 /latest 重定向。 */
export const releaseNotesUrl = (version: string) => `https://github.com/zeno528/Agmux/releases/tag/v${encodeURIComponent(version.replace(/^v/, ""))}`;
interface AppUpdateContextValue {
  /** 已发现的可用更新；null 表示已是最新或尚未检查 */
  update: AppUpdate | null;
  checking: boolean;
  installing: boolean;
  /** 检查更新：结果写入 context，失败时抛错由调用方决定是否提示 */
  check: () => Promise<AppUpdate | null>;
  /** 用户点击升级后才执行：下载安装并重启 */
  install: () => Promise<void>;
}

const AppUpdateContext = createContext<AppUpdateContextValue | null>(null);

export function useAppUpdate() {
  const value = useContext(AppUpdateContext);
  if (!value) throw new Error("useAppUpdate 必须在 AppUpdateProvider 内使用"); // i18n-exempt: 开发者契约错误，用户不可见
  return value;
}

export function AppUpdateProvider({ enabled, ready = true, proxyMode = "auto", proxyUrl = "", children }: {
  enabled: boolean; ready?: boolean; children: ReactNode;
  proxyMode?: Settings["proxy_mode"]; proxyUrl?: string;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("updates");
  const [update, setUpdate] = useState<AppUpdate | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  // StrictMode 下 effect 双跑共用同一组件实例，state 守卫两次都读到旧值，必须用 ref 防重入
  const autoCheckedRef = useRef(false);
  const checkingRef = useRef(false);
  const network = JSON.stringify([proxyMode, proxyMode === "custom" ? proxyUrl : ""]);
  const networkRef = useRef(network);
  const checkedNetworkRef = useRef<string | null>(null);
  networkRef.current = network;

  // 未开始安装的更新结果不能继续沿用旧代理；正在下载的任务保持原网络快照。
  useEffect(() => { setUpdate(null); }, [network]);

  const check = useCallback(async (): Promise<AppUpdate | null> => {
    if (checkingRef.current) return update;
    checkingRef.current = true;
    setChecking(true);
    const checkingNetwork = networkRef.current;
    try {
      const found = await checkForAppUpdate();
      if (checkingNetwork !== networkRef.current) return null;
      checkedNetworkRef.current = checkingNetwork;
      setUpdate(found);
      if (found?.notes) preloadUpdateNotesRenderer();
      return found;
    } finally {
      checkingRef.current = false;
      setChecking(false);
    }
  }, [update]);

  // 首屏就绪后延迟 5 秒静默检查一次：发现新版只让状态栏图标出现，不弹出悬浮卡片、不自动下载
  useEffect(() => {
    if (!enabled || autoCheckedRef.current) return;
    const timer = window.setTimeout(() => {
      autoCheckedRef.current = true;
      void check().catch((error) => console.warn("自动检查更新失败：", updateFailureMessage(error, t))); // i18n-exempt: 仅写控制台，用户不可见
    }, 5000);
    return () => window.clearTimeout(timer);
  }, [enabled, check, t]);

  // 应用内更新重启回来：读到安装时留下的版本标记即弹「更新成功」通知（与 enabled 无关，标记只会在更新后存在一次）。
  // 标记由后端原子落盘（Windows 安装器会立即杀进程，localStorage 异步提交可能丢）；
  // 旧版本写在 localStorage 的键也兜底消费一次，覆盖升级过渡期
  useEffect(() => {
    if (!ready) return;
    void (async () => {
      const legacy = localStorage.getItem(UPDATED_VERSION_KEY);
      if (legacy) localStorage.removeItem(UPDATED_VERSION_KEY);
      const updatedVersion = legacy ?? (await api.takeUpdateMarker().catch(() => null));
      if (!updatedVersion) return;
      feedback.success(t("toast.updated", { version: updatedVersion }));
    })();
  }, [feedback, ready, t]);

  const install = useCallback(async () => {
    if (!update || installing || checkedNetworkRef.current !== networkRef.current) return;
    setInstalling(true);
    try {
      await update.install();
    } catch (error) {
      feedback.error(updateFailureMessage(error, t));
    } finally {
      setInstalling(false);
    }
  }, [update, installing, feedback, t]);

  return <AppUpdateContext.Provider value={{ update, checking, installing, check, install }}>{children}</AppUpdateContext.Provider>;
}

/** 状态栏更新图标：点击直接打开更新日志确认弹窗（先看日志，确认后才安装）。 */
export function UpdateNotice({
  className = "",
  sidebarCollapsed = false,
  sidebarFlyoutArmed = false,
  onMouseEnter,
}: {
  className?: string;
  sidebarCollapsed?: boolean;
  sidebarFlyoutArmed?: boolean;
  onMouseEnter?: () => void;
} = {}) {
  const { update } = useAppUpdate();
  const { t } = useTranslation("updates");
  const [confirming, setConfirming] = useState(false);
  useEffect(() => {
    if (!update) setConfirming(false);
  }, [update]);
  if (!update) return null;
  return (
    <div className={`update-notice ${className}`.trim()} onMouseEnter={onMouseEnter}>
      <button
        type="button"
        className="apple-sidebar-nav-button app-selection-state"
        aria-label={t("notice.title", { version: update.version })}
        aria-expanded={confirming}
        aria-haspopup="dialog"
        onClick={() => setConfirming(true)}
      >
        <span className="relative flex shrink-0">
          <span className="grid h-[var(--sidebar-icon-size)] w-[var(--sidebar-icon-size)] shrink-0 place-items-center rounded-full bg-accent text-white">
            <Download className="!h-3 !w-3" strokeWidth={2.5} aria-hidden="true" />
          </span>
        </span>
        <span className="apple-sidebar-label" aria-hidden={sidebarCollapsed}>{t("notice.available")}</span>
        {sidebarCollapsed && sidebarFlyoutArmed ? <span className="apple-sidebar-flyout" aria-hidden="true">{t("notice.available")}</span> : null}
      </button>
      <UpdateNotesDialog open={confirming} onOpenChange={setConfirming} />
    </div>
  );
}
