import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AppDialog } from "../../components/AppDialog";

/** 三个客户端共用的供应商命名弹窗。 */
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
  // 关闭后 Radix 会保留面板直到退场结束，此时继续显示离场前的文案。
  const [lastMode, setLastMode] = useState(mode);
  if (mode !== null && mode !== lastMode) setLastMode(mode);
  const capture = (mode ?? lastMode) === "capture";
  return (
    <AppDialog
      open={mode !== null}
      onOpenChange={(open) => { if (!open) onClose(); }}
      title={t(capture ? "dialog.captureTitle" : "dialog.renameTitle")}
      initialFocusRef={nameInput}
      footer={<>
        <button type="button" className="apple-action-button" onClick={onClose}>{t("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={busy || !name.trim()} onClick={onSubmit}>{t("dialog.save")}</button>
      </>}
    >
      <div className="space-y-4">
        <p className="muted text-sm">{t(capture ? "dialog.captureDescription" : "dialog.renameDescription")}</p>
        <input
          ref={nameInput} className="app-input" maxLength={50} placeholder={t("dialog.namePlaceholder")}
          value={name} onChange={(event) => onName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) onSubmit(); }}
        />
      </div>
    </AppDialog>
  );
}
