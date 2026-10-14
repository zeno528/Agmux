import { ChevronLeft, CodeXml, Save } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import ConfigTextEditor, { type ConfigTextEditorHandle } from "../../components/ConfigTextEditor";
import { SkillTargetLogo } from "../../components/SkillTargetLogo";
import type { EditorDiagnosticSummary, McpServerSpec } from "../../types";
import McpConnectionForm, { PairEditor, TimeoutInput } from "./McpConnectionForm";
import { McpSourceLabel } from "./McpSourceLabel";
import { pairsToRecord, recordToPairs, type KVPair } from "./mcpFormData";

type Transport = "stdio" | "http";
interface McpEditProps {
  server: McpServerSpec | null;
  create?: boolean;
  onBack: (savedServer?: McpServerSpec) => void;
}

export default function McpEdit({ server, create = false, onBack }: McpEditProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("mcp");
  const [name, setName] = useState(server?.name ?? "");
  const [transport, setTransport] = useState<Transport>(server?.url ? "http" : "stdio");
  const [command, setCommand] = useState(server?.command ?? "");
  const [argsText, setArgsText] = useState((server?.args ?? []).join("\n"));
  const [url, setUrl] = useState(server?.url ?? "");
  const [bearer, setBearer] = useState(server?.bearer_token_env_var ?? "");
  const [startupTimeout, setStartupTimeout] = useState<number | null>(server?.startup_timeout_sec ?? null);
  const [toolTimeout, setToolTimeout] = useState<number | null>(server?.tool_timeout_sec ?? null);
  const [envPairs, setEnvPairs] = useState<KVPair[]>(recordToPairs(server?.env ?? {}));
  const [headerPairs, setHeaderPairs] = useState<KVPair[]>(recordToPairs(server?.http_headers ?? {}));
  const [envHeaderPairs, setEnvHeaderPairs] = useState<KVPair[]>(recordToPairs(server?.env_http_headers ?? {}));
  const [tomlText, setTomlText] = useState("");
  const [initialToml, setInitialToml] = useState("");
  const [initialized, setInitialized] = useState(false);
  const [saving, setSaving] = useState(false);
  const [formatting, setFormatting] = useState(false);
  const [diagnostics, setDiagnostics] = useState<EditorDiagnosticSummary>({ count: 0, firstLine: null });
  const patchSeq = useRef(0);
  const parseSeq = useRef(0);
  const editorRef = useRef<ConfigTextEditorHandle>(null);

  const argsList = () => argsText.split("\n").map((line) => line.trim()).filter(Boolean);
  const formSpec = (): McpServerSpec => ({
    name: name.trim() || server?.name || "",
    enabled: server?.enabled ?? null,
    startup_timeout_sec: startupTimeout,
    tool_timeout_sec: toolTimeout,
    command: transport === "stdio" ? command.trim() || null : null,
    args: transport === "stdio" ? argsList() : [],
    env: transport === "stdio" ? pairsToRecord(envPairs) : {},
    url: transport === "http" ? url.trim() || null : null,
    bearer_token_env_var: transport === "http" ? bearer.trim() || null : null,
    http_headers: transport === "http" ? pairsToRecord(headerPairs) : {},
    env_http_headers: transport === "http" ? pairsToRecord(envHeaderPairs) : {},
  });

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const initial = create
          ? ""
          : (await api.codexGetMcpServerToml(server?.name ?? "")) ?? await api.patchMcpFragment(`[mcp_servers.${server?.name ?? "server"}]\n`, formSpec());
        if (!cancelled) { setTomlText(initial); setInitialToml(initial); setInitialized(true); }
      } catch (error) { if (!cancelled) feedback.error(String(error)); }
    })();
    return () => { cancelled = true; };
    // initial props are fixed for the mounted editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!initialized || (create && !name.trim())) return;
    const seq = ++patchSeq.current;
    void api.patchMcpFragment(tomlText, formSpec()).then((next) => {
      if (seq === patchSeq.current && next !== tomlText) setTomlText(next);
    }).catch(() => undefined);
  }, [argsText, bearer, command, envHeaderPairs, envPairs, headerPairs, initialized, name, startupTimeout, toolTimeout, transport, url]);

  useEffect(() => {
    if (!initialized || !tomlText.trim()) return;
    const seq = ++parseSeq.current;
    void api.parseMcpFragment(tomlText).then((spec) => {
      if (seq !== parseSeq.current) return;
      if (create && /^[A-Za-z0-9_-]+$/.test(spec.name)) setName(spec.name);
      setTransport(spec.url ? "http" : "stdio");
      setCommand(spec.command ?? "");
      setArgsText(spec.args.join("\n"));
      setUrl(spec.url ?? "");
      setBearer(spec.bearer_token_env_var ?? "");
      setStartupTimeout(spec.startup_timeout_sec);
      setToolTimeout(spec.tool_timeout_sec);
      setEnvPairs(recordToPairs(spec.env));
      setHeaderPairs(recordToPairs(spec.http_headers));
      setEnvHeaderPairs(recordToPairs(spec.env_http_headers));
    }).catch(() => undefined);
  }, [create, initialized, tomlText]);

  const formatToml = async () => {
    if (formatting || saving) return;
    setFormatting(true);
    try {
      const formatted = await api.formatToml(tomlText);
      if (formatted === tomlText) feedback.info(t("feedback.formatNoChange")); else { setTomlText(formatted); feedback.success(t("feedback.formatted")); }
    } catch (error) { feedback.error(t("feedback.formatFailed", { error: String(error) })); }
    finally { setFormatting(false); }
  };

  const save = async () => {
    if (saving) return;
    if (!/^[A-Za-z0-9_-]+$/.test(name.trim())) { feedback.error(t("feedback.invalidName")); return; }
    if (transport === "stdio" && !command.trim()) { feedback.error(t("feedback.commandRequired")); return; }
    if (transport === "http" && !url.trim()) { feedback.error(t("feedback.urlRequired")); return; }
    if (transport === "http" && !/^https?:\/\//i.test(url.trim())) { feedback.error(t("feedback.urlScheme")); return; }
    if (startupTimeout !== null && startupTimeout <= 0) { feedback.error(t("feedback.startupTimeoutPositive")); return; }
    if (toolTimeout !== null && toolTimeout <= 0) { feedback.error(t("feedback.toolTimeoutPositive")); return; }
    setSaving(true);
    try { const savedServer = formSpec(); await api.codexSaveMcpServer(server?.name ?? null, savedServer, tomlText); feedback.success(t("feedback.saved")); onBack(savedServer); }
    catch (error) { feedback.error(String(error)); }
    finally { setSaving(false); }
  };

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col" onKeyDown={(event) => {
      if (event.key === "Enter" && !event.nativeEvent.isComposing && !(event.target instanceof Element && event.target.closest("textarea, .apple-editor-shell"))) {
        event.preventDefault();
        void save();
      }
    }}>
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header justify-between">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("edit.back")} onClick={() => onBack()}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />
          <SkillTargetLogo target="codex" variant="title" />
          <span className="apple-title">{create ? t("edit.createTitle") : t("edit.editTitle", { name })}</span>
        </button>
      </div>

      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          <McpConnectionForm
            nameReadOnly={!create}
            name={name} setName={setName} transport={transport} setTransport={(value) => setTransport(value as Transport)}
            command={command} setCommand={setCommand} argsText={argsText} setArgsText={setArgsText}
            url={url} setUrl={setUrl} envPairs={envPairs} setEnvPairs={setEnvPairs}
            headerPairs={headerPairs} setHeaderPairs={setHeaderPairs}
            httpFields={<div className="mt-4">
              <div className="field-label mb-1.5">{t("edit.bearerLabel")}</div>
              <input className="app-input mono" placeholder={t("edit.bearerPlaceholder")} value={bearer} onChange={(event) => setBearer(event.target.value)} />
            </div>}
            httpAdvancedFields={envHeaderPairs.length ? <PairEditor label={t("edit.headerFromEnv")} pairs={envHeaderPairs} onChange={setEnvHeaderPairs} keyPlaceholder={t("edit.headerKeyPlaceholder")} valuePlaceholder={t("edit.envVarPlaceholder")} /> : null}
            timeoutFields={<div className="mt-4 grid gap-4 sm:grid-cols-2">
              <div>
                <div className="field-label mb-1.5">{t("edit.startupTimeout")}</div>
                <TimeoutInput value={startupTimeout} onChange={setStartupTimeout} placeholder={t("edit.startupTimeoutPlaceholder")} />
              </div>
              <div>
                <div className="field-label mb-1.5">{t("edit.toolTimeout")}</div>
                <TimeoutInput value={toolTimeout} onChange={setToolTimeout} placeholder={t("edit.toolTimeoutPlaceholder")} />
              </div>
            </div>}
          />

          <div className="apple-panel-section">
            <div className="mb-1.5 flex min-h-8 items-center justify-between gap-2">
              <McpSourceLabel label={t("edit.tomlSource")} value={tomlText} initialValue={initialToml} initialized={initialized} />
              <button type="button" className="editor-ghost editor-ghost--format shrink-0" disabled={formatting || saving} onClick={() => void formatToml()}>
                <CodeXml className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                <span className="whitespace-nowrap font-medium">{t("edit.format")}</span>
              </button>
            </div>
            <ConfigTextEditor ref={editorRef} value={tomlText} language="toml" placeholder={t("edit.tomlPlaceholder")} onChange={setTomlText} onDiagnostics={setDiagnostics} />
          </div>
        </div>
      </div>

      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        {diagnostics.count > 0 ? <button type="button" className="mr-auto flex min-w-0 items-center gap-1.5 rounded-lg border border-[var(--danger)]/20 bg-(--danger)/10 px-2.5 py-1 text-xs chip-danger" title={t("edit.jumpToError")} aria-live="polite" onClick={() => editorRef.current?.focusFirstDiagnostic()}><span className="h-1.5 w-1.5 shrink-0 rounded-full bg-(--danger)" aria-hidden="true" /><span className="truncate">{t("edit.errorCount", { count: diagnostics.count })}{diagnostics.firstLine !== null ? t("edit.errorLine", { line: diagnostics.firstLine }) : ""}</span></button> : null}
        <button type="button" className="apple-action-button" onClick={() => onBack()}>{t("edit.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={saving} onClick={() => void save()}><Save className="h-4 w-4" strokeWidth={2} />{saving ? t("edit.saving") : t("edit.save")}</button>
      </div>
    </section>
  );
}
