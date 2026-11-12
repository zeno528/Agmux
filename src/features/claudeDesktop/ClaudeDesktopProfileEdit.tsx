import { ChevronLeft, Save } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppSwitch } from "../../components/AppSwitch";
import { ProviderIdentityFields, ProviderModelFields, ProviderSecretField } from "../../components/ProviderFields";
import { ProviderCreate, ProviderPresetControls } from "../../components/ProviderSetup";
import ProfileIconEdit from "../profiles/ProfileIconEdit";
import { connectionErrorMessage, connectionExceptionMessage } from "../profiles/connectionText";
import { claudeBalanceQueryKinds, claudePresets, presetConnectionName, providerPresets, type ClientPreset } from "../../presets";
import type { ClaudeDesktopModelSlot, ClaudeDesktopProfileDetail, ClaudeDesktopProfileSummary } from "../../types";

interface ClaudeDesktopProfileEditProps {
  profile: ClaudeDesktopProfileSummary | null;
  create?: boolean;
  initialDetail?: ClaudeDesktopProfileDetail | null;
  onBack: (saved?: boolean) => void;
}

type SlotState = { name: string; label: string; supports1m: boolean };
const SLOTS = ["sonnet", "opus", "haiku", "fable"] as const;
type SlotKey = (typeof SLOTS)[number];
/** 已保存模型 → 编辑态；新建传空列表，保留四个空角色。 */
export function slotsFromModels(models: ClaudeDesktopModelSlot[]): Record<SlotKey, SlotState> {
  const empty = () => ({ name: "", label: "", supports1m: false });
  const slots = { sonnet: empty(), opus: empty(), haiku: empty(), fable: empty() };
  for (const model of models) {
    const key = SLOTS.find((entry) => entry === model.slot);
    if (key) slots[key] = { name: model.name, label: model.label_override ?? "", supports1m: model.supports_1m };
  }
  return slots;
}

export default function ClaudeDesktopProfileEdit(props: ClaudeDesktopProfileEditProps) {
  // 创建复用 Claude Code 同一供应商选择器与草稿保活；编辑不走选择器。
  if (!props.create) return <ClaudeDesktopEditor {...props} />;
  return (
    <ProviderCreate presets={claudePresets} onBack={() => props.onBack()}>
      {(preset, onChangeProvider) => <ClaudeDesktopEditor {...props} initialPreset={preset} onChangeProvider={onChangeProvider} />}
    </ProviderCreate>
  );
}

function ClaudeDesktopEditor({ profile, create = false, initialDetail = null, onBack, initialPreset, onChangeProvider }: ClaudeDesktopProfileEditProps & {
  initialPreset?: ClientPreset;
  onChangeProvider?: () => void;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("claudeDesktop");
  const { t: tClaude } = useTranslation("claude");
  const { t: tProfiles } = useTranslation("profiles");
  const [name, setName] = useState(() => profile?.name ?? (create && initialPreset && initialPreset.kind !== "custom" ? presetConnectionName(initialPreset) : ""));
  const [description, setDescription] = useState(() => initialDetail?.description ?? "");
  const [baseUrl, setBaseUrl] = useState(() => initialDetail?.base_url ?? initialPreset?.base_url ?? "");
  const [apiKey, setApiKey] = useState(() => initialDetail?.api_key ?? "");
  const [showKey, setShowKey] = useState(false);
  const authScheme = initialDetail?.auth_scheme ?? "bearer";
  const kind = initialDetail?.kind ?? (create && initialPreset && initialPreset.kind !== "custom" ? initialPreset.kind : null);
  // 管理页随预设/已有配置走，本页不单独编辑
  const adminUrl = initialDetail?.admin_url ?? initialPreset?.admin_url ?? null;
  // 图标：点开选择页换（复用 ProfileIconEdit），未单独选过就跟随预设
  const [pickingIcon, setPickingIcon] = useState(false);
  const [selectedIcon, setSelectedIcon] = useState<string | null>(() => initialDetail?.icon ?? null);
  const providerIcon = providerPresets.find((item) => item.kind === kind)?.icon ?? null;
  const icon = selectedIcon ?? providerIcon;
  const [slots, setSlots] = useState(() => slotsFromModels(initialDetail?.models ?? []));
  const [fetchedModels, setFetchedModels] = useState<string[]>(() => initialDetail?.fetched_models ?? []);
  const [fetchingModels, setFetchingModels] = useState(false);
  const supportsBalance = kind ? claudeBalanceQueryKinds.has(kind) : false;
  const [showBalance, setShowBalance] = useState(() => initialDetail?.show_balance ?? (create && initialPreset ? claudeBalanceQueryKinds.has(initialPreset.kind) : false));
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);

  const updateSlot = (key: SlotKey, patch: Partial<SlotState>) => {
    setSlots((current) => ({ ...current, [key]: { ...current[key], ...patch } }));
  };

  const fetchModelList = async () => {
    if (fetchingModels || !baseUrl.trim() || !apiKey.trim()) return;
    setFetchingModels(true);
    try {
      const models = await api.claudeFetchModels(baseUrl.trim(), apiKey.trim());
      setFetchedModels(models);
      if (!models.length) feedback.info(tClaude("noModelsReturned"));
      else feedback.success(tClaude("modelsFetched", { count: models.length }));
    } catch (error) {
      feedback.error(tClaude("fetchFailed", { error: connectionExceptionMessage(error, tProfiles) }));
    } finally {
      setFetchingModels(false);
    }
  };

  // 一键设置把同一模型填进所有角色。
  const applyModelToSlots = (value: string) => {
    const selected = value.trim();
    if (!selected) return;
    setSlots((current) => {
      const next = { ...current };
      for (const key of SLOTS) {
        next[key] = { ...next[key], name: selected };
      }
      return next;
    });
  };

  const mappingFields = SLOTS.map((key) => ({
    key,
    label: t(`edit.slot${key === "sonnet" ? "Sonnet" : key === "opus" ? "Opus" : key === "haiku" ? "Haiku" : "Fable"}`),
    value: slots[key].name,
    onChange: (value: string) => updateSlot(key, { name: value }),
    displayValue: slots[key].label,
    displayPlaceholder: t("edit.modelLabel"),
    onDisplayChange: (value: string) => updateSlot(key, { label: value }),
    oneMillion: slots[key].supports1m,
    onToggleOneMillion: (enabled: boolean) => updateSlot(key, { supports1m: enabled }),
  }));

  const testConnection = async () => {
    if (testing || !baseUrl.trim() || !apiKey.trim()) return;
    setTesting(true);
    try {
      const result = await api.claudeTestConnection(baseUrl.trim(), apiKey.trim());
      if (!result.ok) feedback.error(t("connectionFailed", { ns: "claude", error: connectionErrorMessage(result, tProfiles) }));
      else feedback.success(t("connectionOk", { ns: "claude", latency: ` · ${result.latency_ms}ms` }));
    } catch (error) {
      feedback.error(t("connectionFailed", { ns: "claude", error: connectionExceptionMessage(error, tProfiles) }));
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    if (saving || !name.trim() || (!create && !initialDetail)) return;
    setSaving(true);
    try {
      await api.claudeDesktopSaveProfile({
        id: profile?.id,
        name: name.trim(),
        kind,
        baseUrl: baseUrl.trim() || null,
        apiKey: apiKey.trim() || null,
        authScheme,
        models: SLOTS
          .filter((key) => slots[key].name.trim())
          .map((key) => ({
            slot: key,
            name: slots[key].name.trim(),
            label_override: slots[key].label.trim() || null,
            supports_1m: slots[key].supports1m,
          })),
        adminUrl,
        icon,
        showBalance,
        description: description.trim() || null,
        fetchedModels: fetchedModels.length ? fetchedModels : null,
      });
      feedback.success(create ? tProfiles("edit.providerAdded") : tProfiles("edit.providerUpdated"));
      onBack(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  if (pickingIcon) return <ProfileIconEdit icon={icon} onBack={() => setPickingIcon(false)} onSave={(next) => { setSelectedIcon(next); setPickingIcon(false); }} />;

  return (
    <section
      className="apple-edit-page mx-auto flex w-full max-w-none flex-col"
      onKeyDown={(event) => {
        if (event.key === "Enter" && !event.nativeEvent.isComposing) {
          event.preventDefault();
          void save();
        }
      }}
    >
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={tProfiles("edit.back")} onClick={create && onChangeProvider ? onChangeProvider : () => onBack()}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
          <span className="apple-title">{create ? t("edit.createTitle") : t("edit.editTitle")}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          {create && initialPreset && onChangeProvider ? (
              <ProviderPresetControls
                preset={initialPreset} baseUrl={baseUrl} disabled={saving}
                onChangeProvider={onChangeProvider} onSelect={(connection) => setBaseUrl(connection.base_url)}
              />
          ) : null}
          <div className="apple-panel-section">
            <ProviderIdentityFields
              idPrefix="claude-desktop-profile" name={name} description={description}
              icon={icon}
              onIcon={() => setPickingIcon(true)} onName={setName} onDescription={setDescription}
              labels={{
                changeIcon: tProfiles("edit.changeIcon"), changeIconLabel: tProfiles("edit.changeIconLabel"),
                name: tProfiles("edit.nameLabel"), namePlaceholder: tProfiles("edit.namePlaceholder"),
                description: tProfiles("edit.descriptionLabel"), descriptionPlaceholder: tProfiles("edit.descriptionPlaceholder"),
              }}
            />
            <label className="field-label mb-1.5 mt-4 block">{t("edit.baseUrl")}</label>
            <input className="app-input" placeholder={tClaude("baseUrlPlaceholder")} value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
            <ProviderSecretField
              label={tProfiles("edit.apiKeyLabel")} placeholder={tProfiles("edit.apiKeyPlaceholder")}
              value={apiKey} onChange={setApiKey} visible={showKey}
              onToggle={() => setShowKey((visible) => !visible)}
              showLabel={tProfiles("edit.showApiKey")} hideLabel={tProfiles("edit.hideApiKey")}
              testLabel={tProfiles("edit.testConnection")}
              testTitle={!apiKey.trim() || !baseUrl.trim() ? tProfiles("edit.checkProviderFields") : undefined}
              testing={testing} onTest={() => void testConnection()}
            />
            {supportsBalance ? <div className="mt-4 flex min-h-[var(--input-min-height)] items-center justify-between gap-4"><div className="min-w-0"><div className="setting-title">{tProfiles("edit.balanceUsage")}</div><div className="setting-description mt-0.5">{tProfiles("edit.balanceAutoRefresh")}</div></div><AppSwitch checked={showBalance} disabled={saving} label={tProfiles("edit.balanceUsage")} onCheckedChange={setShowBalance} /></div> : null}
          </div>
          <div className="apple-panel-section">
            <ProviderModelFields
              models={fetchedModels} fetching={fetchingModels}
              disabled={!apiKey.trim() || !baseUrl.trim()} onFetch={() => void fetchModelList()}
              onApplyModel={applyModelToSlots}
              mappingFields={mappingFields}
              labels={{
                model: tProfiles("edit.modelIdLabel"),
                models: tProfiles("edit.modelsLabel"), fetch: tProfiles("edit.fetchModels"),
                available: tProfiles("edit.modelsAvailable", { count: fetchedModels.length }),
                select: tProfiles("edit.selectModel"), fetchFirst: tProfiles("edit.fetchModelsFirst"),
                quickSet: tClaude("quickSet"),
                mappingTitle: t("edit.modelSlots"), mappingDescription: t("edit.modelSlotHint"),
                role: tClaude("modelRoleLabel"), displayName: t("edit.modelLabel"), requestModel: tClaude("requestModelLabel"),
                oneMillion: tClaude("oneMillionLabel"), oneMillionTitle: tClaude("oneMillionTitle"),
                oneMillionColumn: tClaude("oneMillionColumn"),
                clear: tProfiles("edit.clearFile"),
              }}
            />
          </div>
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        <button type="button" className="apple-action-button" onClick={() => onBack()}>{tProfiles("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={saving || !name.trim()} onClick={() => void save()}>
          <Save className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
          {tProfiles("dialog.save")}
        </button>
      </div>
    </section>
  );
}
