import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { AppDialog } from "../../components/AppDialog";

/** Codex 与 Claude 共用的供应商命名弹窗：收录当前配置、重命名供应商。 */
export default function ProfileNameDialog({ mode, name, busy, onName, onClose, onSubmit }: {
  mode: "capture" | "rename" | null;
  name: string;
  busy: boolean;
  onName: (name: string) => void;
  onClose: () => void;
  onSubmit: () => void;
}) {
  const { t } = useTranslation("profiles");
  const nameInput = useRef<HTMLInputElement>(null);
  return (
    <AppDialog
      open={mode !== null}
      onOpenChange={(open) => { if (!open) onClose(); }}
      title={t(mode === "capture" ? "dialog.captureTitle" : "dialog.renameTitle")}
      initialFocusRef={nameInput}
      footer={<>
        <button type="button" className="apple-action-button" onClick={onClose}>{t("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={busy || !name.trim()} onClick={onSubmit}>{t("dialog.save")}</button>
      </>}
    >
      <div className="space-y-4">
        <p className="muted text-sm">{t(mode === "capture" ? "dialog.captureDescription" : "dialog.renameDescription")}</p>
        <input
          ref={nameInput} className="app-input" maxLength={50} placeholder={t("dialog.namePlaceholder")}
          value={name} onChange={(event) => onName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) onSubmit(); }}
        />
      </div>
    </AppDialog>
  );
}
