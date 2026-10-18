import { ChevronLeft, CodeXml, Download, ExternalLink, FileBraces, Save, Settings, Webhook } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { connectionErrorMessage, connectionExceptionMessage, connectionFailureMessage } from "../profiles/connectionText";
import { useFeedback } from "../../app/Feedback";
import { AuthSourceIcon } from "../../components/AuthSourceIcon";
import { AppSelect } from "../../components/AppSelect";
import { AppSwitch } from "../../components/AppSwitch";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { codexEffortLevels } from "../../components/ReasoningEffortSlider";
import ConfigTextEditor, { type ConfigTextEditorHandle } from "../../components/ConfigTextEditor";
import { DiagnosticsChip } from "../../components/DiagnosticsChip";
import EndpointField from "../../components/EndpointField";
import { ProviderCreate, ProviderPresetControls } from "../../components/ProviderSetup";
import { ProviderIdentityFields, ProviderSecretField } from "../../components/ProviderFields";
import {
  balanceQueryProviders,
  codexBuiltinHasCatalog,
  codexPresets,
  codexCustomCatalogTemplate,
  codexCustomConfigTemplate,
  presetConnectionName,
  type ClientPreset,
  type PresetConnection,
} from "../../presets";
import {
  chatgptModelOptions,
  emptyConfigSeed,
  patchModelValue,
  patchProviderFields,
  readCatalogOptions,
  readModelValue,
  readProviderFields,
  resolveAuthSource,
  withMcpSection,
  withoutApiKeyPlaceholder,
} from "./profileEditText";
import ProfileAdvancedControls from "./editor/ProfileAdvancedControls";
import TabFileControls from "./editor/TabFileControls";
import { useProfileAdvancedPatches } from "./editor/useProfileAdvancedPatches";
import type { AuthStatus, EditorDiagnosticSummary, CodexProfileDetail, CodexProfileSummary } from "../../types";
import ProfileIconEdit from "../profiles/ProfileIconEdit";
import { getModelChoices, resolveEffortLevels, resolveModelSelection, selectModel } from "../profiles/ProfileModelSelector";

type EditTab = "config" | "auth" | "models";

interface CodexProfileEditProps {
  profile: CodexProfileSummary | null;
  create?: boolean;
  initialDetail?: CodexProfileDetail | null;
  authStatus: AuthStatus;
  authStatusReady: boolean;
  onBack: () => void;
  onChanged: () => void;
  onManageChatgptAccounts: () => void;
}

const manageChatgptAccountsValue = "__manage_chatgpt_accounts__";

function normalizeNewlines(text: string) {
  return text.replace(/\r\n/g, "\n");
}

export default function CodexProfileEdit(props: CodexProfileEditProps) {
  return props.create ? <ProviderCreate presets={codexPresets} onBack={props.onBack}>
    {(preset, onChangeProvider) => <CodexProfileEditor {...props} initialPreset={preset} onChangeProvider={onChangeProvider} />}
  </ProviderCreate> : <CodexProfileEditor {...props} />;
}

function CodexProfileEditor({
  profile, create = false, initialDetail = null, authStatus, authStatusReady,
  onBack, onChanged, onManageChatgptAccounts, initialPreset, onChangeProvider,
}: CodexProfileEditProps & {
  initialPreset?: ClientPreset;
  onChangeProvider?: () => void;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("profiles");
  const initialConfigText = useMemo(
    () => withoutApiKeyPlaceholder(initialDetail?.raw_config ?? initialDetail?.config_fragment ?? ""),
    [initialDetail],
  );
  // 详情由调用方预载（openEdit）：门控与表单/编辑器内容状态全部同源初始化（初始化器与挂载 effect
  // 同一数据源），首帧即完整内容；挂载后 effect 仍会重取最新值，值未变时 React 跳过重渲染。
  const [detail, setDetail] = useState<CodexProfileDetail | null>(initialDetail);
  const [loadError, setLoadError] = useState("");
  const [saving, setSaving] = useState(false);
  const [formatting, setFormatting] = useState(false);
  const [testing, setTesting] = useState(false);
  const [pickingIcon, setPickingIcon] = useState(false);
  const [name, setName] = useState(() => profile?.name ?? (initialPreset && initialPreset.kind !== "custom" ? presetConnectionName(initialPreset) : ""));
  const [description, setDescription] = useState(initialDetail?.description ?? "");
  const [baseUrl, setBaseUrl] = useState(() => initialDetail?.base_url ?? initialPreset?.base_url ?? "");
  const [apiKey, setApiKey] = useState(() => initialDetail?.api_key ?? "");
  const [showApiKey, setShowApiKey] = useState(false);
  const [modelValue, setModelValue] = useState(() => readModelValue(initialConfigText) ?? initialPreset?.model ?? "");
  const [fetchedModels, setFetchedModels] = useState<string[]>(() => initialDetail?.fetched_models ?? []);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [adminUrl, setAdminUrl] = useState(() => initialDetail?.admin_url ?? initialPreset?.admin_url ?? "");
  const authAccounts = authStatus.accounts;
  const [boundAccountId, setBoundAccountId] = useState<string | null>(() => initialDetail?.account_id ?? null);
  const [selectedIcon, setSelectedIcon] = useState<string | null>(initialDetail?.icon ?? profile?.icon ?? initialPreset?.icon ?? null);
  const [presetKind, setPresetKind] = useState(initialPreset?.kind ?? "");
  const [activeTab, setActiveTab] = useState<EditTab>("config");
  const [configText, setConfigText] = useState(() => initialConfigText);
  const [catalogText, setCatalogText] = useState(() => initialDetail?.raw_catalog ?? initialDetail?.catalog_content ?? "");
  const [authText, setAuthText] = useState(() => initialDetail?.raw_auth ?? "");
  const [configInitial, setConfigInitial] = useState(() => initialConfigText);
  const [catalogInitial, setCatalogInitial] = useState(() => initialDetail?.raw_catalog ?? initialDetail?.catalog_content ?? "");
  const [authInitial, setAuthInitial] = useState(() => initialDetail?.raw_auth ?? "");
  const [authPreviewOnly, setAuthPreviewOnly] = useState(false);
  const [authPreviewReady, setAuthPreviewReady] = useState(false);
  const [configTouched, setConfigTouched] = useState(false);
  const [catalogTouched, setCatalogTouched] = useState(false);
  // 新增态默认开启余额显示；编辑态先复用列表值，避免详情加载后才从关闭态播放到已存开启态。
  const [showBalance, setShowBalance] = useState(create || Boolean(profile?.show_balance));
  const [savingBalance, setSavingBalance] = useState(false);
  const [editorDiagnostics, setEditorDiagnostics] = useState<EditorDiagnosticSummary>({ count: 0, firstLine: null });
  const [mcpSection, setMcpSection] = useState("");
  const initialized = useRef(false);
  const authPreviewRequest = useRef(0);
  const modelListRequest = useRef(0);
  const fetchedModelEfforts = useRef<Record<string, string[]>>(initialDetail?.fetched_model_efforts ?? {});
  const fetchedModelDefaults = useRef<Record<string, string>>(initialDetail?.fetched_model_defaults ?? {});
  const presetTemplateRequest = useRef(0);
  const editorRef = useRef<ConfigTextEditorHandle>(null);
  const editorMinLines = Math.max(configText.split(/\r?\n/).length, catalogText.split(/\r?\n/).length, authText.split(/\r?\n/).length);

  const selectedPreset = initialPreset ?? null;
  // 创建态由分段控件选择端点；已有配置仍按 provider id 反查地址输入框的端点档。
  const presetEndpoints = (!create
    ? codexPresets.find((item) => item.provider != null && item.provider === detail?.provider)?.endpoints : null) ?? null;
  // 端点地址输入：双区域档（EndpointField combobox）与普通输入共用同一受控值，只差形态
  const baseUrlField = (className: string) => (
    <input className={className} placeholder="https://api.example.com/v1" value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
  );
  const isCustom = create && presetKind === "custom";
  const isOfficial = create ? presetKind === "chatgpt" : profile?.kind === "official";
  const isOpenCode = create ? presetKind === "opencode" : detail?.provider === "opencode-go";
  const showProviderFields = create ? (isCustom || Boolean(selectedPreset?.base_url)) : Boolean(detail?.provider);
  const showLongContextOverride = isOfficial;
  const advanced = useProfileAdvancedPatches({
    configText, setConfigText, initialized, showLongContextOverride,
    onPatched: (text, field) => editorRef.current?.revealField(text, field),
  });
  const supportsBalance = create ? presetKind === "chatgpt" || balanceQueryProviders.has(selectedPreset?.provider ?? "") : isOfficial || balanceQueryProviders.has(detail?.provider ?? "");
  // 创建态下预设的 config 原文统一从后端取（单源真相），避免与 Rust 模板双份维护。
  // 与 configText 同源同时设置（selectPreset 内 await 后一起 set），防止异步晚到
  // 触发 configText !== liveConfigFragment 的误判
  const [presetFragment, setPresetFragment] = useState("");
  const authSource = create
    ? boundAccountId ? "oauth" : "desktop"
    : resolveAuthSource(detail);
  const configDirty = normalizeNewlines(configText) !== normalizeNewlines(configInitial);
  const catalogDirty = normalizeNewlines(catalogText) !== normalizeNewlines(catalogInitial);
  const authDirty = normalizeNewlines(authText) !== normalizeNewlines(authInitial);
  const liveCatalogPath = useMemo(() => {
    const match = /^\s*model_catalog_json\s*=\s*(?:"([^"]*)"|'([^']*)'|(\S+))/m.exec(configText);
    return match ? match[1] ?? match[2] ?? match[3] ?? "" : "";
  }, [configText]);
  const catalogOptions = useMemo(() => {
    try { return readCatalogOptions(catalogText); }
    catch { return readCatalogOptions(null); } // 编辑中的无效 JSON 由编辑器诊断，不能中断表单。
  }, [catalogText]);
  const useLocalCatalog = isOfficial && !!liveCatalogPath && !!catalogText.trim();
  const modelEfforts = useLocalCatalog || !isOfficial ? catalogOptions.efforts : fetchedModelEfforts.current;
  const models = useLocalCatalog ? catalogOptions.models : fetchedModels;
  const selection = resolveModelSelection({ model: modelValue, effort: readModelValue(configText, "model_reasoning_effort") ?? "" }, {
    model: "", effort: "", models, defaults: isOfficial ? useLocalCatalog ? catalogOptions.defaults : fetchedModelDefaults.current : undefined,
  });
  const effortLevels = resolveEffortLevels(selection.model, modelEfforts, codexEffortLevels);
  const catalogFileName = liveCatalogPath.split(/[\\/]/).pop() || "models.json";
  const formatTarget = activeTab === "config"
    ? { label: "config.toml", title: t("edit.formatTitle", { label: "config.toml", format: "TOML" }) }
    : activeTab === "auth"
      ? { label: "auth.json", title: t("edit.formatTitle", { label: "auth.json", format: "JSON" }) }
      : { label: catalogFileName, title: t("edit.formatTitle", { label: catalogFileName, format: "JSON" }) };
  // auth.json 仅官方档有认证语义；第三方档只有携带历史 raw_auth 快照时才显示（防御旧数据）。
  const showAuthTab = create
    ? isCustom || (isOfficial && authSource === "desktop")
    : isOfficial || Boolean(detail?.raw_auth);
  const tabs = useMemo(() => {
    const list: { id: EditTab; label: string; title?: string }[] = [{ id: "config", label: "config.toml" }];
    if (liveCatalogPath) list.push({ id: "models", label: catalogFileName, title: liveCatalogPath });
    if (showAuthTab) list.push({ id: "auth", label: "auth.json" });
    return list;
  }, [catalogFileName, liveCatalogPath, showAuthTab]);
  const baseFragment = create ? presetFragment : detail?.config_fragment ?? "";
  const configSeed = emptyConfigSeed(baseFragment);
  const liveConfigFragment = useMemo(() => {
    if (!baseFragment) return "";
    return withMcpSection(patchProviderFields(baseFragment, baseUrl, apiKey), mcpSection);
  }, [apiKey, baseFragment, baseUrl, mcpSection]);
  // 格式化入口与文件标签同一行，文件操作仍保留在编辑器附属条。
  const formatButton = (
    <button type="button" className="editor-ghost editor-ghost--format ml-auto shrink-0" disabled={saving || (activeTab === "auth" && authPreviewOnly)} title={formatTarget.title} onClick={() => void formatCurrentDocument()}>
      <CodeXml className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
      <span className="whitespace-nowrap font-medium">{t("edit.format")}</span>
    </button>
  );
  const canSave = (!create || Boolean(presetFragment) && (isCustom ? Boolean(configText.trim()) : Boolean(selectedPreset))) && (!isOfficial || !authPreviewOnly || authPreviewReady);
  const accountOptions = [
    { label: t("card.authDesktop"), value: "" },
    ...authAccounts.map((account) => ({ label: account.login, value: account.id })),
    { label: t("edit.addChatgptAccount"), value: manageChatgptAccountsValue },
  ];
  const oauthAccountOptions = [
    ...authAccounts.map((account) => ({ label: account.login, value: account.id })),
    { label: t("edit.addChatgptAccount"), value: manageChatgptAccountsValue },
  ];
  const renderAccountLabel = (option: { label: string; value: string }) => {
    if (option.value === manageChatgptAccountsValue) {
      return <span className="font-medium text-accent">{option.label}</span>;
    }
    const desktop = option.value === "";
    return <span className="inline-flex min-w-0 items-center gap-2"><AuthSourceIcon source={desktop ? "desktop" : "oauth"} className="h-3.5 w-3.5 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" /><span className="shrink-0">{desktop ? t("card.authDesktop") : t("card.authOAuth")}</span>{desktop ? null : <><span className="text-[var(--text-secondary)]">·</span><span className="truncate">{option.label}</span></>}</span>;
  };
  const refreshAuthPreview = async (accountId: string) => {
    const requestId = ++authPreviewRequest.current;
    setAuthPreviewOnly(true);
    setAuthPreviewReady(false);
    try {
      const preview = await api.authPreview(accountId);
      if (requestId !== authPreviewRequest.current) return;
      setAuthText(preview ?? "");
      setAuthInitial(preview ?? "");
      setAuthPreviewReady(true);
    } catch {
      // 预览失败时保留当前内容，避免切换过程中出现错误提示或空白闪烁。
    }
  };
  const selectAccount = (value: string) => {
    if (value === manageChatgptAccountsValue) {
      onManageChatgptAccounts();
      return;
    }
    setBoundAccountId(value || null);
    modelListRequest.current++;
    setFetchedModels([]);
    fetchedModelEfforts.current = {};
    fetchedModelDefaults.current = {};
    if (create && value && activeTab === "auth") setActiveTab("config");
    if (!create && authSource === "oauth" && value) void refreshAuthPreview(value);
  };

  useEffect(() => {
    if (authSource === "desktop") modelListRequest.current++;
  }, [authText, authSource]);

  useEffect(() => {
    if (create && initialized.current) return;
    let cancelled = false;
    void (async () => {
      if (create) {
        let initialMcpSection = "";
        try { initialMcpSection = (await api.codexGetMcpSectionToml()).trim(); setMcpSection(initialMcpSection); } catch { /* backend falls back on save */ }
        if (cancelled) return;
        await selectPreset(initialMcpSection);
      } else if (profile) {
        try {
          const loaded = await api.codexGetProfile(profile.id);
          if (cancelled) return;
          const loadedConfigText = withoutApiKeyPlaceholder(
            loaded.raw_config ?? loaded.config_fragment,
          );
          setDetail(loaded);
          setName(loaded.name);
          setDescription(loaded.description ?? "");
          setConfigText(loadedConfigText);
          setCatalogText(loaded.raw_catalog ?? loaded.catalog_content ?? "");
          setAuthText(loaded.raw_auth ?? "");
          setConfigInitial(loadedConfigText);
          setCatalogInitial(loaded.raw_catalog ?? loaded.catalog_content ?? "");
          setAuthInitial(loaded.raw_auth ?? "");
          setBaseUrl(loaded.base_url ?? "");
          setApiKey(loaded.api_key ?? "");
          setModelValue(readModelValue(loadedConfigText) ?? "");
          if (loaded.provider !== null) {
            setFetchedModels(loaded.fetched_models);
            fetchedModelEfforts.current = loaded.fetched_model_efforts ?? {};
            fetchedModelDefaults.current = loaded.fetched_model_defaults ?? {};
          }
          setAdminUrl(loaded.admin_url ?? "");
          setSelectedIcon(loaded.icon);
          setBoundAccountId(loaded.account_id);
          setShowBalance(loaded.show_balance);
          advanced.syncFromConfig(loadedConfigText, loaded.provider === null);
          if (loaded.provider === null) {
            const source = resolveAuthSource(loaded);
            if (source === "oauth") {
              setAuthPreviewOnly(true);
              if (loaded.account_id) {
                await refreshAuthPreview(loaded.account_id);
              } else {
                setAuthPreviewReady(false);
              }
            } else {
              setAuthPreviewOnly(false);
              setAuthPreviewReady(true);
            }
          }
        } catch (error) {
          setLoadError(String(error));
        }
      }
      if (!cancelled) initialized.current = true;
    })();
    return () => { cancelled = true; };
    // profile identity is fixed for this mounted editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (create && catalogDirty) return;
    if (!create || !presetKind || isCustom) {
      if (create && !isCustom) setCatalogText("");
      return;
    }
    if (!codexBuiltinHasCatalog(presetKind)) {
      setCatalogText("");
      return;
    }
    void api.codexGetBuiltinCatalog(presetKind).then((text) => {
      setCatalogText(text ?? "");
      setCatalogInitial(text ?? "");
    }).catch(() => setCatalogText(""));
  }, [create, isCustom, presetKind]);

  useEffect(() => {
    if (!initialized.current) return;
    setConfigText((current) => {
      const source = !current.trim() && (baseUrl.trim() || apiKey.trim()) ? configSeed : current;
      const next = patchProviderFields(source, baseUrl, apiKey);
      return next === current ? current : next;
    });
  }, [apiKey, baseUrl]);

  // 表单模型值 ↔ 编辑器顶层 model 行 双向同步（与地址/密钥同一模式）
  useEffect(() => {
    if (!initialized.current) return;
    setConfigText((current) => {
      const source = !current.trim() && modelValue.trim() ? configSeed : current;
      const effort = readModelValue(source, "model_reasoning_effort") ?? "";
      const picked = selectModel(modelValue, effort, modelEfforts[modelValue]);
      let next = patchModelValue(source, modelValue);
      if (picked.effort !== effort) next = patchModelValue(next, picked.effort, "model_reasoning_effort");
      return next === current ? current : next;
    });
  }, [modelValue]);

  useEffect(() => {
    if (!initialized.current) return;
    const model = readModelValue(configText);
    if (model !== null || !configText.trim()) {
      if ((model ?? "") !== modelValue) setModelValue(model ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configText]);

  useEffect(() => {
    if (!initialized.current) return;
    const fields = readProviderFields(configText);
    if (fields.found || !configText.trim()) {
      if (fields.base_url !== baseUrl) setBaseUrl(fields.base_url);
      if (!fields.tokenMasked) {
        const key = /^<.*>$/.test(fields.experimental_bearer_token) ? "" : fields.experimental_bearer_token;
        if (key !== apiKey) setApiKey(key);
      }
    }
  }, [configText]);

  useEffect(() => {
    if (create && configText !== liveConfigFragment) setConfigTouched(true);
  }, [configText, create, liveConfigFragment]);

  const selectPreset = async (currentMcpSection = mcpSection) => {
    if (!initialPreset) return;
    const preset = initialPreset;
    const kind = preset.kind;
    modelListRequest.current++;
    // 模板取回后才一次性更新全部状态：避免"表单已切、configText 未切"的中间渲染
    // 触发 configText !== liveConfigFragment 的 configTouched 误判
    const requestId = ++presetTemplateRequest.current;
    let template: string;
    try {
      template = kind === "custom" ? codexCustomConfigTemplate : await api.codexGetBuiltinConfig(kind);
    } catch (error) {
      if (requestId === presetTemplateRequest.current) setLoadError(t("edit.errorTemplateRead", { error: String(error) }));
      return;
    }
    if (requestId !== presetTemplateRequest.current) return;
    setLoadError("");
    setPresetKind(kind);
    // 重置为该预设的默认值：支持余额查询的预设默认开启（与 supportsBalance 渲染条件同一规则）
    setShowBalance(kind === "chatgpt" || balanceQueryProviders.has(preset.provider ?? ""));
    if (kind !== "chatgpt") setBoundAccountId(null);
    setConfigTouched(false);
    setCatalogTouched(false);
    setAuthText("");
    setAuthInitial("");
    setName(kind === "custom" ? "" : presetConnectionName(preset));
    setDescription("");
    setBaseUrl(preset.base_url);
    setApiKey("");
    setShowApiKey(false);
    setModelValue(kind === "custom" ? "" : preset.model);
    setFetchedModels([]);
    fetchedModelEfforts.current = {};
    fetchedModelDefaults.current = {};
    setAdminUrl(preset.admin_url ?? "");
    setSelectedIcon(preset.icon);
    setPresetFragment(template);
    if (kind === "custom") {
      setCatalogText(codexCustomCatalogTemplate);
      setCatalogInitial(codexCustomCatalogTemplate);
    }
    const nextConfig = withMcpSection(patchProviderFields(template, preset.base_url, ""), currentMcpSection);
    setConfigText(nextConfig);
    setConfigInitial(nextConfig);
    advanced.syncFromConfig(nextConfig, kind === "chatgpt");
    setActiveTab("config");
  };

  const selectConnection = (connection: PresetConnection) => {
    setBaseUrl(connection.base_url);
    setAdminUrl(connection.admin_url ?? "");
    modelListRequest.current++;
    setFetchingModels(false);
    setFetchedModels([]);
    fetchedModelEfforts.current = {};
    fetchedModelDefaults.current = {};
  };

  const formatCurrentDocument = async () => {
    if (formatting || saving) return;
    if (activeTab === "auth" && authPreviewOnly) { feedback.info(t("edit.authPreviewReadonly")); return; }
    const text = activeTab === "config" ? configText : activeTab === "auth" ? authText : catalogText;
    if (!text.trim()) { feedback.warning(t("edit.formatEmpty")); return; }
    setFormatting(true);
    try {
      const formatted = activeTab === "config" ? await api.formatToml(text) : JSON.stringify(JSON.parse(text), null, 2);
      if (formatted === text) feedback.info(t("edit.formatNoChange", { label: formatTarget.label }));
      else {
        if (activeTab === "config") setConfigText(formatted);
        else if (activeTab === "auth") setAuthText(formatted);
        else setCatalogText(formatted);
        feedback.success(t("edit.formatSuccess", { label: formatTarget.label }));
      }
    } catch (error) { feedback.error(t("edit.formatFailed", { error: String(error) })); }
    finally { setFormatting(false); }
  };

  const testConnection = async () => {
    if (testing) return;
    if (!baseUrl.trim()) { feedback.warning(t("edit.baseUrlRequired")); return; }
    if (!apiKey.trim()) { feedback.warning(t("edit.apiKeyRequired")); return; }
    setTesting(true);
    try {
      const result = create ? await api.codexTestProviderConnection(baseUrl.trim(), apiKey.trim()) : await api.codexTestProfileConnection(profile!.id, baseUrl.trim(), apiKey.trim());
      if (result.ok && result.status != null && result.status >= 400) feedback.info(t("connection.reachable"));
      else if (result.ok) feedback.success(t("connection.ok", { latency: result.latency_ms != null ? ` · ${result.latency_ms}ms` : "" }));
      else feedback.error(connectionFailureMessage(create || profile?.provider ? connectionErrorMessage(result, t) : result.error ?? t("connection.unknownError"), t, result.status));
    } catch (error) { feedback.error(connectionFailureMessage(String(error), t)); }
    finally { setTesting(false); }
  };

  const fetchModelList = async () => {
    if (fetchingModels || useLocalCatalog) return;
    if (!isOfficial && !baseUrl.trim()) { feedback.warning(t("edit.baseUrlRequired")); return; }
    if (!isOfficial && !apiKey.trim()) { feedback.warning(t("edit.apiKeyRequired")); return; }
    setFetchingModels(true);
    const request = ++modelListRequest.current;
    try {
      const catalog = isOfficial
        ? await api.codexFetchChatgptModels(!create && boundAccountId === detail?.account_id ? profile!.id : null, authSource ?? "desktop", boundAccountId) : null;
      const options = chatgptModelOptions(catalog ?? []);
      const models = catalog ? options.models : await api.codexFetchProviderModels(baseUrl.trim(), apiKey.trim());
      if (request !== modelListRequest.current) return;
      setFetchedModels(models);
      fetchedModelEfforts.current = options.efforts;
      fetchedModelDefaults.current = options.defaults;
      if (!create && profile && !isOfficial) await api.codexSetProfileFetchedModels(profile.id, models, fetchedModelEfforts.current, fetchedModelDefaults.current);
      if (models.length === 0) feedback.info(t("edit.noModelsReturned"));
      else feedback.success(t("edit.modelsFetched", { count: models.length }));
    } catch (error) {
      if (request === modelListRequest.current) feedback.error(t("edit.fetchFailed", { error: connectionExceptionMessage(error, t) }));
    }
    finally { if (request === modelListRequest.current) setFetchingModels(false); }
  };

  useEffect(() => {
    if (!isOfficial || useLocalCatalog || (!create && !detail) || (authSource === "desktop" && authDirty)) return;
    setFetchedModels([]);
    fetchedModelEfforts.current = {};
    fetchedModelDefaults.current = {};
    const request = ++modelListRequest.current;
    void api.codexFetchChatgptModels(!create && boundAccountId === detail?.account_id ? profile!.id : null, authSource ?? "desktop", boundAccountId, false).then((catalog) => {
      if (request !== modelListRequest.current) return;
      const options = chatgptModelOptions(catalog);
      setFetchedModels(options.models);
      fetchedModelEfforts.current = options.efforts;
      fetchedModelDefaults.current = options.defaults;
    }).catch((error: unknown) => {
      if (request === modelListRequest.current) feedback.error(t("edit.fetchFailed", { error: connectionExceptionMessage(error, t) }));
    });
    return () => { modelListRequest.current++; setFetchingModels(false); };
    // 认证归属变化时只读缓存，缓存为空不联网补取。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOfficial, useLocalCatalog, boundAccountId, detail?.id, authSource]);

  const saveIcon = async (icon: string | null) => {
    if (saving) return;
    setSaving(true);
    try {
      if (create) setSelectedIcon(icon);
      else await api.codexSetProfileIcon(profile!.id, icon);
      setSelectedIcon(icon);
      onChanged();
      setPickingIcon(false);
    } catch (error) { feedback.error(String(error)); }
    finally { setSaving(false); }
  };

  const toggleBalance = async (enabled: boolean) => {
    if (savingBalance) return;
    setShowBalance(enabled);
    if (create || !profile) return;
    setSavingBalance(true);
    try { await api.codexSetProfileShowBalance(profile.id, enabled); }
    catch (error) { setShowBalance(!enabled); feedback.error(String(error)); }
    finally { setSavingBalance(false); }
  };

  const notifySaved = (message: string) => {
    const missingApiKey = !apiKey.trim();
    const missingBaseUrl = !baseUrl.trim();
    if (!isOfficial && showProviderFields && (missingApiKey || missingBaseUrl)) {
      const fields = missingApiKey && missingBaseUrl
        ? t("edit.apiKeyAndRequestUrl")
        : missingApiKey
          ? t("edit.apiKeyLabel")
          : t("edit.requestUrlLabel");
      feedback.warning(t("edit.savedWithMissingFields", {
        message: create ? t("edit.providerAdded") : t("edit.providerUpdated"),
        fields,
      }));
      return;
    }
    feedback.success(message);
  };

  const save = async () => {
    if (saving || !canSave) return;
    if (create && isCustom && !configText.trim()) { feedback.error(t("edit.saveConfigRequired")); return; }
    setSaving(true);
    try {
      if (create && isCustom) {
        const created = await api.codexAddCustomProfile(name.trim() || t("edit.customProviderName"), description, configText, baseUrl.trim() || undefined, apiKey.trim() || undefined, adminUrl.trim() || undefined, liveCatalogPath && catalogText.trim() ? catalogText : null, authText.trim() ? authText : null);
        if (fetchedModels.length) await api.codexSetProfileFetchedModels(created.id, fetchedModels, fetchedModelEfforts.current, fetchedModelDefaults.current);
        notifySaved(t("edit.customProviderAdded"));
      } else if (create) {
        const created = await api.codexAddBuiltinProfile(presetKind, description, baseUrl.trim() || undefined, apiKey.trim() || undefined, adminUrl.trim() || undefined, isOfficial ? boundAccountId || undefined : undefined);
        const customName = name.trim();
        if (customName && customName !== selectedPreset?.name) await api.renameProfile(created.id, customName);
        const authTextToSave = isOfficial && authSource === "desktop" && authDirty ? authText : null;
        if (configTouched || catalogTouched || authTextToSave !== null) {
          await api.codexUpdateProfileConfig(created.id, configText, liveCatalogPath ? catalogText || null : null, authTextToSave);
        }
        if (!isOfficial && fetchedModels.length) await api.codexSetProfileFetchedModels(created.id, fetchedModels, fetchedModelEfforts.current, fetchedModelDefaults.current);
        if (showBalance) await api.codexSetProfileShowBalance(created.id, true);
        notifySaved(t("edit.builtinProviderAdded"));
      } else {
        const hasProvider = Boolean(detail?.provider);
        await api.codexUpdateProfile(profile!.id, name, description, hasProvider ? baseUrl : undefined, hasProvider ? apiKey : undefined, adminUrl.trim() || undefined);
        const authTextToSave = isOfficial && authSource === "desktop" && authDirty ? authText : null;
        // 清空目录要传空串（后端归一为解除托管）；`|| null` 会把清空吞成"不动目录"
        await api.codexUpdateProfileConfig(profile!.id, configText, liveCatalogPath && catalogDirty ? catalogText : null, authTextToSave);
        if (isOfficial && authSource === "oauth") {
          if (!boundAccountId) throw new Error(t("edit.oauthAccountRequired"));
          await api.codexSetProfileAccount(profile!.id, boundAccountId);
        }
        notifySaved(t("edit.providerUpdated"));
      }
      onChanged();
      onBack();
    } catch (error) { feedback.error(String(error)); }
    finally { setSaving(false); }
  };

  const needsAuthStatus = create ? isOfficial : profile?.auth_source === "oauth" || Boolean(profile?.account_id);
  const authStatusPending = needsAuthStatus && !authStatusReady;
  const effortField = <div className="min-w-0 w-28">
    <div className="field-label mb-1.5 flex h-6 items-center">{t("modelSelection.effort")}</div>
    <AppSelect value={selection.effort} disabled={saving || !effortLevels.length}
      options={[{ value: "", label: t("modelSelection.default") }, ...effortLevels.map((level) => ({ value: level, label: level }))]}
      placeholder={t("modelSelection.effort")}
      onChange={(value) => setConfigText((current) => patchModelValue(current, value, "model_reasoning_effort"))} />
  </div>;
  if (((!create && !detail) || authStatusPending) && !loadError) return null;
  if (pickingIcon) return <ProfileIconEdit icon={selectedIcon} onBack={() => setPickingIcon(false)} onSave={(icon) => void saveIcon(icon)} />;

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col" onKeyDown={(event) => {
      if (event.key === "Enter" && !event.nativeEvent.isComposing && !(event.target instanceof Element && event.target.closest(".apple-editor-shell"))) {
        event.preventDefault();
        void save();
      }
    }}>
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("edit.back")} onClick={create ? onChangeProvider : onBack}><ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" /><span className="apple-title">{create ? t("toolbar.addProvider") : t("edit.editTitle")}</span></button>
      </div>
      <div className="apple-edit-content">
        {loadError ? <p className="muted mt-4 text-sm">{loadError}{create ? <button type="button" className="apple-inline-btn ml-2" onClick={() => void selectPreset()}>{t("edit.retryTemplate")}</button> : null}</p> : null}
        <div className="apple-edit-surface">
          {create && selectedPreset && onChangeProvider ? <ProviderPresetControls preset={selectedPreset} baseUrl={baseUrl}
            disabled={saving || !presetFragment && !loadError} onChangeProvider={onChangeProvider} onSelect={selectConnection} /> : null}
          <fieldset className="apple-panel-section min-w-0" disabled={create && !presetFragment}>
            <ProviderIdentityFields
              idPrefix={create ? `profile-${initialPreset!.kind}-${initialPreset!.plan?.replace(/ /g, "-")}` : "profile"} name={name} description={description} icon={selectedIcon}
              iconName={detail?.name ?? name} onIcon={() => setPickingIcon(true)}
              onName={setName} onDescription={setDescription}
              labels={{
                changeIcon: t("edit.changeIcon"), changeIconLabel: t("edit.changeIconLabel"),
                name: t("edit.nameLabel"), namePlaceholder: t("edit.namePlaceholder"),
                description: t("edit.descriptionLabel"), descriptionPlaceholder: t("edit.descriptionPlaceholder"),
              }}
            />
            {showProviderFields ? (
              <>
                <label className="field-label mb-1.5 mt-4 block">{t("edit.protocolLabel")}</label>
                <div className="app-input flex min-w-0 items-center gap-2">
                  <Webhook className="h-3.5 w-3.5 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
                  <span className="shrink-0 font-medium text-(--text-secondary)">{t("edit.protocolResponses")}</span>
                </div>
                <label className="field-label mb-1.5 mt-4 block">{t("edit.requestUrlLabel")}</label>
                {presetEndpoints ? (
                  <EndpointField value={baseUrl} onChange={setBaseUrl} endpoints={presetEndpoints} onPick={(endpoint) => { if (endpoint.admin_url) setAdminUrl(endpoint.admin_url); }} placeholder="https://api.example.com/v1" label={t("edit.requestUrlLabel")} />
                ) : (
                  baseUrlField("app-input")
                )}
                <ProviderSecretField
                  label={t("edit.apiKeyLabel")} placeholder={t("edit.apiKeyPlaceholder")}
                  value={apiKey} onChange={setApiKey} visible={showApiKey}
                  onToggle={() => setShowApiKey((visible) => !visible)}
                  showLabel={t("edit.showApiKey")} hideLabel={t("edit.hideApiKey")}
                  testLabel={t("edit.testConnection")}
                  testTitle={!apiKey.trim() || !baseUrl.trim() ? t("edit.checkProviderFields") : undefined}
                  testing={testing} onTest={() => void testConnection()}
                  help={isOpenCode && create ? (
                    <button type="button" className="apple-inline-btn !h-5" onClick={() => void api.openUrl("https://opencode.ai/go?ref=APHY0DXATH").catch((error) => feedback.error(String(error)))}>
                      <ExternalLink className="h-3 w-3" strokeWidth={2} />
                      {t("edit.getApiKey")}
                    </button>
                  ) : null}
                />
              </>
            ) : null}
            {isOfficial ? <div className="mt-4"><div className="field-label mb-1.5">{t("edit.authMethodLabel")}</div>{create ? <AppSelect value={boundAccountId ?? ""} options={accountOptions} onChange={selectAccount} placeholder={t("card.authDesktop")} renderLabel={renderAccountLabel} /> : authSource === "oauth" ? <AppSelect value={boundAccountId ?? ""} options={oauthAccountOptions} onChange={selectAccount} placeholder={t("edit.selectOauthAccount")} renderLabel={renderAccountLabel} /> : <div className="app-input flex min-w-0 items-center gap-2"><AuthSourceIcon source="desktop" className="h-3.5 w-3.5 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" /><span className="shrink-0 text-xs font-medium text-[var(--text-secondary)]">{t("card.authDesktop")}</span>{detail?.desktop_login ? <><span className="muted" aria-hidden="true">·</span><span className="min-w-0 truncate text-xs font-medium text-[var(--text-secondary)]" title={detail.desktop_login}>{detail.desktop_login}</span></> : null}</div>}</div> : null}
            {showProviderFields || isOfficial ? <div className="mt-4 grid grid-cols-1 gap-1.5 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
              <div className="min-w-0">
                <div className="field-label mb-1.5 flex h-6 items-center">{t("edit.modelIdLabel")}</div>
                <input className="app-input" placeholder={t("edit.modelIdPlaceholder")} value={selection.model}
                  onChange={(event) => setModelValue(event.target.value)} />
              </div>
              <div className="min-w-0">
                <div className="mb-1.5 flex h-6 items-center gap-2">
                  <span className="field-label">{t("edit.modelsLabel")}</span>
                  <button type="button" className="apple-inline-btn apple-inline-btn--quiet !h-5" disabled={useLocalCatalog || fetchingModels || saving || (isOfficial && authSource === "desktop" && authDirty) || (!isOfficial && (!apiKey.trim() || !baseUrl.trim()))} onClick={() => void fetchModelList()}>
                    {fetchingModels ? <LoadingSpinner /> : <Download className="h-3 w-3" strokeWidth={2} aria-hidden="true" />}
                    {t("edit.fetchModels")}
                  </button>
                  {!!models.length && <span className="muted text-xs">{t("edit.modelsAvailable", { count: models.length })}</span>}
                </div>
                <AppSelect value={selection.model} options={getModelChoices(models.length ? selection.model : "", models).map((value) => ({ value, label: value }))}
                  onChange={setModelValue} disabled={saving || !models.length || (!isOfficial && (!apiKey.trim() || !baseUrl.trim()))} menuWidth="max-content" searchable
                  placeholder={t(models.length ? "edit.selectModel" : "edit.fetchModelsFirst")} />
              </div>
              {effortField}
            </div> : null}
            {(!create || Boolean(selectedPreset?.admin_url)) ? <div className="mt-4"><div className="mb-1.5 flex items-center gap-2"><span className="field-label">{t("edit.adminUrlLabel")}</span><button type="button" className="apple-inline-btn apple-inline-btn--quiet !h-5 shrink-0" disabled={!adminUrl.trim()} aria-label={t("card.openWebsite")} onClick={() => void api.openUrl(adminUrl.trim()).catch((error) => feedback.error(String(error)))}><ExternalLink className="h-3 w-3" strokeWidth={2} aria-hidden="true" />{t("card.openWebsite")}</button></div><input className="app-input" placeholder={t("edit.adminUrlPlaceholder")} value={adminUrl} onChange={(event) => setAdminUrl(event.target.value)} /></div> : null}
              {supportsBalance ? <div className="mt-4 flex min-h-[var(--input-min-height)] items-center justify-between gap-4"><div className="min-w-0"><div className="setting-title">{t("edit.balanceUsage")}</div><div className="setting-description mt-0.5">{t("edit.balanceAutoRefresh")}</div></div><AppSwitch checked={showBalance} disabled={saving} label={t("edit.balanceUsage")} onCheckedChange={(value) => void toggleBalance(value)} /></div> : null}
          </fieldset>
          {create && !presetFragment ? <div className="apple-panel-section muted flex items-center gap-2" role="status">
            {!loadError ? <><LoadingSpinner />{t("loading", { ns: "common" })}</> : null}
          </div> : <div className="apple-panel-section flex flex-col">
              <div className="flex min-h-8 items-center justify-between gap-2">
                <div className="flex min-w-0 flex-wrap gap-1">
                  {tabs.map((tab) => <button key={tab.id} type="button" className={`relative flex h-8 items-center gap-1.5 rounded-[10px] px-3 text-[13px] font-semibold transition-colors ${activeTab === tab.id ? "bg-(--active-bg) text-accent" : "muted hover:bg-(--hover-bg)"}`} aria-pressed={activeTab === tab.id} title={tab.title} onClick={() => { setActiveTab(tab.id); setEditorDiagnostics({ count: 0, firstLine: null }); }}>{tab.id === "config" ? <Settings className="h-3.5 w-3.5" strokeWidth={2} /> : <FileBraces className="h-3.5 w-3.5" strokeWidth={2} />}<span>{tab.label}</span>{((tab.id === "config" && configDirty) || (tab.id === "models" && catalogDirty) || (tab.id === "auth" && authDirty)) ? <span className="absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full bg-accent" /> : null}</button>)}
                </div>
                {formatButton}
              </div>
              {/* 附属条嵌进编辑器托盘顶部（editor chrome），随 tab 切换：config 快捷设置、models/auth 文件操作。 */}
              <div className="editor-attach-group mt-2">
                <div className="editor-attach-bar">
                  {activeTab === "config" ? (
                    <ProfileAdvancedControls advanced={advanced} saving={saving} />
                  ) : (
                    <TabFileControls
                      kind={activeTab === "models" ? "models" : "auth"}
                      editable={activeTab === "models" || !authPreviewOnly}
                      disabled={saving}
                      onClear={() => (activeTab === "models" ? setCatalogText("") : setAuthText(""))}
                    />
                  )}
                </div>
              <div className="flex flex-col">{activeTab === "config" ? <ConfigTextEditor ref={editorRef} value={configText} language="toml" minLines={editorMinLines} placeholder={create ? t("edit.configPlaceholderCreate") : t("edit.configPlaceholderEdit")} onChange={(value) => setConfigText(value)} onDiagnostics={setEditorDiagnostics} /> : activeTab === "auth" ? <ConfigTextEditor ref={editorRef} value={authText} language="json" minLines={editorMinLines} readOnly={authPreviewOnly} placeholder={t("edit.authPlaceholder")} onChange={setAuthText} onDiagnostics={setEditorDiagnostics} /> : <ConfigTextEditor ref={editorRef} value={catalogText} language="json" minLines={editorMinLines} placeholder={t("edit.catalogPlaceholder")} onChange={(value) => setCatalogText(value)} onDiagnostics={setEditorDiagnostics} />}</div>
              </div>
            </div>}
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer"><DiagnosticsChip diagnostics={editorDiagnostics} onFocusFirst={() => editorRef.current?.focusFirstDiagnostic()} /><button type="button" className="apple-action-button" onClick={onBack}>{t("dialog.cancel")}</button><button type="button" className="apple-action-button app-button--primary" disabled={saving || !canSave} onClick={() => void save()}><Save className="h-4 w-4" strokeWidth={2} />{saving ? t("edit.saving") : t("dialog.save")}</button></div>
    </section>
  );
}
