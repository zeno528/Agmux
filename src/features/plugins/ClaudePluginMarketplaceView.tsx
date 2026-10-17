import { Plus, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedClaudePluginMarketplaces, loadClaudePluginMarketplaces } from "../../app/managementDataCache";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { TrashIcon } from "../../components/TrashIcon";
import SourceLink from "./components/SourceLink";
import type { ClaudeMarketplace } from "../../types";

/** 市场源管理弹层的内容：增删改 Claude Code 的插件市场来源。 */
export default function ClaudePluginMarketplaceView({ onChanged }: {
  onChanged: () => Promise<void>;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  const [marketplaces, setMarketplaces] = useState<ClaudeMarketplace[]>(() => getCachedClaudePluginMarketplaces() ?? []);
  const [loaded, setLoaded] = useState(() => getCachedClaudePluginMarketplaces() !== null);
  const [loadError, setLoadError] = useState("");
  const [source, setSource] = useState("");
  const [adding, setAdding] = useState(false);
  const [busyMarket, setBusyMarket] = useState<string | null>(null);
  const refresh = async (force = false) => {
    try { setMarketplaces(await loadClaudePluginMarketplaces(force)); setLoadError(""); }
    catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
  };
  useEffect(() => { void refresh(); void refresh(true); }, []);

  const add = async () => {
    if (!source.trim() || adding || busyMarket) return;
    setAdding(true);
    try {
      const marketplace = await api.claudeAddPluginMarketplace(source.trim());
      setSource("");
      feedback.success(t("claudePlugin.toast.marketAdded", { name: marketplace.name }));
      await refresh(true);
      await onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setAdding(false);
    }
  };

  const remove = async (marketplace: ClaudeMarketplace) => {
    if (busyMarket || adding) return;
    const confirmed = await feedback.confirm({
      title: t("claudePlugin.confirm.removeMarketTitle"),
      description: t("claudePlugin.confirm.removeMarketBody", { name: marketplace.name }),
      confirmText: t("claudePlugin.action.removeMarket"),
      destructive: true,
    });
    if (!confirmed) return;
    setBusyMarket(marketplace.name);
    try {
      await api.claudeRemovePluginMarketplace(marketplace.name);
      feedback.success(t("claudePlugin.toast.marketRemoved", { name: marketplace.name }));
      await refresh(true);
      await onChanged();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusyMarket(null);
    }
  };

  const update = async (marketplace: ClaudeMarketplace) => {
    if (busyMarket || adding) return;
    setBusyMarket(marketplace.name);
    try {
      await api.claudeUpdatePluginMarketplace(marketplace.name);
      await refresh(true);
      await onChanged();
      feedback.success(t("claudePlugin.toast.marketsRefreshed", { count: 1 }));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusyMarket(null);
    }
  };

  return (
    <div className="space-y-4">
      <form className="flex items-center gap-2" onSubmit={(event) => { event.preventDefault(); void add(); }}>
        <input
          className="app-input min-w-0 flex-1"
          aria-label={t("claudePlugin.addMarket.description")}
          placeholder={t("claudePlugin.addMarket.placeholder")}
          value={source}
          onChange={(event) => setSource(event.target.value)}
        />
        <button type="submit" className="apple-action-button app-button--primary" disabled={!source.trim() || adding || busyMarket !== null}>
          {adding ? <LoadingSpinner /> : <Plus size={16} strokeWidth={2} />}
          {t("claudePlugin.addMarket.confirm")}
        </button>
      </form>
      <div className="flex items-center gap-2">
        <div className="field-label">{t("marketplace")}</div>
        {loaded ? <span className="apple-chip">{marketplaces.length}</span> : <LoadingSpinner />}
      </div>
      {loadError ? <p className="muted text-sm">{loadError}</p> : null}
      {marketplaces.length ? (
        <div className="apple-group apple-list-card">
          {marketplaces.map((marketplace) => (
            <div key={marketplace.name} className="apple-list-row">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5">
                  <span className="font-semibold">{marketplace.name}</span>
                  {/* 官方市场挂 Claude 标识，与 VS Code 插件列表一致 */}
                  {marketplace.name === "claude-plugins-official" ? <img src="/claude-code.svg" alt="" className="h-4 w-4 shrink-0" draggable="false" /> : null}
                </div>
                <div className="flex items-center gap-1">
                  {marketplace.repo ? <span className="mono muted meta-xs break-all">{marketplace.repo}</span> : null}
                  <SourceLink source={marketplace.repo ?? marketplace.path ?? marketplace.installLocation ?? ""} />
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                <button type="button" className="apple-icon-button" disabled={busyMarket !== null || adding} title={t("claudePlugin.action.refreshMarkets")} aria-label={t("claudePlugin.refreshMarketAria", { name: marketplace.name })} onClick={() => void update(marketplace)}>
                  {busyMarket === marketplace.name ? <LoadingSpinner /> : <RefreshCw size={16} strokeWidth={2} />}
                </button>
                <button type="button" className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]" disabled={busyMarket !== null || adding} title={t("claudePlugin.action.removeMarket")} aria-label={t("claudePlugin.action.removeMarketAria", { name: marketplace.name })} onClick={() => void remove(marketplace)}>
                  <TrashIcon />
                </button>
              </div>
            </div>
          ))}
        </div>
      ) : loaded ? <p className="muted text-sm">{t("claudePlugin.noMarkets")}</p> : null}
    </div>
  );
}
