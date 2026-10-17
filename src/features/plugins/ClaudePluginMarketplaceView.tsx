import { ChevronLeft, ChevronRight, Download, Plus, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedClaudePlugins, loadClaudePlugins, getCachedClaudePluginMarketplaces, loadClaudePluginMarketplaces } from "../../app/managementDataCache";
import { AppDialog } from "../../components/AppDialog";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { TrashIcon } from "../../components/TrashIcon";
import PluginSearchInput from "./components/PluginSearchInput";
import { MarketplacePage, MarketplaceGroup, MarketplaceRow } from "./components/MarketplaceLayout";
import { MarketplacePluginRow } from "./components/MarketplacePluginRow";
import { marketplaceKindLabels } from "./pluginMeta";
import { claudeInstalledIdSet, groupClaudePluginsByMarketplace } from "./claudePluginMeta";
import { matchesQuery } from "./pluginMeta";
import type { ClaudeAvailablePlugin, ClaudeMarketplace, ClaudePluginInventory } from "../../types";

export default function ClaudePluginMarketplaceView({ onBack, onInstalled }: {
  onBack: () => void;
  onInstalled: () => Promise<void>;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  const [inventory, setInventory] = useState<ClaudePluginInventory>(() => getCachedClaudePlugins(true) ?? { installed: [], available: [] });
  const [loaded, setLoaded] = useState(() => getCachedClaudePlugins(true) !== null);
  const [loadError, setLoadError] = useState("");
  const [marketplaces, setMarketplaces] = useState<ClaudeMarketplace[]>(() => getCachedClaudePluginMarketplaces() ?? []);
  const [query, setQuery] = useState("");
  const [selectedMarketplace, setSelectedMarketplace] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [removingMarket, setRemovingMarket] = useState("");
  const [refreshingMarkets, setRefreshingMarkets] = useState(false);
  const [showAdd, setShowAdd] = useState(false);
  const [addSource, setAddSource] = useState("");
  const [addingMarket, setAddingMarket] = useState(false);
  const addInputRef = useRef<HTMLInputElement>(null);
  const refresh = async (force = false) => {
    try { setInventory(await loadClaudePlugins(force, true)); setLoadError(""); }
    catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
  };
  const refreshMarketplaces = async (force = false) => {
    try { setMarketplaces(await loadClaudePluginMarketplaces(force)); }
    catch (error) { setLoadError(String(error)); }
  };
  useEffect(() => {
    void refresh(); void refresh(true);
    void refreshMarketplaces(); void refreshMarketplaces(true);
  }, []);
  const installedIds = claudeInstalledIdSet(inventory.installed);
  const marketGroups = groupClaudePluginsByMarketplace(inventory.available, inventory.installed);
  const marketNames = [...new Set([...marketplaces.map((marketplace) => marketplace.name), ...marketGroups.map((group) => group.marketplace)])];
  const isOfficial = (name: string) => name === "claude-plugins-official" || name === "anthropic-plugin-directory";
  const selected = marketGroups.find((group) => group.marketplace === selectedMarketplace);
  const visiblePlugins = selected?.plugins.filter((plugin) => matchesQuery(plugin, query)) ?? [];
  const selectedInstalledCount = selected?.plugins.filter((plugin) => installedIds.has(plugin.pluginId)).length ?? 0;

  const install = async (plugin: ClaudeAvailablePlugin) => {
    if (installing) return;
    setInstalling(plugin.pluginId);
    try {
      await api.claudeInstallPlugin(plugin.pluginId);
      feedback.success(t("claudePlugin.toast.installed", { name: plugin.name }));
      await onInstalled();
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setInstalling(null);
    }
  };

  const addMarketplace = async () => {
    const source = addSource.trim();
    if (!source || addingMarket) return;
    setAddingMarket(true);
    try {
      const marketplace = await api.claudeAddPluginMarketplace(source);
      feedback.success(t("claudePlugin.toast.marketAdded", { name: marketplace.name }));
      setShowAdd(false);
      setAddSource("");
      await refreshMarketplaces(true);
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setAddingMarket(false);
    }
  };

  const removeMarketplace = async (marketplace: ClaudeMarketplace) => {
    const confirmed = await feedback.confirm({
      title: t("claudePlugin.confirm.removeMarketTitle"),
      description: t("claudePlugin.confirm.removeMarketBody", { name: marketplace.name }),
      confirmText: t("claudePlugin.action.removeMarket"),
      destructive: true,
    });
    if (!confirmed || removingMarket) return;
    setRemovingMarket(marketplace.name);
    try {
      await api.claudeRemovePluginMarketplace(marketplace.name);
      feedback.success(t("claudePlugin.toast.marketRemoved", { name: marketplace.name }));
      await onInstalled();
      await refreshMarketplaces(true);
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setRemovingMarket("");
    }
  };

  const refreshMarkets = async () => {
    if (refreshingMarkets) return;
    setRefreshingMarkets(true);
    let failures = 0;
    // 逐市场顺序刷新：一次全量刷新容易超出单次命令限时
    for (const marketplace of marketplaces) {
      try {
        await api.claudeUpdatePluginMarketplace(marketplace.name);
      } catch (error) {
        failures += 1;
        feedback.error(String(error));
      }
    }
    await Promise.all([refresh(true), refreshMarketplaces(true)]);
    if (failures === 0 && marketplaces.length > 0) {
      feedback.success(t("claudePlugin.toast.marketsRefreshed", { count: marketplaces.length }));
    }
    setRefreshingMarkets(false);
  };

  if (selectedMarketplace) {
    return (
      <MarketplacePage
        onBack={() => setSelectedMarketplace(null)}
        backLabel={t("nav.backToMarketplace")}
        heading={<ManagementPageTitle
          className="management-page-title--compact"
          icon={<ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />}
          title={selectedMarketplace}
          count={loaded ? selectedInstalledCount : undefined}
          countLabel={t("marketDetail.installedCountAria", { count: selectedInstalledCount })}
          loadingLabel={t("marketDetail.loadingAria")}
        />}
        actions={<PluginSearchInput value={query} onChange={setQuery} />}
      >
        <div className="apple-edit-surface">
          <div className="apple-panel-section">
            <div className="flex flex-wrap items-center gap-2">
              <div className="field-label">{t("marketDetail.browsable")}</div>
              <span className="apple-chip">{t(marketplaceKindLabels[isOfficial(selectedMarketplace) ? "official" : "third-party"])}</span>
              <span className="apple-chip">{selected?.plugins.length ?? 0}</span>
            </div>
            {loadError ? <p className="muted mt-2 text-sm">{loadError}</p> : null}
            {loaded && !visiblePlugins.length ? <p className="muted mt-2 text-sm">{t(query.trim() ? "list.noMatch" : "marketDetail.empty")}</p> : null}
          </div>
          {visiblePlugins.map((plugin) => (
            <MarketplacePluginRow
              key={plugin.pluginId}
              name={plugin.name}
              description={plugin.description}
              installed={installedIds.has(plugin.pluginId)}
              actions={!installedIds.has(plugin.pluginId) ? (
                <button type="button" className="apple-action-button app-button--primary shrink-0" disabled={installing !== null} onClick={() => void install(plugin)}>
                  {installing === plugin.pluginId ? <LoadingSpinner /> : <Download className="h-4 w-4" strokeWidth={2} />}
                  {t("action.install")}
                </button>
              ) : null}
            />
          ))}
        </div>
      </MarketplacePage>
    );
  }

  return (
    <>
      <MarketplacePage
        onBack={onBack}
        backLabel={t("nav.backToPlugins")}
        heading={<><ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} /><span className="apple-title">{t("marketplace")}</span></>}
        actions={<>
          <button type="button" className="apple-action-button" disabled={refreshingMarkets} title={t("claudePlugin.action.refreshMarkets")} onClick={() => void refreshMarkets()}>
            {refreshingMarkets ? <LoadingSpinner /> : <RefreshCw className="h-4 w-4" strokeWidth={2} />}
          </button>
          <button type="button" className="apple-action-button app-button--primary" onClick={() => setShowAdd(true)}>
            <Plus className="h-4 w-4" strokeWidth={2} />{t("claudePlugin.action.addMarket")}
          </button>
        </>}
      >
        {loadError ? <p className="muted text-sm">{loadError}</p> : null}
        {(["official", "third-party"] as const).map((kind) => {
          const names = marketNames.filter((name) => isOfficial(name) === (kind === "official"));
          return (
            <MarketplaceGroup key={kind} title={t(marketplaceKindLabels[kind])} count={loaded ? names.length : undefined}>
              {names.map((name) => {
                const meta = marketplaces.find((marketplace) => marketplace.name === name);
                return (
                  <MarketplaceRow
                    key={name}
                    name={name}
                    source={meta?.repo ?? meta?.path ?? ""}
                    count={loaded ? marketGroups.find((group) => group.marketplace === name)?.plugins.length ?? 0 : undefined}
                    actions={<>
                      {meta ? (
                        <button type="button" className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]" disabled={Boolean(removingMarket)} title={t("claudePlugin.action.removeMarket")} aria-label={t("claudePlugin.action.removeMarketAria", { name })} onClick={() => void removeMarketplace(meta)}>
                          {removingMarket === name ? <LoadingSpinner /> : <TrashIcon />}
                        </button>
                      ) : null}
                      <button type="button" className="apple-action-button" onClick={() => { setQuery(""); setSelectedMarketplace(name); }}>
                        <ChevronRight className="h-4 w-4" strokeWidth={2} />{t("action.browse")}
                      </button>
                    </>}
                  />
                );
              })}
            </MarketplaceGroup>
          );
        })}
      </MarketplacePage>
      <AppDialog
        open={showAdd}
        onOpenChange={setShowAdd}
        title={t("claudePlugin.addMarket.title")}
        description={t("claudePlugin.addMarket.description")}
        initialFocusRef={addInputRef}
        footer={
          <>
            <button type="button" className="apple-action-button" onClick={() => setShowAdd(false)}>
              {t("claudePlugin.addMarket.cancel")}
            </button>
            <button
              type="button"
              className="apple-action-button app-button--primary"
              disabled={!addSource.trim() || addingMarket}
              onClick={() => void addMarketplace()}
            >
              {addingMarket ? <LoadingSpinner /> : <Plus className="h-4 w-4" strokeWidth={2} />}
              {t("claudePlugin.addMarket.confirm")}
            </button>
          </>
        }
      >
        <input
          ref={addInputRef}
          className="app-input w-full"
          placeholder={t("claudePlugin.addMarket.placeholder")}
          value={addSource}
          onChange={(event) => setAddSource(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && addSource.trim()) void addMarketplace();
          }}
        />
      </AppDialog>
    </>
  );
}
