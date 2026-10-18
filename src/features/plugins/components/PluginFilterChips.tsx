import { useTranslation } from "react-i18next";
import { LoadingSpinner } from "../../../components/LoadingSpinner";

export type PluginFilter = "all" | "installed" | "available" | "updates";

/** 看板一级过滤：「有更新」chip 只在有待更新时出现（计数为 0 或调用方没给——如 Claude 无批量更新检查——都不显示）。
 *  计数 null = 还在加载，用旋转指示器占位，chips 始终固定渲染不塌行。 */
export default function PluginFilterChips({ value, onChange, counts }: {
  value: PluginFilter;
  onChange: (filter: PluginFilter) => void;
  counts: Partial<Record<PluginFilter, number | null>>;
}) {
  const { t } = useTranslation("plugins");
  const chips: { key: PluginFilter; label: string; alert?: boolean }[] = [
    { key: "all", label: t("board.filterAll") },
    { key: "installed", label: t("board.filterInstalled") },
    { key: "available", label: t("board.filterAvailable") },
  ];
  if (counts.updates) chips.push({ key: "updates", label: t("board.filterUpdates"), alert: true });
  return (
    <div className="plugin-filter-chips" role="group">
      {chips.map((chip) => (
        <button
          key={chip.key}
          type="button"
          className={`plugin-filter-chip${chip.alert ? " plugin-filter-chip--alert" : ""}`}
          aria-pressed={value === chip.key}
          onClick={() => onChange(chip.key)}
        >
          {chip.label}
          <span className="plugin-filter-chip__count">
            {counts[chip.key] === null ? <LoadingSpinner /> : counts[chip.key] ?? 0}
          </span>
        </button>
      ))}
    </div>
  );
}
