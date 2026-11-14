import { useMenuExit } from "../../components/useMenuExit";
import { Activity, Check, ChevronDown, Layers2, Play, Plus, Plug, Route, RotateCw, Undo2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { DndContext, DragOverlay, closestCenter } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { createPortal } from "react-dom";
import { api } from "../../api";
import { AppTabBar } from "../../components/AppTabBar";
import { AppTooltip } from "../../components/AppTooltip";
import { AppSwitch } from "../../components/AppSwitch";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { useMenuDismiss } from "../../components/useMenuDismiss";
import { getCachedClaudeDesktopProfiles, loadClaudeDesktopProfiles, setClaudeDesktopProfilesCache } from "../../app/managementDataCache";
import { connectionErrorMessage, connectionExceptionMessage } from "../profiles/connectionText";
import { useFeedback } from "../../app/Feedback";
import { SyncFailureIndicator } from "../../components/SyncFailureIndicator";
import SortableCard, { CardDragPreview } from "../../components/SortableCard";
import { useCardDragReorder } from "../../components/useCardDragReorder";
import { ProfileCardActions, ProfileCardContent, connectionGate } from "../profiles/ProfileCard";
import ClaudeDesktopProfileEdit from "./ClaudeDesktopProfileEdit";
import ClaudeDesktopDiagnostics from "./ClaudeDesktopDiagnostics";
import type { ClaudeDesktopAppStatus, ClaudeDesktopMode, ClaudeDesktopProfileDetail, ClaudeDesktopProfileSummary, ProfileBalanceInfo, SyncFailure } from "../../types";
import { applyProfileWithConfirmation } from "../profiles/applyProfile";
import { useProfileBalance } from "../profiles/useProfileBalance";
import { claudeBalanceQueryKinds } from "../../presets";

function DesktopActionsMenu({ status, busy, restarting, onRestart, onRestore, onDiagnostics }: {
  status: ClaudeDesktopAppStatus;
  busy: boolean;
  restarting: boolean;
  onDiagnostics: () => void;
  onRestart: () => void;
  onRestore: () => void;
}) {
  const { t } = useTranslation("claudeDesktop");
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const diagnosticsPending = useRef(false);
  const position = useFixedMenuPosition(open, trigger.current, menu, "end");
  useMenuDismiss(open, trigger, menu, setOpen);
  useMenuExit(open, menu);
  const actions = [
    { label: t("diagnostics.title"), icon: Activity, disabled: false, run: () => {
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) onDiagnostics();
      else diagnosticsPending.current = true;
    } },
    { label: t(status.running ? "status.restartButton" : "status.startButton"), icon: status.running ? RotateCw : Play, disabled: !status.installed || restarting, run: onRestart },
    { label: t("status.restoreOfficial"), icon: Undo2, disabled: status.deployment_mode !== "3p", run: onRestore },
  ];
  return <>
    <button ref={trigger} type="button" className="apple-action-button" aria-haspopup="menu" aria-expanded={open} disabled={busy || restarting} onClick={() => { diagnosticsPending.current = false; setOpen((value) => !value); }}>
      {t("status.more")}<ChevronDown size={16} strokeWidth={2} aria-hidden="true" />
    </button>
    {(open || position) ? createPortal(<div ref={menu} className="app-select-menu app-popover" data-open={open} data-popover-in={open ? "" : undefined} inert={!open} role="menu" aria-label={t("status.more")} style={position}
      onAnimationEnd={(event) => {
        if (event.target !== event.currentTarget || event.animationName !== "app-select-menu-out" || !diagnosticsPending.current) return;
        diagnosticsPending.current = false;
        onDiagnostics();
      }}>
      {actions.map(({ label, icon: Icon, disabled, run }) => <button key={label} type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={busy || disabled} onClick={() => { setOpen(false); run(); }}>
        <span className="flex items-center gap-2"><Icon size={16} strokeWidth={2} aria-hidden="true" />{label}</span>
      </button>)}
    </div>, document.body) : null}
  </>;
}

function cardProfile(profile: ClaudeDesktopProfileSummary) {
  return {
    name: profile.name,
    icon: profile.icon,
    kind: "third_party" as const,
    provider: profile.kind ?? null,
    model: profile.model,
    reasoning_effort: null,
    plan_type: null,
    admin_url: profile.admin_url,
    show_balance: profile.show_balance,
  };
}

function ClaudeDesktopProfileCard({ profile, active, reapply, busy, testing, activationEpoch, coldStart, balanceCache, onEdit, onApply, onDuplicate, onTest, onRemove, member, onMember, defaultMember, onDefaultMember }: {
  member?: boolean;
  defaultMember?: boolean;
  onDefaultMember?: () => void;
  onMember?: () => void;
  profile: ClaudeDesktopProfileSummary;
  active: boolean;
  reapply: boolean;
  activationEpoch: number;
  coldStart: boolean;
  balanceCache: Record<string, ProfileBalanceInfo>;
  onApply?: () => void;
  busy: boolean;
  testing: boolean;
  onEdit: () => void;
  onDuplicate: () => void;
  onTest: () => void;
  onRemove: () => void;
}) {
  const feedback = useFeedback();
  const { t: tProfiles } = useTranslation("profiles");
  const { t: tClaude } = useTranslation("claude");
  const { t: tDesktop } = useTranslation("claudeDesktop");
  const supportsBalance = claudeBalanceQueryKinds.has(profile.kind ?? "");
  const balance = useProfileBalance({
    profileId: profile.id,
    showBalance: profile.show_balance,
    supportsBalance,
    hasCredential: profile.has_key,
    active,
    activationEpoch,
    coldStart,
    cachedBalance: balanceCache[profile.id],
    source: "claude-desktop",
  });
  const connection = connectionGate(Boolean(profile.base_url), profile.has_key, {
    ready: tClaude("testConnection"),
    missingEndpoint: tClaude("checkProviderFields"),
    missingKey: tClaude("checkProviderFields"),
  });
  return (
    <SortableCard id={profile.id} active={active && Boolean(onApply)} onClick={onEdit} title={tProfiles("card.clickToEdit")} handleTitle={tProfiles("card.dragToReorder")}>
      <ProfileCardContent profile={cardProfile(profile)} hideModel titleBadge={defaultMember ? tDesktop("modes.defaultProvider") : undefined} balanceInfos={balance.balanceInfos} balanceError={balance.balanceError} balanceRefreshing={balance.balanceRefreshing} onRefreshBalance={balance.refreshBalance} onOpenAdmin={() => void api.openUrl(profile.admin_url!).catch((error) => feedback.error(String(error)))} />
      <ProfileCardActions active={active} reapply={reapply} busy={busy} testing={testing}
        modelControl={onMember ? (
          <span className="inline-flex" title={tDesktop("modes.include")}>
            <AppSwitch checked={Boolean(member)} disabled={busy} onCheckedChange={onMember} label={`${tDesktop("modes.include")} ${profile.name}`} />
          </span>
        ) : undefined}
        firstMenuAction={onDefaultMember ? {
          label: tDesktop(defaultMember ? "modes.defaultProvider" : "modes.setDefault"),
          selected: defaultMember,
          onSelect: onDefaultMember,
        } : undefined}
        connectionDisabled={connection.disabled} connectionTitle={connection.title} onApply={onApply} onDuplicate={onDuplicate} onTest={onTest} onRemove={onRemove} />
    </SortableCard>
  );
}

interface ClaudeDesktopProfilesViewProps {
  activeId: string | null;
  status: ClaudeDesktopAppStatus;
  onChanged: () => Promise<void>;
  activationEpoch: number;
  coldStart: boolean;
  balanceCache: Record<string, ProfileBalanceInfo>;
  popEpoch: number;
  syncFailure: SyncFailure | undefined;
}

export function selectedAggregateMembers(status: Pick<ClaudeDesktopAppStatus, "members" | "selected_members">, activeId: string | null): string[] {
  return status.selected_members ?? [...new Set([...(activeId ? [activeId] : []), ...status.members])];
}

export default function ClaudeDesktopProfilesView({ activeId, status, onChanged, activationEpoch, coldStart, balanceCache, popEpoch, syncFailure }: ClaudeDesktopProfilesViewProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("claudeDesktop");
  const { t: tProfiles } = useTranslation("profiles");
  const { t: tClaude } = useTranslation("claude");
  const [mode, setMode] = useState<ClaudeDesktopMode>(status.mode);
  const [members, setMembers] = useState<string[]>(() => selectedAggregateMembers(status, activeId));
  const [defaultId, setDefaultId] = useState(activeId);
  const [editedActiveId, setEditedActiveId] = useState<string | null>(null);
  useEffect(() => { setMode(status.mode); }, [status.mode]);
  useEffect(() => { setDefaultId(activeId); }, [activeId]);
  useEffect(() => { setMembers(selectedAggregateMembers(status, activeId)); }, [activeId, status.members.join("\0"), status.selected_members?.join("\0")]);
  const cached = getCachedClaudeDesktopProfiles();
  const [items, setItems] = useState<ClaudeDesktopProfileSummary[]>(cached ?? []);
  const [loaded, setLoaded] = useState(cached !== null);
  const [busy, setBusy] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [editingProfile, setEditingProfile] = useState<ClaudeDesktopProfileSummary | null>(null);
  const [editDetail, setEditDetail] = useState<ClaudeDesktopProfileDetail | null>(null);
  const [creatingProfile, setCreatingProfile] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setItems(await loadClaudeDesktopProfiles(true));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setLoaded(true);
    }
  };

  useEffect(() => { void refresh(); }, [activationEpoch]);

  // 侧栏重复点击本视图 = 弹回一级（对齐 ClaudeProfilesView）。
  useEffect(() => {
    if (!popEpoch) return;
    setEditingProfile(null);
    setEditDetail(null);
    setCreatingProfile(false);
    setDiagnosticsOpen(false);
  }, [popEpoch]);

  const persistOrder = async (previous: ClaudeDesktopProfileSummary[], next: ClaudeDesktopProfileSummary[]) => {
    try {
      await api.claudeDesktopReorderProfiles(next.map((item) => item.id));
      setClaudeDesktopProfilesCache(next);
    } catch (error) {
      setItems(previous);
      feedback.error(String(error));
    }
  };
  const { sensors, dragPreview, onDragStart, onDragEnd, onDragCancel } = useCardDragReorder(items, setItems, persistOrder);

  const openEdit = async (profile: ClaudeDesktopProfileSummary) => {
    try {
      const detail = await api.claudeDesktopGetProfile(profile.id);
      setEditDetail(detail);
      setEditingProfile(profile);
    } catch (error) {
      feedback.error(String(error));
    }
  };

  const closeEdit = async (saved = false) => {
    if (saved && editingProfile?.id === activeId) setEditedActiveId(activeId);
    setEditingProfile(null);
    setEditDetail(null);
    setCreatingProfile(false);
    await refresh();
    onChanged();
  };

  const restartDesktop = async () => {
    if (restarting) return;
    setRestarting(true);
    try {
      await api.claudeDesktopRestart();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setRestarting(false);
    }
  };

  const restoreOfficial = async () => {
    if (busy) return;
    const confirmed = await feedback.confirm({
      title: t("status.restoreTitle"),
      description: t("status.restoreDescription"),
      confirmText: t("status.restoreOfficial"),
      destructive: true,
    });
    if (!confirmed) return;
    setBusy(true);
    try {
      await api.claudeDesktopRestoreOfficial();
      feedback.success(t("status.restoreDone"));
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  if (diagnosticsOpen) return <ClaudeDesktopDiagnostics activeProfile={items.find((profile) => profile.id === activeId)} onBack={() => setDiagnosticsOpen(false)} />;

  if (creatingProfile || editingProfile) {
    return (
      <ClaudeDesktopProfileEdit
        profile={editingProfile}
        create={creatingProfile}
        initialDetail={editDetail}
        onBack={(saved) => void closeEdit(saved)}
      />
    );
  }

  const applyDesktopProfile = async (profile: ClaudeDesktopProfileSummary) => {
    if (busy) return;
    let targetMode = mode;
    const nextMembers = targetMode === "aggregate"
      ? [...new Set([profile.id, ...members.filter((id) => items.some((item) => item.id === id))])]
      : status.members;
    setBusy(true);
    try {
      if (targetMode === "direct" && profile.requires_routing) {
        if (!await feedback.confirm({ title: t("modes.mappingTitle"), description: t("modes.mappingDescription"), confirmText: t("modes.useRouting") })) return;
        targetMode = "route";
      }
      const applied = await applyProfileWithConfirmation(
        (overwrite) => api.claudeDesktopApplyProfile(profile.id, overwrite, targetMode, nextMembers),
        () => feedback.confirm({ title: t("writeback.overwriteTitle", { ns: "common" }), description: t("writeback.overwriteDescription", { ns: "common", file: "claude_desktop_config.json", name: profile.name }), confirmText: t("writeback.overwriteConfirm", { ns: "common" }), destructive: true }),
      );
      if (!applied) return;
      setEditedActiveId(null);
      if (targetMode === "aggregate") setMembers(nextMembers);
      setMode(targetMode);
      feedback.success(t("status.applyRestartHint"));
      await refresh();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const toggleMember = async (id: string, asDefault = false) => {
    if (busy || restarting) return;
    const included = members.includes(id);
    if (asDefault && included) {
      setDefaultId(id);
      return;
    }
    const previous = members;
    const next = included ? members.filter((member) => member !== id) : [...members, id];
    setBusy(true);
    setMembers(next);
    try {
      await api.claudeDesktopSetMembers(next.filter((member) => items.some((item) => item.id === member)));
      await onChanged();
      if (asDefault) setDefaultId(id);
      else if (included && defaultId === id) setDefaultId(null);
    } catch (error) {
      setMembers(previous);
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const duplicateProfile = async (profile: ClaudeDesktopProfileSummary) => {
    if (busy) return;
    setBusy(true);
    try {
      await api.claudeDesktopDuplicateProfile(profile.id);
      feedback.success(tProfiles("feedback.providerDuplicated"));
      await refresh();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const testProfile = async (profile: ClaudeDesktopProfileSummary) => {
    if (testingId) return;
    setTestingId(profile.id);
    try {
      const result = await api.claudeDesktopTestProfile(profile.id);
      if (!result.ok) feedback.error(tClaude("connectionFailed", { error: connectionErrorMessage(result, tProfiles) }));
      else feedback.success(tClaude("connectionOk", { latency: ` · ${result.latency_ms}ms` }));
    } catch (error) {
      feedback.error(tClaude("connectionFailed", { error: connectionExceptionMessage(error, tProfiles) }));
    } finally {
      setTestingId(null);
    }
  };

  const deleteProfile = async (profile: ClaudeDesktopProfileSummary) => {
    if (busy) return;
    const confirmed = await feedback.confirm({
      title: tProfiles("confirm.deleteTitle"),
      description: <Trans ns="profiles" i18nKey="confirm.deleteConfirm" values={{ name: profile.name }} components={{ strong: <strong /> }} />,
      confirmText: tProfiles("confirm.delete"),
      destructive: true,
    });
    if (!confirmed) return;
    setBusy(true);
    try {
      await api.claudeDesktopDeleteProfile(profile.id);
      feedback.success(tProfiles("feedback.providerDeleted"));
      await refresh();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const appliedMembers = [...new Set([...(activeId ? [activeId] : []), ...status.members])];
  const membershipChanged = members.length !== appliedMembers.length || members.some((id) => !appliedMembers.includes(id));
  const aggregate = mode === "aggregate";
  const selectedDefault = items.find((profile) => profile.id === defaultId && members.includes(profile.id));
  const aggregateChanged = status.mode !== "aggregate" || defaultId !== activeId || membershipChanged || Boolean(editedActiveId);
  const aggregateApplied = !aggregateChanged && status.gateway_running;
  const modes = ["direct", "route", "aggregate"] as const;
  const icons = { direct: Plug, route: Route, aggregate: Layers2 };
  const working = busy || restarting;

  return (
    <section className="apple-scroll-page mx-auto w-full max-w-none">
      <header className="apple-page-bar flex-wrap justify-between gap-4">
        <div className="provider-page-brand">
          <img src="/claude.svg" alt="" className="provider-page-brand__logo" draggable="false" />
          <span>{t("list.title")}</span>
          <span className={`app-process-status codex-status--${status.running ? "running" : "stopped"}`} role="status" aria-live="polite" aria-atomic="true" title={status.installed ? `${t("status.path")}: ${status.display_path}` : undefined}>
            <span className="codex-status__signal" aria-hidden="true"><span className="codex-status__signal-dot" /></span>
            <span>{t(status.running ? "status.running" : status.installed ? "status.notRunning" : "status.notInstalled")}</span>
          </span>
          <SyncFailureIndicator failure={syncFailure} />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <DesktopActionsMenu status={status} busy={busy} restarting={restarting} onDiagnostics={() => setDiagnosticsOpen(true)} onRestart={() => void restartDesktop()} onRestore={() => void restoreOfficial()} />
          <button type="button" className="apple-action-button app-button--primary disabled:!opacity-100" disabled={working} onClick={() => setCreatingProfile(true)}>
            <Plus size={16} strokeWidth={2} aria-hidden="true" />{tProfiles("toolbar.addProvider")}
          </button>
        </div>
      </header>
      <div className="settings-tab-bar flex items-center gap-1">
        <AppTabBar selected={mode} onSelect={setMode} label={t("modes.label")} items={modes.map((id) => {
          const Icon = icons[id];
          return {
            id,
            label: t(`modes.${id}`),
            icon: <Icon size={16} strokeWidth={2} aria-hidden="true" />,
            count: activeId && status.mode === id ? (
              <span role="img" aria-label={t("modes.appliedMode")} title={t("modes.appliedMode")} className="codex-status--running shrink-0">
                <span className="codex-status__signal" aria-hidden="true"><span className="codex-status__signal-dot" /></span>
              </span>
            ) : undefined,
          };
        })} />
        <AppTooltip label={t("modes.label")}>
          <div className="field-subtitle">{t("modes.label")}</div>
          {modes.map((id) => <p key={id} className="muted meta-xs"><strong className="font-semibold">{t(`modes.${id}`)}</strong>{" · "}{t(`modes.${id}Hint`)}</p>)}
        </AppTooltip>
      </div>
      <div className="apple-edit-content">
        {activeId && status.mode !== "direct" && !status.gateway_running ? <p className="setting-description mb-4" role="status">{t("modes.gatewayStopped")}</p> : null}
        {!status.installed ? <p className="setting-description mb-4">{t("status.notInstalledHint")}</p> : null}
        {!loaded ? <EmptyStateCard icon={<Layers2 size={18} strokeWidth={2} />} loading>{null}</EmptyStateCard> : items.length === 0 ? (
          <EmptyStateCard icon={<Layers2 size={18} strokeWidth={2} />}>
            <p className="muted">{t("list.emptyHint")}</p>
          </EmptyStateCard>
        ) : (
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragStart={onDragStart} onDragCancel={onDragCancel} onDragEnd={onDragEnd}>
            <div className="profile-list relative space-y-[var(--gap-page)]">
              {aggregate ? (
                <div className={`apple-group flex items-center justify-between gap-4 px-5 py-3.5${aggregateApplied ? " is-active brand-gradient-surface" : ""}`} role="status">
                  <div className="flex min-w-0 items-center gap-3">
                    <Layers2 className="h-5 w-5 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
                    <div className="min-w-0">
                      <div className="field-label">{t("modes.aggregate")}</div>
                      <div className="muted flex flex-wrap gap-x-2 text-sm">
                        <span>{t("modes.memberCount", { count: items.filter((profile) => members.includes(profile.id)).length })}</span>
                        {!selectedDefault ? <span>{t("modes.chooseDefault")}</span> : aggregateChanged ? <span>{t("modes.pending")}</span> : null}
                      </div>
                    </div>
                  </div>
                  <div className="profile-card-actions flex shrink-0 items-center gap-2" title="">
                    <div className="profile-card-action-buttons flex shrink-0 items-center gap-2">
                      <button type="button" className={`apple-action-button relative !h-[var(--icon-button-size)] !rounded-[var(--radius-control)] app-button--primary shrink-0 ${aggregateApplied ? "disabled:opacity-100" : "disabled:!opacity-100"}`} disabled={working || !selectedDefault || aggregateApplied}
                        aria-pressed={aggregateApplied} aria-label={aggregateApplied ? tProfiles("actions.inUse") : t("modes.applyAggregate")} title={aggregateApplied ? tProfiles("actions.inUse") : t("modes.applyAggregate")} onClick={() => { if (selectedDefault) void applyDesktopProfile(selectedDefault); }}>
                        <span className={aggregateApplied ? "invisible" : ""} aria-hidden="true">{aggregateApplied ? tProfiles("actions.switch") : t("modes.applyAggregate")}</span>
                        {aggregateApplied ? <Check className="absolute inset-0 m-auto" size={18} strokeWidth={2.5} aria-hidden="true" /> : null}
                      </button>
                    </div>
                  </div>
                </div>
              ) : null}
              <SortableContext items={items.map((item) => item.id)} strategy={verticalListSortingStrategy}>
                {items.map((profile) => <ClaudeDesktopProfileCard key={profile.id} profile={profile}
                  active={profile.id === activeId} reapply={mode !== status.mode || profile.id === editedActiveId || (mode !== "direct" && !status.gateway_running)}
                  busy={working} testing={testingId === profile.id} activationEpoch={activationEpoch} coldStart={coldStart} balanceCache={balanceCache}
                  member={members.includes(profile.id)} onMember={aggregate ? () => void toggleMember(profile.id) : undefined}
                  defaultMember={aggregate && selectedDefault?.id === profile.id} onDefaultMember={aggregate ? () => void toggleMember(profile.id, true) : undefined}
                  onEdit={() => void openEdit(profile)} onApply={aggregate ? undefined : () => void applyDesktopProfile(profile)} onDuplicate={() => void duplicateProfile(profile)} onTest={() => void testProfile(profile)} onRemove={() => void deleteProfile(profile)} />)}
              </SortableContext>
            </div>
            {createPortal(<DragOverlay dropAnimation={null}>{dragPreview ? <CardDragPreview card={dragPreview} /> : null}</DragOverlay>, document.body)}
          </DndContext>
        )}
      </div>
    </section>
  );
}
