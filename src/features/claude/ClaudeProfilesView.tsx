import { Camera, Layers2, Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { DndContext, DragOverlay, closestCenter } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { createPortal } from "react-dom";
import { api } from "../../api";
import { connectionErrorMessage, connectionExceptionMessage } from "../profiles/connectionText";
import { useFeedback } from "../../app/Feedback";
import { getCachedClaudeProfiles, loadClaudeProfiles, setClaudeProfilesCache } from "../../app/managementDataCache";
import { CliUpgradePill } from "../../components/CliUpgradePill";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import SortableCard, { CardDragPreview } from "../../components/SortableCard";
import { useCardDragReorder } from "../../components/useCardDragReorder";
import { ProfileCardActions, ProfileCardContent, connectionGate } from "../profiles/ProfileCard";
import { useProfileBalance } from "../profiles/useProfileBalance";
import ProfileNameDialog from "../profiles/ProfileNameDialog";
import { claudeBalanceQueryKinds } from "../../presets";
import ClaudeProfileEdit from "./ClaudeProfileEdit";
import type { ClaudeProfileDetail, ClaudeProfileSummary, ProfileBalanceInfo } from "../../types";
import ProfileModelSelector from "../profiles/ProfileModelSelector";
import { claudeEffortLevels } from "../../components/ReasoningEffortSlider";
import { buildSettingsText, patchEnvValue, readAdvancedSettings, readEnvValue, readModelMappings, resolveOneMillionByDeclaration, setOneMillionModelSuffix } from "./profileEnvText";

function cardProfile(profile: ClaudeProfileSummary) {
  return {
    name: profile.name,
    icon: profile.icon,
    kind: "third_party" as const,
    provider: profile.kind ?? null,
    model: profile.model,
    reasoning_effort: profile.reasoning_effort,
    plan_type: null,
    admin_url: profile.admin_url,
    show_balance: profile.show_balance,
  };
}

function ClaudeProfileCard({ profile, active, busy, testing, activationEpoch, coldStart, balanceCache, onRename, onEdit, onApply, onDuplicate, onTest, onRemove, onChanged }: {
  profile: ClaudeProfileSummary;
  active: boolean;
  busy: boolean;
  testing: boolean;
  activationEpoch: number;
  coldStart: boolean;
  balanceCache: Record<string, ProfileBalanceInfo>;
  onRename: () => void;
  onEdit: () => void;
  onApply: () => void;
  onDuplicate: () => void;
  onTest: () => void;
  onRemove: () => void;
  onChanged: () => Promise<void>;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("profiles");
  const { t: tClaude } = useTranslation("claude");
  const supportsBalance = claudeBalanceQueryKinds.has(profile.kind ?? "");
  const balance = useProfileBalance({
    profileId: profile.id,
    showBalance: profile.show_balance,
    supportsBalance,
    hasCredential: Boolean(profile.has_token),
    active,
    activationEpoch,
    coldStart,
    cachedBalance: balanceCache[profile.id],
    source: "claude",
  });
  const connection = connectionGate(Boolean(profile.base_url), profile.has_token, {
    ready: tClaude("testConnection"),
    missingEndpoint: tClaude("checkProviderFields"),
    missingKey: tClaude("checkProviderFields"),
  });
  return (
    <SortableCard id={profile.id} active={active} onClick={onEdit} title={t("card.clickToEdit")} handleTitle={t("card.dragToReorder")}>
      <ProfileCardContent profile={cardProfile(profile)} hideModel balanceInfos={balance.balanceInfos} balanceError={balance.balanceError} balanceRefreshing={balance.balanceRefreshing} onRefreshBalance={balance.refreshBalance} onOpenAdmin={() => void api.openUrl(profile.admin_url!).catch((error) => feedback.error(String(error)))} onRename={onRename} />
      <ProfileCardActions modelControl={
        <ProfileModelSelector model={profile.model} effort={profile.reasoning_effort} levels={claudeEffortLevels} disabled={busy}
          formatModelLabel={(value) => setOneMillionModelSuffix(value, false)}
          onLoad={async () => {
            const detail = await api.claudeGetProfile(profile.id);
            const text = buildSettingsText(detail);
            // 对齐编辑页：映射里在用的自定义 id 也进列表；每个候选按 1M 声明解析成最终形态，
            // 列表所见即所选所写，未声明 1M 的模型一眼可辨（要开就回编辑页）。
            const mappings = Object.values(readModelMappings(text)).map((value) => value.trim()).filter(Boolean);
            const models = [...detail.fetched_models, ...mappings].map((value) => resolveOneMillionByDeclaration(text, value));
            return { model: readEnvValue(text, "ANTHROPIC_MODEL"), effort: readAdvancedSettings(text).effortLevel, models };
          }}
          onSave={async (changes) => {
            const detail = await api.claudeGetProfile(profile.id);
            let rawSettings = buildSettingsText(detail);
            if (changes.model !== undefined) {
              const model = resolveOneMillionByDeclaration(rawSettings, changes.model);
              rawSettings = patchEnvValue(rawSettings, "ANTHROPIC_MODEL", model || null);
            }
            if (changes.effort !== undefined) rawSettings = patchEnvValue(rawSettings, "CLAUDE_CODE_EFFORT_LEVEL", changes.effort || null, "effortLevel");
            await api.claudeSaveProfile({ id: detail.id, name: detail.name, baseUrl: detail.base_url, authToken: detail.auth_token,
              model: detail.model, description: detail.description, fetchedModels: detail.fetched_models, kind: detail.kind,
              adminUrl: detail.admin_url, extraEnv: detail.extra_env, rawSettings, icon: detail.icon, showBalance: detail.show_balance });
            await onChanged();
          }} />
      } active={active} busy={busy} testing={testing} connectionDisabled={connection.disabled} connectionTitle={connection.title} terminalProfileId={profile.id} onApply={onApply} onDuplicate={onDuplicate} onTest={onTest} onRemove={onRemove} />
    </SortableCard>
  );
}

// 页面数据只在进页时加载（AppShell 按 view==="claude" 挂载本组件），不入启动关键路径。
export default function ClaudeProfilesView({ activeId, onChanged, activationEpoch, coldStart, balanceCache, popEpoch }: { activeId: string | null; onChanged: () => void; activationEpoch: number; coldStart: boolean; balanceCache: Record<string, ProfileBalanceInfo>; popEpoch: number }) {
  const feedback = useFeedback();
  const { t } = useTranslation("claude");
  // 卡片外壳/操作行的悬停文案直接用 profiles 资源（与 Codex 同词），不另造 key
  const { t: tProfiles } = useTranslation("profiles");
  // 缓存直出（同插件/Skill/MCP 页契约）：首帧就是完整列表，进页静默强刷取新
  const cached = getCachedClaudeProfiles();
  const [items, setItems] = useState<ClaudeProfileSummary[]>(cached ?? []);
  const [loaded, setLoaded] = useState(cached !== null);
  const [busy, setBusy] = useState(false);
  const [modal, setModal] = useState<"capture" | "rename" | null>(null);
  const [modalProfile, setModalProfile] = useState<ClaudeProfileSummary | null>(null);
  const [profileName, setProfileName] = useState("");
  const [editingProfile, setEditingProfile] = useState<ClaudeProfileSummary | null>(null);
  const [editDetail, setEditDetail] = useState<ClaudeProfileDetail | null>(null);
  const [creatingProfile, setCreatingProfile] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setItems(await loadClaudeProfiles(true));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setLoaded(true);
    }
  };

  useEffect(() => { void refresh(); }, []);

  // 侧栏重复点击本视图 = 弹回一级；只清子页状态，不刷新（保存流程自带刷新，弹回不产生数据变化）。
  useEffect(() => {
    if (!popEpoch) return;
    setEditingProfile(null);
    setEditDetail(null);
    setCreatingProfile(false);
  }, [popEpoch]);

  const persistOrder = async (previous: ClaudeProfileSummary[], next: ClaudeProfileSummary[]) => {
    try {
      await api.claudeReorderProfiles(next.map((item) => item.id));
      // 排序不整页强刷：本地顺序即新顺序，直接回写缓存，下次进页直出的顺序才正确
      setClaudeProfilesCache(next);
    } catch (error) {
      setItems(previous);
      feedback.error(String(error));
    }
  };
  const { sensors, dragPreview, onDragStart, onDragEnd, onDragCancel } = useCardDragReorder(items, setItems, persistOrder);

  const openCreate = () => {
    if (!busy) setCreatingProfile(true);
  };

  const openRename = (profile: ClaudeProfileSummary) => { setModal("rename"); setModalProfile(profile); setProfileName(profile.name); };

  const submitModal = async () => {
    if (busy || !modal || !profileName.trim()) return;
    setBusy(true);
    try {
      if (modal === "capture") {
        await api.claudeCaptureProfile(profileName.trim());
        feedback.success(tProfiles("feedback.captureSuccess"));
      } else if (modalProfile) {
        await api.renameProfile(modalProfile.id, profileName.trim(), "claude");
        feedback.success(tProfiles("feedback.providerRenamed"));
      }
      setModal(null);
      await refresh();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const openEdit = async (profile: ClaudeProfileSummary) => {
    // 详情完整加载后才切换；失败保留列表，避免空表单覆盖已有配置。
    try {
      const detail = await api.claudeGetProfile(profile.id);
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

  if (creatingProfile || editingProfile) {
    return (
      <ClaudeProfileEdit
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

  const claudeApplyProfile = async (profile: ClaudeProfileSummary) => {
    if (busy) return;
    setBusy(true);
    try {
      await api.claudeApplyProfile(profile.id);
      feedback.success(t("feedback.applySuccess", { ns: "common" }));
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const claudeDuplicateProfile = async (profile: ClaudeProfileSummary) => {
    if (busy) return;
    setBusy(true);
    try {
      await api.claudeDuplicateProfile(profile.id);
      feedback.success(tProfiles("feedback.providerDuplicated"));
      await refresh();
      onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const testProfile = async (profile: ClaudeProfileSummary) => {
    if (testingId) return;
    setTestingId(profile.id);
    try {
      const result = await api.claudeTestProfile(profile.id);
      if (!result.ok) feedback.error(t("connectionFailed", { error: connectionErrorMessage(result, tProfiles) }));
      else feedback.success(t("connectionOk", { latency: ` · ${result.latency_ms}ms` }));
    } catch (error) {
      feedback.error(t("connectionFailed", { error: connectionExceptionMessage(error, tProfiles) }));
    } finally {
      setTestingId(null);
    }
  };

  const claudeDeleteProfile = async (profile: ClaudeProfileSummary) => {
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
      await api.claudeDeleteProfile(profile.id);
      feedback.success(tProfiles("feedback.providerDeleted"));
      await refresh();
      onChanged();
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
          <img src="/claude-code.svg" alt="" className="provider-page-brand__logo" draggable="false" />
          <span>Claude Code</span>
          <CliUpgradePill client="claude" />
        </div>
        <div className="flex items-center gap-2">
          <button type="button" className="apple-action-button app-button--primary disabled:!opacity-100" disabled={busy} onClick={openCreate}>
            <Plus className="h-4 w-4" />
            {tProfiles("toolbar.addProvider")}
          </button>
        </div>
      </header>
      <div className="apple-edit-content">
        {!loaded ? null : items.length === 0 ? (
          <EmptyStateCard icon={<Layers2 className="h-5 w-5" strokeWidth={2} />}>
            <p className="muted">{tProfiles("empty.description")}</p>
            <button type="button" className="apple-action-button app-button--primary" disabled={busy} onClick={() => { setProfileName(""); setModalProfile(null); setModal("capture"); }}>
              <Camera className="h-4 w-4" strokeWidth={2} />
              {tProfiles("toolbar.capture")}
            </button>
          </EmptyStateCard>
        ) : (
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragStart={onDragStart} onDragCancel={onDragCancel} onDragEnd={onDragEnd}>
            <SortableContext items={items.map((item) => item.id)} strategy={verticalListSortingStrategy}>
              <div className="profile-list relative space-y-[var(--gap-page)]">
                {items.map((profile) => {
                  return (
                    <ClaudeProfileCard key={profile.id} profile={profile} active={profile.id === activeId} busy={busy} testing={testingId === profile.id} activationEpoch={activationEpoch} coldStart={coldStart} balanceCache={balanceCache} onChanged={async () => { await refresh(); onChanged(); }} onRename={() => openRename(profile)} onEdit={() => void openEdit(profile)} onApply={() => void claudeApplyProfile(profile)} onDuplicate={() => void claudeDuplicateProfile(profile)} onTest={() => void testProfile(profile)} onRemove={() => void claudeDeleteProfile(profile)} />
                  );
                })}
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
