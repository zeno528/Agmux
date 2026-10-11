import { ChevronLeft } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import ContainsChips from "./components/ContainsChips";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { originLabels } from "./pluginMeta";
import type { PluginSkill, PluginSummary } from "../../types";

export default function PluginDetailView({ plugin, onBack }: { plugin: PluginSummary; onBack: () => void }) {
  const { t } = useTranslation("plugins");
  const [skills, setSkills] = useState<PluginSkill[]>([]);
  const [skillsLoaded, setSkillsLoaded] = useState(false);
  const [skillsError, setSkillsError] = useState("");

  useEffect(() => {
    let cancelled = false;
    setSkills([]);
    setSkillsLoaded(false);
    setSkillsError("");
    void api.listPluginSkills(plugin.name, plugin.store_path)
      .then((items) => {
        if (!cancelled) setSkills(items);
      })
      .catch((error) => {
        if (!cancelled) setSkillsError(String(error));
      })
      .finally(() => {
        if (!cancelled) setSkillsLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [plugin.name]);

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("nav.backToPlugins")} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />
          <span className="apple-title">{plugin.display_name ?? plugin.name}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          <div className="apple-panel-section apple-panel-section--compact">
            <div className="flex flex-wrap items-center gap-2">
              <span className="title-md">{plugin.display_name ?? plugin.name}</span>
              {plugin.version ? <span className="apple-chip">v{plugin.version}</span> : null}
              {plugin.enabled ? null : <span className="apple-chip chip-warn">{t("detail.disabled")}</span>}
            </div>
            {plugin.description ? <p className="muted mt-1.5 text-sm">{plugin.description}</p> : null}
            {plugin.category || plugin.capabilities.length ? <div className="mt-2 flex flex-wrap items-center gap-2">
              {plugin.category ? <span className="apple-chip">{t("detail.category", { category: plugin.category })}</span> : null}
              {plugin.capabilities.length ? <ContainsChips items={plugin.capabilities} /> : null}
            </div> : null}
          </div>
          <div className="apple-panel-section apple-panel-section--compact">
            <div className="flex flex-wrap items-baseline gap-x-6 gap-y-2">
              <div className="flex items-baseline gap-2">
                <span className="field-label shrink-0">{t("detail.origin")}</span>
                <span className="text-sm">{originLabels[plugin.origin] ? t(originLabels[plugin.origin]!) : plugin.origin}</span>
              </div>
              <div className="flex items-baseline gap-2">
                <span className="field-label shrink-0">{t("detail.marketplace")}</span>
                <span className="mono text-sm">{plugin.marketplace ?? t("detail.local")}</span>
              </div>
            </div>
            <div className="mt-2 flex min-w-0 items-baseline gap-2">
              <span className="field-label shrink-0">{t("detail.installPath")}</span>
              <span className="mono muted min-w-0 break-all text-sm">{plugin.store_path}</span>
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span className="field-label shrink-0">{t("detail.composition")}</span>
              {plugin.contains.length ? <ContainsChips items={plugin.contains} /> : <span className="muted text-sm">{t("detail.compositionEmpty")}</span>}
            </div>
          </div>
          <div className="apple-panel-section">
            <div className="flex items-center gap-2">
              <div className="field-label">Skills</div>
              {skillsLoaded ? <span className="apple-chip" aria-label={t("detail.skillCount", { count: skills.length })}>{skills.length}</span> : null}
            </div>
            {!skillsLoaded ? (
              <div className="muted mt-3 flex items-center gap-2 text-sm"><LoadingSpinner />{t("detail.loadingSkills")}</div>
            ) : skillsError ? (
              <p className="muted mt-2 text-sm">{skillsError}</p>
            ) : skills.length ? (
              <div className="mt-3 space-y-2">
                {skills.map((skill) => (
                  <div key={skill.path} className="rounded-[var(--radius-control)] bg-black/3 p-3 shadow-[0_0_0_1px_var(--panel-ring)] dark:bg-white/4">
                    <div className="font-semibold">{skill.name}</div>
                    <div className="mono muted meta-xs mt-1 break-all">{skill.path}</div>
                    {skill.description ? <div className="muted mt-1.5 text-sm">{skill.description}</div> : null}
                  </div>
                ))}
              </div>
            ) : <p className="muted mt-2 text-sm">{t("detail.skillsEmpty")}</p>}
          </div>
        </div>
      </div>
    </section>
  );
}
