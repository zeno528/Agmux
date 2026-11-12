import { AppWindow, ArrowUp, Blocks, Download, RefreshCw, Store } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import {
  getCachedPluginCatalog,
  getCachedPluginMarketplaces,
  getCachedPlugins,
  getCachedCodexApps,
  loadCodexApps,
  loadPluginMarketplaces,
  loadPlugins,
  loadPluginCatalog,
  setCachedPluginCatalog,
  setCachedPlugins,
} from "../../app/managementDataCache";
import { AppDialog } from "../../components/AppDialog";
import { AppTooltip } from "../../components/AppTooltip";
import { AppSwitch } from "../../components/AppSwitch";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { TrashIcon } from "../../components/TrashIcon";
import PluginDetailView from "./PluginDetailView";
import PluginFilterChips, { type PluginFilter } from "./components/PluginFilterChips";
import { PluginCatalogPagination, usePluginCatalogPage } from "./components/PluginCatalogPagination";
import { PluginCardSkeletons } from "./components/PluginCardSkeletons";
import { PluginCard } from "./components/PluginCard";
import { PluginStatusDot } from "./components/PluginStatusDot";
import PluginMarketplaceSources, { PluginMarketplaceRow } from "./components/PluginMarketplaceSources";
import {
  compareMarketplacePlugins,
  comparePlugins,
  matchesQuery,
  originLabels,
  resolveAliasInstalled,
  resolveAppIcon,
} from "./pluginMeta";
import type { PluginHeaderState } from "./PluginsView";
import type { AppState, MarketplacePlugin, PluginMarketplace, PluginSummary, PluginUpdate } from "../../types";

type AvailablePlugin = MarketplacePlugin & { marketplaceName: string };

export default function CodexPluginsView({
  state,
  activationEpoch,
  popEpoch,
  query,
  onHeaderChange,
}: {
  state: AppState;
  activationEpoch: number;
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
  const [section, setSection] = useState<"plugins" | "apps">("plugins");
  const [apps, setApps] = useState(() => getCachedCodexApps() ?? []);
  const [appsLoaded, setAppsLoaded] = useState(() => getCachedCodexApps() !== null);
  const [appsError, setAppsError] = useState("");
  const [marketplaces, setMarketplaces] = useState<PluginMarketplace[]>(() => getCachedPluginMarketplaces() ?? []);
  const [marketsLoaded, setMarketsLoaded] = useState(() => getCachedPluginMarketplaces() !== null);
  const [catalog, setCatalog] = useState(() => getCachedPluginCatalog() ?? []);
  const [catalogLoaded, setCatalogLoaded] = useState(() => getCachedPluginCatalog() !== null);
  const [catalogError, setCatalogError] = useState("");
  const [catalogRefreshed, setCatalogRefreshed] = useState(false);
  const [updates, setUpdates] = useState<PluginUpdate[]>([]);
  const [checking, setChecking] = useState(false);
  const [upgradingAll, setUpgradingAll] = useState(false);
  const [selectedPlugin, setSelectedPlugin] = useState<PluginSummary | null>(null);
  const [showSources, setShowSources] = useState(false);
  const [filter, setFilter] = useState<PluginFilter>("all");
  const showAvailable = filter === "all" || filter === "available";
  const [pluginAction, setPluginAction] = useState<{ id: string; kind: "install" | "remove" | "toggle" } | null>(null);
  const installing = pluginAction?.id ?? "";
  const [removingMarket, setRemovingMarket] = useState("");
  const [refreshingMarket, setRefreshingMarket] = useState("");
  const thirdPartyProfile =
    state.codex_profiles.find((profile) => profile.id === state.active_codex_profile_id)?.kind === "third_party";

  const refreshCatalog = async (changed?: { id: string; installed: boolean }) => {
    try {
      const items = await loadPluginCatalog(true);
      const next = changed ? items.map((item) => item.plugin_id === changed.id ? { ...item, installed: changed.installed } : item) : items;
      if (changed) setCachedPluginCatalog(next);
      setCatalog(next);
      setCatalogRefreshed(true);
      setCatalogError("");
    } catch (error) {
      setCatalogError(String(error));
      throw error;
    } finally {
      setCatalogLoaded(true);
    }
  };

  const refresh = async (force = false) => {
    try {
      const items = await loadPlugins(force);
      setPlugins(items);
      setSelectedPlugin((previous) => previous
        ? items.find((item) => item.desktop_visible && item.name === previous.name && item.marketplace === previous.marketplace) ?? null
        : null);
      setLoadError("");
    } catch (error) {
      setLoadError(String(error));
    } finally {
      setLoaded(true);
    }
  };
  useEffect(() => {
    // 先显缓存（命中即回），后台再强制取一次最新，保证外部变更能浮上来
    setCatalogRefreshed(false);
    void refresh();
    void refresh(true);
  }, [activationEpoch]);
  useEffect(() => {
    if (loaded && showAvailable && !catalogRefreshed) void refreshCatalog().catch(() => undefined);
  }, [loaded, showAvailable, catalogRefreshed]);
  useEffect(() => {
    let cancelled = false;
    void loadCodexApps(true).then((items) => {
      if (!cancelled) { setApps(items); setAppsError(""); }
    }).catch((error: unknown) => {
      if (!cancelled) setAppsError(String(error));
    }).finally(() => {
      if (!cancelled) setAppsLoaded(true);
    });
    return () => { cancelled = true; };
  }, [activationEpoch]);
  useEffect(() => {
    void refreshMarketplaces(true);
  }, []);

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

  const chip = (label: string, official: boolean) => (
    <span className={`shrink-0 rounded-md px-1.5 py-px font-medium tracking-wide meta-xs ${official ? "origin-chip--official" : "origin-chip--third-party"}`}>
      {label}
    </span>
  );

  const originChip = (plugin: PluginSummary) => {
    const label = originLabels[plugin.origin];
    return label ? chip(t(label), plugin.origin === "official") : null;
  };

  const installedIds = useMemo(() => new Set(plugins.map((plugin) => `${plugin.name}@${plugin.marketplace}`)), [plugins]);
  const orderedPlugins = useMemo(() => plugins.filter((plugin) => plugin.desktop_visible).sort(comparePlugins), [plugins]);
  const visibleInstalled = orderedPlugins.filter((plugin) => matchesQuery(plugin, query));

  // 使用同一份远程目录；安装结果优先于目录的旧快照，搜索覆盖未渲染的条目。
  // 宽搜索（名+显示名+描述）与 Claude 可安装区同一模式：拼串复用 matchesQuery。
  const available = useMemo<AvailablePlugin[]>(() => {
    return resolveAliasInstalled(catalog, installedIds).filter((plugin) => !plugin.installed
      && matchesQuery({ name: `${plugin.name} ${plugin.display_name ?? ""} ${plugin.description ?? ""}` }, query))
      .map((plugin) => ({ ...plugin, marketplaceName: plugin.plugin_id.slice(plugin.plugin_id.lastIndexOf("@") + 1) }))
      .sort(compareMarketplacePlugins);
  }, [catalog, installedIds, query]);

  const catalogPage = usePluginCatalogPage(available.length, query);
  const catalogLoading = showAvailable && !catalogRefreshed && !catalogError;

  const visibleUpdates = useMemo(
    () => updates.filter((update) => visibleInstalled.some((plugin) => plugin.name === update.name)),
    [updates, visibleInstalled],
  );

  const install = async (plugin: AvailablePlugin) => {
    if (installing || checking || upgradingAll) return;
    setPluginAction({ id: plugin.plugin_id, kind: "install" });
    try {
      const items = await api.installMarketplacePlugin(plugin.marketplaceName, plugin.name);
      setCachedPlugins(items);
      setPlugins(items);
      void refreshCatalog({ id: plugin.plugin_id, installed: true }).catch(() => undefined);
      // ON_INSTALL 插件的 OAuth 弹窗是桌面端 app-server 私有能力，CLI 无法触发；
      // 安装成功后明确引导用户去桌面端完成授权，避免「装了但不可用」的静默缺失。
      if (plugin.auth_policy === "ON_INSTALL") {
        feedback.warning(t("toast.installedNeedsAuth", { name: plugin.name }));
      } else {
        feedback.success(t("toast.installed", { name: plugin.name }));
      }
    } catch (reason) {
      feedback.error(String(reason));
    } finally {
      setPluginAction(null);
    }
  };

  const remove = async (plugin: PluginSummary) => {
    if (installing || checking || upgradingAll || !plugin.can_uninstall || !plugin.marketplace) return;
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
    setPluginAction({ id: `${plugin.name}@${plugin.marketplace}`, kind: "remove" });
    try {
      const items = await api.uninstallPlugin(plugin.name, plugin.marketplace);
      setCachedPlugins(items);
      setPlugins(items);
      void refreshCatalog({ id: `${plugin.name}@${plugin.marketplace}`, installed: false }).catch(() => undefined);
      feedback.success(t("toast.uninstalled", { name: plugin.display_name ?? plugin.name }));
    } catch (error) {
      feedback.error(String(error));
      await refresh(true);
    } finally {
      setPluginAction(null);
    }
  };

  const checkUpdates = async () => {
    if (checking || upgradingAll || installing) return;
    setChecking(true);
    try {
      const items = await api.checkPluginUpdates();
      setUpdates(items);
      feedback.success(items.length ? t("market.foundUpdates", { client: "Codex", count: items.length }) : t("market.upToDateToast", { client: "Codex" }));
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
    if (!updates.length || upgradingAll || checking || installing) return;
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
      void refreshCatalog().catch(() => undefined);
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
    void refreshCatalog().catch(() => undefined);
    return marketplace;
  };

  const toggleEnabled = async (plugin: PluginSummary, enabled: boolean) => {
    if (installing || checking || upgradingAll || !plugin.can_toggle || !plugin.marketplace) return;
    const pluginId = `${plugin.name}@${plugin.marketplace}`;
    setPluginAction({ id: pluginId, kind: "toggle" });
    try {
      const items = await api.setPluginEnabled(pluginId, enabled);
      setCachedPlugins(items);
      setPlugins(items);
    } catch (error) {
      feedback.error(String(error));
      await refresh(true);
    } finally {
      setPluginAction(null);
    }
  };

  const updateMarketplace = async (marketplace: PluginMarketplace) => {
    if (refreshingMarket || removingMarket || installing) return;
    setRefreshingMarket(marketplace.name);
    try {
      await api.refreshPluginMarketplace(marketplace.name);
      await refreshCatalog();
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
  const boardEmpty = loaded && catalogLoaded && !catalogLoading && !catalogError && !visibleInstalled.length && !available.length && !visibleUpdates.length;

  const renderSourceRow = (marketplace: PluginMarketplace) => (
    <PluginMarketplaceRow
      key={marketplace.name}
      name={marketplace.display_name ?? marketplace.name}
      source={marketplace.source_url ?? marketplace.root}
      icon={chip(t(marketplace.kind === "official" ? "origin.official" : "origin.thirdParty"), marketplace.kind === "official")}
    >
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

  const appsCount = !appsLoaded ? <LoadingSpinner /> : appsError ? "—" : apps.length;

  if (section === "apps") {
    const needle = query.trim().toLowerCase();
    const visible = apps.filter((app) => `${app.name} ${app.description ?? ""}`.toLowerCase().includes(needle));
    return <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="field-label flex items-center gap-2">{t("inventory.apps")} <span className="apple-chip">{appsCount}</span></h2>
        <button type="button" className="apple-action-button apple-action-button--small" onClick={() => setSection("plugins")}>
          {t("nav.backToPlugins")}
        </button>
      </div>
      {appsError ? <p className="muted text-sm">{appsError}</p> : null}
      {!appsLoaded ? (
        <div className="plugin-board" aria-busy="true">
          <span className="sr-only" role="status">{t("inventory.loadingApps")}</span>
          <PluginCardSkeletons />
        </div>
      ) : (
        <div className="plugin-board">
          {visible.map((app) => <PluginCard key={app.id} name={app.name} description={app.description}
            iconPath={resolveAppIcon(app, plugins)}
            action={<div className="flex shrink-0 items-center gap-2">
              <PluginStatusDot enabled={app.enabled} />
              <span className="muted meta-xs">{t("action.managed")}</span>
            </div>}
          />)}
        </div>
      )}
      {appsLoaded && !appsError && !visible.length ? <p className="muted text-sm">{t("inventory.noApps")}</p> : null}
    </div>;
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <PluginFilterChips value={filter} onChange={setFilter} updateCount={visibleUpdates.length} />
        <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
          <button type="button" className="apple-action-button apple-action-button--small" onClick={() => setSection("apps")}>
            <AppWindow className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
            {t("inventory.apps")} <span className="muted meta-xs">{appsCount}</span>
          </button>
          <button type="button" className="apple-action-button apple-action-button--small" onClick={() => setShowSources(true)}>
            <Store className="h-4 w-4" strokeWidth={2} />
            {t("sources.title")}
          </button>
          <button type="button" className="apple-action-button apple-action-button--small" disabled={checking || upgradingAll || (Boolean(installing) && pluginAction?.kind !== "toggle")} aria-disabled={pluginAction?.kind === "toggle" || undefined} onClick={() => void (updates.length ? upgradeAll() : checkUpdates())}>
            <RefreshCw className={`h-4 w-4 ${checking || upgradingAll ? "animate-spin" : ""}`} strokeWidth={2} />
            {updates.length ? t("action.upgradeAll") : t("action.checkUpdates")}
          </button>
        </div>
      </div>
      {loadError ? <p className="muted text-sm">{loadError}</p> : null}
      {catalogError && catalogError !== loadError ? <p className="muted text-sm">{catalogError}</p> : null}
      {thirdPartyProfile ? (
        <div className="apple-edit-surface">
          <div className="apple-panel-section">
            <p className="muted text-sm">{t("marketDetail.thirdPartyNotice")}</p>
          </div>
        </div>
      ) : null}
      {!loaded && showInstalled ? (
        <section className="plugin-section" aria-busy="true">
          <div className="field-label plugin-section-head">{t("board.sectionInstalled")}</div>
          <div className="plugin-board"><PluginCardSkeletons installed /></div>
        </section>
      ) : null}
      {!loaded && !catalogLoading ? null : (
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
                <AppTooltip label={t("board.sectionInstalled")}>
                  <p>{t("board.codexInstalledHelp")}</p>
                </AppTooltip>
                <span className="apple-chip">{visibleInstalled.length}</span>
              </div>
              <div className="plugin-board">
                {visibleInstalled.map((plugin) => (
                  <PluginCard
                    key={`${plugin.name}@${plugin.marketplace}`}
                    name={plugin.display_name ?? plugin.name}
                    source={plugin.marketplace ? `@${plugin.marketplace}` : undefined}
                    description={plugin.description}
                    iconPath={plugin.icon_path}
                    onOpen={() => setSelectedPlugin(plugin)}
                    action={<div className="flex shrink-0 items-center gap-2">
                      {plugin.can_toggle ? <AppSwitch
                        size="sm"
                        checked={plugin.enabled}
                        disabled={Boolean(installing) || checking || upgradingAll}
                        label={t("action.toggleLabel", { name: plugin.display_name ?? plugin.name })}
                        onCheckedChange={(next) => void toggleEnabled(plugin, next)}
                      /> : <>
                        <PluginStatusDot enabled={plugin.enabled} />
                        <span className="muted meta-xs" title={t("action.managedHelp")}>{t("action.managed")}</span>
                      </>}
                    </div>}
                    foot={plugin.can_uninstall ? (
                      <button
                        type="button"
                        className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]"
                        title={t("action.uninstall")}
                        disabled={Boolean(installing) || checking || upgradingAll}
                        aria-label={t("list.uninstallAria", { name: plugin.name })}
                        onClick={() => void remove(plugin)}
                      >
                        {pluginAction?.kind === "remove" && installing === `${plugin.name}@${plugin.marketplace}` ? <LoadingSpinner /> : <TrashIcon />}
                      </button>
                    ) : null}
                    meta={<>
                      {plugin.version ? <span className="plan-badge max-w-24 truncate" title={plugin.version}>v{plugin.version}</span> : null}
                      {originChip(plugin)}
                    </>}
                  />
                ))}
              </div>
            </section>
          ) : null}
          {showAvailable && (catalogLoading || available.length > 0) ? (
            <section className="plugin-section">
              <div className="field-label plugin-section-head">
                {t("board.sectionAvailable")}
                <AppTooltip label={t("board.sectionAvailable")}>
                  <p>{t("board.codexAvailableHelp")}</p>
                </AppTooltip>
                {catalogLoading ? (
                  <span className="muted inline-flex items-center" role="status" title={t("board.loadingCatalog")}>
                    <LoadingSpinner />
                    <span className="sr-only">{t("board.loadingCatalog")}</span>
                  </span>
                ) : catalogRefreshed ? <span className="apple-chip">{available.length}</span> : null}
              </div>
              <div className="plugin-board" ref={catalogPage.setGrid} tabIndex={-1} aria-busy={catalogLoading}>
                {catalogLoading && !available.length ? <PluginCardSkeletons count={catalogPage.columns * 2} /> : null}
                {available.slice(catalogPage.start, catalogPage.end).map((plugin) => (
                  <PluginCard
                    key={plugin.plugin_id}
                    name={plugin.display_name ?? plugin.name}
                    source={`@${plugin.marketplaceName}`}
                    description={plugin.description}
                    iconPath={plugin.icon_path}
                    action={
                      <button type="button" className="apple-icon-button shrink-0 text-accent hover:bg-(--accent)/10" disabled={Boolean(installing) || checking || upgradingAll} title={t("action.install")} aria-label={t("action.install")} onClick={() => void install(plugin)}>
                        {pluginAction?.kind === "install" && installing === plugin.plugin_id ? <LoadingSpinner /> : <Download className="h-4 w-4" strokeWidth={2} />}
                      </button>
                    }
                  />
                ))}
              </div>
              <PluginCatalogPagination start={catalogPage.start} end={catalogPage.end} total={available.length} loading={catalogLoading} move={catalogPage.move} />
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
              {filter === "available" && catalogLoaded && !catalogLoading && !catalogError && !available.length ? <p className="muted text-sm">{t("list.noMatch")}</p> : null}
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
