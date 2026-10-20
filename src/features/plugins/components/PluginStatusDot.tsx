import { useTranslation } from "react-i18next";

export function PluginStatusDot({ enabled }: { enabled: boolean }) {
  const { t } = useTranslation("plugins");
  return <span
    className={`h-2 w-2 shrink-0 rounded-full ${enabled ? "bg-success" : "bg-(--text-secondary)"}`}
    role="img"
    aria-label={t(enabled ? "status.on" : "status.off")}
  />;
}
