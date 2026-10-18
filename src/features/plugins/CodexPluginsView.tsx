import { ArrowUp, Blocks, Download, RefreshCw, Store } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import {
  getCachedMarketplacePlugins,
  getCachedPluginMarketplaces,
  getCachedPlugins,
  loadPluginMarketplaces,
  loadPlugins,
  refreshMarketplacePlugins,
  setCachedMarketplacePlugins,
} from "../../app/managementDataCache";
import { AppDialog } from "../../components/AppDialog";
import { AppTooltip } from "../../components/AppTooltip";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { TrashIcon } from "../../components/TrashIcon";
import PluginDetailView from "./PluginDetailView";
import ContainsChips from "./components/ContainsChips";
import PluginFilterChips, { type PluginFilter } from "./components/PluginFilterChips";
import { PluginCard } from "./components/PluginCard";
import PluginMarketplaceSources, { PluginMarketplaceRow } from "./components/PluginMarketplaceSources";
import {
  compareMarketplacePlugins,
  comparePlugins,
  matchesQuery,
  originLabels,
  removableOrigins,
  resolveAliasInstalled,
} from "./pluginMeta";
import type { PluginHeaderState } from "./PluginsView";
import type { AppState, MarketplacePlugin, PluginMarketplace, PluginSummary, PluginUpdate } from "../../types";

type AvailablePlugin = MarketplacePlugin & { marketplaceName: string };

export default function CodexPluginsView({
  state,
  popEpoch,
  query,
  onHeaderChange,
}: {
  state: AppState;
  popEpoch: number;
  query: string;
  onHeaderChange: (header: PluginHeaderState) => void;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  // 缓存命中即首帧直出，不等微任务回填；未命中时保持加载态走正常请求
  const [plugins, setPlugins] = useState<PluginSummary[]>(() => getCachedPlugins() ?? []);
  const [loaded, setLoaded] = useState(() => getCachedPlugins() !== null);
  const [loadError, setLoadError] = useState("");
  const [marketplaces, setMarketplaces] = useState<PluginMarketplace[]>(() => getCachedPluginMarketplaces() ?? []);
  const [marketsLoaded, setMarketsLoaded] = useState(() => getCachedPluginMarketplaces() !== null);
  const [marketplacePlugins, setMarketplacePlugins] = useState<Record<string, MarketplacePlugin[]>>(() => {
    // 市场清单缓存直出：拼平进「可安装」区不再经过市场子页，无缓存的市场留给静默刷新回填
    const cached = getCachedPluginMarketplaces() ?? [];
    return Object.fromEntries(
      cached
        .map((marketplace) => [marketplace.name, getCachedMarketplacePlugins(marketplace.name)] as const)
        .filter((entry): entry is readonly [string, MarketplacePlugin[]] => entry[1] !== null),
    );
  });
  const [updates, setUpdates] = useState<PluginUpdate[]>([]);
  const [checking, setChecking] = useState(false);
  const [upgradingAll, setUpgradingAll] = useState(false);
  const [selectedPlugin, setSelectedPlugin] = useState<PluginSummary | null>(null);
  const [showSources, setShowSources] = useState(false);
  const [filter, setFilter] = useState<PluginFilter>("all");
  const [installing, setInstalling] = useState("");
  const [removingMarket, setRemovingMarket] = useState("");
  const [refreshingMarket, setRefreshingMarket] = useState("");
  const thirdPartyProfile =
    state.codex_profiles.find((profile) => profile.id === state.active_codex_profile_id)?.kind === "third_party";

  const refresh = async (force = false) => {
    try {
      setPlugins(await loadPlugins(force));
      setLoadError("");
    } catch (error) {
      setLoadError(String(error));
    } finally {
      setLoaded(true);
    }
  };
  useEffect(() => {
    // 先显缓存（命中即回），后台再强制取一次最新，保证外部变更能浮上来
    void refresh();
    void refresh(true);
    void refreshMarketplaces(true);
  }, []);

  // 各市场插件清单拼平：与旧市场页的数量刷新同构，只是把列表留下来进「可安装」区
  useEffect(() => {
    let cancelled = false;
    void Promise.all(
      marketplaces.map(async (marketplace) => {
        try {
          return [marketplace.name, await refreshMarketplacePlugins(marketplace.name, marketplace.root)] as const;
        } catch {
          // 单个市场加载失败只缺该市场的条目，不拖垮整个看板
          return null;
        }
      }),
    ).then((entries) => {
      if (cancelled) return;
      setMarketplacePlugins((previous) => {
        const next = { ...previous };
        for (const entry of entries) {
          if (entry) next[entry[0]] = entry[1];
        }
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [marketplaces]);

  // 侧栏重复点击本视图 = 弹回一级；只清子页与弹层状态，不刷新（安装/卸载流程自带刷新）。
  useEffect(() => {
    if (!popEpoch) return;
    setSelectedPlugin(null);
    setShowSources(false);
  }, [popEpoch]);

  const refreshMarketplaces = async (force = false) => {
    try {
      setMarketplaces(await loadPluginMarketplaces(force));
    } catch {
      // 市场源刷新失败保留旧列表；看板仍可用已缓存内容
    } finally {
      setMarketsLoaded(true);
    }
  };

  const originChip = (plugin: PluginSummary) => originLabels[plugin.origin] ? (
    <span className={`shrink-0 rounded-md px-1.5 py-px font-medium tracking-wide meta-xs ${plugin.origin === "official" ? "origin-chip--official" : "origin-chip--third-party"}`}>
      {t(originLabels[plugin.origin]!)}
    </span>
  ) : null;

  const installedNames = useMemo(() => new Set(plugins.map((plugin) => plugin.name)), [plugins]);
  const orderedPlugins = useMemo(() => [...plugins].sort(comparePlugins), [plugins]);
  const visibleInstalled = orderedPlugins.filter((plugin) => matchesQuery(plugin, query));

  // 全市场可安装清单：别名纠正 → 过滤已装与当前搜索 → 按 plugin_id 去重（多市场同插件取先出现的市场）
  const available = useMemo<AvailablePlugin[]>(() => {
    const byId = new Map<string, AvailablePlugin>();
    for (const marketplace of marketplaces) {
      const items = marketplacePlugins[marketplace.name];
      if (!items) continue;
      for (const plugin of resolveAliasInstalled(items, installedNames)) {
        if (plugin.installed || byId.has(plugin.plugin_id) || !matchesQuery(plugin, query)) continue;
        byId.set(plugin.plugin_id, { ...plugin, marketplaceName: marketplace.name });
      }
    }
    return [...byId.values()].sort(compareMarketplacePlugins);
  }, [marketplaces, marketplacePlugins, installedNames, query]);

  const visibleUpdates = useMemo(
    () => updates.filter((update) => visibleInstalled.some((plugin) => plugin.name === update.name)),
    [updates, visibleInstalled],
  );
  const counts = {
    all: visibleInstalled.length + available.length,
    installed: visibleInstalled.length,
    available: available.length,
    updates: visibleUpdates.length,
  };

  /// 本地与全局市场缓存同步打补丁，避免卸载/安装后「可安装」区闪回旧状态。
  const markInstalledEverywhere = (name: string, installed: boolean) => {
    const next: Record<string, MarketplacePlugin[]> = {};
    for (const [market, items] of Object.entries(marketplacePlugins)) {
      next[market] = items.some((item) => item.name === name)
        ? items.map((item) => (item.name === name ? { ...item, installed } : item))
        : items;
      if (next[market] !== items) setCachedMarketplacePlugins(market, next[market]);
    }
    setMarketplacePlugins(next);
  };

  const install = async (plugin: AvailablePlugin) => {
    if (installing) return;
    setInstalling(plugin.plugin_id);
    try {
      await api.installMarketplacePlugin(plugin.marketplaceName, plugin.name);
      const next = (marketplacePlugins[plugin.marketplaceName] ?? []).map((item) =>
        item.plugin_id === plugin.plugin_id ? { ...item, installed: true } : item,
      );
      setMarketplacePlugins((previous) => ({ ...previous, [plugin.marketplaceName]: next }));
      setCachedMarketplacePlugins(plugin.marketplaceName, next);
      // ON_INSTALL 插件的 OAuth 弹窗是桌面端 app-server 私有能力，CLI 无法触发；
      // 安装成功后明确引导用户去桌面端完成授权，避免「装了但不可用」的静默缺失。
      if (plugin.auth_policy === "ON_INSTALL") {
        feedback.warning(t("toast.installedNeedsAuth", { name: plugin.name }));
      } else {
        feedback.success(t("toast.installed", { name: plugin.name }));
      }
      await refresh(true);
    } catch (reason) {
      feedback.error(String(reason));
    } finally {
      setInstalling("");
    }
  };

  const remove = async (plugin: PluginSummary) => {
    const confirmed = await feedback.confirm({
      title: t("confirm.uninstallPluginTitle"),
      description: t("list.removeConfirm", {
        name: plugin.display_name ?? plugin.name,
        source: plugin.marketplace ? t("list.sourceKeptMarket", { marketplace: plugin.marketplace }) : t("list.sourceKept"),
      }),
      confirmText: t("action.uninstall"),
      destructive: true,
    });
    if (!confirmed) return;
    try {
      await api.uninstallPlugin(plugin.name);
      markInstalledEverywhere(plugin.name, false);
      feedback.success(t("toast.uninstalled", { name: plugin.display_name ?? plugin.name }));
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    }
  };

  const checkUpdates = async () => {
    if (checking || upgradingAll) return;
    setChecking(true);
    try {
      const items = await api.checkPluginUpdates();
      setUpdates(items);
      feedback.success(items.length ? t("market.foundUpdates", { count: items.length }) : t("market.upToDate"));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setChecking(false);
    }
  };

  const upgrade = async (update: PluginUpdate) => {
    await api.upgradeMarketplacePlugin(update.marketplace, update.name);
    setUpdates((items) => items.filter((item) => item.name !== update.name || item.marketplace !== update.marketplace));
  };

  const upgradeAll = async () => {
    if (!updates.length || upgradingAll || checking) return;
    setUpgradingAll(true);
    try {
      for (const update of updates) await upgrade(update);
      feedback.success(t("market.allUpgraded"));
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setUpgradingAll(false);
    }
  };

  const removeMarketplace = async (marketplace: PluginMarketplace) => {
    if (removingMarket || refreshingMarket || marketplace.kind === "official") return;
    const confirmed = await feedback.confirm({
      title: t("market.removeConfirmTitle"),
      description: t("market.removeConfirm", { name: marketplace.name }),
      confirmText: t("action.removeMarket"),
      destructive: true,
    });
    if (!confirmed) return;
    setRemovingMarket(marketplace.name);
    try {
      await api.removePluginMarketplace(marketplace.name);
      feedback.success(t("market.removedToast", { name: marketplace.name }));
      await refreshMarketplaces(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setRemovingMarket("");
    }
  };

  const addMarketplace = async (source: string) => {
    const marketplace = await api.addPluginMarketplace(source);
    setMarketplaces(await loadPluginMarketplaces(true));
    setMarketsLoaded(true);
    return marketplace;
  };

  const updateMarketplace = async (marketplace: PluginMarketplace) => {
    if (refreshingMarket || removingMarket || installing) return;
    setRefreshingMarket(marketplace.name);
    try {
      await api.refreshPluginMarketplace(marketplace.name);
      const plugins = await refreshMarketplacePlugins(marketplace.name, marketplace.root);
      setMarketplacePlugins((previous) => ({ ...previous, [marketplace.name]: plugins }));
      await refresh(true);
      feedback.success(t("claudePlugin.toast.marketsRefreshed", { count: 1 }));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setRefreshingMarket("");
    }
  };

  useLayoutEffect(() => {
    onHeaderChange({
      editing: Boolean(selectedPlugin),
      title: <ManagementPageTitle
        icon={<span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><Blocks className="h-[18px] w-[18px]" strokeWidth={2} /></span>}
        title={t("title")}
        count={null}
      />,
    });
  }, [onHeaderChange, selectedPlugin, t]);

  if (selectedPlugin) {
    return <PluginDetailView plugin={selectedPlugin} onBack={() => setSelectedPlugin(null)} />;
  }

  const showUpdates = filter === "all" || filter === "updates";
  const showInstalled = filter === "all" || filter === "installed";
  const showAvailable = filter === "all" || filter === "available";
  const boardEmpty = loaded && !visibleInstalled.length && !available.length && !visibleUpdates.length;

  const renderSourceRow = (marketplace: PluginMarketplace) => (
    <PluginMarketplaceRow key={marketplace.name} name={marketplace.display_name ?? marketplace.name} source={marketplace.source_url ?? marketplace.root}>
      {marketplace.kind === "third-party" ? (
        <>
          <button type="button" className="apple-icon-button" title={t("claudePlugin.action.refreshMarkets")} aria-label={t("claudePlugin.refreshMarketAria", { name: marketplace.name })} onClick={() => void updateMarketplace(marketplace)}>
            {refreshingMarket === marketplace.name ? <LoadingSpinner /> : <RefreshCw size={16} strokeWidth={2} />}
          </button>
          <button
            type="button"
            className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]"
            title={t("action.removeMarket")}
            aria-label={t("market.removeAria", { name: marketplace.display_name ?? marketplace.name })}
            disabled={Boolean(removingMarket)}
            onClick={() => void removeMarketplace(marketplace)}
          >
            {removingMarket === marketplace.name ? <LoadingSpinner /> : <TrashIcon />}
          </button>
        </>
      ) : null}
    </PluginMarketplaceRow>
  );

  return (
    <div className="space-y-4">
      {loadError ? <p className="muted text-sm">{loadError}</p> : null}
      {thirdPartyProfile ? (
        <div className="apple-edit-surface">
          <div className="apple-panel-section">
            <p className="muted text-sm">{t("marketDetail.thirdPartyNotice")}</p>
          </div>
        </div>
      ) : null}
      <div className="flex flex-wrap items-center justify-between gap-3">
        {loaded ? <PluginFilterChips value={filter} onChange={setFilter} counts={counts} /> : null}
        <div className="ml-auto flex items-center gap-2">
          <button type="button" className="apple-action-button" disabled={checking || upgradingAll} onClick={() => void (updates.length ? upgradeAll() : checkUpdates())}>
            <RefreshCw className={`h-4 w-4 ${checking || upgradingAll ? "animate-spin" : ""}`} strokeWidth={2} />
            {updates.length ? t("action.upgradeAll") : t("action.checkUpdates")}
          </button>
          <button type="button" className="apple-action-button" onClick={() => setShowSources(true)}>
            <Store className="h-4 w-4" strokeWidth={2} />
            {t("sources.title")}
          </button>
        </div>
      </div>
      {!loaded ? (
        <EmptyStateCard loading icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
          <p className="muted">{t("list.loading")}</p>
        </EmptyStateCard>
      ) : (
        <>
          {showUpdates && visibleUpdates.length ? (
            <section className="plugin-section">
              <div className="field-label plugin-section-head">
                {t("board.sectionUpdates")}
                <span className="apple-chip">{visibleUpdates.length}</span>
              </div>
              <div className="apple-group apple-list-card">
                {visibleUpdates.map((update) => {
                  const plugin = plugins.find((item) => item.name === update.name);
                  return (
                    <div key={`${update.marketplace}/${update.name}`} className="apple-list-row">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate font-semibold">{plugin?.display_name ?? update.name}</span>
                          {plugin?.version ? <span className="plan-badge shrink-0">v{plugin.version}</span> : null}
                        </div>
                        {plugin?.description ? <div className="muted meta-xs truncate">{plugin.description}</div> : null}
                      </div>
                      <button type="button" className="apple-action-button shrink-0" onClick={() => {
                        void upgrade(update)
                          .then(() => refresh(true))
                          .catch((error) => feedback.error(String(error)));
                      }}>
                        <ArrowUp className="h-4 w-4" strokeWidth={2} />
                        {t("board.upgradeTo", { version: update.version })}
                      </button>
                    </div>
                  );
                })}
              </div>
            </section>
          ) : null}
          {showInstalled && visibleInstalled.length ? (
            <section className="plugin-section">
              <div className="field-label plugin-section-head">
                {t("board.sectionInstalled")}
                <span className="apple-chip">{visibleInstalled.length}</span>
                <AppTooltip label={t("board.sectionInstalled")}>
                  <p>{t("board.codexInstalledHelp")}</p>
                </AppTooltip>
              </div>
              <div className="plugin-board">
                {visibleInstalled.map((plugin) => (
                  <PluginCard
                    key={plugin.name}
                    name={plugin.display_name ?? plugin.name}
                    source={plugin.marketplace ? `@${plugin.marketplace}` : undefined}
                    description={plugin.description}
                    iconPath={plugin.icon_path}
                    status={plugin.enabled ? null : (
                      <span className="plugin-status-dot" role="img" aria-label={t("detail.disabled")} title={t("detail.disabled")} />
                    )}
                    onOpen={() => setSelectedPlugin(plugin)}
                    action={removableOrigins.includes(plugin.origin) ? (
                      <button
                        type="button"
                        className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]"
                        title={t("action.uninstall")}
                        aria-label={t("list.uninstallAria", { name: plugin.name })}
                        onClick={() => void remove(plugin)}
                      >
                        <TrashIcon />
                      </button>
                    ) : null}
                    meta={<>
                      {plugin.version ? <span className="plan-badge max-w-24 truncate" title={plugin.version}>v{plugin.version}</span> : null}
                      {originChip(plugin)}
                      <ContainsChips items={plugin.contains} />
                    </>}
                  />
                ))}
              </div>
            </section>
          ) : null}
          {showAvailable && available.length ? (
            <section className="plugin-section">
              <div className="field-label plugin-section-head">
                {t("board.sectionAvailable")}
                <span className="apple-chip">{available.length}</span>
                <AppTooltip label={t("board.sectionAvailable")}>
                  <p>{t("board.codexAvailableHelp")}</p>
                </AppTooltip>
              </div>
              <div className="plugin-board">
                {available.map((plugin) => (
                  <PluginCard
                    key={plugin.plugin_id}
                    name={plugin.display_name ?? plugin.name}
                    source={`@${plugin.marketplaceName}`}
                    description={plugin.description}
                    iconPath={plugin.icon_path}
                    action={
                      <button type="button" className="apple-icon-button shrink-0 text-accent hover:bg-(--accent)/10" disabled={Boolean(installing)} title={t("action.install")} aria-label={t("action.install")} onClick={() => void install(plugin)}>
                        {installing === plugin.plugin_id ? <LoadingSpinner /> : <Download className="h-4 w-4" strokeWidth={2} />}
                      </button>
                    }
                    meta={<ContainsChips items={plugin.contains} />}
                  />
                ))}
              </div>
            </section>
          ) : null}
          {boardEmpty ? (
            query.trim()
              ? <p className="muted text-sm">{t("list.noMatch")}</p>
              : <EmptyStateCard icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
                  <p className="muted">{t("list.empty")}</p>
                </EmptyStateCard>
          ) : !query.trim() ? (
            <>
              {filter === "updates" && !visibleUpdates.length ? <p className="muted text-sm">{t("market.upToDate")}</p> : null}
              {filter === "installed" && !visibleInstalled.length ? <p className="muted text-sm">{t("list.empty")}</p> : null}
              {filter === "available" && !available.length ? <p className="muted text-sm">{t("claudePlugin.noMarkets")}</p> : null}
            </>
          ) : null}
        </>
      )}

      <AppDialog
        open={showSources}
        onOpenChange={setShowSources}
        title={t("sources.title")}
        description={t("sources.description")}
      >
        <PluginMarketplaceSources target="codex" marketplaces={marketplaces} loaded={marketsLoaded} busy={Boolean(removingMarket || refreshingMarket || installing)} onAdd={addMarketplace}>
          {marketplaces.map(renderSourceRow)}
        </PluginMarketplaceSources>
      </AppDialog>
    </div>
  );
}
