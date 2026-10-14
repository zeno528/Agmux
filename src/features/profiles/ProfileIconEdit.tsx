import { ChevronLeft, Save } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ProfileIconTile } from "../../components/ProfileIconTile";
import { providerIcons } from "../../icons";

interface ProfileIconEditProps {
  icon: string | null;
  onBack: () => void;
  onSave: (icon: string | null) => void;
}

/** 图标选择页：图标主导的方形网格，Codex / Claude Code 两个编辑页共用。
 *  格内不再显示名称文字——写死 6 列时 19 个图标要占 4 行且末行只落 1 个；
 *  名称改走 aria-label/title，已选项在网格下方常显一行，选中后不必靠记忆确认。 */
export default function ProfileIconEdit({ icon, onBack, onSave }: ProfileIconEditProps) {
  const { t } = useTranslation("profiles");
  const [selected, setSelected] = useState<string | null>(icon);
  const icons = providerIcons();
  const current = icons.find((item) => item.id === selected);
  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("edit.back")} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
          <span className="apple-title">{t("icons.pageTitle")}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="apple-edit-surface apple-panel-section">
          {/* 默认态不带描边，悬停才浮现一圈中性色，选中才是强调色。 */}
          <div className="apple-tile-grid">
            {icons.map((item) => (
              <button
                key={item.id}
                type="button"
                className={`grid place-items-center rounded-lg p-2 transition-colors ${selected === item.id ? "shadow-[0_0_0_1px_var(--accent)] bg-(--active-bg)" : "hover:shadow-[0_0_0_1px_var(--panel-ring)] hover:bg-(--hover-bg)"}`}
                aria-label={item.label}
                aria-pressed={selected === item.id}
                title={item.label}
                onClick={() => setSelected(item.id)}
              >
                <ProfileIconTile name={item.label} icon={item.id} />
              </button>
            ))}
          </div>
          {/* 这一行同时是"当前状态"和"不使用图标"的入口：选中时显示已选名称，
              点它回到不使用图标；格内不再有名称文字，状态就不必另起一行。
              12px 常规字重太细、600 又太重，卡在 medium：只加一档字重，不动字号。 */}
          <button
            type="button"
            className={`mt-2 w-full rounded-lg border border-dashed px-2 py-2.5 text-xs font-medium transition-colors ${selected === null ? "border-accent text-accent bg-(--active-bg)" : "muted border-(--panel-border) hover:bg-(--hover-bg)"}`}
            aria-pressed={selected === null}
            onClick={() => setSelected(null)}
          >
            {current ? t("icons.selected", { name: current.label }) : t("icons.none")}
          </button>
        </div>
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        <button type="button" className="apple-action-button" onClick={onBack}>{t("dialog.cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" onClick={() => onSave(selected)}>
          <Save className="h-4 w-4" strokeWidth={2} aria-hidden="true" /> {t("dialog.save")}
        </button>
      </div>
    </section>
  );
}
