import { Plus } from "lucide-react";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useFeedback } from "../../../app/Feedback";
import { LoadingSpinner } from "../../../components/LoadingSpinner";
import type { ClientTarget } from "../../../components/ClientTargetSwitch";
import { findConfiguredMarketplace, recommendedMarketplaces } from "../pluginMeta";
import SourceLink from "./SourceLink";

/** 市场名称、来源链接和右侧操作共用一套行布局。 */
export function PluginMarketplaceRow({ name, source, icon, children }: {
  name: string;
  source: string;
  icon?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="apple-list-row">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate font-semibold">{name}</span>
          {icon}
        </div>
        <div className="flex items-center gap-1">
          <SourceLink source={source} />
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1.5">{children}</div>
    </div>
  );
}

/** 两端共用添加表单与推荐入口；市场行和 CLI 操作由各客户端提供。 */
export default function PluginMarketplaceSources({ target, marketplaces, loaded, busy, onAdd, children }: {
  target: ClientTarget;
  marketplaces: { name: string }[];
  loaded: boolean;
  busy: boolean;
  onAdd: (source: string) => Promise<{ name: string }>;
  children: ReactNode;
}) {
  const { t } = useTranslation("plugins");
  const feedback = useFeedback();
  const [source, setSource] = useState("");
  const [adding, setAdding] = useState("");
  const pending = recommendedMarketplaces.filter(
    (item) => item.clients.some((client) => client === target) && !findConfiguredMarketplace(item, marketplaces),
  );
  const add = async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed || adding || busy) return;
    setAdding(trimmed);
    try {
      const marketplace = await onAdd(trimmed);
      if (trimmed === source.trim()) setSource("");
      feedback.success(t("sources.addedToast", { name: marketplace.name }));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setAdding("");
    }
  };
  return (
    <fieldset className="min-w-0 space-y-4" disabled={busy || Boolean(adding)}>
      <form className="flex items-center gap-2" onSubmit={(event) => { event.preventDefault(); return add(source); }}>
        <input className="app-input min-w-0 flex-1" aria-label={t("add.marketSourceTitle")} placeholder={t("add.marketPlaceholder")} value={source} onChange={(event) => setSource(event.target.value)} />
        <button type="submit" className="apple-action-button apple-action-button--small app-button--primary" disabled={!source.trim()}>
          {adding && adding === source.trim() ? <LoadingSpinner /> : <Plus size={16} strokeWidth={2} />}
          {t("sources.add")}
        </button>
      </form>
      <div className="flex items-center gap-2">
        <div className="field-label">{t("marketplace")}</div>
        {loaded ? <span className="apple-chip">{marketplaces.length}</span> : <LoadingSpinner />}
      </div>
      {marketplaces.length ? <div className="apple-group apple-list-card">{children}</div> : loaded ? <p className="muted text-sm">{t("claudePlugin.noMarkets")}</p> : null}
      {loaded && pending.length ? (
        <div className="apple-group apple-list-card">
          {pending.map((item) => (
            <div key={item.name} className="apple-list-row">
              <div className="min-w-0 flex-1">
                <div className="font-semibold">{item.displayName}</div>
                <div className="muted mt-0.5 break-words text-sm">{t(item.descriptionKey)}</div>
                <div className="mono muted meta-xs mt-0.5 truncate">{item.source}</div>
              </div>
              <button type="button" className="apple-action-button apple-action-button--small app-button--primary shrink-0" onClick={() => add(item.source)}>
                {adding === item.source ? <LoadingSpinner /> : <Plus size={16} strokeWidth={2} />}
                {t("sources.add")}
              </button>
            </div>
          ))}
        </div>
      ) : null}
    </fieldset>
  );
}
