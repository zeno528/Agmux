import { ChevronLeft, Download } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedMarketplacePlugins, refreshMarketplacePlugins, setCachedMarketplacePlugins } from "../../app/managementDataCache";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { TrashIcon } from "../../components/TrashIcon";
import ContainsChips from "./components/ContainsChips";
import PluginSearchInput from "./components/PluginSearchInput";
import SourceLink from "./components/SourceLink";
import { compareMarketplacePlugins, marketplaceKindLabels, matchesQuery, resolveAliasInstalled } from "./pluginMeta";
import type { MarketplacePlugin, PluginMarketplace } from "../../types";

export default function MarketplaceDetailView({
  marketplace,
  onBack,
  onInstalled,
  installedNames,
  thirdPartyProfile,
}: {
  marketplace: PluginMarketplace;
  onBack: () => void;
  onInstalled: () => Promise<void>;
  installedNames: ReadonlySet<string>;
  thirdPartyProfile: boolean;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  // 缓存直出到首帧：初始 state 直接取同步缓存，避免「未加载骨架屏先画一帧」的白条闪烁
  const [plugins, setPlugins] = useState<MarketplacePlugin[]>(() => getCachedMarketplacePlugins(marketplace.name) ?? []);
  const [loaded, setLoaded] = useState(() => getCachedMarketplacePlugins(marketplace.name) !== null);
  const [error, setError] = useState("");
  const [installing, setInstalling] = useState("");
  const [uninstalling, setUninstalling] = useState("");
  const [query, setQuery] = useState("");
  const resolvedPlugins = resolveAliasInstalled(plugins, installedNames);
  const installedPluginCount = resolvedPlugins.filter((plugin) => plugin.installed).length;
  const visiblePlugins = [...resolvedPlugins].sort(compareMarketplacePlugins).filter((plugin) => matchesQuery(plugin, query));

  useEffect(() => {
    let cancelled = false;
    setError("");
    // 缓存直出旧目录，静默刷新后整体替换；刷新失败且有缓存时保留旧数据、不报错。
    const cached = getCachedMarketplacePlugins(marketplace.name);
    setPlugins(cached ?? []);
    setLoaded(cached !== null);
    void refreshMarketplacePlugins(marketplace.name, marketplace.root)
      .then((items) => {
        if (!cancelled) {
          setPlugins(items);
          setLoaded(true);
        }
      })
      .catch((reason) => {
        if (!cancelled) {
          if (cached === null) setError(String(reason));
          setLoaded(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [marketplace.name]);

  /// 本地与全局缓存同步打补丁，避免下次进入时缓存先闪回旧状态。
  const applyPluginPatch = (pluginId: string, patch: Partial<MarketplacePlugin>) => {
    const next = plugins.map((item) => item.plugin_id === pluginId ? { ...item, ...patch } : item);
    setPlugins(next);
    setCachedMarketplacePlugins(marketplace.name, next);
  };

  const install = async (plugin: MarketplacePlugin) => {
    if (installing || uninstalling) return;
    setInstalling(plugin.name);
    try {
      await api.installMarketplacePlugin(marketplace.name, plugin.name);
      applyPluginPatch(plugin.plugin_id, { installed: true });
      // ON_INSTALL 插件的 OAuth 弹窗是桌面端 app-server 私有能力，CLI 无法触发；
      // 安装成功后明确引导用户去桌面端完成授权，避免「装了但不可用」的静默缺失。
      if (plugin.auth_policy === "ON_INSTALL") {
        feedback.warning(t("toast.installedNeedsAuth", { name: plugin.name }));
      } else {
        feedback.success(t("toast.installed", { name: plugin.name }));
      }
      await onInstalled();
    } catch (reason) {
      feedback.error(String(reason));
    } finally {
      setInstalling("");
    }
  };

  const uninstall = async (plugin: MarketplacePlugin) => {
    if (uninstalling || installing) return;
    const confirmed = await feedback.confirm({
      title: t("confirm.uninstallPluginTitle"),
      description: t("confirm.removePlugin", { name: plugin.name }),
      confirmText: t("action.uninstall"),
      destructive: true,
    });
    if (!confirmed) return;
    setUninstalling(plugin.name);
    try {
      await api.uninstallPlugin(plugin.name);
      applyPluginPatch(plugin.plugin_id, { installed: false });
      feedback.success(t("toast.uninstalled", { name: plugin.name }));
      await onInstalled();
    } catch (reason) {
      feedback.error(String(reason));
    } finally {
      setUninstalling("");
    }
  };

  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={t("nav.backToMarketplace")} onClick={onBack}>
          <ManagementPageTitle
            className="management-page-title--compact"
            icon={<ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />}
            title={marketplace.display_name ?? marketplace.name}
            count={loaded ? installedPluginCount : undefined}
            countLabel={loaded ? t("marketDetail.installedCountAria", { count: installedPluginCount }) : undefined}
            loadingLabel={t("marketDetail.loadingAria")}
          />
        </button>
        <div className="ml-auto flex items-center gap-2">
          <PluginSearchInput value={query} onChange={setQuery} />
        </div>
      </div>
      <div className="apple-edit-content">
        <div className="space-y-4">
          {thirdPartyProfile ? (
            <div className="apple-edit-surface">
              <div className="apple-panel-section">
                <p className="muted text-sm">
                  {t("marketDetail.thirdPartyNotice")}
                </p>
              </div>
            </div>
          ) : null}
          <div className="apple-edit-surface">
            <div className="apple-panel-section">
              <div className="flex flex-wrap items-center gap-2">
                <div className="field-label">{t("marketDetail.browsable")}</div>
                <span className="apple-chip">{t(marketplaceKindLabels[marketplace.kind])}</span>
                {loaded ? <span className="apple-chip" aria-label={t("marketDetail.browsableAria", { count: plugins.length })}>{plugins.length}</span> : <LoadingSpinner />}
                {marketplace.source_url ? <SourceLink source={marketplace.source_url} /> : null}
              </div>
              {marketplace.description ? <p className="muted mt-2 text-sm">{marketplace.description}</p> : null}
              {error ? <p className="muted mt-2 text-sm">{error}</p> : null}
              {loaded && !plugins.length && !error ? <p className="muted mt-2 text-sm">{t("marketDetail.empty")}</p> : null}
              {loaded && !error && plugins.length > 0 && !visiblePlugins.length ? <p className="muted mt-2 text-sm">{t("list.noMatch")}</p> : null}
            </div>
            {!loaded ? [0, 1, 2].map((index) => (
              <div key={index} className="apple-panel-section apple-panel-section--compact" aria-busy="true" aria-label={t("marketDetail.loadingAria")}>
                <div className="animate-pulse space-y-2">
                  <div className="h-4 w-36 rounded bg-black/5 dark:bg-white/10" />
                  <div className="h-3 w-2/3 rounded bg-black/5 dark:bg-white/10" />
                </div>
              </div>
            )) : visiblePlugins.map((plugin) => (
              <div key={plugin.plugin_id} className="apple-panel-section apple-panel-section--compact">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-semibold">{plugin.display_name ?? plugin.name}</span>
                      {plugin.version ? <span className="apple-chip">v{plugin.version}</span> : null}
                      {plugin.installed ? <span className="apple-chip apple-chip--accent">{t("marketDetail.installed")}</span> : null}
                    </div>
                    {plugin.description ? <div className="muted mt-1 break-words text-sm">{plugin.description}</div> : null}
                    {(plugin.category || plugin.capabilities.length || plugin.contains.length) ? (
                      <div className="mt-1.5 flex flex-wrap items-center gap-2">
                        {plugin.category ? <span className="apple-chip">{t("detail.category", { category: plugin.category })}</span> : null}
                        {plugin.capabilities.length ? <ContainsChips items={plugin.capabilities} /> : null}
                        {plugin.contains.length ? <ContainsChips items={plugin.contains} /> : null}
                      </div>
                    ) : null}
                  </div>
                  {!plugin.installed || marketplace.kind === "third-party" ? (
                    <button
                      type="button"
                      className={`apple-action-button shrink-0 ${plugin.installed ? "app-button--danger" : "app-button--primary"}`}
                      disabled={Boolean(installing) || Boolean(uninstalling)}
                      onClick={() => void (plugin.installed ? uninstall(plugin) : install(plugin))}
                    >
                      {installing === plugin.name || uninstalling === plugin.name ? <LoadingSpinner /> : plugin.installed ? <TrashIcon /> : <Download className="h-4 w-4" strokeWidth={2} />}
                      {plugin.installed ? t("action.uninstall") : t("action.install")}
                    </button>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
