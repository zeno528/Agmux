import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { api } from "../../api";
import { profileAuthQuotaCacheKey } from "../../app/authQuotaCache";
import { balanceQueryProviders } from "../../presets";
import type { ProfileBalanceInfo, CodexProfileSummary } from "../../types";
import { useFeedback } from "../../app/Feedback";
import SortableCard from "../../components/SortableCard";
import { ProfileCardActions, ProfileCardContent, connectionGate } from "../profiles/ProfileCard";
import { useProfileBalance } from "../profiles/useProfileBalance";
import { connectionErrorMessage, connectionFailureMessage } from "../profiles/connectionText";
import ProfileModelSelector from "../profiles/ProfileModelSelector";
import { codexEffortLevels } from "../../components/ReasoningEffortSlider";
import { chatgptModelOptions, readCatalogOptions, readModelValue } from "./profileEditText";

interface ProfileCardProps {
  profile: CodexProfileSummary;
  active: boolean;
  busy: boolean;
  activationEpoch: number;
  /// 本次进程启动还没走完（首屏尚未出窗）——只有这时才值得把余额刷新往后放
  coldStart: boolean;
  balanceCache?: Record<string, ProfileBalanceInfo>;
  onApply: () => void;
  onRename: () => void;
  onEdit: () => void;
  onRemove: () => void;
  onDuplicate: () => void;
  onChanged: () => Promise<void>;
}

/** Codex 侧壳：官方订阅（无第三方 provider）不做门控；第三方供应商映射到通用判定。 */
export function profileConnectionGate(profile: CodexProfileSummary, t: TFunction<"profiles">) {
  if (!profile.provider) return { disabled: false, title: t("connection.test") };
  return connectionGate(profile.has_base_url, profile.has_key, {
    ready: t("connection.test"),
    missingEndpoint: t("connection.missingApiEndpointWarning"),
    missingKey: t("connection.missingApiKeyWarning"),
  });
}

export default function CodexProfileCard({
  profile,
  active,
  busy,
  activationEpoch,
  coldStart,
  balanceCache,
  onApply,
  onRename,
  onEdit,
  onRemove,
  onDuplicate,
  onChanged,
}: ProfileCardProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("profiles");
  const authQuotaKey = profileAuthQuotaCacheKey(profile);
  const [testing, setTesting] = useState(false);
  const supportsBalance = profile.kind === "official" || balanceQueryProviders.has(profile.provider ?? "");
  const { balanceInfos, balanceError, balanceRefreshing, refreshBalance } = useProfileBalance({
    profileId: profile.id,
    showBalance: profile.show_balance,
    supportsBalance,
    hasCredential: profile.kind === "official" || profile.has_key,
    active,
    activationEpoch,
    coldStart,
    cachedBalance: balanceCache?.[profile.id],
    authQuotaKey,
    source: "codex",
  });

  const testConnection = async () => {
    if (testing) return;
    if (profile.provider && !profile.has_base_url) {
      feedback.warning(t("edit.baseUrlRequired"));
      return;
    }
    if (profile.provider && !profile.has_key) {
      feedback.warning(t("connection.missingApiKeyWarning"));
      return;
    }
    setTesting(true);
    try {
      const result = await api.codexTestProfileConnection(profile.id);
      if (result.ok) {
        if (result.status != null && result.status >= 400) feedback.info(t("connection.reachable"));
        else feedback.success(t("connection.ok", { latency: result.latency_ms != null ? ` · ${result.latency_ms}ms` : "" }));
      } else {
        feedback.error(connectionFailureMessage(profile.provider ? connectionErrorMessage(result, t) : result.error ?? t("connection.unknownError"), t, result.status));
      }
    } catch (error) {
      feedback.error(connectionFailureMessage(String(error), t));
    } finally {
      setTesting(false);
    }
  };

  const connection = profileConnectionGate(profile, t);
  return (
    <SortableCard id={profile.id} active={active} onClick={onEdit} title={t("card.clickToEdit")} handleTitle={t("card.dragToReorder")}>
      <ProfileCardContent
        profile={profile}
        hideModel
        balanceInfos={balanceInfos}
        balanceError={balanceError}
        balanceRefreshing={balanceRefreshing}
        onRefreshBalance={refreshBalance}
        onOpenAdmin={() => void api.openUrl(profile.admin_url!).catch((error) => feedback.error(String(error)))}
        onRename={onRename}
      />
      <ProfileCardActions modelControl={
        <ProfileModelSelector model={profile.model} effort={profile.reasoning_effort} levels={codexEffortLevels} disabled={busy}
          supportsFastMode={profile.kind === "official" && profile.provider === null}
          fast={profile.fast_mode}
          onLoad={async (refreshDefaults = false) => {
            const detail = await api.codexGetProfile(profile.id);
            const official = profile.kind === "official" && profile.provider === null;
            const localText = detail.raw_catalog ?? detail.catalog_content;
            const local = readCatalogOptions(localText);
            const useLocal = official && !!detail.model_values.model_catalog_json && !!localText?.trim();
            const catalog = official && !useLocal
              ? await api.codexFetchChatgptModels(profile.id, detail.auth_source ?? "desktop", detail.account_id, refreshDefaults) : null;
            const options = catalog ? chatgptModelOptions(catalog) : null;
            const fetchedEfforts = options?.efforts ?? detail.fetched_model_efforts ?? {};
            const efforts = useLocal || !official ? local.efforts : fetchedEfforts;
            const defaults = options?.defaults ?? detail.fetched_model_defaults ?? {};
            return {
              model: readModelValue(`model = ${detail.model_values.model ?? '""'}`) ?? "",
              effort: readModelValue(`model = ${detail.model_values.model_reasoning_effort ?? '""'}`) ?? "",
              fast: readModelValue(`model = ${detail.model_values.service_tier ?? '""'}`) === "fast",
              models: useLocal ? local.models : options ? options.models : [...detail.fetched_models, ...Object.keys(efforts)],
              efforts,
              defaults: official ? useLocal ? local.defaults : defaults : undefined,
            };
          }}
          onSave={async (changes) => { await api.codexSetProfileModel(profile.id, changes); await onChanged(); }} />
      } active={active} busy={busy} allowInactiveDeleteWhileBusy testing={testing} connectionDisabled={connection.disabled} connectionTitle={connection.title} onApply={onApply} onDuplicate={onDuplicate} onTest={() => void testConnection()} onRemove={onRemove} />
    </SortableCard>
  );
}
