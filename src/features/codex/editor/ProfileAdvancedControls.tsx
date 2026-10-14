import { useTranslation } from "react-i18next";
import type { ProfileAdvancedPatches } from "./useProfileAdvancedPatches";

/** 编辑器附属条内的 config 快捷设置：长上下文 / 上下文管理 / 系统代理。ghost 权重低于 tab。 */
export default function ProfileAdvancedControls({ advanced, saving }: { advanced: ProfileAdvancedPatches; saving: boolean }) {
  const { t } = useTranslation("profiles");
  return (
    <>
      {advanced.showLongContextOverride ? (
        <div className="editor-ghost-group">
          <label className="editor-ghost" title={t("edit.longContextTitle")}>
            <input type="checkbox" checked={advanced.longContextEnabled} disabled={advanced.patchingLongContext || saving} onChange={(event) => void advanced.toggleLongContext(event.target.checked)} />
            <span className="whitespace-nowrap font-medium">{t("edit.longContextLabel")}</span>
          </label>
          <span className="editor-ghost-group__separator" aria-hidden="true" />
          <label className={`editor-ghost ${advanced.longContextEnabled ? "" : "opacity-40 pointer-events-none"}`} title={t("edit.compactLimitTitle")}>
            <span className="whitespace-nowrap">{t("edit.compactLimitLabel")}</span>
            <input className={`app-input app-input--compact compact-token-input ${advanced.longContextEnabled ? "compact-token-input--on" : ""} h-6 text-center`} type="number" min={1} max={1_000_000} step={1} inputMode="numeric" value={advanced.compactTokenLimit} disabled={!advanced.longContextEnabled || advanced.patchingLongContext || saving} onChange={(event) => advanced.updateCompactTokenLimitInput(event.target.value)} onBlur={() => void advanced.updateCompactTokenLimit()} />
          </label>
        </div>
      ) : null}
      <label className="editor-ghost" title={t("edit.contextMgmtTitle")}>
        <input type="checkbox" checked={advanced.contextMgmtEnabled} disabled={advanced.patchingContextMgmt || saving} onChange={(event) => void advanced.toggleContextManagement(event.target.checked)} />
        <span className="whitespace-nowrap font-medium">{t("edit.contextMgmtLabel")}</span>
        <span className="meta-xs muted">{t("edit.experimental")}</span>
      </label>
      <label className="editor-ghost" title={t("edit.systemProxyTitle")}>
        <input type="checkbox" checked={advanced.systemProxyEnabled} disabled={advanced.patchingSystemProxy || saving} onChange={(event) => void advanced.toggleSystemProxy(event.target.checked)} />
        <span className="whitespace-nowrap font-medium">{t("edit.systemProxyLabel")}</span>
      </label>
    </>
  );
}
