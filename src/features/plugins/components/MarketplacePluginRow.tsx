import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

export function MarketplacePluginRow({ name, version, description, installed, actions, children }: {
  name: string;
  version?: string | null;
  description?: string | null;
  installed: boolean;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  const { t } = useTranslation("plugins");
  return (
    <div className="apple-panel-section apple-panel-section--compact">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-semibold">{name}</span>
            {version ? <span className="apple-chip">v{version}</span> : null}
            {installed ? <span className="apple-chip apple-chip--accent">{t("marketDetail.installed")}</span> : null}
          </div>
          {description ? <div className="muted mt-1 break-words text-sm">{description}</div> : null}
          {children}
        </div>
        {actions}
      </div>
    </div>
  );
}
