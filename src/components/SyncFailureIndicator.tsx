import { CircleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { SyncFailure } from "../types";
import { AppTooltip } from "./AppTooltip";

export function SyncFailureIndicator({ failure }: { failure: SyncFailure | undefined }) {
  const { t } = useTranslation();
  if (!failure) return null;
  const file = failure.client === "codex" ? "~/.codex/config.toml" : "~/.claude/settings.json";
  const format = failure.client === "codex" ? "TOML" : "JSON";
  const parseFailed = failure.reason === "live_parse_error";
  const position = parseFailed && failure.position ? t("writeback.position", { line: failure.position[0], column: failure.position[1] }) : "";
  const title = t(parseFailed ? "writeback.paused" : "writeback.failed");
  return (
    <AppTooltip label={title} icon={<CircleAlert className="h-4 w-4 text-(--danger)" aria-hidden="true" />}>
      <p className="text-sm font-medium">{title}</p>
      <p className="text-xs break-words text-(--text-secondary)">{t(`writeback.reason.${failure.reason}`, { file, format, position })}</p>
      <p className="text-xs text-(--text-secondary)">{t(parseFailed ? "writeback.parseAction" : "writeback.protected")}</p>
    </AppTooltip>
  );
}
