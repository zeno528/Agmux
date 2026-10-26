import { Camera, Layers2, Plus, RotateCw, Undo2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { DndContext, DragOverlay, closestCenter } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { createPortal } from "react-dom";
import { api } from "../../api";
import { connectionErrorMessage, connectionExceptionMessage } from "../profiles/connectionText";
import { useFeedback } from "../../app/Feedback";
import { AppSegmentedControl } from "../../components/AppSegmentedControl";
import { SyncFailureIndicator } from "../../components/SyncFailureIndicator";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import SortableCard, { CardDragPreview } from "../../components/SortableCard";
import { useCardDragReorder } from "../../components/useCardDragReorder";
import { ProfileCardActions, ProfileCardContent, connectionGate } from "../profiles/ProfileCard";
import ProfileNameDialog from "../profiles/ProfileNameDialog";
import ClaudeDesktopProfileEdit from "./ClaudeDesktopProfileEdit";
import type { ClaudeDesktopAppStatus, ClaudeDesktopProfileDetail, ClaudeDesktopProfileSummary, SyncFailure } from "../../types";
import { applyProfileWithConfirmation } from "../profiles/applyProfile";
import { useProfileBalance } from "../profiles/useProfileBalance";
import { claudeBalanceQueryKinds } from "../../presets";

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

function ClaudeDesktopProfileCard({ profile, active, busy, testing, onEdit, onApply, onDuplicate, onTest, onRemove }: {
  profile: ClaudeDesktopProfileSummary;
  active: boolean;
  busy: boolean;
  testing: boolean;
  onEdit: () => void;
  onApply: () => void;
  onDuplicate: () => void;
  onTest: () => void;
  onRemove: () => void;
}) {
  const feedback = useFeedback();
  const { t: tProfiles } = useTranslation("profiles");
  const { t: tClaude } = useTranslation("claude");
  const supportsBalance = claudeBalanceQueryKinds.has(profile.kind ?? "");
  const balance = useProfileBalance({
    profileId: profile.id,
    showBalance: profile.show_balance,
    supportsBalance,
    hasCredential: profile.has_key,
    active,
    activationEpoch: 0,
    coldStart: false,
    source: "claude-desktop",
  });
  const connection = connectionGate(Boolean(profile.base_url), profile.has_key, {
    ready: tClaude("testConnection"),
    missingEndpoint: tClaude("checkProviderFields"),
    missingKey: tClaude("checkProviderFields"),
  });
  return (
    <SortableCard id={profile.id} active={active} onClick={onEdit} title={tProfiles("card.clickToEdit")} handleTitle={tProfiles("card.dragToReorder")}>
      <ProfileCardContent profile={cardProfile(profile)} balanceInfos={balance.balanceInfos} balanceError={balance.balanceError} balanceRefreshing={balance.balanceRefreshing} onRefreshBalance={balance.refreshBalance} onOpenAdmin={() => void api.openUrl(profile.admin_url!).catch((error) => feedback.error(String(error)))} />
      <ProfileCardActions active={active} busy={busy} testing={testing} connectionDisabled={connection.disabled} connectionTitle={connection.title} onApply={onApply} onDuplicate={onDuplicate} onTest={onTest} onRemove={onRemove} />
    </SortableCard>
  );
}

// 页面数据只在进页时加载（AppShell 按 view 挂载本组件），不入启动关键路径。
export default function ClaudeDesktopProfilesView({ activeId, status, onChanged, popEpoch, syncFailure }: { activeId: string | null; status: ClaudeDesktopAppStatus; onChanged: () => void; popEpoch: number; syncFailure: SyncFailure | undefined }) {
  const feedback = useFeedback();
  const { t } = useTranslation("claudeDesktop");
  const { t: tProfiles } = useTranslation("profiles");
  const { t: tClaude } = useTranslation("claude");
  const [items, setItems] = useState<ClaudeDesktopProfileSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [restarting, setRestarting] = useState(false);
  // 只有捕获弹窗需要取名；卡片改名走编辑页（通用 rename 命令只覆盖 codex/claude 两表）
  const [modal, setModal] = useState<"capture" | null>(null);
  const [profileName, setProfileName] = useState("");
  const [editingProfile, setEditingProfile] = useState<ClaudeDesktopProfileSummary | null>(null);
  const [editDetail, setEditDetail] = useState<ClaudeDesktopProfileDetail | null>(null);
  const [creatingProfile, setCreatingProfile] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setItems(await api.claudeDesktopListProfiles());
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setLoaded(true);
    }
  };

  useEffect(() => { void refresh(); }, []);

  // 侧栏重复点击本视图 = 弹回一级（对齐 ClaudeProfilesView）。
  useEffect(() => {
    if (!popEpoch) return;
    setEditingProfile(null);
    setEditDetail(null);
    setCreatingProfile(false);
  }, [popEpoch]);

  const persistOrder = async (previous: ClaudeDesktopProfileSummary[], next: ClaudeDesktopProfileSummary[]) => {
    try {
      await api.claudeDesktopReorderProfiles(next.map((item) => item.id));
    } catch (error) {
      setItems(previous);
      feedback.error(String(error));
    }
  };
  const { sensors, dragPreview, onDragStart, onDragEnd, onDragCancel } = useCardDragReorder(items, setItems, persistOrder);

  const submitModal = async () => {
    if (busy || modal !== "capture" || !profileName.trim()) return;
    setBusy(true);
    try {
      await api.claudeDesktopCaptureProfile(profileName.trim());
      feedback.success(tProfiles("feedback.captureSuccess"));
      setModal(null);
      await refresh();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const openEdit = async (profile: ClaudeDesktopProfileSummary) => {
    try {
      const detail = await api.claudeDesktopGetProfile(profile.id);
      setEditDetail(detail);
      setEditingProfile(profile);
    } catch (error) {
      feedback.error(String(error));
    }
  };

  const closeEdit = async () => {
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

  if (creatingProfile || editingProfile) {
    return (
      <ClaudeDesktopProfileEdit
        profile={editingProfile}
        create={creatingProfile}
        initialDetail={editDetail}
        onBack={() => void closeEdit()}
        onChanged={() => {
          void refresh();
          onChanged();
        }}
      />
    );
  }

  const applyDesktopProfile = async (profile: ClaudeDesktopProfileSummary) => {
    if (busy) return;
    setBusy(true);
    try {
      const applied = await applyProfileWithConfirmation(
        (overwrite) => api.claudeDesktopApplyProfile(profile.id, overwrite),
        () => feedback.confirm({ title: t("writeback.overwriteTitle", { ns: "common" }), description: t("writeback.overwriteDescription", { ns: "common", file: "claude_desktop_config.json", name: profile.name }), confirmText: t("writeback.overwriteConfirm", { ns: "common" }), destructive: true }),
      );
      if (!applied) return;
      feedback.success(t("status.applyRestartHint"));
      onChanged();
    } catch (error) {
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

  // 状态与操作全部并入标题行：chip 显示安装/运行/模式（悬浮看路径），路由滑块与添加供应商同区。
  const modeLabel = status.deployment_mode === "3p" ? t("status.mode3p") : status.deployment_mode === "1p" ? t("status.mode1p") : t("status.modeUnknown");
  const activeProfile = items.find((item) => item.id === activeId) ?? null;

  // 头部直连/路由切换 = 改当前激活 profile 的模式并重新应用（写盘后仍需完全重启桌面端）
  const switchMode = async (target: "direct" | "route") => {
    const current = items.find((item) => item.id === activeId);
    if (busy || !current || (current.mode ?? "direct") === target) return;
    setBusy(true);
    try {
      const detail = await api.claudeDesktopGetProfile(current.id);
      await api.claudeDesktopSaveProfile({
        id: detail.id, name: detail.name, kind: detail.kind,
        baseUrl: detail.base_url, apiKey: detail.api_key, authScheme: detail.auth_scheme,
        models: detail.models, adminUrl: detail.admin_url, icon: detail.icon,
        showBalance: detail.show_balance, description: detail.description,
        mode: target, upstreamBaseUrl: detail.upstream_base_url, upstreamApiKey: detail.upstream_api_key,
        fetchedModels: detail.fetched_models,
      });
      const applied = await applyProfileWithConfirmation(
        (overwrite) => api.claudeDesktopApplyProfile(current.id, overwrite),
        () => feedback.confirm({ title: t("writeback.overwriteTitle", { ns: "common" }), description: t("writeback.overwriteDescription", { ns: "common", file: "claude_desktop_config.json", name: current.name }), confirmText: t("writeback.overwriteConfirm", { ns: "common" }), destructive: true }),
      );
      if (applied) {
        feedback.success(t("status.applyRestartHint"));
        await refresh();
        onChanged();
      }
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="apple-scroll-page mx-auto w-full max-w-none">
      <header className="apple-page-bar flex-wrap justify-between gap-4">
        <div className="provider-page-brand">
          <img src="/claude.svg" alt="" className="provider-page-brand__logo" draggable="false" />
          <span>{t("list.title")}</span>
          <span
            className={`apple-chip ${status.installed ? (status.running ? "chip-success" : "") : "chip-warn"}`}
            title={status.installed ? `${t("status.path")}: ${status.display_path}` : undefined}
          >
            {status.installed ? `${status.running ? t("status.running") : t("status.notRunning")} · ${modeLabel}` : t("status.notInstalled")}
          </span>
          <SyncFailureIndicator failure={syncFailure} />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <AppSegmentedControl
            className="h-[var(--toolbar-control-height)] shrink-0"
            role="tablist"
            label={t("status.modeSwitchLabel")}
            selectedIndex={activeProfile?.mode === "route" ? 1 : 0}
          >
            <button type="button" role="tab" aria-selected={(activeProfile?.mode ?? "direct") === "direct"} className="apple-action-button" disabled={busy || !activeProfile} title={!activeProfile ? t("status.switchNeedsActive") : undefined} onClick={() => void switchMode("direct")}>{t("status.directMode")}</button>
            <button type="button" role="tab" aria-selected={activeProfile?.mode === "route"} className="apple-action-button" disabled={busy || !activeProfile} title={!activeProfile ? t("status.switchNeedsActive") : undefined} onClick={() => void switchMode("route")}>{t("status.routeMode")}</button>
          </AppSegmentedControl>
          <button type="button" className="apple-action-button" aria-label={t("status.restartButton")} title={t("status.restartButton")} disabled={busy || restarting || !status.installed} onClick={() => void restartDesktop()}>
            <RotateCw className="h-4 w-4" />
          </button>
          <button type="button" className="apple-action-button" aria-label={t("status.restoreOfficial")} title={t("status.restoreOfficial")} disabled={busy} onClick={() => void restoreOfficial()}>
            <Undo2 className="h-4 w-4" />
          </button>
          <button type="button" className="apple-action-button app-button--primary disabled:!opacity-100" disabled={busy} onClick={() => { if (!busy) setCreatingProfile(true); }}>
            <Plus className="h-4 w-4" />
            {tProfiles("toolbar.addProvider")}
          </button>
        </div>
      </header>
      <div className="apple-edit-content">
        {!status.installed ? (
          <EmptyStateCard icon={<Layers2 className="h-5 w-5" strokeWidth={2} />}>
            <p className="muted">{t("status.notInstalledHint")}</p>
          </EmptyStateCard>
        ) : !loaded ? null : items.length === 0 ? (
          <EmptyStateCard icon={<Layers2 className="h-5 w-5" strokeWidth={2} />}>
            <p className="muted">{t("list.emptyHint")}</p>
            <button type="button" className="apple-action-button app-button--primary" disabled={busy} onClick={() => { setProfileName(""); setModal("capture"); }}>
              <Camera className="h-4 w-4" strokeWidth={2} />
              {tProfiles("toolbar.capture")}
            </button>
          </EmptyStateCard>
        ) : (
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragStart={onDragStart} onDragCancel={onDragCancel} onDragEnd={onDragEnd}>
            <SortableContext items={items.map((item) => item.id)} strategy={verticalListSortingStrategy}>
              <div className="profile-list relative space-y-[var(--gap-page)]">
                {items.map((profile) => (
                  <ClaudeDesktopProfileCard key={profile.id} profile={profile} active={profile.id === activeId} busy={busy} testing={testingId === profile.id} onEdit={() => void openEdit(profile)} onApply={() => void applyDesktopProfile(profile)} onDuplicate={() => void duplicateProfile(profile)} onTest={() => void testProfile(profile)} onRemove={() => void deleteProfile(profile)} />
                ))}
              </div>
            </SortableContext>
            {createPortal(
              <DragOverlay dropAnimation={null}>
                {dragPreview ? <CardDragPreview card={dragPreview} /> : null}
              </DragOverlay>,
              document.body,
            )}
          </DndContext>
        )}
      </div>
      <ProfileNameDialog mode={modal} name={profileName} busy={busy} onName={setProfileName} onClose={() => setModal(null)} onSubmit={() => void submitModal()} />
    </section>
  );
}
