import {
  AppWindow,
  ArrowUpCircle,
  Info,
  Cog,
  Database,
  MousePointerClick,
  RotateCw,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppSwitch } from "../../components/AppSwitch";
import { AppSelect } from "../../components/AppSelect";
import { AppTabBar } from "../../components/AppTabBar";
import type { AppState, PathInfo, Settings } from "../../types";
import { ProxySettings } from "./ProxySettings";
import { SettingsAbout, SettingsAdvanced, SettingsGeneral, SettingsPanelSection } from "./SettingsSections";

type Section = "general" | "codex" | "advanced" | "about";
interface SettingsViewProps { state: AppState; onPreviewTheme: (theme: Settings["theme"]) => void; onRefresh: () => Promise<void>; onSaved: (settings: Settings) => void; }

export default function SettingsView({ state, onPreviewTheme, onRefresh, onSaved }: SettingsViewProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("settings");
  const [form, setForm] = useState<Settings>(state.settings);
  const [section, setSection] = useState<Section>("general");
  const [saving, setSaving] = useState(false);
  const [backupsEpoch, setBackupsEpoch] = useState(0);
  const [openingPath, setOpeningPath] = useState<string | null>(null);

  useEffect(() => setForm(state.settings), [state.settings]);
  useEffect(() => { void api.getSettings().then((settings) => { setForm(settings); onSaved(settings); }).catch((error) => feedback.error(String(error))); }, []);

  const saveGeneral = async (patch: Partial<Settings>) => {
    if (saving) return false;
    const previous = form;
    const next = { ...form, ...patch };
    setForm(next);
    if (patch.theme) onPreviewTheme(patch.theme);
    setSaving(true);
    try { const saved = await api.saveSettings(next); setForm(saved); onSaved(saved); if (saved.database_backup_keep_count !== previous.database_backup_keep_count) setBackupsEpoch((epoch) => epoch + 1); return true; }
    catch (error) { setForm(previous); onPreviewTheme(previous.theme); feedback.error(String(error)); return false; }
    finally { setSaving(false); }
  };

  const openPath = async (item: PathInfo) => { if (openingPath) return; setOpeningPath(item.path); try { await api.openPath(item.path); } catch (error) { feedback.error(String(error)); } finally { setOpeningPath(null); } };

  return <section className="settings-page mx-auto flex w-full max-w-none flex-col">
    <header className="apple-page-bar">
      <h1 className="apple-title">{t("view.title")}</h1>
    </header>
    <AppTabBar<Section> selected={section} onSelect={setSection} label={t("view.sectionsLabel")} items={[
      { id: "general", label: <><Cog className="h-4 w-4 shrink-0" strokeWidth={2} />{t("view.tabGeneral")}</> },
      { id: "codex", label: <><AppWindow className="h-4 w-4 shrink-0" strokeWidth={2} />{t("view.tabApp")}</> },
      { id: "advanced", label: <><Database className="h-4 w-4 shrink-0" strokeWidth={2} />{t("view.tabAdvanced")}</> },
      { id: "about", label: <><Info className="h-4 w-4 shrink-0" strokeWidth={2} />{t("view.tabAbout")}</> },
    ]} />
    <div key={section} className="apple-edit-content">
    {section === "general" ? (
      <div className="flex flex-col gap-[var(--gap-section)]">
        <SettingsGeneral form={form} onPatch={(patch) => void saveGeneral(patch)} />
        <ProxySettings settings={state.settings} saving={saving} onSave={saveGeneral} />
      </div>
    ) : null}
    {section === "codex" ? (
      <SettingsPanelSection id="codex" label={t("codex.sectionTitle")}>
        <div className="apple-group px-[var(--gap-card-inline)]">
        <div className="flex flex-col divide-y divide-[var(--panel-divider)]">
          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex min-w-0 items-start gap-3">
              <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                <MousePointerClick className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
              </span>
              <div className="min-w-0">
                <div className="setting-title">{t("codex.trayClickTitle")}</div>
                <div className="setting-description mt-0.5">{t("codex.trayClickDescription")}</div>
              </div>
            </div>
            <div className="w-72 shrink-0">
              <AppSelect
                value={form.tray_click_action}
                options={[
                  { label: t("codex.trayClickShowWindow"), value: "show_window" as const },
                  { label: t("codex.trayClickShowMenu"), value: "show_menu" as const },
                ]}
                onChange={(value) => void saveGeneral({ tray_click_action: value })}
              />
            </div>
          </div>
          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex items-start gap-3">
              <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                <RotateCw className="h-[18px] w-[18px]" strokeWidth={2} />
              </span>
              <div>
                <div className="setting-title">{t("codex.autoRestartTitle")}</div>
                <div className="setting-description mt-0.5">
                  {t("codex.autoRestartDescription")}
                </div>
              </div>
            </div>
            <AppSwitch
              checked={form.auto_restart}
              onCheckedChange={(value) => void saveGeneral({ auto_restart: value })}
            />
          </div>
          <div className="flex items-center justify-between gap-4 py-4">
            <div className="flex items-start gap-3">
              <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                <ArrowUpCircle className="h-[18px] w-[18px]" strokeWidth={2} />
              </span>
              <div>
                <div className="setting-title">{t("codex.autoCheckTitle")}</div>
                <div className="setting-description mt-0.5">
                  {t("codex.autoCheckDescription")}
                </div>
              </div>
            </div>
            <AppSwitch
              checked={form.auto_check_update}
              onCheckedChange={(value) => void saveGeneral({ auto_check_update: value })}
            />
          </div>
        </div>
        </div>
      </SettingsPanelSection>
    ) : null}
    {section === "advanced" ? (
      <SettingsAdvanced form={form} onPatch={(patch) => void saveGeneral(patch)} paths={state.paths} backupsEpoch={backupsEpoch} onOpenPath={openPath} openingPath={openingPath} onRefresh={onRefresh} />
    ) : null}
    {section === "about" ? (
      <SettingsPanelSection id="about" label={t("about.sectionTitle")}>
        <SettingsAbout />
      </SettingsPanelSection>
    ) : null}
    </div>
  </section>;
}
