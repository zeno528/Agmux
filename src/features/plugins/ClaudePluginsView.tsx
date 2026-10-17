import { Blocks, Download, Store } from "lucide-react";
import { useEffect, useLayoutEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedClaudePlugins, loadClaudePlugins, setClaudePluginsCache } from "../../app/managementDataCache";
import { AppSwitch } from "../../components/AppSwitch";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { TrashIcon } from "../../components/TrashIcon";
import ClaudePluginMarketplaceView from "./ClaudePluginMarketplaceView";
import { claudePluginParts } from "./claudePluginMeta";
import { matchesQuery } from "./pluginMeta";
import type { PluginHeaderState } from "./PluginsView";
import type { ClaudePluginEntry, ClaudePluginInventory } from "../../types";

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
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [browsingMarketplace, setBrowsingMarketplace] = useState(false);
  const refresh = async (force = false) => {
    try { setInventory(await loadClaudePlugins(force)); setLoadError(""); }
    catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
  };
  useEffect(() => { void refresh(); void refresh(true); }, []);
  useEffect(() => { if (popEpoch) setBrowsingMarketplace(false); }, [popEpoch]);
  const visibleInstalled = inventory.installed.filter((plugin) => matchesQuery(claudePluginParts(plugin.id), query));

  const toggleEnabled = async (entry: ClaudePluginEntry, next: boolean) => {
    const key = `${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`;
    if (busyKey) return;
    setBusyKey(key);
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
      setBusyKey(null);
    }
  };

  const upgrade = async (entry: ClaudePluginEntry) => {
    if (busyKey) return;
    setBusyKey(`${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`);
    try {
      const result = await api.claudeUpdatePlugin(entry.id, entry.scope, entry.projectPath);
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
      setBusyKey(null);
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
    setBusyKey(`${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`);
    try {
      await api.claudeUninstallPlugin(entry.id, entry.scope, entry.projectPath);
      feedback.success(t("claudePlugin.toast.uninstalled", { name }));
      await refresh(true);
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusyKey(null);
    }
  };

  useLayoutEffect(() => {
    onHeaderChange({
      editing: browsingMarketplace,
      title: <ManagementPageTitle
          icon={<span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><Blocks className="h-[18px] w-[18px]" strokeWidth={2} /></span>}
          title={t("title")}
          count={loaded ? inventory.installed.length : undefined}
          countLabel={loaded ? t("claudePlugin.installedCountAria", { count: inventory.installed.length }) : undefined}
          loadingLabel={t("claudePlugin.loadingAria")}
      />,
      actions: <button type="button" className="apple-action-button app-button--primary" onClick={() => setBrowsingMarketplace(true)}>
        <Store className="h-4 w-4" strokeWidth={2} />
        {t("marketplace")}
        <span className="rounded-full bg-white/95 px-1.5 py-px font-bold tracking-wide text-accent meta-xs">Beta</span>
      </button>,
    });
  }, [onHeaderChange, loaded, inventory.installed.length, browsingMarketplace, t]);

  if (browsingMarketplace) {
    return <ClaudePluginMarketplaceView onBack={() => setBrowsingMarketplace(false)} onInstalled={() => refresh(true)} />;
  }
  return (
    <>
        {loadError ? <p className="muted text-sm">{loadError}</p> : null}
        {!loaded ? (
          <EmptyStateCard loading icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("claudePlugin.loading")}</p>
          </EmptyStateCard>
        ) : inventory.installed.length === 0 ? (
          <EmptyStateCard icon={<Blocks className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("claudePlugin.empty")}</p>
          </EmptyStateCard>
        ) : visibleInstalled.length ? (
          <div className="apple-group apple-list-card">
            {visibleInstalled.map((entry) => {
              const key = `${entry.id}|${entry.scope}|${entry.projectPath ?? ""}`;
              const { name, marketplace } = claudePluginParts(entry.id);
              const busy = busyKey === key;
              const detail = entry.projectPath ?? entry.installPath ?? "";
              return (
                <div key={key} className="apple-list-row">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="min-w-0 truncate font-semibold">{name}</span>
                      {entry.version ? (
                        <span className="shrink-0 rounded-md bg-black/5 px-1.5 py-px font-medium tracking-wide muted meta-xs dark:bg-white/10">v{entry.version}</span>
                      ) : null}
                      {marketplace ? <span className="shrink-0 rounded-md bg-black/5 px-1.5 py-px font-medium tracking-wide muted meta-xs dark:bg-white/10">@{marketplace}</span> : null}
                      <span className="apple-chip shrink-0">{entry.scope}</span>
                      {entry.enabled ? null : <span className="apple-chip chip-warn shrink-0">{t("claudePlugin.disabled")}</span>}
                    </div>
                    {detail ? <div className="muted meta-xs truncate" title={detail}>{detail}</div> : null}
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {busy ? (
                      <LoadingSpinner />
                    ) : (
                      <>
                        <button
                          type="button"
                          className="apple-action-button"
                          disabled={busyKey !== null}
                          title={t("claudePlugin.action.upgradeTitle")}
                          onClick={() => void upgrade(entry)}
                        >
                          <Download className="h-4 w-4" strokeWidth={2} />
                          {t("claudePlugin.action.upgrade")}
                        </button>
                        <AppSwitch
                          size="sm"
                          checked={entry.enabled}
                          label={t("claudePlugin.toggleLabel", { name })}
                          onCheckedChange={(next) => void toggleEnabled(entry, next)}
                        />
                        <button
                          type="button"
                          className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]"
                          title={t("claudePlugin.action.uninstall")}
                          aria-label={t("claudePlugin.action.uninstallAria", { name })}
                          onClick={() => void uninstall(entry)}
                        >
                          <TrashIcon />
                        </button>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        ) : null}
      {loaded && query.trim() && !visibleInstalled.length ? <p className="muted mt-4 text-sm">{t("list.noMatch")}</p> : null}
    </>
  );
}
