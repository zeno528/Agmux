import { Check, Copy, Gauge, Globe, MoreHorizontal, Wifi } from "lucide-react";
import { useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { balanceChipClass, balanceQueryProviders, usageQueryProviders } from "../../presets";
import type { ProfileBalanceInfo, CodexProfileSummary } from "../../types";
import { PlanBadge } from "../../components/PlanBadge";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ProfileIconTile } from "../../components/ProfileIconTile";
import { TrashIcon } from "../../components/TrashIcon";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { useMenuDismiss } from "../../components/useMenuDismiss";
import { localizeBalanceLabel } from "./balanceLabel";
import ProfileTerminalMenu from "./ProfileTerminalMenu";

interface ProfileCardContentProps {
  profile: Pick<CodexProfileSummary, "name" | "icon" | "kind" | "provider" | "model" | "reasoning_effort" | "plan_type" | "admin_url" | "show_balance">;
  balanceInfos: ProfileBalanceInfo[];
  balanceError: string;
  balanceRefreshing: boolean;
  hideModel?: boolean;
  onRefreshBalance?: (manual?: boolean) => void;
  onOpenAdmin?: () => void;
  onRename?: () => void;
}

export function ProfileCardContent({
  profile,
  balanceInfos,
  balanceError,
  balanceRefreshing,
  hideModel = false,
  onRefreshBalance,
  onOpenAdmin,
  onRename,
}: ProfileCardContentProps) {
  const { t } = useTranslation("profiles");
  const balanceInfo = balanceInfos[0] ?? null;
  const isSubscriptionProfile = profile.kind === "official";
  const supportsBalance = isSubscriptionProfile || balanceQueryProviders.has(profile.provider ?? "");
  const isUsageProvider = usageQueryProviders.has(profile.provider ?? "");
  // 后端回传的窗口标签按当前语言换词；后端没给时才用本语言兜底（映射见 balanceLabel.ts）
  const primaryLabel = localizeBalanceLabel(balanceInfo?.usage_label, t) ?? (isUsageProvider ? t("balance.window5h") : t("card.usage"));
  const weeklyLabel = localizeBalanceLabel(balanceInfo?.weekly_label, t) ?? (isUsageProvider ? t("balance.window7d") : t("balance.period"));
  const balanceLabel = isSubscriptionProfile || isUsageProvider ? t("card.usage") : t("card.balance");
  const primaryUsagePercent = balanceInfo?.usage_percent != null ? (isSubscriptionProfile ? 100 - balanceInfo.usage_percent : balanceInfo.usage_percent) : null;
  const weeklyUsagePercent = balanceInfo?.weekly_usage_percent != null ? (isSubscriptionProfile ? 100 - balanceInfo.weekly_usage_percent : balanceInfo.weekly_usage_percent) : null;
  const primaryUsageText = isSubscriptionProfile ? t("card.usageRemaining", { label: primaryLabel }) : isUsageProvider ? `${primaryLabel}:` : `${primaryLabel} `;
  const weeklyUsageText = isSubscriptionProfile ? t("card.usageRemaining", { label: weeklyLabel }) : isUsageProvider ? `${weeklyLabel}:` : `${weeklyLabel} `;

  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <ProfileIconTile name={profile.name} icon={profile.icon} overlay={
        profile.admin_url ? (
          <button
            type="button"
            className="absolute inset-0 grid cursor-pointer place-items-center rounded-xl bg-(--provider-icon-bg) text-accent opacity-0 outline-none transition-opacity duration-150 group-hover/tile:opacity-100 focus-visible:opacity-100"
            title={t("card.openWebsite")}
            aria-label={t("card.openWebsite")}
            onClick={(event) => { event.stopPropagation(); onOpenAdmin?.(); }}
          ><Globe className="h-4.5 w-4.5" strokeWidth={2} aria-hidden="true" /></button>
        ) : null
      } />
      <div className="profile-card-content__text min-w-0 flex-1">
        <div className="flex min-h-7 items-center gap-2">
          <h3 className="title-md cursor-pointer truncate leading-normal transition-colors hover:text-accent" title={t("card.clickToRename")} onClick={(event) => { event.stopPropagation(); onRename?.(); }}>{profile.name}</h3>
          {isSubscriptionProfile ? <PlanBadge plan={profile.plan_type} /> : null}
        </div>
        {!hideModel || (supportsBalance && profile.show_balance) ? <div className="profile-card-meta muted mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-xs">
          {!hideModel ? <><span className="min-w-0 truncate">{profile.model ?? t("card.notSet")}</span>{profile.reasoning_effort ? <><span aria-hidden="true">·</span><span>{profile.reasoning_effort}</span></> : null}</> : null}
          {supportsBalance && profile.show_balance ? <button type="button" className="apple-chip" title={balanceError ? t("balance.queryFailedRetry") : t("balance.clickToRefresh")} aria-label={isSubscriptionProfile ? t("balance.chatgptQuota") : balanceLabel} aria-busy={balanceRefreshing} onClick={(event) => { event.stopPropagation(); onRefreshBalance?.(true); }}>
            {balanceRefreshing ? <LoadingSpinner size="sm" /> : <Gauge className={`h-3 w-3${balanceError ? " chip-danger" : ""}`} strokeWidth={2} aria-hidden="true" />}
            {balanceError ? <span>{t("balance.queryFailed")}</span> : primaryUsagePercent != null ? <><span>{primaryUsageText}</span><span className={balanceChipClass(balanceInfo?.usage_percent ?? null)}>{primaryUsagePercent}%</span>{balanceInfo?.usage_reset ? <span> {balanceInfo.usage_reset}</span> : null}{weeklyUsagePercent != null ? <><span> · {weeklyUsageText}</span><span className={balanceChipClass(balanceInfo?.weekly_usage_percent ?? null)}>{weeklyUsagePercent}%</span>{balanceInfo?.weekly_reset ? <span> {balanceInfo.weekly_reset}</span> : null}</> : null}</> : balanceInfo && !isUsageProvider ? <><span>{t("balance.balancePrefix")}</span>{balanceInfos.map((info, index) => <span key={info.currency || index} className="inline-flex items-center gap-1">{index > 0 ? <span aria-hidden="true">/</span> : null}<span className={balanceChipClass(null, info.total_balance)}>{info.total_balance.startsWith("-") ? "-" : ""}{info.currency === "USD" ? "$" : "¥"}{info.total_balance.replace(/^-/, "")}</span><span> {info.currency}</span></span>)}</> : <span>{`${balanceLabel} --`}</span>}
          </button> : null}
        </div> : null}
      </div>
    </div>
  );
}

/** 卡片测试连通按钮的通用门控与悬停文案：缺什么报什么，其余一律 ready 文案。
 *  Codex 与 Claude 卡片共用同一判定，避免按钮行各写一套。 */
export function connectionGate(hasBaseUrl: boolean, hasKey: boolean, titles: { ready: string; missingEndpoint: string; missingKey: string }) {
  const disabled = !hasBaseUrl || !hasKey;
  return {
    disabled,
    title: !disabled ? titles.ready : !hasBaseUrl ? titles.missingEndpoint : titles.missingKey,
  };
}

interface ProfileCardActionsProps {
  active: boolean;
  busy: boolean;
  testing: boolean;
  allowInactiveDeleteWhileBusy?: boolean;
  modelControl?: ReactNode;
  /** 测试连通按钮是否禁用（调用方按各自领域判定：缺地址/缺密钥）。 */
  connectionDisabled: boolean;
  connectionTitle: string;
  /** 传入供应商 id 才渲染终端入口：目前只有 Claude Code 卡片要，Codex 不传即不出现。 */
  terminalProfileId?: string;
  onApply?: () => void;
  onDuplicate?: () => void;
  onTest?: () => void;
  onRemove?: () => void;
}

export function ProfileCardActions({ active, busy, testing, allowInactiveDeleteWhileBusy = false, modelControl, connectionDisabled, connectionTitle, terminalProfileId, onApply, onDuplicate, onTest, onRemove }: ProfileCardActionsProps) {
  const { t } = useTranslation("profiles");
  const [menuOpen, setMenuOpen] = useState(false);
  const menuTriggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuStyle = useFixedMenuPosition(menuOpen, menuTriggerRef.current, menuRef, "end");
  useMenuDismiss(menuOpen, menuTriggerRef, menuRef, setMenuOpen);
  const busyForDelete = busy && !allowInactiveDeleteWhileBusy;
  const menu = menuOpen ? createPortal(
    <div ref={menuRef} className="app-select-menu app-popover" data-open="true" data-popover-in role="menu" aria-label={t("actions.more")} style={{ ...menuStyle, minWidth: "10rem" }}>
      <button type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={connectionDisabled || testing} title={connectionTitle} onClick={() => { setMenuOpen(false); onTest?.(); }}>
        <span className="flex items-center gap-2">{testing ? <LoadingSpinner size="sm" /> : <Wifi className="h-4 w-4" strokeWidth={2} aria-hidden="true" />}{t("connection.test")}</span>
      </button>
      <button type="button" role="menuitem" className="app-select-option app-selection-state" onClick={() => { setMenuOpen(false); onDuplicate?.(); }}>
        <span className="flex items-center gap-2"><Copy className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{t("actions.duplicate")}</span>
      </button>
      <button type="button" role="menuitem" className="app-select-option app-selection-state app-select-option--danger disabled:cursor-not-allowed disabled:opacity-40" disabled={busyForDelete || active} onClick={() => { setMenuOpen(false); onRemove?.(); }}>
        <span className="flex items-center gap-2"><TrashIcon />{t("actions.delete")}</span>
      </button>
    </div>,
    document.body,
  ) : null;
  return (
    <div className="profile-card-actions flex shrink-0 items-center gap-2" title="" onClick={(event) => event.stopPropagation()} onMouseDown={(event) => event.preventDefault()}>
      {modelControl}
      <div className="profile-card-action-buttons flex shrink-0 items-center gap-2">
        <button type="button" className={`apple-action-button relative !h-[var(--icon-button-size)] !rounded-[var(--radius-control)] app-button--primary ${active ? "disabled:opacity-100" : "disabled:!opacity-100"}`} disabled={busy || active} aria-pressed={active} aria-label={active ? t("actions.inUse") : t("actions.switch")} title={active ? t("actions.inUse") : t("actions.switch")} onClick={onApply}>
          <span className={active ? "invisible" : ""} aria-hidden="true">{t("actions.switch")}</span>
          {active ? <Check className="absolute inset-0 m-auto" size={18} strokeWidth={2.5} aria-hidden="true" /> : null}
        </button>
        {terminalProfileId ? <ProfileTerminalMenu profileId={terminalProfileId} /> : null}
        <button ref={menuTriggerRef} type="button" className="apple-icon-button text-[var(--text-secondary)] hover:text-accent" aria-haspopup="menu" aria-expanded={menuOpen} aria-label={t("actions.more")} title={t("actions.more")} aria-busy={testing} onClick={() => setMenuOpen((open) => !open)}>{testing ? <LoadingSpinner size="md" /> : <MoreHorizontal className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />}</button>
        {menu}
      </div>
    </div>
  );
}
