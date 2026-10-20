import { ChevronLeft } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { originLabels } from "./pluginMeta";
import { PluginStatusDot } from "./components/PluginStatusDot";
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
  }, [plugin.name, plugin.store_path]);

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
              <PluginStatusDot enabled={plugin.enabled} />
              <span className="apple-chip">{originLabels[plugin.origin] ? t(originLabels[plugin.origin]!) : plugin.origin}</span>
            </div>
            {plugin.description ? <p className="muted mt-1.5 text-sm">{plugin.description}</p> : null}
            {plugin.marketplace || plugin.store_path ? (
              <div className="mt-3 space-y-2">
                {plugin.marketplace ? <div className="flex items-baseline gap-2">
                  <span className="field-label shrink-0">{t("detail.marketplace")}</span>
                  <span className="mono muted text-sm">{plugin.marketplace}</span>
                </div> : null}
                {plugin.store_path ? <div className="flex min-w-0 items-baseline gap-2">
                  <span className="field-label shrink-0">{t("detail.installPath")}</span>
                  <span className="mono muted min-w-0 break-all text-sm">{plugin.store_path}</span>
                </div> : null}
              </div>
            ) : null}
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
              <div className="apple-list-card mt-3">
                {skills.map((skill) => (
                  <div key={skill.path} className="apple-list-row">
                    <div className="min-w-0 flex-1">
                      <div className="font-semibold">{skill.name}</div>
                      {skill.description ? <div className="muted mt-0.5 break-words text-sm">{skill.description}</div> : null}
                    </div>
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
