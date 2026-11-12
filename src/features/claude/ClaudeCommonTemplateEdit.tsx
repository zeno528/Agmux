import { useEffect, useState } from "react";
import { ChevronLeft, CodeXml, Eraser, FileBraces, Save } from "lucide-react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import ConfigTextEditor from "../../components/ConfigTextEditor";
import { extractClaudeCommonSettings, formatJsonText } from "./profileEnvText";

export default function ClaudeCommonTemplateEdit({ sourceText, onBack }: {
  sourceText: string;
  onBack: () => void;
}) {
  const { t } = useTranslation("claude");
  const { t: tProfiles } = useTranslation("profiles");
  const feedback = useFeedback();
  const [text, setText] = useState("{}");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [diagnostics, setDiagnostics] = useState({ count: 0, firstLine: null as number | null });

  useEffect(() => {
    let cancelled = false;
    void api.claudeGetCommonSettings().then((saved) => {
      if (!cancelled) {
        setText(saved ?? "{}");
        setLoading(false);
      }
    }).catch((error) => {
      if (!cancelled) {
        feedback.error(String(error));
        onBack();
      }
    });
    return () => { cancelled = true; };
  }, []);

  const extract = () => {
    if (loading || saving) return;
    const common = extractClaudeCommonSettings(sourceText);
    if (common === null) {
      feedback.error(t("envInvalid"));
      return;
    }
    setText(common);
    feedback.success(t("commonTemplate.extracted"));
  };

  const save = async () => {
    if (loading || saving) return;
    const common = extractClaudeCommonSettings(text);
    if (common === null) {
      feedback.error(t("envInvalid"));
      return;
    }
    setSaving(true);
    try {
      await api.claudeSaveCommonSettings(common === "{}" ? null : common);
      feedback.success(t("commonTemplate.saved"));
      onBack();
    } catch (error) {
      feedback.error(String(error));
      setSaving(false);
    }
  };

  const format = () => {
    const label = t("commonTemplate.title");
    const outcome = formatJsonText(text);
    if (outcome.status === "empty") feedback.warning(tProfiles("edit.formatEmpty"));
    else if (outcome.status === "failed") feedback.error(tProfiles("edit.formatFailed", { error: outcome.error }));
    else if (outcome.status === "unchanged") feedback.info(tProfiles("edit.formatNoChange", { label }));
    else {
      setText(outcome.text);
      feedback.success(tProfiles("edit.formatSuccess", { label }));
    }
  };

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <header className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={tProfiles("edit.back")} disabled={saving} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />
          <span className="apple-title">{t("commonTemplate.title")}</span>
        </button>
      </header>
      <div className="apple-edit-content provider-config-view">
        <div className="apple-edit-surface">
          <div className="apple-panel-section flex flex-col">
            <div className="apple-editor-toolbar">
              <div className="flex min-h-8 flex-wrap items-center justify-between gap-2">
                <span className="field-label flex h-8 shrink-0 items-center gap-1.5 rounded-[10px] bg-(--active-bg) px-3 text-accent">
                  <FileBraces className="h-3.5 w-3.5" strokeWidth={2} aria-hidden="true" />
                  {t("commonTemplate.title")}
                </span>
                <button type="button" className="editor-ghost editor-ghost--format shrink-0" disabled={loading || saving} title={tProfiles("edit.formatTitle", { label: t("commonTemplate.title"), format: "JSON" })} onClick={format}>
                  <CodeXml className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                  <span className="whitespace-nowrap font-medium">{tProfiles("edit.format")}</span>
                </button>
              </div>
              <div className="editor-attach-bar">
                <p className="setting-description min-w-0 flex-1">{t("commonTemplate.description")}</p>
                <button type="button" className="editor-ghost editor-ghost--danger ml-auto shrink-0" disabled={loading || saving} onClick={() => setText("{}")}>
                  <Eraser className="h-3.5 w-3.5" strokeWidth={2} aria-hidden="true" />
                  <span className="whitespace-nowrap font-medium">{tProfiles("edit.clearFile")}</span>
                </button>
              </div>
            </div>
            <div className="editor-attach-group">
              {loading ? (
                <div className="muted flex items-center gap-2 p-3 text-sm" role="status">
                  <LoadingSpinner />{t("loading", { ns: "common" })}
                </div>
              ) : (
                <ConfigTextEditor value={text} language="json" readOnly={saving} onChange={setText} onDiagnostics={setDiagnostics} />
              )}
            </div>
          </div>
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        <button type="button" className="apple-action-button mr-auto" disabled={loading || saving} onClick={extract}>{t("commonTemplate.extract")}</button>
        <button type="button" className="apple-action-button" disabled={saving} onClick={onBack}>{tProfiles("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={loading || saving || diagnostics.count > 0} onClick={() => void save()}>
          <Save className="h-4 w-4" strokeWidth={2} />
          {saving ? tProfiles("edit.saving") : tProfiles("dialog.save")}
        </button>
      </div>
    </section>
  );
}
