import * as AlertDialog from "@radix-ui/react-alert-dialog";
import * as Toast from "@radix-ui/react-toast";
import i18next from "i18next";
import { Check, Info, Trash2, TriangleAlert, X } from "lucide-react";
import { createContext, useCallback, useContext, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

type ToastTone = "success" | "error" | "warning" | "info";

interface ConfirmOptions {
  title: string;
  description: ReactNode;
  confirmText?: string;
  cancelText?: string;
  destructive?: boolean;
}

interface FeedbackContextValue {
  showToast: (tone: ToastTone, message: string) => void;
  success: (message: string) => void;
  error: (message: string) => void;
  warning: (message: string) => void;
  info: (message: string) => void;
  confirm: (options: ConfirmOptions) => Promise<boolean>;
}

const FeedbackContext = createContext<FeedbackContextValue | null>(null);

interface ToastState {
  id: number;
  tone: ToastTone;
  message: string;
  open: boolean;
}

interface ConfirmationState extends ConfirmOptions {
  resolve: (confirmed: boolean) => void;
}

const toastIcons = { success: Check, error: X, warning: TriangleAlert, info: Info } as const;
const MAX_TOASTS = 3;

export function normalizeToastMessage(message: string) {
  // 下面两条匹配的是 Rust 侧原文，不是待展示文案：翻译会破坏去重逻辑。
  const normalized = message
    .trim()
    .replace(/^Error:\s*/i, "")
    .replace(/连接失败[：:]\s*连接失败(?:[：:]\s*)?/g, "连接失败：") // i18n-exempt: 匹配后端原文
    .replace(/连接失败：\s*$/, "连接失败"); // i18n-exempt: 匹配后端原文
  return normalized || i18next.t("feedback.genericError");
}

export function FeedbackProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [toasts, setToasts] = useState<ToastState[]>([]);
  const [confirmation, setConfirmation] = useState<ConfirmationState | null>(null);
  const confirmActionRef = useRef<HTMLButtonElement>(null);
  const nextToastId = useRef(0);

  const removeToast = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const closeToast = useCallback((id: number) => {
    setToasts((current) => current.map((toast) => toast.id === id ? { ...toast, open: false } : toast));
    window.setTimeout(() => removeToast(id), 220);
  }, [removeToast]);

  const showToast = useCallback((tone: ToastTone, message: string) => {
    const id = ++nextToastId.current;
    setToasts((current) => [...current, { id, tone, message: normalizeToastMessage(message), open: true }].slice(-MAX_TOASTS));
    window.setTimeout(() => closeToast(id), 4000);
  }, [closeToast]);

  const confirm = useCallback((options: ConfirmOptions) => {
    return new Promise<boolean>((resolve) => setConfirmation({ ...options, resolve }));
  }, []);

  const closeConfirmation = useCallback((confirmed: boolean) => {
    setConfirmation((current) => {
      current?.resolve(confirmed);
      return null;
    });
  }, []);

  const [toastExpanded, setToastExpanded] = useState(false);

  // context value 必须引用稳定：toast 出现/消失只动 toasts state，不能反过来让
  // 依赖 useFeedback() 的 effect（编辑页加载、更新检查等）跟着重跑。
  const value = useMemo<FeedbackContextValue>(() => ({
    showToast,
    success: (message) => showToast("success", message),
    error: (message) => showToast("error", message),
    warning: (message) => showToast("warning", message),
    info: (message) => showToast("info", message),
    confirm,
  }), [showToast, confirm]);

  return (
    <FeedbackContext.Provider value={value}>
      <Toast.Provider swipeDirection="right">
        {children}
        {[...toasts].reverse().map((toast, index) => {
          const ToastIcon = toastIcons[toast.tone];
          const collapsedScale = Math.max(0.84, 1 - index * 0.04);
          const collapsedOpacity = Math.max(0.58, 1 - index * 0.08);
          const collapsedOffset = index * 10;
          const expandedOffset = index * 52;
          const toastStyle = {
            "--toast-offset": `${collapsedOffset}px`,
            "--toast-scale": collapsedScale,
            "--toast-opacity": collapsedOpacity,
            "--toast-expanded-offset": `${expandedOffset}px`,
            "--toast-resting-offset": `${toastExpanded ? expandedOffset : collapsedOffset}px`,
            "--toast-resting-scale": toastExpanded ? 1 : collapsedScale,
            "--toast-transition-delay": `${Math.min(index * 18, 72)}ms`,
            zIndex: toasts.length - index,
          } as CSSProperties;
          return (
            <Toast.Root
              key={toast.id}
              forceMount
              open={toast.open}
              duration={Infinity}
              onOpenChange={(open) => {
                if (!open) closeToast(toast.id);
              }}
              style={toastStyle}
              className={`app-toast app-toast--${toast.tone}`}
            >
              <ToastIcon className={`app-toast__icon app-toast__icon--${toast.tone}`} size={20} strokeWidth={2.5} aria-hidden="true" />
              <Toast.Description className="app-toast__content">{toast.message}</Toast.Description>
              <Toast.Close className="app-toast__close" aria-label={t("feedback.dismissToast")} title={t("feedback.dismissToast")}>
                <X size={14} strokeWidth={2.5} aria-hidden="true" />
              </Toast.Close>
            </Toast.Root>
          );
        })}
        <Toast.Viewport
          className="app-toast-viewport"
          style={{ "--toast-stack-height": `${toasts.length ? (toasts.length - 1) * 52 + 48 : 0}px` } as CSSProperties}
          data-expanded={toastExpanded}
          onPointerEnter={() => setToastExpanded(true)}
          onPointerLeave={() => setToastExpanded(false)}
          onFocusCapture={() => setToastExpanded(true)}
          onBlurCapture={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setToastExpanded(false);
          }}
        />
      </Toast.Provider>

      <AlertDialog.Root
        open={confirmation !== null}
        onOpenChange={(open) => {
          if (!open) closeConfirmation(false);
        }}
      >
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="app-dialog-overlay" />
          <AlertDialog.Content
            className="app-dialog-content app-dialog-content--confirm"
            onOpenAutoFocus={(event) => {
              if (!confirmActionRef.current) return;
              event.preventDefault();
              confirmActionRef.current.focus({ preventScroll: true });
            }}
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              (document.activeElement as HTMLElement | null)?.blur();
            }}
          >
            <AlertDialog.Title className="app-dialog-title flex items-center gap-1.5">
              {confirmation?.destructive ? <Trash2 className="relative top-px shrink-0 text-[var(--danger)]" size={18} strokeWidth={2} aria-hidden="true" /> : null}
              {confirmation?.title}
            </AlertDialog.Title>
            <AlertDialog.Description className="app-dialog-description">
              {confirmation?.description}
            </AlertDialog.Description>
            <div className="app-dialog-actions">
              <AlertDialog.Cancel className="apple-action-button" onClick={() => closeConfirmation(false)}>
                {confirmation?.cancelText ?? t("feedback.cancel")}
              </AlertDialog.Cancel>
              <AlertDialog.Action
                ref={confirmActionRef}
                className={`apple-action-button ${confirmation?.destructive ? "app-button--danger" : "app-button--primary"}`}
                onClick={() => closeConfirmation(true)}
              >
                {confirmation?.confirmText ?? t("feedback.confirm")}
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </FeedbackContext.Provider>
  );
}

export function useFeedback(): FeedbackContextValue {
  const value = useContext(FeedbackContext);
  if (!value) throw new Error("useFeedback must be used inside FeedbackProvider");
  return value;
}
