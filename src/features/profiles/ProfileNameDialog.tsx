import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { AppDialog } from "../../components/AppDialog";

/** Codex 与 Claude Code 共用的配置收录弹窗。 */
export default function ProfileNameDialog({ open, name, busy, onName, onClose, onSubmit }: {
  open: boolean;
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
      open={open}
      onOpenChange={(open) => { if (!open) onClose(); }}
      title={t("dialog.captureTitle")}
      initialFocusRef={nameInput}
      footer={<>
        <button type="button" className="apple-action-button" onClick={onClose}>{t("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={busy || !name.trim()} onClick={onSubmit}>{t("dialog.save")}</button>
      </>}
    >
      <div className="space-y-4">
        <p className="muted text-sm">{t("dialog.captureDescription")}</p>
        <input
          ref={nameInput} className="app-input" maxLength={50} placeholder={t("dialog.namePlaceholder")}
          value={name} onChange={(event) => onName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) onSubmit(); }}
        />
      </div>
    </AppDialog>
  );
}
