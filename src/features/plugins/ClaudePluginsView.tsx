import { ArrowUp, Blocks, Download, RefreshCw, Store } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedClaudePlugins, loadClaudePlugins, setClaudePluginsCache } from "../../app/managementDataCache";
import { AppDialog } from "../../components/AppDialog";
import { AppSwitch } from "../../components/AppSwitch";
import { AppTooltip } from "../../components/AppTooltip";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { TrashIcon } from "../../components/TrashIcon";
import ClaudePluginMarketplaceView from "./ClaudePluginMarketplaceView";
import { CLAUDE_OFFICIAL_MARKETPLACE, claudeInstalledIdSet, claudePluginParts } from "./claudePluginMeta";
import PluginFilterChips, { type PluginFilter } from "./components/PluginFilterChips";
import { PluginCatalogPagination, usePluginCatalogPage } from "./components/PluginCatalogPagination";
import { PluginCardSkeletons } from "./components/PluginCardSkeletons";
import { PluginCard } from "./components/PluginCard";
import { matchesQuery } from "./pluginMeta";
import type { PluginHeaderState } from "./PluginsView";
import type { ClaudePluginEntry, ClaudeAvailablePlugin, ClaudePluginInventory } from "../../types";

const scopeLabels: Record<string, "claudePlugin.scopeUser" | "claudePlugin.scopeProject" | "claudePlugin.scopeLocal"> = {
  user: "claudePlugin.scopeUser",
  project: "claudePlugin.scopeProject",
  local: "claudePlugin.scopeLocal",
};

export default function ClaudePluginsView({ popEpoch, query, onHeaderChange }: {
  popEpoch: number;
  query: string;
  onHeaderChange: (header: PluginHeaderState) => void;
}) {
  const feedback = useFeedback();
  const { t } = useTranslation("plugins");
  const [inventory, setInventory] = useState<ClaudePluginInventory>(() => getCachedClaudePlugins() ?? { installed: [], available: [] });
  const [loaded, setLoaded] = useState(() => getCachedClaudePlugins() !== null);
  const [loadError, setLoadError] = useState("");
  const [pluginAction, setPluginAction] = useState<{ key: string; kind: "install" | "toggle" | "upgrade" | "uninstall" } | null>(null);
  const busyKey = pluginAction?.key ?? null;
  const [showSources, setShowSources] = useState(false);
  // 升级图标只在「检查更新」确认有新版本后出现：id → 目标版本（悬停提示用）
  const [updatable, setUpdatable] = useState<ReadonlyMap<string, string>>(() => new Map());
  const [checking, setChecking] = useState(false);
  const [filter, setFilter] = useState<PluginFilter>("all");
  const [catalogRefreshed, setCatalogRefreshed] = useState(false);
  const catalogLoading = filter !== "installed" && !catalogRefreshed && !loadError;
  const refresh = async (force = false, includeAvailable = filter !== "installed") => {
    try {
      setInventory(await loadClaudePlugins(force, includeAvailable));
      if (includeAvailable) setCatalogRefreshed(true);
      setLoadError("");
    }
    catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
  };
  useEffect(() => { void refresh(false, false); void refresh(true, false); }, []);
  useEffect(() => {
    if (loaded && filter !== "installed" && !catalogRefreshed) void refresh(true, true);
  }, [loaded, filter, catalogRefreshed]);
  useEffect(() => { if (popEpoch) setShowSources(false); }, [popEpoch]);

  const visibleInstalled = useMemo(
    () => inventory.installed.filter((plugin) => matchesQuery({ name: claudePluginParts(plugin.id).name }, query)),
    [inventory.installed, query],
  );
  const available = useMemo(() => {
    const installedIds = claudeInstalledIdSet(inventory.installed);
    return inventory.available.filter((plugin) => !installedIds.has(plugin.pluginId)
      && matchesQuery({ name: `${plugin.name} ${plugin.description ?? ""} ${plugin.marketplaceName}` }, query))
      .sort((left, right) => Number(left.marketplaceName === CLAUDE_OFFICIAL_MARKETPLACE)
        - Number(right.marketplaceName === CLAUDE_OFFICIAL_MARKETPLACE));
  }, [inventory.installed, inventory.available, query]);

  const catalogPage = usePluginCatalogPage(available.length, query);

  const install = async (plugin: ClaudeAvailablePlugin) => {
    if (busyKey) return;
    setPluginAction({ key: plugin.pluginId, kind: "install" });
    try {
      await api.claudeInstallPlugin(plugin.pluginId);
      feedback.success(t("claudePlugin.toast.installed", { name: plugin.name }));
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setPluginAction(null);
    }
  };

  const toggleEnabled = async (entry: ClaudePluginEntry, next: boolean) => {
    const key = `${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`;
    if (busyKey) return;
    setPluginAction({ key, kind: "toggle" });
    // 乐观翻转：先写状态与缓存，失败回滚（与 Skills 页开关同一模式）
    const previous = inventory;
    const optimistic = {
      ...inventory,
      installed: inventory.installed.map((item) =>
        item.id === entry.id && item.scope === entry.scope && item.projectPath === entry.projectPath ? { ...item, enabled: next } : item,
      ),
    };
    setInventory(optimistic);
    setClaudePluginsCache(optimistic);
    try {
      await api.claudeSetPluginEnabled(entry.id, entry.scope, next, entry.projectPath);
    } catch (error) {
      setInventory(previous);
      setClaudePluginsCache(previous);
      feedback.error(String(error));
    } finally {
      setPluginAction(null);
    }
  };

  /// 刷新市场目录并按 CLI 的版本语义比对已装插件，只有真的落后才点亮升级图标。
  const checkUpdates = async () => {
    if (checking || busyKey) return;
    setChecking(true);
    try {
      const updates = await api.claudeCheckPluginUpdates();
      setUpdatable(new Map(updates.map((update) => [update.id, update.version])));
      feedback.success(updates.length ? t("market.foundUpdates", { client: "Claude Code", count: updates.length }) : t("market.upToDateToast", { client: "Claude Code" }));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setChecking(false);
    }
  };

  const upgrade = async (entry: ClaudePluginEntry) => {
    if (busyKey || checking) return;
    const key = `${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`;
    setPluginAction({ key, kind: "upgrade" });
    try {
      const result = await api.claudeUpdatePlugin(entry.id, entry.scope, entry.projectPath);
      setUpdatable((items) => new Map(items).set(entry.id, result.newVersion || entry.version || ""));
      const name = claudePluginParts(entry.id).name;
      if (result.updateOutcome === "updated") {
        feedback.success(t("claudePlugin.toast.updated", {
          name,
          old: result.oldVersion,
          new: result.newVersion,
        }));
      } else {
        feedback.info(t("claudePlugin.toast.upToDate", { name, version: result.newVersion || entry.version || "" }));
      }
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setPluginAction(null);
    }
  };

  const uninstall = async (entry: ClaudePluginEntry) => {
    const { name } = claudePluginParts(entry.id);
    const confirmed = await feedback.confirm({
      title: t("claudePlugin.confirm.uninstallTitle"),
      description: t("claudePlugin.confirm.uninstallBody", { name, scope: entry.scope }),
      confirmText: t("claudePlugin.action.uninstall"),
      destructive: true,
    });
    if (!confirmed || busyKey) return;
    const key = `${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`;
    setPluginAction({ key, kind: "uninstall" });
    try {
      await api.claudeUninstallPlugin(entry.id, entry.scope, entry.projectPath);
      feedback.success(t("claudePlugin.toast.uninstalled", { name }));
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setPluginAction(null);
    }
  };

  useLayoutEffect(() => {
    onHeaderChange({
      title: <ManagementPageTitle
          icon={<span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><Blocks className="h-[18px] w-[18px]" strokeWidth={2} /></span>}
          title={t("title")}
          count={null}
      />,
    });
  }, [onHeaderChange, t]);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <PluginFilterChips value={filter} onChange={setFilter} />
        <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
          <button type="button" className="apple-action-button apple-action-button--small" onClick={() => setShowSources(true)}>
            <Store className="h-4 w-4" strokeWidth={2} />
            {t("sources.title")}
          </button>
          <button type="button" className="apple-action-button apple-action-button--small" disabled={checking || (busyKey !== null && pluginAction?.kind !== "toggle")} aria-disabled={pluginAction?.kind === "toggle" || undefined} onClick={() => void checkUpdates()}>
            <RefreshCw className={`h-4 w-4 ${checking ? "animate-spin" : ""}`} strokeWidth={2} />
            {t("action.checkUpdates")}
          </button>
        </div>
      </div>
      {loadError ? <p className="muted text-sm">{loadError}</p> : null}
      {!loaded && filter !== "available" ? (
        <section className="plugin-section" aria-busy="true">
          <div className="field-label plugin-section-head">{t("board.sectionInstalled")}</div>
          <div className="plugin-board"><PluginCardSkeletons installed /></div>
        </section>
      ) : null}
      {!loaded && !catalogLoading ? null : (
        <>
          {filter !== "available" && visibleInstalled.length ? (
            <section className="plugin-section">
              <div className="field-label plugin-section-head">
                {t("board.sectionInstalled")}
                <span className="apple-chip">{visibleInstalled.length}</span>
                <AppTooltip label={t("board.sectionInstalled")}>
                  <p>{t("claudePlugin.installedHelp")}</p>
                </AppTooltip>
              </div>
              <div className="plugin-board">
                {visibleInstalled.map((entry) => {
                  const key = `${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`;
                  const { name, marketplace } = claudePluginParts(entry.id);
                  const busy = busyKey === key && (pluginAction?.kind === "upgrade" || pluginAction?.kind === "uninstall");
                  const updateVersion = updatable.get(entry.id);
                  return (
                    <PluginCard
                      key={key}
                      name={name}
                      source={marketplace ? `@${marketplace}` : undefined}
                      description={null}
                      iconPath={entry.iconPath}
                      action={<AppSwitch
                        size="sm"
                        checked={entry.enabled}
                        label={t("claudePlugin.toggleLabel", { name })}
                        onCheckedChange={(next) => void toggleEnabled(entry, next)}
                      />}
                      meta={<>
                        {entry.version ? <span className="plan-badge max-w-24 truncate" title={entry.version}>v{entry.version}</span> : null}
                        <span className="apple-chip apple-chip--usage">{t(scopeLabels[entry.scope] ?? "claudePlugin.scopeLocal")}</span>
                      </>}
                      foot={busy ? <LoadingSpinner /> : <>
                        {updateVersion && updateVersion !== entry.version ? (
                          <button
                            type="button"
                            className="apple-icon-button chip-success"
                            title={t("claudePlugin.action.upgradeTitle", { version: updateVersion })}
                            aria-label={t("claudePlugin.action.upgradeTo", { version: updateVersion })}
                            onClick={() => void upgrade(entry)}
                          >
                            <ArrowUp className="h-4 w-4" strokeWidth={2} />
                          </button>
                        ) : null}
                        <button
                          type="button"
                          className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]"
                          title={t("claudePlugin.action.uninstall")}
                          aria-label={t("claudePlugin.action.uninstallAria", { name })}
                          onClick={() => void uninstall(entry)}
                        >
                          <TrashIcon />
                        </button>
                      </>}
                    />
                  );
                })}
              </div>
            </section>
          ) : null}
          {filter !== "installed" && (catalogLoading || available.length > 0) ? (
            <section className="plugin-section">
              <div className="field-label plugin-section-head">
                {t("board.sectionAvailable")}
                <AppTooltip label={t("board.sectionAvailable")}>
                  <p>{t("claudePlugin.availableHelp")}</p>
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
                    key={plugin.pluginId}
                    name={plugin.name}
                    source={`@${plugin.marketplaceName}`}
                    description={plugin.description}
                    iconPath={plugin.iconPath}
                    action={
                      <button type="button" className="apple-icon-button shrink-0 text-accent hover:bg-(--accent)/10" disabled={busyKey !== null} title={t("action.install")} aria-label={t("action.install")} onClick={() => void install(plugin)}>
                        {pluginAction?.kind === "install" && busyKey === plugin.pluginId ? <LoadingSpinner /> : <Download className="h-4 w-4" strokeWidth={2} />}
                      </button>
                    }
                  />
                ))}
              </div>
              <PluginCatalogPagination start={catalogPage.start} end={catalogPage.end} total={available.length} loading={catalogLoading} move={catalogPage.move} />
            </section>
          ) : null}
          {!catalogLoading && !loadError && !visibleInstalled.length && !available.length ? (
            query.trim()
              ? <p className="muted text-sm">{t("list.noMatch")}</p>
              : <EmptyStateCard icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
                  <p className="muted">{t("claudePlugin.empty")}</p>
                </EmptyStateCard>
          ) : !query.trim() ? (
            <>
              {filter === "installed" && !visibleInstalled.length ? <p className="muted text-sm">{t("claudePlugin.empty")}</p> : null}
              {filter === "available" && !catalogLoading && !loadError && !available.length ? <p className="muted text-sm">{t("list.noMatch")}</p> : null}
            </>
          ) : null}
        </>
      )}

      <AppDialog
        open={showSources}
        onOpenChange={setShowSources}
        title={t("sources.title")}
        description={t("claudePlugin.addMarket.description")}
      >
        <ClaudePluginMarketplaceView onChanged={() => refresh(true, true)} />
        </AppDialog>
    </div>
  );
}
