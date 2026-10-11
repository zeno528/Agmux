import { ChevronLeft, Download, Plus, Search } from "lucide-react";
import { useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppSelect } from "../../components/AppSelect";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import ContainsChips from "./components/ContainsChips";
import type { PluginCandidate, PluginMarketplace, PluginPreview } from "../../types";

export default function AddPluginView({
  onBack,
  onMarketplaceAdded,
  onInstalled,
}: {
  onBack: () => void;
  onMarketplaceAdded: (marketplace: PluginMarketplace) => void | Promise<void>;
  onInstalled: () => Promise<void>;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  const [method, setMethod] = useState<"marketplace" | "repository">("marketplace");
  const [url, setUrl] = useState("");
  const [adding, setAdding] = useState(false);
  const [preview, setPreview] = useState<PluginPreview | null>(null);
  const [installing, setInstalling] = useState("");

  const addMarketplace = async () => {
    if (adding) return;
    if (!url.trim()) {
      feedback.warning(t("add.missingMarketUrl"));
      return;
    }
    setAdding(true);
    try {
      const marketplace = await api.addPluginMarketplace(url.trim());
      feedback.success(t("add.added", { name: marketplace.name }));
      await onMarketplaceAdded(marketplace);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setAdding(false);
    }
  };

  const openPreview = async () => {
    if (adding) return;
    if (!url.trim()) {
      feedback.warning(t("add.missingRepoUrl"));
      return;
    }
    setAdding(true);
    try {
      setPreview(await api.previewPlugin(url.trim()));
    } catch (error) {
      feedback.error(String(error));
      setPreview(null);
    } finally {
      setAdding(false);
    }
  };

  const install = async (candidate: PluginCandidate) => {
    if (installing) return;
    setInstalling(candidate.name);
    try {
      const summary = await api.installPlugin(url.trim(), candidate.sub_path || null);
      feedback.success(t("toast.installed", { name: summary.display_name ?? summary.name }));
      setPreview(null);
      await onInstalled();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setInstalling("");
    }
  };

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("nav.backToMarketplace")} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />
          <span className="apple-title">{t("add.title")}</span>
        </button>
      </div>
      <div className="apple-edit-content">
        <div className="space-y-4">
          <div className="apple-edit-surface">
            <div className="apple-panel-section">
              <div className="field-label mb-1.5">{t("add.method")}</div>
              <AppSelect
                value={method}
                options={[
                  { label: t("add.methodMarketplace"), value: "marketplace" as const },
                  { label: t("add.methodRepository"), value: "repository" as const },
                ]}
                onChange={(value) => { setMethod(value); setPreview(null); }}
              />
              <div className="field-label mt-4">{method === "marketplace" ? t("add.marketSourceTitle") : t("add.repoTitle")}</div>
              <p className="muted mt-2 text-sm">{method === "marketplace" ? t("add.marketSourceHint") : t("add.repoHint")}</p>
              <div className="mt-4 flex w-full flex-wrap items-center gap-2">
                <input
                  className="app-input min-w-0 flex-1"
                  placeholder={method === "marketplace" ? t("add.marketPlaceholder") : t("add.repoPlaceholder")}
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && !event.nativeEvent.isComposing) void (method === "marketplace" ? addMarketplace() : openPreview());
                  }}
                />
                <button type="button" className="apple-action-button app-button--primary" disabled={adding} onClick={() => void (method === "marketplace" ? addMarketplace() : openPreview())}>
                  {adding ? <LoadingSpinner /> : method === "marketplace" ? <Plus className="h-4 w-4" strokeWidth={2} /> : <Search className="h-4 w-4" strokeWidth={2} />}
                  {method === "marketplace" ? t("add.methodMarketplace") : t("action.fetchList")}
                </button>
              </div>
            </div>
          </div>
          {method === "repository" && preview ? (
            <div className="apple-edit-surface">
              <div className="apple-panel-section">
                <div className="field-label">{t("add.candidates")}</div>
                <p className="muted mt-1.5 text-sm">
                  <Trans ns="plugins" i18nKey="add.repoSummary" values={{ repo: preview.repo, reference: preview.reference }} components={{ repo: <span className="mono" />, reference: <span className="mono" /> }} />
                  {preview.reference !== preview.default_branch ? t("add.defaultBranch", { branch: preview.default_branch }) : ""}
                </p>
              </div>
              <div className="space-y-3">
                {preview.candidates.map((candidate) => (
                  <div key={candidate.sub_path || candidate.name} className="apple-panel-section">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="truncate font-semibold">{candidate.display_name ?? candidate.name}</span>
                          {candidate.version ? <span className="apple-chip">v{candidate.version}</span> : null}
                        </div>
                        <div className="muted meta-xs truncate">{candidate.description ?? (candidate.sub_path || t("add.rootDir"))}</div>
                      </div>
                      <button type="button" className="apple-action-button app-button--primary" disabled={installing !== ""} onClick={() => void install(candidate)}>
                        {installing === candidate.name ? <LoadingSpinner /> : <Download className="h-4 w-4" strokeWidth={2} />}
                        {t("action.install")}
                      </button>
                    </div>
                    <div className="mt-2"><ContainsChips items={candidate.contains} /></div>
                    <details className="mt-2">
                      <summary className="muted meta-xs cursor-pointer select-none">{t("add.files", { count: candidate.files.length })}</summary>
                      <ul className="mono muted mt-1.5 flex flex-col gap-0.5">
                        {candidate.files.slice(0, 40).map((file) => <li key={file} className="truncate">{file}</li>)}
                        {candidate.files.length > 40 ? <li>{t("add.filesMore", { count: candidate.files.length - 40 })}</li> : null}
                      </ul>
                    </details>
                  </div>
                ))}
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}
