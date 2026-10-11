import { ChevronLeft, ChevronRight, Plus, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedMarketplacePlugins, getCachedPluginMarketplaces, loadPluginMarketplaces, refreshMarketplacePlugins } from "../../app/managementDataCache";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { TrashIcon } from "../../components/TrashIcon";
import AddPluginView from "./AddPluginView";
import MarketplaceDetailView from "./MarketplaceDetailView";
import SourceLink from "./components/SourceLink";
import { findConfiguredMarketplace, marketplaceKindLabels, recommendedMarketplaces, splitMarketplaceCards } from "./pluginMeta";
import type { PluginMarketplace, PluginUpdate } from "../../types";

export default function PluginMarketplaceView({
  onBack,
  onInstalled,
  installedNames,
  thirdPartyProfile,
}: {
  onBack: () => void;
  onInstalled: () => Promise<void>;
  installedNames: ReadonlySet<string>;
  thirdPartyProfile: boolean;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  // 与 MCP / Skill / 插件列表同款首帧直出：缓存命中时第一帧就是内容，不画一帧转圈
  const cachedMarketplaces = getCachedPluginMarketplaces();
  const [marketplaces, setMarketplaces] = useState<PluginMarketplace[]>(cachedMarketplaces ?? []);
  const [marketplacesLoaded, setMarketplacesLoaded] = useState(cachedMarketplaces !== null);
  const [marketplacesError, setMarketplacesError] = useState("");
  const [marketplacePluginCounts, setMarketplacePluginCounts] = useState<Record<string, number | null>>({});
  const [adding, setAdding] = useState("");
  const [removing, setRemoving] = useState("");
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [upgradingAll, setUpgradingAll] = useState(false);
  const [updates, setUpdates] = useState<PluginUpdate[]>([]);
  const [selectedMarketplace, setSelectedMarketplace] = useState<PluginMarketplace | null>(null);
  const [showAddPlugin, setShowAddPlugin] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const scrollTop = useRef(0);

  const refreshMarketplaces = async (force = false) => {
    try {
      setMarketplaces(await loadPluginMarketplaces(force));
      setMarketplacesError("");
    } catch (error) {
      setMarketplacesError(String(error));
    } finally {
      setMarketplacesLoaded(true);
    }
  };

  useEffect(() => {
    // 缓存已在 useState 里同步吃掉；这里只负责刷新回填——有缓存强制静默刷新，无缓存正常加载。
    void refreshMarketplaces(Boolean(cachedMarketplaces));
  }, []);

  useEffect(() => {
    // 缓存直出数量，随后静默刷新回填；刷新失败时保留缓存值，不再整组重置转圈。
    setMarketplacePluginCounts(Object.fromEntries(
      marketplaces.map((marketplace) => {
        const cached = getCachedMarketplacePlugins(marketplace.name);
        return [marketplace.name, cached ? cached.length : null] as const;
      }),
    ));
    let cancelled = false;
    void Promise.all(marketplaces.map(async (marketplace) => {
      try {
        const plugins = await refreshMarketplacePlugins(marketplace.name, marketplace.root);
        return [marketplace.name, plugins.length] as const;
      } catch {
        return null;
      }
    })).then((entries) => {
      if (cancelled) return;
      setMarketplacePluginCounts((prev) => {
        const next = { ...prev };
        for (const entry of entries) {
          if (entry) next[entry[0]] = entry[1];
        }
        return next;
      });
    });
    return () => { cancelled = true; };
  }, [marketplaces]);

  useEffect(() => {
    if (!selectedMarketplace && contentRef.current) contentRef.current.scrollTop = scrollTop.current;
  }, [selectedMarketplace]);

  const openMarketplace = (marketplace: PluginMarketplace) => {
    scrollTop.current = contentRef.current?.scrollTop ?? 0;
    setSelectedMarketplace(marketplace);
  };

  const browseRecommended = async (recommended: typeof recommendedMarketplaces[number]) => {
    const existing = findConfiguredMarketplace(recommended, marketplaces);
    if (existing) {
      openMarketplace(existing);
      return;
    }
    if (adding) return;
    setAdding(recommended.name);
    try {
      const marketplace = await api.addPluginMarketplace(recommended.source);
      feedback.success(t("market.entered", { name: marketplace.name }));
      await refreshMarketplaces(true);
      openMarketplace(marketplace);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setAdding("");
    }
  };

  const removeMarketplace = async (marketplace: PluginMarketplace) => {
    if (removing || marketplace.kind === "official") return;
    const confirmed = await feedback.confirm({
      title: t("market.removeConfirmTitle"),
      description: t("market.removeConfirm", { name: marketplace.name }),
      confirmText: t("action.removeMarket"),
      destructive: true,
    });
    if (!confirmed) return;
    setRemoving(marketplace.name);
    try {
      await api.removePluginMarketplace(marketplace.name);
      feedback.success(t("market.removedToast", { name: marketplace.name }));
      await refreshMarketplaces(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setRemoving("");
    }
  };

  const checkUpdates = async () => {
    if (checkingUpdates || upgradingAll) return;
    setCheckingUpdates(true);
    try {
      const items = await api.checkPluginUpdates();
      setUpdates(items);
      feedback.success(items.length ? t("market.foundUpdates", { count: items.length }) : t("market.upToDate"));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setCheckingUpdates(false);
    }
  };

  const upgrade = async (update: PluginUpdate) => {
    await api.upgradeMarketplacePlugin(update.marketplace, update.name);
    setUpdates((items) => items.filter((item) => item.name !== update.name || item.marketplace !== update.marketplace));
  };

  const upgradeAll = async () => {
    if (!updates.length || upgradingAll || checkingUpdates) return;
    setUpgradingAll(true);
    try {
      for (const update of updates) await upgrade(update);
      feedback.success(t("market.allUpgraded"));
      await onInstalled();
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setUpgradingAll(false);
    }
  };

  const { official, thirdParty } = splitMarketplaceCards(marketplaces);

  const renderMarketplaceRow = (marketplace: PluginMarketplace) => {
    const pluginCount = marketplacePluginCounts[marketplace.name];
    return (
      <div key={marketplace.name} className="apple-list-row">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-semibold">{marketplace.display_name ?? marketplace.name}</span>
            {pluginCount === undefined ? (
              <span role="status" aria-label={t("marketDetail.loadingAria")}><LoadingSpinner /></span>
            ) : pluginCount === null ? (
              <span className="apple-chip" aria-label={t("market.pluginCountUnavailable")}>—</span>
            ) : (
              <span className="apple-chip" aria-label={t("marketDetail.browsableAria", { count: pluginCount })}>
                {t("market.pluginCount", { count: pluginCount })}
              </span>
            )}
          </div>
          <div className="mono muted meta-xs mt-1 break-all">{marketplace.root}</div>
        </div>
        {/* 行内动作与 Skill/MCP 列表同构：图标按钮放右侧动作区，不进标题行（32px 图标会把行撑高 10px） */}
        <div className="flex shrink-0 items-center gap-2">
          {marketplace.kind === "third-party" ? (
            <button
              type="button"
              className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]"
              title={t("action.removeMarket")}
              aria-label={t("market.removeAria", { name: marketplace.display_name ?? marketplace.name })}
              disabled={Boolean(removing)}
              onClick={() => void removeMarketplace(marketplace)}
            >
              {removing === marketplace.name ? <LoadingSpinner /> : <TrashIcon />}
            </button>
          ) : null}
          <button type="button" className="apple-action-button" onClick={() => openMarketplace(marketplace)}>
            <ChevronRight className="h-4 w-4" strokeWidth={2} />
            {t("action.browse")}
          </button>
        </div>
      </div>
    );
  };

  // 未配置的推荐条目：卡内兜底安装行，安装成功后按已配置市场归位
  const renderPendingRow = (recommended: (typeof recommendedMarketplaces)[number]) => (
    <div key={recommended.name} className="apple-list-row">
      <div className="min-w-0 flex-1">
        <span className="font-semibold">{recommended.displayName}</span>
        <div className="muted mt-0.5 break-words text-sm">{t(recommended.descriptionKey)}</div>
        <div className="mt-0.5 flex flex-wrap items-center gap-2">
          <span className="mono muted meta-xs break-all">{recommended.source}</span>
          <SourceLink source={recommended.source} />
        </div>
      </div>
      <button type="button" className="apple-action-button app-button--primary shrink-0" disabled={Boolean(adding) || !marketplacesLoaded} onClick={() => void browseRecommended(recommended)}>
        {adding === recommended.name ? <LoadingSpinner /> : <Plus className="h-4 w-4" strokeWidth={2} />}
        {t("action.install")}
      </button>
    </div>
  );

  // 与一级列表同构：分类标题在卡外，卡片本体 = apple-group apple-list-card，行直接进卡
  const renderMarketplaceCard = (kind: PluginMarketplace["kind"]) => {
    const card = kind === "official" ? official : thirdParty;
    return (
      <>
        <div className="flex items-center gap-2">
          <div className="field-label">{t(marketplaceKindLabels[kind])}</div>
          {marketplacesLoaded ? (
            <span className="apple-chip" aria-label={t("market.cardCountAria", { count: card.marketplaces.length })}>{card.marketplaces.length}</span>
          ) : <LoadingSpinner />}
        </div>
        <div className="apple-group apple-list-card">
          {card.marketplaces.map(renderMarketplaceRow)}
          {card.pending.map(renderPendingRow)}
        </div>
      </>
    );
  };

  if (selectedMarketplace) {
    return (
      <MarketplaceDetailView
        marketplace={selectedMarketplace}
        onBack={() => setSelectedMarketplace(null)}
        onInstalled={onInstalled}
        installedNames={installedNames}
        thirdPartyProfile={thirdPartyProfile}
      />
    );
  }
  if (showAddPlugin) {
    return <AddPluginView onBack={() => setShowAddPlugin(false)} onMarketplaceAdded={async (marketplace) => { await refreshMarketplaces(); setShowAddPlugin(false); openMarketplace(marketplace); }} onInstalled={onInstalled} />;
  }

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("nav.backToPlugins")} onClick={onBack}>
          <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />
          <span className="apple-title">{t("marketplace")}</span>
        </button>
        <div className="ml-auto flex items-center gap-2">
          <button type="button" className="apple-action-button" disabled={checkingUpdates || upgradingAll} onClick={() => void (updates.length ? upgradeAll() : checkUpdates())}>
            <RefreshCw className={`h-4 w-4 ${checkingUpdates || upgradingAll ? "animate-spin" : ""}`} strokeWidth={2} />
            {updates.length ? t("action.upgradeAll") : t("action.checkUpdates")}
          </button>
          <button type="button" className="apple-action-button app-button--primary" onClick={() => setShowAddPlugin(true)}>
            <Plus className="h-4 w-4" strokeWidth={2} />
            {t("add.title")}
          </button>
        </div>
      </div>
      <div ref={contentRef} className="apple-edit-content">
        <div className="space-y-4">
          {marketplacesError ? <p className="muted text-sm">{marketplacesError}</p> : null}
          {renderMarketplaceCard("official")}
          {renderMarketplaceCard("third-party")}
        </div>
      </div>
    </section>
  );
}
