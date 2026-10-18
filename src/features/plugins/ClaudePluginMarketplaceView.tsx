import { RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedClaudePluginMarketplaces, loadClaudePluginMarketplaces } from "../../app/managementDataCache";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { TrashIcon } from "../../components/TrashIcon";
import PluginMarketplaceSources, { PluginMarketplaceRow } from "./components/PluginMarketplaceSources";
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
  const [busyMarket, setBusyMarket] = useState<string | null>(null);
  const refresh = async (force = false) => {
    try { setMarketplaces(await loadClaudePluginMarketplaces(force)); setLoadError(""); }
    catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
  };
  useEffect(() => { void refresh(); void refresh(true); }, []);

  const add = async (source: string) => {
    const marketplace = await api.claudeAddPluginMarketplace(source);
    setMarketplaces(await loadClaudePluginMarketplaces(true));
    setLoadError("");
    await onChanged();
    return marketplace;
  };

  const remove = async (marketplace: ClaudeMarketplace) => {
    if (busyMarket) return;
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
    if (busyMarket) return;
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
      {loadError ? <p className="muted text-sm">{loadError}</p> : null}
      <PluginMarketplaceSources target="claude" marketplaces={marketplaces} loaded={loaded} busy={busyMarket !== null} onAdd={add}>
        {marketplaces.map((marketplace) => (
          <PluginMarketplaceRow key={marketplace.name} name={marketplace.name} source={marketplace.repo ?? marketplace.path ?? marketplace.installLocation ?? ""}
            icon={marketplace.name === "claude-plugins-official" ? <img src="/claude-code.svg" alt="" className="h-4 w-4 shrink-0" draggable="false" /> : null}
          >
            <button type="button" className="apple-icon-button" disabled={busyMarket !== null} title={t("claudePlugin.action.refreshMarkets")} aria-label={t("claudePlugin.refreshMarketAria", { name: marketplace.name })} onClick={() => void update(marketplace)}>
              {busyMarket === marketplace.name ? <LoadingSpinner /> : <RefreshCw size={16} strokeWidth={2} />}
            </button>
            <button type="button" className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]" disabled={busyMarket !== null} title={t("claudePlugin.action.removeMarket")} aria-label={t("claudePlugin.action.removeMarketAria", { name: marketplace.name })} onClick={() => void remove(marketplace)}>
              <TrashIcon />
            </button>
          </PluginMarketplaceRow>
        ))}
      </PluginMarketplaceSources>
    </div>
  );
}
