import { ChevronLeft, Save, Webhook } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppSelect } from "../../components/AppSelect";
import { AppSegmentedControl } from "../../components/AppSegmentedControl";
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
  onBack: () => void;
  onChanged: () => void;
}

/** 无 desktop 块的供应商走自定义路径：填 claude 端点但需路由网关，直连会被上游拒。 */
type SlotState = { name: string; label: string; supports1m: boolean; upstreamModel: string };
const SLOTS = ["sonnet", "opus", "haiku", "fable"] as const;
type SlotKey = (typeof SLOTS)[number];
type DesktopMode = "direct" | "route";
/** 路由模式下每个槽位的固定目录名（桌面端只认 Anthropic 目录真名，网关按它映射）。 */
const CANONICAL: Record<SlotKey, string> = {
  sonnet: "claude-sonnet-5-5",
  opus: "claude-opus-5-5",
  haiku: "claude-haiku-5-5",
  fable: "claude-fable-5-1",
};

/** live/预设的模型列表 → 四槽位编辑态（编辑页与测试共用）。 */
export function slotsFromModels(models: ClaudeDesktopModelSlot[]): Record<SlotKey, SlotState> {
  const empty = () => ({ name: "", label: "", supports1m: false, upstreamModel: "" });
  const slots = { sonnet: empty(), opus: empty(), haiku: empty(), fable: empty() };
  for (const model of models) {
    const key = SLOTS.find((entry) => entry === model.slot);
    if (key) slots[key] = { name: model.name, label: model.label_override ?? "", supports1m: model.supports_1m, upstreamModel: model.upstream_model ?? "" };
  }
  return slots;
}

function desktopPresetSlots(models: readonly { slot: string; name: string; labelOverride?: string }[]): ClaudeDesktopModelSlot[] {
  return models.map((model) => ({ slot: model.slot, name: model.name, label_override: model.labelOverride ?? null, supports_1m: false }));
}

export default function ClaudeDesktopProfileEdit(props: ClaudeDesktopProfileEditProps) {
  // 创建复用 Claude Code 同一供应商选择器与草稿保活；编辑不走选择器。
  if (!props.create) return <ClaudeDesktopEditor {...props} />;
  return (
    <ProviderCreate presets={claudePresets} onBack={props.onBack}>
      {(preset, onChangeProvider) => <ClaudeDesktopEditor {...props} initialPreset={preset} onChangeProvider={onChangeProvider} />}
    </ProviderCreate>
  );
}

function ClaudeDesktopEditor({ profile, create = false, initialDetail = null, onBack, onChanged, initialPreset, onChangeProvider }: ClaudeDesktopProfileEditProps & {
  initialPreset?: ClientPreset;
  onChangeProvider?: () => void;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("claudeDesktop");
  const { t: tClaude } = useTranslation("claude");
  const { t: tProfiles } = useTranslation("profiles");
  // placeholder 与无 desktop 块时的槽位参考都用 DeepSeek 直连模板
  const deepseek = providerPresets.find((item) => item.kind === "deepseek")!.desktop!;
  const initialProvider = create && initialPreset ? providerPresets.find((item) => item.kind === initialPreset.kind) : undefined;
  const initialDesktop = initialProvider?.desktop;
  const [name, setName] = useState(() => profile?.name ?? (create && initialPreset && initialPreset.kind !== "custom" ? presetConnectionName(initialPreset) : ""));
  const [description, setDescription] = useState(() => initialDetail?.description ?? "");
  const [mode, setMode] = useState<DesktopMode>(() => (initialDetail?.mode === "route" ? "route" : "direct"));
  // 直连/路由共用同一对输入：语义随模式切换（直连=Anthropic 端点，路由=上游端点）
  const [baseUrl, setBaseUrl] = useState(() =>
    initialDetail
      ? initialDetail.mode === "route"
        ? initialDetail.upstream_base_url ?? ""
        : initialDetail.base_url ?? ""
      : initialDesktop?.base_url ?? initialPreset?.base_url ?? "",
  );
  const [apiKey, setApiKey] = useState(() => initialDetail?.api_key ?? "");
  const [showKey, setShowKey] = useState(false);
  const [authScheme, setAuthScheme] = useState(() => initialDetail?.auth_scheme ?? "bearer");
  const kind = initialDetail?.kind ?? (create && initialPreset && initialPreset.kind !== "custom" ? initialPreset.kind : null);
  // 管理页随预设/已有配置走，本页不单独编辑
  const adminUrl = initialDetail?.admin_url ?? initialDesktop?.admin_url ?? initialPreset?.admin_url ?? null;
  // 图标：点开选择页换（复用 ProfileIconEdit），未单独选过就跟随预设
  const [pickingIcon, setPickingIcon] = useState(false);
  const [selectedIcon, setSelectedIcon] = useState<string | null>(() => initialDetail?.icon ?? null);
  const providerIcon = providerPresets.find((item) => item.kind === kind)?.icon ?? null;
  const icon = selectedIcon ?? providerIcon;
  const [slots, setSlots] = useState(() => slotsFromModels(initialDetail?.models ?? (create ? desktopPresetSlots(initialDesktop?.default_models ?? []) : [])));
  const [fetchedModels, setFetchedModels] = useState<string[]>(() => initialDetail?.fetched_models ?? []);
  const [fetchingModels, setFetchingModels] = useState(false);
  const modelListRequest = useRef(0);
  const supportsBalance = kind ? claudeBalanceQueryKinds.has(kind) : false;
  const [showBalance, setShowBalance] = useState(() => initialDetail?.show_balance ?? (create && initialPreset ? claudeBalanceQueryKinds.has(initialPreset.kind) : false));
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);

  const updateSlot = (key: SlotKey, patch: Partial<SlotState>) => {
    setSlots((current) => ({ ...current, [key]: { ...current[key], ...patch } }));
  };

  // 与 Claude Code 完全同构：guard→请求序号防串→原样拉取→空/计数/失败三种反馈
  const fetchModelList = async (): Promise<string[]> => {
    if (!baseUrl.trim()) {
      feedback.warning(tClaude("baseUrlRequired"));
      return [];
    }
    if (!apiKey.trim()) {
      feedback.warning(tClaude("tokenRequired"));
      return [];
    }
    setFetchingModels(true);
    const request = ++modelListRequest.current;
    try {
      const models = await api.claudeFetchModels(baseUrl.trim(), apiKey.trim());
      if (request !== modelListRequest.current) return [];
      setFetchedModels(models);
      if (!models.length) feedback.info(tClaude("noModelsReturned"));
      else feedback.success(tClaude("modelsFetched", { count: models.length }));
      return models;
    } catch (error) {
      if (request === modelListRequest.current) feedback.error(tClaude("fetchFailed", { error: connectionExceptionMessage(error, tProfiles) }));
      return [];
    } finally {
      if (request === modelListRequest.current) setFetchingModels(false);
    }
  };

  // 一键设置 = Claude Code 同款：把选中模型填进所有行的请求模型列（路由=上游名，直连=目录名，其余进菜单显示名）
  const applyModelToSlots = (value: string) => {
    const selected = value.trim();
    if (!selected) return;
    setSlots((current) => {
      const next = { ...current };
      for (const key of SLOTS) {
        if (mode === "route") next[key] = { ...next[key], upstreamModel: selected };
        else if (selected.startsWith("claude-")) next[key] = { ...next[key], name: selected };
        else next[key] = { ...next[key], label: selected };
      }
      return next;
    });
  };

  const mappingFields = SLOTS.map((key) => ({
    key,
    label: t(`edit.slot${key === "sonnet" ? "Sonnet" : key === "opus" ? "Opus" : key === "haiku" ? "Haiku" : "Fable"}`),
    value: mode === "route" ? slots[key].upstreamModel : slots[key].name,
    onChange: (value: string) => updateSlot(key, mode === "route" ? { upstreamModel: value } : { name: value }),
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
        // 直连存端点本体；路由存上游端点，桌面端写的是本地网关地址（后端在应用时生成）
        mode,
        baseUrl: mode === "route" ? null : baseUrl.trim() || null,
        apiKey: mode === "route" ? null : apiKey.trim() || null,
        upstreamBaseUrl: mode === "route" ? baseUrl.trim() || null : null,
        upstreamApiKey: mode === "route" ? apiKey.trim() || null : null,
        authScheme,
        models: SLOTS
          .filter((key) => (mode === "route" ? slots[key].upstreamModel.trim() : slots[key].name.trim()))
          .map((key) => ({
            slot: key,
            name: mode === "route" ? CANONICAL[key] : slots[key].name.trim(),
            label_override: slots[key].label.trim() || null,
            supports_1m: slots[key].supports1m,
            upstream_model: mode === "route" ? slots[key].upstreamModel.trim() : null,
          })),
        adminUrl,
        icon,
        showBalance,
        description: description.trim() || null,
        fetchedModels: fetchedModels.length ? fetchedModels : null,
      });
      feedback.success(create ? tProfiles("edit.providerAdded") : tProfiles("edit.providerUpdated"));
      onChanged();
      onBack();
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
        <button type="button" className="apple-page-header apple-back-button" aria-label={tProfiles("edit.back")} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
          <span className="apple-title">{create ? t("edit.createTitle") : t("edit.editTitle")}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          {create && initialPreset && onChangeProvider ? (
            <div className="apple-panel-section">
              <ProviderPresetControls
                preset={initialPreset} baseUrl={baseUrl} disabled={saving}
                onChangeProvider={onChangeProvider} onSelect={(connection) => setBaseUrl(connection.base_url)}
              />
              {!initialDesktop && initialPreset.kind !== "custom" ? <p className="setting-description mt-3">{t("list.needsGatewayHint")}</p> : null}
            </div>
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
            <div className="mt-4 flex min-h-[var(--input-min-height)] items-center justify-between gap-4">
              <div className="min-w-0">
                <div className="setting-title">{t("status.modeSwitchLabel")}</div>
                <p className="setting-description mt-0.5">{mode === "route" ? t("edit.routeHint") : t("edit.directHint")}</p>
              </div>
              <AppSegmentedControl className="h-9 w-64 max-w-full" label={t("status.modeSwitchLabel")} selectedIndex={mode === "route" ? 1 : 0}>
                <button type="button" role="tab" aria-selected={mode === "direct"} className="apple-action-button" onClick={() => setMode("direct")}>{t("status.directMode")}</button>
                <button type="button" role="tab" aria-selected={mode === "route"} className="apple-action-button" onClick={() => setMode("route")}>{t("status.routeMode")}</button>
              </AppSegmentedControl>
            </div>
            <label className="field-label mb-1.5 mt-4 block">{t("edit.protocolLabel")}</label>
            <div className="app-input flex min-w-0 items-center gap-2">
              <Webhook className="h-3.5 w-3.5 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
              <span className="shrink-0 font-medium text-(--text-secondary)">{t("edit.protocolAnthropic")}</span>
            </div>
            <label className="field-label mb-1.5 mt-4 block">{mode === "route" ? t("edit.upstreamBaseUrl") : t("edit.baseUrl")}</label>
            <input className="app-input" placeholder={deepseek.base_url} value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
            <ProviderSecretField
              label={tProfiles("edit.apiKeyLabel")} placeholder={tProfiles("edit.apiKeyPlaceholder")}
              value={apiKey} onChange={setApiKey} visible={showKey}
              onToggle={() => setShowKey((visible) => !visible)}
              showLabel={tProfiles("edit.showApiKey")} hideLabel={tProfiles("edit.hideApiKey")}
              testLabel={tProfiles("edit.testConnection")}
              testTitle={!apiKey.trim() || !baseUrl.trim() ? tProfiles("edit.checkProviderFields") : undefined}
              testing={testing} onTest={() => void testConnection()}
            />
            <label className="field-label mb-1.5 mt-4 block">{t("edit.authScheme")}</label>
            <div className="max-w-56">
              <AppSelect
                value={authScheme}
                options={[{ value: "bearer", label: t("edit.bearer") }, { value: "x-api-key", label: t("edit.xApiKey") }]}
                onChange={setAuthScheme}
                compact
              />
            </div>
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
                mappingTitle: t("edit.modelSlots"), mappingDescription: mode === "route" ? t("edit.routeModelHint") : t("edit.modelSlotHint"),
                role: tClaude("modelRoleLabel"), displayName: tClaude("displayNameLabel"), requestModel: tClaude("requestModelLabel"),
                oneMillion: tClaude("oneMillionLabel"), oneMillionTitle: tClaude("oneMillionTitle"),
                oneMillionColumn: tClaude("oneMillionColumn"),
                clear: tProfiles("edit.clearFile"),
              }}
            />
          </div>
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        <button type="button" className="apple-action-button" onClick={onBack}>{tProfiles("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={saving || !name.trim()} onClick={() => void save()}>
          <Save className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
          {tProfiles("dialog.save")}
        </button>
      </div>
    </section>
  );
}
