import { ChevronLeft, CodeXml, Save } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import ConfigTextEditor, { type ConfigTextEditorHandle } from "../../components/ConfigTextEditor";
import { DiagnosticsChip } from "../../components/DiagnosticsChip";
import { SkillTargetLogo } from "../../components/SkillTargetLogo";
import type { EditorDiagnosticSummary, McpServerSpec } from "../../types";
import McpConnectionForm, { TimeoutInput } from "./McpConnectionForm";
import { McpSourceLabel } from "./McpSourceLabel";
import { patchClaudeMcpForm, readClaudeMcpForm, type ClaudeMcpForm } from "./mcpFormData";

interface ClaudeMcpEditProps {
  server: McpServerSpec | null;
  create?: boolean;
  onBack: () => void;
  onSaved: (name: string) => void;
}

export default function ClaudeMcpEdit({ server, create = false, onBack, onSaved }: ClaudeMcpEditProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("mcp");
  const [name, setName] = useState(server?.name ?? "");
  const [jsonText, setJsonText] = useState("{\n  \"type\": \"stdio\",\n  \"command\": \"\"\n}");
  const [initialJson, setInitialJson] = useState(jsonText);
  const [form, setForm] = useState(() => readClaudeMcpForm(jsonText));
  const [formValid, setFormValid] = useState(true);
  const [initialized, setInitialized] = useState(create);
  const [saving, setSaving] = useState(false);
  const [formatting, setFormatting] = useState(false);
  const [diagnostics, setDiagnostics] = useState<EditorDiagnosticSummary>({ count: 0, firstLine: null });
  const editorRef = useRef<ConfigTextEditorHandle>(null);

  const editJson = (text: string) => {
    setJsonText(text);
    try {
      setForm(readClaudeMcpForm(text));
      setFormValid(true);
    } catch {
      setFormValid(false);
    }
  };

  const editField = <K extends keyof ClaudeMcpForm>(field: K, value: ClaudeMcpForm[K]) => {
    if (!initialized || !formValid) return;
    const next = { ...form, [field]: value };
    setJsonText(patchClaudeMcpForm(jsonText, next, field));
    setForm(next);
  };

  useEffect(() => {
    if (create || !server) return;
    let cancelled = false;
    void api.claudeGetMcpServerJson(server.name).then((value) => {
      if (cancelled) return;
      if (value) { setInitialJson(value); editJson(value); }
      setInitialized(true);
    }).catch((error) => feedback.error(String(error)));
    return () => { cancelled = true; };
  }, [create, feedback, server]);

  const formatJson = () => {
    if (formatting || saving) return;
    setFormatting(true);
    try {
      editJson(`${JSON.stringify(JSON.parse(jsonText), null, 2)}\n`);
      feedback.success(t("feedback.formatted"));
    } catch (error) {
      feedback.error(t("feedback.formatFailed", { error: String(error) }));
    } finally {
      setFormatting(false);
    }
  };

  const save = async () => {
    if (saving || !initialized) return;
    const trimmedName = name.trim();
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(trimmedName)) {
      feedback.error(t("feedback.invalidName"));
      return;
    }
    try {
      const parsed = readClaudeMcpForm(jsonText);
      if (parsed.transport === "stdio" && !parsed.command.trim()) {
        feedback.error(t("feedback.commandRequired"));
        return;
      }
      if (parsed.transport !== "stdio" && !parsed.url.trim()) {
        feedback.error(t("feedback.urlRequired"));
        return;
      }
      if (parsed.timeout !== null && (!Number.isInteger(parsed.timeout) || parsed.timeout < 1000)) {
        feedback.error(t("feedback.toolTimeoutMilliseconds"));
        return;
      }
    } catch {
      feedback.error(t("feedback.invalidJson"));
      editorRef.current?.focusFirstDiagnostic();
      return;
    }
    setSaving(true);
    try {
      await api.claudeSaveMcpServer(server?.name ?? null, trimmedName, jsonText);
      feedback.success(t("feedback.saved"));
      onSaved(trimmedName);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col" onKeyDown={(event) => {
      if (event.key === "Enter" && !event.nativeEvent.isComposing && !(event.target instanceof Element && event.target.closest("textarea, .apple-editor-shell"))) {
        event.preventDefault();
        void save();
      }
    }}>
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header justify-between">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("edit.back")} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
          <SkillTargetLogo target="claude" variant="title" />
          <span className="apple-title">{create ? t("edit.createTitle") : t("edit.editTitle", { name })}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          <McpConnectionForm
            nameReadOnly={!create}
            name={name} setName={setName} transport={form.transport} setTransport={(value) => editField("transport", value)}
            command={form.command} setCommand={(value) => editField("command", value)}
            argsText={form.argsText} setArgsText={(value) => editField("argsText", value)}
            url={form.url} setUrl={(value) => editField("url", value)}
            envPairs={form.envPairs} setEnvPairs={(value) => editField("envPairs", value)}
            headerPairs={form.headerPairs} setHeaderPairs={(value) => editField("headerPairs", value)}
            disabled={!initialized || !formValid || saving}
            extraTransports={[
              { label: t("edit.transportSse"), value: "sse" },
              ...(!["stdio", "http", "sse"].includes(form.transport) ? [{ label: form.transport, value: form.transport }] : []),
            ]}
            timeoutFields={<div className="mt-4">
              <div className="field-label mb-1.5">{t("edit.toolTimeoutMilliseconds")}</div>
              <TimeoutInput value={form.timeout} onChange={(value) => editField("timeout", value)} placeholder={t("edit.toolTimeoutMillisecondsPlaceholder")} min={1000} step={1000} />
            </div>}
          />
          <div className="apple-panel-section">
            <div className="mb-1.5 flex min-h-8 items-center justify-between gap-2">
              <McpSourceLabel label={t("edit.jsonSource")} value={jsonText} initialValue={initialJson} initialized={initialized} />
              <button type="button" className="editor-ghost editor-ghost--format shrink-0" disabled={formatting || saving || !initialized} onClick={formatJson}>
                <CodeXml className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                <span className="whitespace-nowrap font-medium">{t("edit.format")}</span>
              </button>
            </div>
            <ConfigTextEditor ref={editorRef} value={jsonText} language="json" minLines={12} readOnly={!initialized} placeholder={t("edit.jsonPlaceholder")} onChange={editJson} onDiagnostics={setDiagnostics} />
          </div>
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        <DiagnosticsChip diagnostics={diagnostics} onFocusFirst={() => editorRef.current?.focusFirstDiagnostic()} />
        <button type="button" className="apple-action-button" onClick={onBack}>{t("edit.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={saving || !initialized} onClick={() => void save()}><Save className="h-4 w-4" strokeWidth={2} />{saving ? t("edit.saving") : t("edit.save")}</button>
      </div>
    </section>
  );
}
