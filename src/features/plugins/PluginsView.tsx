import { Blocks, Store } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedPlugins, loadPlugins } from "../../app/managementDataCache";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { PlanBadge } from "../../components/PlanBadge";
import { TrashIcon } from "../../components/TrashIcon";
import PluginDetailView from "./PluginDetailView";
import PluginMarketplaceView from "./PluginMarketplaceView";
import ContainsChips from "./components/ContainsChips";
import PluginSearchInput from "./components/PluginSearchInput";
import SourceLink from "./components/SourceLink";
import { comparePlugins, matchesQuery, originLabels, removableOrigins } from "./pluginMeta";
import type { AppState, PluginSummary } from "../../types";

export default function PluginsView({ state, popEpoch }: { state: AppState; popEpoch: number }) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  // 缓存命中即首帧直出，不等微任务回填；未命中时保持加载态走正常请求
  const [plugins, setPlugins] = useState<PluginSummary[]>(() => getCachedPlugins() ?? []);
  const [loaded, setLoaded] = useState(() => getCachedPlugins() !== null);
  const [loadError, setLoadError] = useState("");
  const [selectedPlugin, setSelectedPlugin] = useState<PluginSummary | null>(null);
  const [addingMarketplace, setAddingMarketplace] = useState(false);
  const [query, setQuery] = useState("");
  const thirdPartyProfile =
    state.codex_profiles.find((profile) => profile.id === state.active_codex_profile_id)?.kind === "third_party";

  const refresh = async (force = false) => {
    try {
      const items = await loadPlugins(force);
      setPlugins(items);
      setLoadError("");
    } catch (error) {
      setLoadError(String(error));
    } finally {
      setLoaded(true);
    }
  };
  useEffect(() => {
    // 先显缓存（命中即回），后台再强制取一次最新，保证外部变更能浮上来；
    // 无缓存时第二次调用与第一次共享同一个在途请求，不会重复加载。
    void refresh();
    void refresh(true);
  }, []);

  // 侧栏重复点击本视图 = 弹回一级；只清子页状态，不刷新（安装/卸载流程自带刷新）。
  // addingMarketplace 关闭即卸载市场页，其内部的添加子页随挂载消失。
  useEffect(() => {
    if (!popEpoch) return;
    setSelectedPlugin(null);
    setAddingMarketplace(false);
  }, [popEpoch]);

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
      feedback.success(t("toast.uninstalled", { name: plugin.display_name ?? plugin.name }));
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    }
  };

  const orderedPlugins = [...plugins].sort(comparePlugins);
  const visiblePlugins = orderedPlugins.filter((plugin) => matchesQuery(plugin, query));

  if (selectedPlugin) {
    return <PluginDetailView plugin={selectedPlugin} onBack={() => setSelectedPlugin(null)} />;
  }
  if (addingMarketplace) {
    return (
      <PluginMarketplaceView
        onBack={() => setAddingMarketplace(false)}
        onInstalled={() => refresh(true)}
        installedNames={new Set(plugins.map((plugin) => plugin.name))}
        thirdPartyProfile={thirdPartyProfile}
      />
    );
  }

  return (
    <section className="apple-scroll-page mx-auto w-full max-w-none">
      <header className="apple-page-bar flex-wrap justify-between gap-4">
        <div className="flex items-center gap-3">
          <ManagementPageTitle
            icon={<span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><Blocks className="h-[18px] w-[18px]" strokeWidth={2} /></span>}
            title={t("title")}
            count={loaded ? plugins.length : undefined}
            countLabel={loaded ? t("marketDetail.installedCountAria", { count: plugins.length }) : undefined}
            loadingLabel={t("marketDetail.loadingAria")}
          />
          <PlanBadge plan="Codex" />
        </div>
        <div className="flex items-center gap-2">
          <PluginSearchInput value={query} onChange={setQuery} />
          <button type="button" className="apple-action-button app-button--primary" onClick={() => setAddingMarketplace(true)}>
            <Store className="h-4 w-4" strokeWidth={2} />
            {t("marketplace")}
            <span className="rounded-full bg-white/95 px-1.5 py-px font-bold tracking-wide text-accent meta-xs">Beta</span>
          </button>
        </div>
      </header>
      <div className="apple-edit-content">
        {loadError ? <p className="muted mt-4 text-sm">{loadError}</p> : null}
        {!loaded ? (
          <EmptyStateCard loading icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("list.loading")}</p>
          </EmptyStateCard>
        ) : plugins.length === 0 ? (
          <EmptyStateCard icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("list.empty")}</p>
          </EmptyStateCard>
        ) : plugins.length ? (
          <div className="apple-group apple-list-card">
            {visiblePlugins.map((plugin) => (
              <div key={plugin.name} className="apple-list-row">
                <div
                  className="group min-w-0 flex-1 cursor-pointer text-left"
                  role="button"
                  tabIndex={0}
                  aria-label={t("list.viewDetailAria", { name: plugin.display_name ?? plugin.name })}
                  title={t("list.viewDetailTitle")}
                  onClick={() => setSelectedPlugin(plugin)}
                  onKeyDown={(event) => {
                    if (event.target !== event.currentTarget) return;
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedPlugin(plugin);
                    }
                  }}
                >
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 truncate font-semibold transition-colors group-hover:text-accent">{plugin.display_name ?? plugin.name}</span>
                    {plugin.version ? (
                      <span className="shrink-0 rounded-md bg-black/5 px-1.5 py-px font-medium tracking-wide muted meta-xs dark:bg-white/10">v{plugin.version}</span>
                    ) : null}
                    <ContainsChips items={plugin.contains} />
                    {plugin.enabled ? null : <span className="apple-chip chip-warn shrink-0">{t("detail.disabled")}</span>}
                    {originLabels[plugin.origin] ? (
                      <span className={`shrink-0 rounded-md px-1.5 py-px font-medium tracking-wide meta-xs ${plugin.origin === "official" ? "origin-chip--official" : "origin-chip--third-party"}`}>{t(originLabels[plugin.origin]!)}</span>
                    ) : null}
                    {plugin.source_url ? <SourceLink source={plugin.source_url} /> : null}
                  </div>
                  <div className="muted meta-xs truncate">
                    {plugin.description ?? plugin.name}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  {removableOrigins.includes(plugin.origin) ? (
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
                </div>
              </div>
            ))}
            {visiblePlugins.length ? null : <p className="muted mt-4 text-sm">{t("list.noMatch")}</p>}
          </div>
        ) : null}
      </div>
    </section>
  );
}
