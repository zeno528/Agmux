import { CircleAlert, CreditCard, Download, Flame, LogIn, MoreHorizontal, Plus, RefreshCw, ShieldCheck } from "lucide-react";
import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { authQuotaCacheKey, authQuotaErrorKind, clearAuthQuotaError, getAuthQuotaBalance, getAuthQuotaError, getVisibleAuthQuota, setAuthQuotaFailure, setAuthQuotaSuccess } from "../../app/authQuotaCache";
import { useFeedback } from "../../app/Feedback";
import { AuthSourceIcon } from "../../components/AuthSourceIcon";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { PlanBadge } from "../../components/PlanBadge";
import { TrashIcon } from "../../components/TrashIcon";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { useMenuDismiss } from "../../components/useMenuDismiss";
import { chatgptLogo } from "../../icons";
import { balanceChipClass } from "../../presets";
import { isWeeklyWindowLabel, localizeBalanceLabel } from "../profiles/balanceLabel";
import { connectionExceptionMessage } from "../profiles/connectionText";
import type { AuthStatus, BrowserLoginStart, ChatgptResetCredit, ProfileBalanceInfo } from "../../types";
import { AddAccountDialog } from "./AddAccountDialog";

export function isOAuthLoginExpiredError(message: string) {
  return authQuotaErrorKind(message) === "auth_expired";
}

function remainingPercent(usedPercent: number) {
  return 100 - Math.min(100, Math.max(0, usedPercent));
}

function daysRemaining(time?: number | null) {
  return time == null ? null : Math.max(0, Math.ceil((time - Date.now()) / 86_400_000));
}

export function expiryColorClass(days: number | null) {
  return days == null ? "muted" : days <= 3 ? "text-(--danger)" : days <= 7 ? "text-(--warning)" : "muted";
}

function formatLocalTime(time: number, language: string) {
  return new Intl.DateTimeFormat(language, { year: "numeric", month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(time);
}

function formatShortLocalTime(time: number, language: string) {
  return new Intl.DateTimeFormat(language, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(time);
}

function localTimeZone(language: string) {
  return new Intl.DateTimeFormat(language, { hour: "numeric", timeZoneName: "short" }).formatToParts().find((part) => part.type === "timeZoneName")?.value ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
}

const quotaProgressAnimationDuration = 1000;

export function animateQuotaProgress(fill: HTMLSpanElement, startScale: number, endScale: number) {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  // 显式关键帧不依赖重挂载元素的 CSS 起始样式，增长和缩短走同一条动画路径。
  const animation = fill.animate([
    { transform: `scaleX(${startScale})` },
    { transform: `scaleX(${endScale})` },
  ], { duration: quotaProgressAnimationDuration, easing: "cubic-bezier(0.645, 0.045, 0.355, 1)" });
  return () => animation.cancel();
}

function QuotaProgressBar({ label, usedPercent, resetAt, resetIn, animationRevision, animationFromRemaining }: { label: string; usedPercent: number; resetAt?: number | null; resetIn?: string | null; animationRevision: number; animationFromRemaining?: number }) {
  const { t, i18n } = useTranslation("settings");
  // 窗口标签的文案在 profiles 命名空间，另取一个对应的 t
  const { t: tBalance } = useTranslation("profiles");
  const used = Math.min(100, Math.max(0, usedPercent));
  const remaining = remainingPercent(usedPercent);
  const animationStart = animationFromRemaining ?? 0;
  const animationMax = Math.max(animationStart, remaining);
  const animationStartScale = animationMax === 0 ? 1 : animationStart / animationMax;
  const animationEndScale = animationMax === 0 ? 1 : remaining / animationMax;
  const fillClass = used >= 90 ? "bg-(--danger)" : used >= 70 ? "bg-(--warning)" : "bg-(--chip-success)";
  const title = isWeeklyWindowLabel(label) ? t("account.weeklyLimit") : t("account.usageLimit", { label: localizeBalanceLabel(label, tBalance) ?? label });
  // 重置时间按当前界面语言本地化，不写死中文日期习惯
  const localizedResetIn = resetIn && i18n.language.startsWith("zh") ? resetIn.replace(/d/g, "天").replace(/h/g, "小时").replace(/m/g, "分钟") : resetIn; // i18n-exempt: 仅中文界面展示后端固定的 d/h/m 倒计时单位
  const reset = resetAt == null ? null : <>{t("account.resetTime", { time: formatShortLocalTime(resetAt, i18n.language) })}{localizedResetIn ? t("account.resetCountdown", { time: localizedResetIn }) : null}</>;
  const fillRef = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    const fill = fillRef.current;
    if (!fill || animationRevision === 0) return;
    return animateQuotaProgress(fill, animationStartScale, animationEndScale);
  }, [animationRevision, animationStartScale, animationEndScale]);

  return <div className="min-w-0 space-y-2 text-xs">
    <div className="min-w-0">
      <div className="field-subtitle">{title}</div>
      <div className="mt-0.5 flex items-center justify-between gap-2">
        <span className="meta-xs min-w-0 muted">{reset}</span>
        <span className="meta-xs shrink-0 whitespace-nowrap">{t("account.remainingShort")} <span className={`font-semibold ${balanceChipClass(used)}`}>{remaining}%</span></span>
      </div>
    </div>
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-black/6 dark:bg-white/8" role="progressbar" aria-label={t("account.remainingAria", { title })} aria-valuemin={0} aria-valuemax={100} aria-valuenow={remaining}>
      <span ref={fillRef} key={animationRevision} className={`origin-left block h-full rounded-full ${fillClass}`} style={{ width: `${animationRevision ? animationMax : remaining}%`, transform: animationRevision ? `scaleX(${animationEndScale})` : undefined }} />
    </div>
  </div>;
}

function AccountCard({ source, accountId, login, plan, expiresAt, cachedBalance, onRefreshed, active, onRemove, onRelogin, reloginDisabled }: {
  source: "desktop" | "oauth";
  accountId: string;
  login: string;
  plan?: string | null;
  expiresAt?: number | null;
  cachedBalance?: ProfileBalanceInfo;
  onRefreshed: () => Promise<void>;
  active: boolean;
  onRemove?: () => void;
  onRelogin?: () => void;
  reloginDisabled?: boolean;
}) {
  const { t } = useTranslation("settings");
  const feedback = useFeedback();
  // 窗口标签的文案在 profiles 命名空间，另取一个对应的 t
  const { t: tBalance } = useTranslation("profiles");
  const cacheKey = authQuotaCacheKey(source, accountId);
  const initialQuota = getVisibleAuthQuota(cacheKey, cachedBalance ?? null);
  const [quota, setQuota] = useState<ProfileBalanceInfo | null>(initialQuota);
  const [error, setError] = useState(() => getAuthQuotaError(cacheKey));
  const [loading, setLoading] = useState(false);
  const [animationRevision, setAnimationRevision] = useState(0);
  const [animationFromQuota, setAnimationFromQuota] = useState<ProfileBalanceInfo | null>(null);
  const displayedQuotaRef = useRef(getAuthQuotaBalance(cacheKey) ?? cachedBalance ?? null);
  const loadingRef = useRef(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuTriggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuStyle = useFixedMenuPosition(menuOpen, menuTriggerRef.current, menuRef, "end");
  useMenuDismiss(menuOpen, menuTriggerRef, menuRef, setMenuOpen);

  const refresh = async (manual = false) => {
    if (loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    try {
      const result = await api.authGetQuota(source, accountId);
      const info = result.balance_infos[0];
      if (!info) throw new Error("用量查询未返回数据"); // i18n-exempt: 内部错误信息，界面只按 error 真假渲染固定文案
      const previousQuota = displayedQuotaRef.current;
      displayedQuotaRef.current = info;
      setAnimationFromQuota(previousQuota);
      setQuota(info);
      setAnimationRevision((revision) => revision + 1);
      setAuthQuotaSuccess(cacheKey, info);
      // 复用现有持久化余额缓存，只用 auth 命名空间隔离账号。
      void api.setProfileBalance(cacheKey, info);
      setError("");
      await onRefreshed();
    } catch (cause) {
      const message = String(cause);
      setAuthQuotaFailure(cacheKey, message);
      setQuota(null);
      setError(message);
      if (manual) {
        const loginExpired = source === "oauth" && isOAuthLoginExpiredError(message);
        feedback.error(loginExpired ? t("account.quotaLoginExpired") : t("account.quotaRefreshFailed"));
      }
    }
    finally {
      loadingRef.current = false;
      setLoading(false);
    }
  };

  const warmup = async () => {
    if (loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    try {
      await api.authWarmup(source, accountId);
      feedback.success(t("account.warmupComplete"));
    } catch (cause) {
      feedback.error(t("account.warmupFailed", { error: String(cause) }));
      return;
    } finally {
      loadingRef.current = false;
      setLoading(false);
    }
    await refresh(true);
  };

  const fetchModels = async () => {
    if (loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    try {
      const models = await api.codexFetchChatgptModels(null, source, accountId);
      feedback.success(tBalance("edit.modelsFetched", { count: models.length }));
    } catch (cause) {
      feedback.error(tBalance("edit.fetchFailed", { error: connectionExceptionMessage(cause, tBalance) }));
    } finally {
      loadingRef.current = false;
      setLoading(false);
    }
  };

  useEffect(() => {
    const knownError = getAuthQuotaError(cacheKey);
    const nextQuota = getAuthQuotaBalance(cacheKey) ?? cachedBalance ?? null;
    displayedQuotaRef.current = nextQuota;
    setQuota(knownError ? null : nextQuota);
    setError(knownError);
    // 页面切入时静默重试一次；失败仍留在当前错误态，不打扰用户。
    if (active) void refresh();
    // Cache identity changes and page activation are the reload triggers; refresh keeps the latest value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, cacheKey]);

  // 后端回传的窗口标签按当前语言换词；后端没给时才用本语言兜底（映射见 balanceLabel.ts）
  const primaryLabel = localizeBalanceLabel(quota?.usage_label, tBalance) ?? t("account.quotaLabel");
  const weeklyLabel = localizeBalanceLabel(quota?.weekly_label, tBalance) ?? t("account.periodLabel");
  const loginExpired = source === "oauth" && isOAuthLoginExpiredError(error);
  const errorAction = loginExpired && onRelogin ? onRelogin : () => void refresh(true);

  return <div className="apple-group p-3">
    <div className="flex min-h-8 min-w-0 flex-nowrap items-center gap-3">
      <AuthSourceIcon source={source} className="h-5 w-5 shrink-0 text-accent" strokeWidth={2} />
      <div className="flex min-w-0 flex-1 items-baseline gap-2 whitespace-nowrap">
        <span className="mono min-w-0 truncate title-sm">{login}</span>
        <span className="apple-chip muted shrink-0">{t(source === "desktop" ? "account.followCodex" : "account.oauthDeviceLogin")}</span>
      </div>
      <button ref={menuTriggerRef} type="button" className="apple-icon-button shrink-0 text-[var(--text-secondary)] hover:text-accent" aria-haspopup="menu" aria-expanded={menuOpen} aria-label={t("account.more")} title={t("account.more")} aria-busy={loading} onClick={() => setMenuOpen((open) => !open)}>
        {loading ? <LoadingSpinner size="md" /> : <MoreHorizontal size={18} strokeWidth={2} aria-hidden="true" />}
      </button>
      {menuOpen ? createPortal(
        <div ref={menuRef} className="app-select-menu app-popover" data-open="true" data-popover-in role="menu" aria-label={t("account.more")} style={{ ...menuStyle, minWidth: "10rem" }}>
          <button type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={loading} onClick={() => { setMenuOpen(false); void refresh(true); }}>
            <span className="flex items-center gap-2"><RefreshCw size={16} strokeWidth={2} aria-hidden="true" />{t("account.refreshQuota")}</span>
          </button>
          <button type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={loading} onClick={() => { setMenuOpen(false); void fetchModels(); }}>
            <span className="flex items-center gap-2"><Download size={16} strokeWidth={2} aria-hidden="true" />{t("account.fetchModels")}</span>
          </button>
          <button type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={loading || loginExpired} title={t("account.warmupHint")} onClick={() => { setMenuOpen(false); void warmup(); }}>
            <span className="flex items-center gap-2"><Flame size={16} strokeWidth={2} aria-hidden="true" />{t("account.warmup")}</span>
          </button>
          {onRemove ? <button type="button" role="menuitem" className="app-select-option app-selection-state app-select-option--danger disabled:cursor-not-allowed disabled:opacity-40" disabled={loading} onClick={() => { setMenuOpen(false); onRemove(); }}>
            <span className="flex items-center gap-2"><TrashIcon />{t("account.remove")}</span>
          </button> : null}
        </div>, document.body,
      ) : null}
    </div>
    <SubscriptionExpiry plan={plan} expiresAt={expiresAt} />
    <div className="mt-3 border-t border-[var(--panel-divider)] pt-3">
    {quota?.usage_percent != null ? (
      <div className="space-y-2">
        <QuotaProgressBar
          label={primaryLabel}
          usedPercent={quota.usage_percent}
          resetAt={quota.usage_reset_at}
          resetIn={quota.usage_reset}
          animationRevision={animationRevision}
          animationFromRemaining={animationFromQuota?.usage_percent == null ? undefined : remainingPercent(animationFromQuota.usage_percent)}
        />
        {quota.weekly_usage_percent != null ? <QuotaProgressBar
          label={weeklyLabel}
          usedPercent={quota.weekly_usage_percent}
          resetAt={quota.weekly_reset_at}
          resetIn={quota.weekly_reset}
          animationRevision={animationRevision}
          animationFromRemaining={animationFromQuota?.weekly_usage_percent == null ? undefined : remainingPercent(animationFromQuota.weekly_usage_percent)}
        /> : null}
      </div>
    ) : error ? (
      <div role="alert" className="flex min-w-0 items-center justify-between gap-3 rounded-xl border border-[var(--panel-border)] bg-(--profile-chip-bg) px-3 py-2.5">
        <div className="flex min-w-0 items-start gap-2">
          {/* 感叹号撑满文本块两行高度：拉伸自适应行高，宽度按 1:1 viewBox 跟随 */}
          <CircleAlert className={`h-auto w-auto self-stretch shrink-0 ${loginExpired ? "text-[var(--warning)]" : "text-[var(--danger)]"}`} strokeWidth={2} />
          <div className="min-w-0">
            <div className="field-label">{loginExpired ? t("account.quotaLoginExpiredTitle") : t("account.quotaFailed")}</div>
            <p className="setting-description mt-0.5">{loginExpired ? t("account.quotaLoginExpiredDescription") : t("account.quotaRetryHint")}</p>
          </div>
        </div>
        <button type="button" className="apple-action-button app-button--primary shrink-0" disabled={loading || (loginExpired && reloginDisabled)} onClick={errorAction}>
          {loginExpired && onRelogin ? <LogIn className="h-4 w-4" strokeWidth={2} /> : <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} strokeWidth={2} />}
          {loginExpired && onRelogin ? t("account.relogin") : t("account.retryQuota")}
        </button>
      </div>
    ) : (
      <p className="mt-1 text-xs muted">{t("account.quotaLoading")}</p>
    )}
    {quota && (quota.reset_credits_available ?? 0) > 0 ? <ResetCredits availableCount={quota.reset_credits_available ?? 0} credits={quota.reset_credits} /> : null}
    </div>
  </div>;
}

function ResetCredits({ availableCount, credits }: { availableCount: number; credits?: ChatgptResetCredit[] | null }) {
  const { t, i18n } = useTranslation("settings");
  const formatExpiry = (expiresAt?: number | null) => expiresAt == null ? t("account.resetCreditExpiryUnknown") : formatShortLocalTime(expiresAt, i18n.language);
  const resetTitle = (resetType?: string | null) => ["full", "codex_rate_limits"].includes(resetType ?? "")
    ? t("account.resetCreditFullTitle")
    : t("account.resetCreditTitle");

  return <section className="mt-3 border-t border-[var(--panel-divider)] pt-3">
    <div className="flex items-center gap-3">
      <CreditCard className="h-5 w-5 shrink-0 text-accent" strokeWidth={2} />
      <div className="flex min-w-0 items-baseline gap-2"><div className="setting-title">{t("account.resetCreditsTitle")}</div><span className="apple-chip muted shrink-0">{t("account.resetCredits", { count: availableCount })}</span></div>
    </div>
    {/* 复用用量失败卡的同款内嵌卡片容器，一张次数一张卡 */}
    {credits?.length ? <div className="mt-3 space-y-2">
      {credits.map((credit) => {
        const days = daysRemaining(credit.expires_at);
        return (
          <div key={credit.id} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-xl border border-(--panel-border) bg-(--main-surface-bg) px-3 py-2.5">
            <div className="min-w-0 text-xs">{resetTitle(credit.reset_type)}</div>
            <div className="whitespace-nowrap text-xs">
              {t("account.resetCreditExpiry", { time: formatExpiry(credit.expires_at) })}
              {days == null ? null : <> · <span className={expiryColorClass(days)}>{t("account.resetCreditDaysRemaining", { count: days })}</span></>}
            </div>
          </div>
        );
      })}
    </div> : null}
  </section>;
}

function SubscriptionExpiry({ plan, expiresAt }: { plan?: string | null; expiresAt?: number | null }) {
  const { t, i18n } = useTranslation("settings");
  const subscriptionExpiry = plan?.toLowerCase() === "free" ? null : expiresAt;
  if (!plan && !subscriptionExpiry) return null;
  const days = daysRemaining(subscriptionExpiry);
  return <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1"><PlanBadge plan={plan ?? null} />{subscriptionExpiry ? <span className="meta-xs muted">{t("account.subscriptionRenewal", { time: formatLocalTime(subscriptionExpiry, i18n.language), timeZone: localTimeZone(i18n.language) })}{days == null ? null : <> · <span className={expiryColorClass(days)}>{t("account.subscriptionDaysRemaining", { count: days })}</span></>}</span> : null}</div>;
}

export default function AccountsView({ initialStatus, balanceCache, onAuthStatusChange, active = true }: { initialStatus: AuthStatus; balanceCache?: Record<string, ProfileBalanceInfo>; onAuthStatusChange?: (status: AuthStatus) => void; active?: boolean }) {
  const feedback = useFeedback();
  const { t } = useTranslation("settings");
  const [status, setStatus] = useState(initialStatus);
  const [loadError, setLoadError] = useState("");
  const [busy, setBusy] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [browserLogin, setBrowserLogin] = useState<BrowserLoginStart | null>(null);
  const disposed = useRef(false);
  const pollCancelled = useRef(false);

  const refreshStatus = async () => {
    try { const next = await api.authGetStatus(); if (!disposed.current) { setStatus(next); onAuthStatusChange?.(next); setLoadError(""); } }
    catch (error) { if (!disposed.current) setLoadError(String(error)); }
  };
  useEffect(() => { disposed.current = false; void refreshStatus(); return () => { disposed.current = true; }; }, []);
  useEffect(() => setStatus(initialStatus), [initialStatus]);

  const pollBrowser = async (current: BrowserLoginStart) => {
    try {
      const deadline = Date.now() + current.expires_in * 1000;
      while (!disposed.current && !pollCancelled.current && Date.now() < deadline) {
        const account = await api.authPollBrowserLogin();
        if (account) {
          setBrowserLogin(null);
          setAddOpen(false);
          // 重新授权成功：旧失败态缓存即刻作废，卡片重挂载后按无错误路径自动刷新出成功态
          clearAuthQuotaError(authQuotaCacheKey("oauth", account.id));
          await refreshStatus();
          feedback.success(t("account.addedToast", { login: account.login }));
          return;
        }
        await new Promise((resolve) => window.setTimeout(resolve, 1000));
      }
      if (!disposed.current && !pollCancelled.current) { setBrowserLogin(null); feedback.error(t("account.loginTimeout")); }
    } catch (error) { if (!disposed.current) { feedback.error(String(error)); setBrowserLogin(null); } }
    finally { if (!disposed.current) setBusy(false); }
  };

  // 浏览器授权码登录（PKCE 回环回调）
  const startLogin = async () => {
    if (busy) return;
    setBusy(true); setBrowserLogin(null); pollCancelled.current = false;
    try {
      const next = await api.authStartBrowserLogin();
      // 弹窗在启动期间被关闭：取消态已建立，不进入等待视图
      if (pollCancelled.current) { setBusy(false); return; }
      setBrowserLogin(next);
      await api.openUrl(next.authorize_url);
      void pollBrowser(next);
    }
    catch (error) { const text = String(error); feedback.error(text.includes("unsupported_country_region_territory") ? t("account.regionBlocked") : text); setBusy(false); }
  };

  const cancelBrowserLogin = () => {
    pollCancelled.current = true;
    void api.authCancelBrowserLogin().catch(() => {});
    setBrowserLogin(null);
    setBusy(false);
  };

  const removeAccount = async (accountId: string, login: string) => {
    if (!await feedback.confirm({ title: t("account.removeTitle"), description: t("account.removeDescription", { login }), confirmText: t("account.remove"), destructive: true })) return;
    try { await api.authRemoveAccount(accountId); feedback.success(t("account.removedToast")); await refreshStatus(); }
    catch (error) { feedback.error(String(error)); }
  };

  const page = (content: ReactNode) => (
    <section className="accounts-page apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <header className="apple-page-bar justify-between gap-4">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px]">
            {chatgptLogo ? <img src={chatgptLogo} alt="" className="h-[18px] w-[18px] invert dark:invert-0" /> : null}
          </span>
          <span className="apple-title">{t("account.sectionTitle")}</span>
        </div>
        <button type="button" className="apple-action-button app-button--primary" disabled={busy} onClick={() => setAddOpen(true)}><Plus className="h-4 w-4" strokeWidth={2} />{t("account.addAnother")}</button>
      </header>
      <div className="apple-edit-content">{content}</div>
      <AddAccountDialog
        open={addOpen}
        onOpenChange={(next) => {
          setAddOpen(next);
          // 等待授权期间关闭弹窗 = 取消登录，避免留下无人认领的轮询
          if (!next && (browserLogin || busy)) cancelBrowserLogin();
        }}
        busy={busy}
        browserLogin={browserLogin}
        onStartLogin={() => void startLogin()}
        onReopen={() => { if (browserLogin) void api.openUrl(browserLogin.authorize_url); }}
        onCancel={cancelBrowserLogin}
      />
    </section>
  );

  if (status.authenticated) return page(
    <div className="grid grid-cols-1 gap-[var(--gap-card)] md:grid-cols-2">
      {status.external.map((account) => (
        <AccountCard key={account.id} source="desktop" accountId={account.id} login={account.login} plan={account.plan_type} expiresAt={account.subscription_active_until} cachedBalance={balanceCache?.[authQuotaCacheKey("desktop", account.id)]} onRefreshed={refreshStatus} active={active} />
      ))}
      {status.accounts.map((account) => <AccountCard key={`${account.id}:${account.authenticated_at}`} source="oauth" accountId={account.id} login={account.login} plan={account.plan_type} expiresAt={account.subscription_active_until} cachedBalance={balanceCache?.[authQuotaCacheKey("oauth", account.id)]} onRefreshed={refreshStatus} active={active} onRemove={() => void removeAccount(account.id, account.login)} onRelogin={() => { setAddOpen(true); void startLogin(); }} reloginDisabled={busy} />)}
    </div>
  );

  return page(<div><div className="apple-group p-3"><div className="flex items-start gap-3"><span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-accent/10 text-accent"><ShieldCheck className="h-[18px] w-[18px]" strokeWidth={2} /></span><div><div className="setting-title">{t("account.notConnected")}</div><p className="setting-description mt-0.5">{t("account.notConnectedDescription")}</p></div></div></div>{loadError ? <p className="muted mt-3 text-sm">{loadError}</p> : null}</div>);
}
