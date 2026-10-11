import { Camera, Layers2, Play, Plus, RefreshCw } from "lucide-react";
import { DndContext, DragOverlay, closestCenter } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { createPortal } from "react-dom";
import { useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { authQuotaErrorKind } from "../../app/authQuotaCache";
import { useFeedback } from "../../app/Feedback";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { CliUpgradePill } from "../../components/CliUpgradePill";
import { useCardDragReorder } from "../../components/useCardDragReorder";
import type { AppState, CodexProfileDetail, CodexProfileSummary } from "../../types";
import CodexProfileCard from "./CodexProfileCard";
import { CardDragPreview } from "../../components/SortableCard";
import CodexProfileEdit from "./CodexProfileEdit";
import ProfileNameDialog from "../profiles/ProfileNameDialog";

interface ProfilesViewProps {
  state: AppState;
  authStatusReady: boolean;
  activationEpoch: number;
  /// 本次进程启动还没走完（首屏尚未出窗）。只有这个窗口内的余额刷新值得延后——
  /// 它会跟首屏抢资源；窗口已经起来之后，切页和聚焦都不该再等。
  coldStart: boolean;
  onRefresh: () => Promise<void>;
  onManageChatgptAccounts: () => void;
  /// 侧栏重复点击本视图时递增；视图据此弹回一级。一级时已是初始值，setState 同值跳过，零反应。
  popEpoch: number;
}

export function codexActionFor(running: boolean) {
  return running ? "restart" : "start";
}

export default function CodexProfilesView({ state, authStatusReady, activationEpoch, coldStart, onRefresh, onManageChatgptAccounts, popEpoch }: ProfilesViewProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("profiles");
  const [items, setItems] = useState(state.codex_profiles);
  const [busy, setBusy] = useState(false);
  const [codexAction, setCodexAction] = useState<"restart" | "start" | null>(null);
  const [editingProfile, setEditingProfile] = useState<CodexProfileSummary | null>(null);
  const [editDetail, setEditDetail] = useState<CodexProfileDetail | null>(null);
  const [creatingProfile, setCreatingProfile] = useState(false);
  const [modal, setModal] = useState<"capture" | "rename" | null>(null);
  const [modalProfile, setModalProfile] = useState<CodexProfileSummary | null>(null);
  const [profileName, setProfileName] = useState("");
  const duplicatingProfileRef = useRef(false);

  useEffect(() => setItems(state.codex_profiles), [state.codex_profiles]);

  // 侧栏重复点击本视图 = 弹回一级；只清子页状态，不刷新（保存流程自带刷新，弹回不产生数据变化）。
  useEffect(() => {
    if (!popEpoch) return;
    setEditingProfile(null);
    setEditDetail(null);
    setCreatingProfile(false);
  }, [popEpoch]);

  const persistOrder = async (previous: CodexProfileSummary[], next: CodexProfileSummary[]) => {
    try {
      await api.codexReorderProfiles(next.map((item) => item.id));
      await onRefresh();
    } catch (error) {
      setItems(previous);
      feedback.error(String(error));
      await onRefresh();
    }
  };
  const { sensors, dragPreview, onDragStart, onDragEnd, onDragCancel } = useCardDragReorder(items, setItems, persistOrder);

  const openCapture = () => { setModal("capture"); setModalProfile(null); setProfileName(""); };
  const openRename = (profile: CodexProfileSummary) => { setModal("rename"); setModalProfile(profile); setProfileName(profile.name); };

  const submitModal = async () => {
    if (busy || !modal) return;
    setBusy(true);
    try {
      if (modal === "capture") {
        await api.codexCaptureProfile(profileName.trim());
        feedback.success(t("feedback.captureSuccess"));
      } else if (modalProfile) {
        await api.renameProfile(modalProfile.id, profileName.trim());
        feedback.success(t("feedback.providerRenamed"));
      }
      setModal(null);
      await onRefresh();
    } catch (error) { feedback.error(String(error)); }
    finally { setBusy(false); }
  };

  const restart = async (force = false, notifySuccess = true) => {
    if (busy && !force) return null;
    setBusy(true);
    // 这次调用到底是"启动"还是"重启"由点击时的 Codex 状态决定，通知文案跟随它，不写死"已重启"
    const action = codexActionFor(state.codex.running);
    setCodexAction(action);
    try {
      await api.restartCodex();
      if (notifySuccess) feedback.success(t(action === "restart" ? "feedback.codexRestarted" : "feedback.codexStarted"));
      await onRefresh();
      return action;
    } catch (error) {
      feedback.error(String(error));
      return null;
    } finally {
      setCodexAction(null);
      setBusy(false);
    }
  };

  const codexApplyProfile = async (profile: CodexProfileSummary) => {
    if (busy) return;
    setBusy(true);
    try {
      await api.codexApplyProfile(profile.id);
      await onRefresh();
      if (!state.settings.auto_restart || await restart(true, false)) {
        feedback.success(t("feedback.applySuccess", { ns: "common" }));
      }
    } catch (error) {
      const message = String(error);
      // 凭证失效类错误出本地化的可行动文案，其余保持后端原文
      feedback.error(authQuotaErrorKind(message) === "auth_expired" ? t("balance.authInvalidToast") : message);
    }
    finally { setBusy(false); }
  };

  const removeProfile = async (profile: CodexProfileSummary) => {
    const confirmed = await feedback.confirm({ title: t("confirm.deleteTitle"), description: <Trans ns="profiles" i18nKey="confirm.deleteConfirm" values={{ name: profile.name }} components={{ strong: <strong /> }} />, confirmText: t("confirm.delete"), destructive: true });
    if (!confirmed) return;
    const previousIndex = items.findIndex((item) => item.id === profile.id);
    setItems((current) => current.filter((item) => item.id !== profile.id));
    try {
      await api.codexDeleteProfile(profile.id);
      feedback.success(t("feedback.providerDeleted"));
      await onRefresh();
    } catch (error) {
      setItems((current) => {
        if (current.some((item) => item.id === profile.id)) return current;
        const index = Math.max(0, Math.min(previousIndex, current.length));
        return [...current.slice(0, index), profile, ...current.slice(index)];
      });
      feedback.error(String(error));
    }
  };

  const codexDuplicateProfile = async (profile: CodexProfileSummary) => {
    if (busy || duplicatingProfileRef.current) return;
    duplicatingProfileRef.current = true;
    try { await api.codexDuplicateProfile(profile.id); feedback.success(t("feedback.providerDuplicated")); await onRefresh(); }
    catch (error) { feedback.error(String(error)); }
    finally { duplicatingProfileRef.current = false; }
  };

  const closeEdit = async () => { setEditingProfile(null); setEditDetail(null); setCreatingProfile(false); await onRefresh(); };
  // 先预载详情再切换：ProfileEdit 在详情未就绪时整页返回 null，直接切换会空一帧（进入编辑页的闪烁）。
  // 预载失败不拦截切换，ProfileEdit 内部按原有失败路径展示错误。
  const openEdit = async (profile: CodexProfileSummary) => {
    let detail: CodexProfileDetail | null = null;
    try {
      detail = await api.codexGetProfile(profile.id);
    } catch { /* 详情由 ProfileEdit 挂载后重取 */ }
    setEditDetail(detail);
    setEditingProfile(profile);
  };

  if (editingProfile || creatingProfile) {
    return <CodexProfileEdit profile={editingProfile} create={creatingProfile} initialDetail={editDetail} authStatus={state.auth_status} authStatusReady={authStatusReady} onBack={() => void closeEdit()} onChanged={() => void onRefresh()} onManageChatgptAccounts={onManageChatgptAccounts} />;
  }

  const nextCodexAction = codexActionFor(state.codex.running);

  return (
    <section className="apple-scroll-page mx-auto w-full max-w-none">
      <header className="apple-page-bar flex-wrap justify-between gap-4">
        <div className="provider-page-brand">
          <img src="/codex.svg" alt="" className="provider-page-brand__logo" draggable="false" />
          <span>Codex</span>
          <CliUpgradePill client="codex" />
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-2 text-sm">
          <div className={`codex-status-control codex-status--${state.codex.running ? "running" : "stopped"} text-xs font-medium`}>
            <span className="codex-status" role="status" aria-live="polite" aria-atomic="true">
              <span className="codex-status__signal" aria-hidden="true"><span className="codex-status__signal-dot" /></span>
              <span className="codex-status__name">Codex</span>
              <span className="codex-status__label">{state.codex.running ? t("status.running") : t("status.stopped")}</span>
            </span>
            <button type="button" className="codex-status__action" disabled={busy} title={t(`toolbar.${nextCodexAction}`)} aria-label={codexAction ? t(`toolbar.${codexAction}ing`) : t(`toolbar.${nextCodexAction}`)} onClick={() => void restart(false)}>
              {/* 动作语义分离：重启全程 RefreshCw 自旋（重启内部先停后启，nextCodexAction 中途会翻转，图标以进行中动作优先）；
                  启动动作用项目旋转指示器 LoadingSpinner，Play 只在空闲态出现 */}
              {codexAction === "start" ? <LoadingSpinner size="md" /> : (codexAction ?? nextCodexAction) === "restart" ? <RefreshCw className={`h-4 w-4 ${codexAction ? "animate-spin" : ""}`} strokeWidth={2} /> : <Play className="h-4 w-4" strokeWidth={2} />}
            </button>
          </div>
          <button type="button" className="apple-action-button app-button--primary disabled:!opacity-100" disabled={busy}
            onClick={() => setCreatingProfile(true)}>
            <Plus className="h-4 w-4" strokeWidth={2} />{t("toolbar.addProvider")}
          </button>
        </div>
      </header>
      <div className="apple-edit-content">
            <div>{items.length === 0 ? <EmptyStateCard icon={<Layers2 className="h-5 w-5" strokeWidth={2} />}><p className="muted">{t("empty.description")}</p><button type="button" className="apple-action-button app-button--primary" disabled={busy} onClick={openCapture}><Camera className="h-4 w-4" strokeWidth={2} />{t("toolbar.capture")}</button></EmptyStateCard> : <DndContext sensors={sensors} collisionDetection={closestCenter} onDragStart={onDragStart} onDragCancel={onDragCancel} onDragEnd={onDragEnd}><SortableContext items={items.map((item) => item.id)} strategy={verticalListSortingStrategy}><div className="profile-list relative space-y-[var(--gap-page)]">{items.map((profile) => <CodexProfileCard key={profile.id} profile={profile} active={profile.id === state.active_codex_profile_id} busy={busy} activationEpoch={activationEpoch} coldStart={coldStart} balanceCache={state.balance_cache} onChanged={onRefresh} onApply={() => void codexApplyProfile(profile)} onRename={() => openRename(profile)} onEdit={() => void openEdit(profile)} onRemove={() => void removeProfile(profile)} onDuplicate={() => void codexDuplicateProfile(profile)} />)}</div></SortableContext>{createPortal(<DragOverlay dropAnimation={null}>{dragPreview ? <CardDragPreview card={dragPreview} /> : null}</DragOverlay>, document.body)}</DndContext>}</div>
      </div>
      <ProfileNameDialog mode={modal} name={profileName} busy={busy} onName={setProfileName} onClose={() => setModal(null)} onSubmit={() => void submitModal()} />
    </section>
  );
}
