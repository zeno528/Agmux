import { ChevronLeft, CodeXml, ExternalLink, FileBraces, LayoutTemplate, Save, Webhook } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppSelect } from "../../components/AppSelect";
import { AppSwitch } from "../../components/AppSwitch";
import ConfigTextEditor, { type ConfigTextEditorHandle } from "../../components/ConfigTextEditor";
import EndpointField from "../../components/EndpointField";
import { ProviderCreate, ProviderPresetControls } from "../../components/ProviderSetup";
import { ProviderIdentityFields, ProviderModelFields, ProviderSecretField } from "../../components/ProviderFields";
import {
  claudeBalanceQueryKinds, claudePresets, claudePresetByKind,
  presetConnectionName, type ClientPreset, type PresetConnection,
} from "../../presets";
import ProfileIconEdit from "../profiles/ProfileIconEdit";
import { connectionErrorMessage, connectionExceptionMessage } from "../profiles/connectionText";
import ClaudeCommonTemplateDialog from "./ClaudeCommonTemplateDialog";
import { extractClaudeCommonSettings, fillClaudeCommonSettings, patchBypassPermissions } from "./profileEnvText";
import { buildSettingsText, formatJsonText, hasOneMillionModelSuffix, patchEnvFields, patchEnvValue, patchGitAttribution, patchModelDisplayNames, patchModelMappings, readAdvancedSettings, readEnvFields, readEnvValue, readGitAttributionDisabled, readModelDisplayNames, readModelMappings, setOneMillionModelSuffix, splitEnvExtras, type ClaudeModelDisplayKey, type ClaudeModelDisplayNames, type ClaudeModelMappingKey, type ClaudeModelMappings } from "./profileEnvText";
import type { ClaudeProfileDetail, ClaudeProfileSummary } from "../../types";

function sameModelMappings(left: ClaudeModelMappings, right: ClaudeModelMappings) {
  return Object.keys(left).every((key) => left[key as ClaudeModelMappingKey] === right[key as ClaudeModelMappingKey]);
}

function sameModelDisplayNames(left: ClaudeModelDisplayNames, right: ClaudeModelDisplayNames) {
  return Object.keys(left).every((key) => left[key as ClaudeModelDisplayKey] === right[key as ClaudeModelDisplayKey]);
}

const modelDisplayKeyByModelKey: Partial<Record<ClaudeModelMappingKey, ClaudeModelDisplayKey>> = {
  ANTHROPIC_DEFAULT_FABLE_MODEL: "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
  ANTHROPIC_DEFAULT_OPUS_MODEL: "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
  ANTHROPIC_DEFAULT_SONNET_MODEL: "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
  ANTHROPIC_DEFAULT_HAIKU_MODEL: "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
  ANTHROPIC_CUSTOM_MODEL_OPTION: "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
};

const effortLevels = ["low", "medium", "high", "xhigh", "max"] as const;

interface ClaudeProfileEditProps {
  profile: ClaudeProfileSummary | null;
  create?: boolean;
  initialDetail?: ClaudeProfileDetail | null;
  onBack: () => void;
  onChanged: () => void;
}

export default function ClaudeProfileEdit(props: ClaudeProfileEditProps) {
  return props.create ? <ProviderCreate presets={claudePresets} onBack={props.onBack}>
    {(preset, onChangeProvider) => <ClaudeProfileEditor {...props} initialPreset={preset} onChangeProvider={onChangeProvider} />}
  </ProviderCreate> : <ClaudeProfileEditor {...props} />;
}

function ClaudeProfileEditor({
  profile, create = false, initialDetail = null, onBack, onChanged, initialPreset, onChangeProvider,
}: ClaudeProfileEditProps & {
  initialPreset?: ClientPreset;
  onChangeProvider?: () => void;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("claude");
  // 图标编辑复用 Codex 的 ProfileIconEdit 页（文案也直接用 profiles 资源，不另造 key）
  const { t: tProfiles } = useTranslation("profiles");
  // 新建从干净 settings.json 开始；只有编辑已有配置才回显已保存全文。
  const initialEnvText = useMemo(
    () => create ? patchEnvFields("{}", { baseUrl: initialPreset!.base_url, authToken: "", model: "" }, initialPreset!.kind) : buildSettingsText({
      raw_settings: initialDetail?.raw_settings ?? null,
      base_url: initialDetail?.base_url ?? null,
      auth_token: initialDetail?.auth_token ?? null,
      model: initialDetail?.model ?? null,
      extra_env: initialDetail?.extra_env ?? null,
      kind: initialDetail?.kind ?? null,
    }),
    [create, initialDetail, initialPreset],
  );
  const [name, setName] = useState(() => profile?.name ?? (create && initialPreset!.kind !== "custom"
    ? presetConnectionName(initialPreset!) : ""));
  const [baseUrl, setBaseUrl] = useState(() => initialDetail?.base_url ?? initialPreset?.base_url ?? "");
  const [authToken, setAuthToken] = useState(() => initialDetail?.auth_token ?? "");
  const [showToken, setShowToken] = useState(false);
  const [description, setDescription] = useState(() => initialDetail?.description ?? "");
  const [fetchedModels, setFetchedModels] = useState<string[]>(() => initialDetail?.fetched_models ?? []);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [testing, setTesting] = useState(false);
  const [adminUrl, setAdminUrl] = useState(() => initialDetail?.admin_url ?? initialPreset?.admin_url ?? "");
  const [kind, setKind] = useState(() => initialDetail?.kind ?? (create && initialPreset!.kind !== "custom" ? initialPreset!.kind : null));
  const [showBalance, setShowBalance] = useState(() => Boolean(initialDetail?.show_balance ?? profile?.show_balance ?? (create && claudeBalanceQueryKinds.has(initialPreset!.kind))));
  const [savingBalance, setSavingBalance] = useState(false);
  const [saving, setSaving] = useState(false);
  // 图标编辑：复用 Codex 的铅笔角标 + ProfileIconEdit 选择页，立即落库（create 态只记本地）
  const [pickingIcon, setPickingIcon] = useState(false);
  const [selectedIcon, setSelectedIcon] = useState<string | null>(() => initialDetail?.icon ?? profile?.icon ?? initialPreset?.icon ?? null);
  const [modelMappings, setModelMappings] = useState<ClaudeModelMappings>(() => readModelMappings(initialEnvText));
  const [modelDisplayNames, setModelDisplayNames] = useState<ClaudeModelDisplayNames>(() => readModelDisplayNames(initialEnvText));
  const model = modelMappings.ANTHROPIC_MODEL;
  const [envText, setEnvText] = useState(initialEnvText);
  const envTextRef = useRef(envText);
  envTextRef.current = envText;
  const templateRequest = useRef(0);
  const modelListRequest = useRef(0);
  const [fillingTemplate, setFillingTemplate] = useState(false);
  const [templateOpen, setTemplateOpen] = useState(false);
  const [editorDiagnostics, setEditorDiagnostics] = useState({ count: 0, firstLine: null as number | null });
  const editorRef = useRef<ConfigTextEditorHandle>(null);
  const envDirty = envText !== initialEnvText;
  const envMinLines = Math.max(envText.split(/\r?\n/).length, 12);
  const { autoCompactDisabled, autoCompactWindow, effortLevel, autoMemoryEnabled, bashEditDiffEnabled, bypassPermissionsEnabled } = readAdvancedSettings(envText);
  const gitAttributionHidden = readGitAttributionDisabled(envText);
  const agentTeamsEnabled = readEnvValue(envText, "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS") === "1";
  useEffect(() => () => { templateRequest.current += 1; }, []);

  const fillCommonTemplate = async () => {
    if (fillingTemplate || saving) return;
    const request = ++templateRequest.current;
    setFillingTemplate(true);
    try {
      const template = await api.claudeGetCommonSettings();
      if (request !== templateRequest.current) return;
      if (template === null || extractClaudeCommonSettings(template) === "{}") {
        feedback.info(t("commonTemplate.empty"));
        return;
      }
      // 等待读取期间仍可编辑：合并到最新草稿，不能覆盖期间输入的内容。
      const current = envTextRef.current;
      const next = fillClaudeCommonSettings(current, template);
      if (next === null) feedback.error(t("envInvalid"));
      else if (next === current) feedback.info(t("commonTemplate.noChange"));
      else {
        setEnvText(next);
        feedback.success(t("commonTemplate.filled"));
      }
    } catch (error) {
      if (request === templateRequest.current) feedback.error(String(error));
    } finally {
      if (request === templateRequest.current) setFillingTemplate(false);
    }
  };
  // 创建态取所选预设；编辑态行的 kind 反查（对齐 Codex 双区域端点按 provider 反查的思路）
  const selectedPreset = useMemo(
    () => (create ? initialPreset ?? null : claudePresetByKind(kind)),
    [create, kind, initialPreset],
  );
  const presetEndpoints = !create ? selectedPreset?.endpoints ?? null : null;
  const accountLogin = kind === "claude-account";
  const supportsBalance = claudeBalanceQueryKinds.has(kind ?? "");

  const updateModelMapping = (key: ClaudeModelMappingKey, value: string) => {
    setModelMappings((current) => current[key] === value ? current : { ...current, [key]: value });
    const displayKey = modelDisplayKeyByModelKey[key];
    if (!value.trim() && displayKey) {
      setModelDisplayNames((current) => current[displayKey] ? { ...current, [displayKey]: "" } : current);
    }
  };
  const modelMappingLabels = [
    ["ANTHROPIC_MODEL", "current"],
    ["ANTHROPIC_DEFAULT_FABLE_MODEL", "fable"],
    ["ANTHROPIC_DEFAULT_OPUS_MODEL", "opus"],
    ["ANTHROPIC_DEFAULT_SONNET_MODEL", "sonnet"],
    ["ANTHROPIC_DEFAULT_HAIKU_MODEL", "haiku"],
    ["CLAUDE_CODE_SUBAGENT_MODEL", "subagent"],
    ["ANTHROPIC_CUSTOM_MODEL_OPTION", "customOption"],
  ] as const;
  const applyModelToMappings = (value: string) => {
    const selected = value.trim();
    if (!selected) return;
    setModelMappings((current) => {
      const next = { ...current };
      for (const [key] of modelMappingLabels) next[key] = setOneMillionModelSuffix(selected, hasOneMillionModelSuffix(current[key]));
      return next;
    });
  };
  const modelMappingFields = modelMappingLabels.map(([key, label]) => {
    const displayKey = modelDisplayKeyByModelKey[key];
    // 悬停提示 = 环境变量名 + 作用简述（只有主会话/新会话/子代理/自定义有简述，其余只给变量名）
    const purpose = t(`modelMappingHints.${label}`, { defaultValue: "" });
    return {
      key,
      label: t(`modelMappings.${label}`),
      value: modelMappings[key],
      hint: purpose ? `${key} — ${purpose}` : key,
      displayValue: displayKey ? modelDisplayNames[displayKey] : undefined,
      displayPlaceholder: displayKey ? t("displayNamePlaceholder") : key === "CLAUDE_CODE_SUBAGENT_MODEL" ? t("displayNameUnavailable") : t("displayNameNotApplicable"),
      displayDisabled: !displayKey,
      onDisplayChange: displayKey ? (value: string) => setModelDisplayNames((current) => ({ ...current, [displayKey]: value })) : undefined,
      oneMillion: hasOneMillionModelSuffix(modelMappings[key]),
      onChange: (value: string) => updateModelMapping(key, hasOneMillionModelSuffix(modelMappings[key]) ? setOneMillionModelSuffix(value, true) : value),
      onToggleOneMillion: (enabled: boolean) => updateModelMapping(key, setOneMillionModelSuffix(modelMappings[key], enabled)),
    };
  });

  const selectConnection = (connection: PresetConnection) => {
    setKind(connection.kind);
    setBaseUrl(connection.base_url);
    setAdminUrl(connection.admin_url ?? "");
    setEnvText((current) => patchEnvFields(current, { baseUrl: connection.base_url, authToken, model }, connection.kind));
    modelListRequest.current++;
    setFetchingModels(false);
    setFetchedModels([]);
  };

  // 表单三键 ↔ env 编辑器双向同步（与 Codex 地址/密钥/model 的 effect 对同一模式）：
  // 表单改动 patch 进 env 文本；编辑器改动读回表单。同值 patch 幂等，不会成环。
  useEffect(() => {
    setEnvText((current) => {
      const next = patchEnvFields(current, { baseUrl, authToken, model }, kind);
      const mapped = patchModelMappings(next, modelMappings);
      const named = patchModelDisplayNames(mapped, modelDisplayNames);
      return named === current ? current : named;
    });
  }, [baseUrl, authToken, model, modelMappings, modelDisplayNames, kind]);
  useEffect(() => {
    const fields = readEnvFields(envText, kind);
    if (!fields) return;
    if (fields.baseUrl !== baseUrl) setBaseUrl(fields.baseUrl);
    if (fields.authToken !== authToken) setAuthToken(fields.authToken);
    const nextMappings = readModelMappings(envText);
    if (!sameModelMappings(nextMappings, modelMappings)) setModelMappings(nextMappings);
    const nextDisplayNames = readModelDisplayNames(envText);
    if (!sameModelDisplayNames(nextDisplayNames, modelDisplayNames)) setModelDisplayNames(nextDisplayNames);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [envText]);

  // 获取模型与测试连通复用 GET /models，不发送推理请求。
  const fetchModelList = async (): Promise<string[]> => {
    if (!baseUrl.trim()) {
      feedback.warning(t("baseUrlRequired"));
      return [];
    }
    if (!authToken.trim()) {
      feedback.warning(t("tokenRequired"));
      return [];
    }
    setFetchingModels(true);
    const request = ++modelListRequest.current;
    try {
      const models = await api.claudeFetchModels(baseUrl.trim(), authToken.trim());
      if (request !== modelListRequest.current) return [];
      setFetchedModels(models);
      if (!models.length) feedback.info(t("noModelsReturned"));
      else feedback.success(t("modelsFetched", { count: models.length }));
      return models;
    } catch (error) {
      if (request === modelListRequest.current) feedback.error(t("fetchFailed", { error: connectionExceptionMessage(error, tProfiles) }));
      return [];
    } finally {
      if (request === modelListRequest.current) setFetchingModels(false);
    }
  };

  const testConnection = async () => {
    if (testing || !baseUrl.trim() || !authToken.trim()) return;
    setTesting(true);
    try {
      const result = await api.claudeTestConnection(baseUrl.trim(), authToken.trim());
      if (!result.ok) feedback.error(t("connectionFailed", { error: connectionErrorMessage(result, tProfiles) }));
      else feedback.success(t("connectionOk", { latency: ` · ${result.latency_ms}ms` }));
    } catch (error) {
      feedback.error(t("connectionFailed", { error: connectionExceptionMessage(error, tProfiles) }));
    } finally {
      setTesting(false);
    }
  };

  // 附加 env 排版：与通用模板弹窗共用 formatJsonText（提示里的标签固定是 settings.json）
  const formatEnv = () => {
    const label = "settings.json";
    const outcome = formatJsonText(envText);
    if (outcome.status === "empty") feedback.warning(tProfiles("edit.formatEmpty"));
    else if (outcome.status === "failed") feedback.error(tProfiles("edit.formatFailed", { error: outcome.error }));
    else if (outcome.status === "unchanged") feedback.info(tProfiles("edit.formatNoChange", { label }));
    else {
      setEnvText(outcome.text);
      feedback.success(tProfiles("edit.formatSuccess", { label }));
    }
  };

  const updateEnvValue = (key: string, value: string | null, fallbackSettingsKey?: string) => {
    const next = patchEnvValue(envText, key, value, fallbackSettingsKey);
    if (value?.trim()) editorRef.current?.revealField(next, key);
    setEnvText(next);
  };

  const toggleGitAttribution = (enabled: boolean) => {
    const next = patchGitAttribution(envText, enabled);
    if (enabled) editorRef.current?.revealField(next, "commit");
    setEnvText(next);
  };

  const toggleBalance = async (enabled: boolean) => {
    if (savingBalance) return;
    setShowBalance(enabled);
    if (create || !profile) return;
    setSavingBalance(true);
    try {
      await api.claudeSetProfileShowBalance(profile.id, enabled);
      onChanged();
    } catch (error) {
      setShowBalance(!enabled);
      feedback.error(String(error));
    } finally {
      setSavingBalance(false);
    }
  };

  const save = async () => {
    if (saving || fillingTemplate || !name.trim() || (!create && !initialDetail)) return;
    // 非法 JSON 不落库：托管键以外的内容不会被静默丢弃
    const settingsText = patchEnvFields(envText, { baseUrl, authToken, model }, kind);
    const extras = splitEnvExtras(settingsText);
    if (extras === undefined) {
      feedback.error(t("envInvalid"));
      return;
    }
    setSaving(true);
    try {
      await api.claudeSaveProfile({
        id: profile?.id,
        name: name.trim(),
        baseUrl: baseUrl.trim() || null,
        authToken: authToken.trim() || null,
        model: model.trim() || null,
        description: description.trim() || null,
        fetchedModels: fetchedModels.length ? fetchedModels : null,
        kind: kind ?? null,
        adminUrl: adminUrl.trim() || null,
        extraEnv: extras,
        rawSettings: settingsText,
        icon: selectedIcon,
        showBalance,
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

  // 图标立即落库（对齐 Codex saveIcon：create 态只记本地，编辑态走专用命令）
  const saveIcon = async (icon: string | null) => {
    if (saving) return;
    setSaving(true);
    try {
      if (!create && profile) await api.claudeSetProfileIcon(profile.id, icon);
      setSelectedIcon(icon);
      onChanged();
      setPickingIcon(false);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  if (pickingIcon) return <ProfileIconEdit icon={selectedIcon} onBack={() => setPickingIcon(false)} onSave={(icon) => void saveIcon(icon)} />;

  return (
    <section
      className="apple-edit-page mx-auto flex w-full max-w-none flex-col"
      onKeyDown={(event) => {
        // 弹窗通过 Portal 冒泡到本页；Enter 只处理弹窗操作，不保存背景配置。
        if (templateOpen) return;
        if (event.key === "Enter" && !event.nativeEvent.isComposing && !(event.target instanceof Element && event.target.closest(".apple-editor-shell"))) {
          event.preventDefault();
          void save();
        }
      }}
    >
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={tProfiles("edit.back")} onClick={create ? onChangeProvider : onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
          <span className="apple-title">{create ? tProfiles("toolbar.addProvider") : tProfiles("edit.editTitle")}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          {create && selectedPreset && onChangeProvider ? <ProviderPresetControls preset={selectedPreset} baseUrl={baseUrl}
            disabled={saving || fillingTemplate} onChangeProvider={onChangeProvider} onSelect={selectConnection} /> : null}
          <div className="apple-panel-section">
            <ProviderIdentityFields
              idPrefix={create ? `claude-profile-${initialPreset!.kind}-${initialPreset!.plan?.replace(/ /g, "-")}` : "claude-profile"} name={name} description={description}
              icon={selectedIcon ?? claudePresetByKind(kind)?.icon ?? null}
              onIcon={() => setPickingIcon(true)} onName={setName} onDescription={setDescription}
              labels={{
                changeIcon: tProfiles("edit.changeIcon"), changeIconLabel: tProfiles("edit.changeIconLabel"),
                name: tProfiles("edit.nameLabel"), namePlaceholder: tProfiles("edit.namePlaceholder"),
                description: tProfiles("edit.descriptionLabel"), descriptionPlaceholder: tProfiles("edit.descriptionPlaceholder"),
              }}
            />
            {accountLogin ? <p className="setting-description mt-4 rounded-lg border-l-2 border-(--accent) bg-(--selection-bg) px-3 py-2">{t("accountLoginHelp")}</p> : <>
            <label className="field-label mb-1.5 mt-4 block">{t("protocolLabel")}</label>
            <div className="app-input flex min-w-0 items-center gap-2">
              <Webhook className="h-3.5 w-3.5 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
              <span className="shrink-0 font-medium text-(--text-secondary)">{t("protocolAnthropic")}</span>
            </div>
            <label className="field-label mb-1.5 mt-4 block">{t("baseUrlLabel")}</label>
            {presetEndpoints ? (
              <EndpointField
                value={baseUrl}
                onChange={setBaseUrl}
                endpoints={presetEndpoints}
                onPick={(endpoint) => { if (endpoint.admin_url) setAdminUrl(endpoint.admin_url); }}
                placeholder={t("baseUrlPlaceholder")}
                label={t("baseUrlLabel")}
              />
            ) : (
              <input className="app-input" placeholder={t("baseUrlPlaceholder")} value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
            )}
            <ProviderSecretField
              label={tProfiles("edit.apiKeyLabel")} placeholder={tProfiles("edit.apiKeyPlaceholder")}
              value={authToken} onChange={setAuthToken} visible={showToken}
              onToggle={() => setShowToken((visible) => !visible)}
              showLabel={tProfiles("edit.showApiKey")} hideLabel={tProfiles("edit.hideApiKey")}
              testLabel={tProfiles("edit.testConnection")}
              testTitle={!authToken.trim() || !baseUrl.trim() ? tProfiles("edit.checkProviderFields") : undefined}
              testing={testing} onTest={() => void testConnection()}
            />
            </>}
            <div className="mt-4">
              <div className="mb-1.5 flex items-center gap-2">
                <span className="field-label">{tProfiles("edit.adminUrlLabel")}</span>
                <button type="button" className="apple-inline-btn apple-inline-btn--quiet !h-5 shrink-0" disabled={!adminUrl.trim()} aria-label={tProfiles("card.openWebsite")} onClick={() => void api.openUrl(adminUrl.trim()).catch((error) => feedback.error(String(error)))}>
                  <ExternalLink className="h-3 w-3" strokeWidth={2} aria-hidden="true" />
                  {tProfiles("card.openWebsite")}
                </button>
              </div>
              <input className="app-input" placeholder={t("adminUrlPlaceholder")} value={adminUrl} onChange={(event) => setAdminUrl(event.target.value)} />
            </div>
            {supportsBalance ? <div className="mt-4 flex min-h-[var(--input-min-height)] items-center justify-between gap-4"><div className="min-w-0"><div className="setting-title">{tProfiles("edit.balanceUsage")}</div><div className="setting-description mt-0.5">{tProfiles("edit.balanceAutoRefresh")}</div></div><AppSwitch checked={showBalance} disabled={saving} label={tProfiles("edit.balanceUsage")} onCheckedChange={(value) => void toggleBalance(value)} /></div> : null}
          </div>
          <div className="apple-panel-section">
            <ProviderModelFields
              models={fetchedModels} fetching={fetchingModels}
              disabled={!authToken.trim() || !baseUrl.trim()} onFetch={() => void fetchModelList()}
              onApplyModel={applyModelToMappings}
              mappingFields={modelMappingFields}
              labels={{
                model: tProfiles("edit.modelIdLabel"),
                models: tProfiles("edit.modelsLabel"), fetch: tProfiles("edit.fetchModels"),
                available: tProfiles("edit.modelsAvailable", { count: fetchedModels.length }),
                select: tProfiles("edit.selectModel"), fetchFirst: tProfiles("edit.fetchModelsFirst"),
                quickSet: t("quickSet"),
                mappingTitle: t("mappingTitle"), mappingDescription: t("mappingDescription"),
                role: t("modelRoleLabel"), displayName: t("displayNameLabel"), requestModel: t("requestModelLabel"),
                oneMillion: t("oneMillionLabel"), oneMillionTitle: t("oneMillionTitle"),
                oneMillionColumn: t("oneMillionColumn"),
                clear: tProfiles("edit.clearFile"),
              }}
            />
          </div>
          <div className="apple-panel-section flex flex-col">
            <div className="flex min-h-8 flex-wrap items-center justify-between gap-2">
              <button type="button" className="relative flex h-8 shrink-0 items-center gap-1.5 rounded-[10px] bg-(--active-bg) px-3 text-[13px] font-semibold text-accent transition-colors" aria-pressed="true" title="settings.json">
                <FileBraces className="h-3.5 w-3.5" strokeWidth={2} />
                <span>settings.json</span>
                {envDirty ? <span className="absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full bg-accent" /> : null}
              </button>
              <div className="editor-ghost-group ml-auto h-8 shrink-0 gap-1 whitespace-nowrap bg-(--active-bg) px-2">
                <span className="field-label font-normal! flex shrink-0 items-center gap-1.5 whitespace-nowrap">
                  <LayoutTemplate className="shrink-0" size={16} strokeWidth={2} aria-hidden="true" />
                  {t("commonTemplate.title")}
                </span>
                <span className="editor-ghost-group__separator" aria-hidden="true" />
                <button type="button" className="editor-ghost shrink-0 enabled:hover:text-accent!" disabled={saving || fillingTemplate} onClick={() => void fillCommonTemplate()}>
                  {t("commonTemplate.fill")}
                </button>
                <button type="button" className="editor-ghost shrink-0 enabled:hover:text-accent!" disabled={saving || fillingTemplate} onClick={() => setTemplateOpen(true)}>
                  {t("commonTemplate.edit")}
                </button>
              </div>
              <button type="button" className="editor-ghost editor-ghost--format shrink-0" disabled={saving} title={tProfiles("edit.formatTitle", { label: "settings.json", format: "JSON" })} onClick={formatEnv}>
                <CodeXml className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                <span className="whitespace-nowrap font-medium">{tProfiles("edit.format")}</span>
              </button>
            </div>
            <div className="mt-2 flex flex-col">
              <div className="editor-attach-group">
                <div className="editor-attach-bar flex-nowrap! items-start!">
                  <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
                    <div className="editor-ghost-group pl-1">
                      <label className="editor-ghost" title={t("advanced.autoCompactTitle")}>
                        <input type="checkbox" checked={!autoCompactDisabled} onChange={(event) => updateEnvValue("DISABLE_AUTO_COMPACT", event.target.checked ? null : "1", "autoCompactEnabled")} />
                        <span className="whitespace-nowrap font-medium">{t("advanced.autoCompactLabel")}</span>
                      </label>
                      <span className="editor-ghost-group__separator" aria-hidden="true" />
                      <label className={`editor-ghost ${autoCompactDisabled ? "opacity-40 pointer-events-none" : ""}`} title={t("advanced.compactThresholdTitle")}>
                        <span className="whitespace-nowrap">{t("advanced.compactThresholdLabel")}</span>
                        <input className={`app-input app-input--compact compact-token-input ${autoCompactDisabled ? "" : "compact-token-input--on"} h-6 text-center`} type="number" min={100000} max={1000000} step={100000} inputMode="numeric" value={autoCompactWindow} placeholder={t("advanced.compactWindowPlaceholder")} disabled={autoCompactDisabled} onChange={(event) => updateEnvValue("CLAUDE_CODE_AUTO_COMPACT_WINDOW", event.target.value, "autoCompactWindow")} />
                      </label>
                    </div>
                    <label className="editor-ghost" title={t("advanced.gitAttributionTitle")}>
                      <input type="checkbox" checked={gitAttributionHidden} onChange={(event) => toggleGitAttribution(event.target.checked)} />
                      <span className="whitespace-nowrap font-medium">{t("advanced.gitAttributionLabel")}</span>
                    </label>
                    <label className="editor-ghost" title={t("advanced.agentTeamsTitle")}>
                      <input type="checkbox" checked={agentTeamsEnabled} onChange={(event) => updateEnvValue("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", event.target.checked ? "1" : null)} />
                      <span className="whitespace-nowrap font-medium">{t("advanced.agentTeamsLabel")}</span>
                    </label>
                    <div
                      className="inline-flex h-7 items-center gap-2 px-2.5 field-label muted"
                      title={t("advanced.effortTitle")}
                      onKeyDown={(event) => { if (event.key === "Enter") event.stopPropagation(); }}
                    >
                      <span className="whitespace-nowrap font-medium">{t("advanced.effortLabel")}</span>
                      <div className="editor-ghost--format bg-transparent! w-20 shrink-0 rounded-md">
                        <AppSelect
                          value={effortLevel}
                          options={[
                            { value: "", label: "Default" },
                            ...(effortLevel && !effortLevels.some((level) => level === effortLevel)
                              ? [{ value: effortLevel, label: effortLevel }] : []),
                            ...effortLevels.map((level) => ({ value: level, label: level })),
                          ]}
                          onChange={(value) => updateEnvValue("CLAUDE_CODE_EFFORT_LEVEL", value || null, "effortLevel")}
                          placeholder={t("advanced.effortLabel")}
                          disabled={saving}
                          menuWidth="7rem"
                          compact
                        />
                      </div>
                    </div>
                  </div>
                  <div className="shrink-0" onKeyDown={(event) => { if (event.key === "Enter") event.stopPropagation(); }}>
                    <AppSelect
                      value={null}
                      options={[
                        { value: "bypassPermissions", label: t("advanced.bypassPermissionsLabel") },
                        { value: "bashEditDiff", label: t("advanced.bashEditDiffLabel") },
                        { value: "autoMemory", label: t("advanced.autoMemoryLabel") },
                      ]}
                      checkedValues={[
                        ...(!autoMemoryEnabled ? ["autoMemory"] : []),
                        ...(bashEditDiffEnabled ? ["bashEditDiff"] : []),
                        ...(bypassPermissionsEnabled ? ["bypassPermissions"] : []),
                      ]}
                      onChange={(value) => {
                        if (value === "autoMemory") updateEnvValue("CLAUDE_CODE_DISABLE_AUTO_MEMORY", autoMemoryEnabled ? "1" : null, "autoMemoryEnabled");
                        else if (value === "bashEditDiff") updateEnvValue("CLAUDE_CODE_BASH_EDIT_DIFF", bashEditDiffEnabled ? null : "1", "bashEditDiffEnabled");
                        else if (value === "bypassPermissions") {
                          const next = patchBypassPermissions(envText, !bypassPermissionsEnabled);
                          if (!bypassPermissionsEnabled) editorRef.current?.revealField(next, "defaultMode");
                          setEnvText(next);
                        }
                      }}
                      renderLabel={(option) => (
                        <span title={t(option.value === "autoMemory" ? "advanced.autoMemoryTitle"
                          : option.value === "bashEditDiff" ? "advanced.bashEditDiffTitle" : "advanced.bypassPermissionsTitle")}>
                          {option.label}
                        </span>
                      )}
                      placeholder={t("advanced.moreLabel")}
                      disabled={saving}
                      iconOnly
                      compact
                    />
                  </div>
                </div>
                <div className="flex flex-col">
                  <ConfigTextEditor ref={editorRef} value={envText} language="json" minLines={envMinLines} placeholder={t("envPlaceholder")} onChange={setEnvText} onDiagnostics={setEditorDiagnostics} />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        {editorDiagnostics.count > 0 ? (
          <button type="button" className="chip-danger mr-auto flex min-w-0 items-center gap-1.5 rounded-lg border border-[var(--danger)]/20 bg-(--danger)/10 px-2.5 py-1 text-xs" aria-live="polite" onClick={() => editorRef.current?.focusFirstDiagnostic()}>
            <span className="h-1.5 w-1.5 rounded-full bg-(--danger)" />
            {tProfiles("edit.diagnosticsErrors", { count: editorDiagnostics.count })}
            {editorDiagnostics.firstLine !== null ? tProfiles("edit.diagnosticsLine", { line: editorDiagnostics.firstLine }) : ""}
          </button>
        ) : null}
        <button type="button" className="apple-action-button" onClick={onBack}>{tProfiles("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={saving || fillingTemplate || !name.trim()} onClick={() => void save()}>
          <Save className="h-4 w-4" strokeWidth={2} />
          {saving ? tProfiles("edit.saving") : tProfiles("dialog.save")}
        </button>
      </div>
      {templateOpen ? <ClaudeCommonTemplateDialog sourceText={envText} onClose={() => setTemplateOpen(false)} /> : null}
    </section>
  );
}
