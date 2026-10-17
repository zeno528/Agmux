import type { ReactNode, Ref } from "react";
import { useTranslation } from "react-i18next";
import { LoadingSpinner } from "../../../components/LoadingSpinner";

/** 两端市场共用页头与正文滚动布局，客户端只提供内容和动作。 */
export function MarketplacePage({ heading, backLabel, onBack, actions, contentRef, children }: {
  heading: ReactNode;
  backLabel: string;
  onBack: () => void;
  actions?: ReactNode;
  contentRef?: Ref<HTMLDivElement>;
  children: ReactNode;
}) {
  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-page-bar--roomy apple-edit-toolbar apple-edit-toolbar--header">
        <button type="button" className="apple-page-header apple-back-button" aria-label={backLabel} onClick={onBack}>{heading}</button>
        <div className="ml-auto flex items-center gap-2">{actions}</div>
      </div>
      <div ref={contentRef} className="apple-edit-content">
        <div className="space-y-4">{children}</div>
      </div>
    </section>
  );
}

export function MarketplaceGroup({ title, count, children }: { title: string; count?: number; children: ReactNode }) {
  const { t } = useTranslation("plugins");
  return (
    <>
      <div className="flex items-center gap-2">
        <div className="field-label">{title}</div>
        {count === undefined ? <LoadingSpinner /> : <span className="apple-chip" aria-label={t("market.cardCountAria", { count })}>{count}</span>}
      </div>
      <div className="apple-group apple-list-card">{children}</div>
    </>
  );
}

export function MarketplaceRow({ name, source, count, actions }: {
  name: string;
  source: string;
  count: number | null | undefined;
  actions: ReactNode;
}) {
  const { t } = useTranslation("plugins");
  return (
    <div className="apple-list-row">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="font-semibold">{name}</span>
          {count === undefined ? <span role="status" aria-label={t("marketDetail.loadingAria")}><LoadingSpinner /></span>
            : count === null ? <span className="apple-chip" aria-label={t("market.pluginCountUnavailable")}>—</span>
            : <span className="apple-chip" aria-label={t("marketDetail.browsableAria", { count })}>{t("market.pluginCount", { count })}</span>}
        </div>
        {source ? <div className="mono muted meta-xs mt-1 break-all">{source}</div> : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">{actions}</div>
    </div>
  );
}

