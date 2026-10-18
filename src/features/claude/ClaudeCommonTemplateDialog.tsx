import { useEffect, useState } from "react";
import { CodeXml, Eraser } from "lucide-react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppDialog } from "../../components/AppDialog";
import ConfigTextEditor from "../../components/ConfigTextEditor";
import { extractClaudeCommonSettings, formatJsonText } from "./profileEnvText";

/** 列表管理与编辑页提取共用；只在打开时读取模板。 */
export default function ClaudeCommonTemplateDialog({ sourceText, onClose }: {
  sourceText?: string;
  onClose: () => void;
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
        onClose();
      }
    });
    return () => { cancelled = true; };
  }, []);

  const extract = () => {
    if (loading || saving || sourceText === undefined) return;
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
      onClose();
    } catch (error) {
      feedback.error(String(error));
      setSaving(false);
    }
  };

  // 排版逻辑与编辑页 settings.json 共用 formatJsonText，只有提示标签是本弹窗自己的
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
    <AppDialog
      open={!loading}
      onOpenChange={(open) => { if (!open && !saving) onClose(); }}
      title={t("commonTemplate.title")}
      description={t("commonTemplate.description")}
      className="claude-template-dialog"
      footer={(
        <>
          {sourceText !== undefined ? (
            <button type="button" className="apple-action-button" disabled={loading || saving} onClick={extract}>{t("commonTemplate.extract")}</button>
          ) : null}
          <button type="button" className="apple-action-button" disabled={saving} onClick={onClose}>{tProfiles("dialog.cancel")}</button>
          <button type="button" className="apple-action-button app-button--primary" disabled={loading || saving || diagnostics.count > 0} onClick={() => void save()}>
            {saving ? tProfiles("edit.saving") : tProfiles("dialog.save")}
          </button>
        </>
      )}
    >
      {/* 清空与格式化都是编辑器工具而非弹窗动作：放进编辑器附属条（与编辑页 models/auth
          文件操作同一模式），底部只留 提取/取消/保存 */}
      <div className="editor-attach-group">
        <div className="editor-attach-bar">
          <button type="button" className="editor-ghost editor-ghost--danger ml-auto shrink-0" disabled={loading || saving} onClick={() => setText("{}")}>
            <Eraser className="h-3.5 w-3.5" strokeWidth={2} aria-hidden="true" />
            <span className="whitespace-nowrap font-medium">{tProfiles("edit.clearFile")}</span>
          </button>
          <button type="button" className="editor-ghost editor-ghost--format shrink-0" disabled={loading || saving} title={tProfiles("edit.formatTitle", { label: t("commonTemplate.title"), format: "JSON" })} onClick={format}>
            <CodeXml className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
            <span className="whitespace-nowrap font-medium">{tProfiles("edit.format")}</span>
          </button>
        </div>
        <ConfigTextEditor value={text} language="json" minLines={12} readOnly={loading || saving} onChange={setText} onDiagnostics={setDiagnostics} />
      </div>
    </AppDialog>
  );
}
