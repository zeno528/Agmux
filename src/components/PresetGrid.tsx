import { presetEntries, presetEntryCategory, type ClientPreset } from "../presets";

/** “全部”视图的分组标题键；分类视图单组无标题。 */
const GROUP_LABEL_KEYS = { accounts: "edit.providerFilterAccounts", plans: "edit.providerFilterPlans", api: "edit.providerFilterApi", custom: "edit.providerFilterCustom" } as const;
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ProfileIconTile } from "./ProfileIconTile";

interface PresetGridProps {
  presets: readonly ClientPreset[];
  selectedKind: string;
  onSelect: (kind: string) => void;
  category: "all" | "api" | "plans" | "accounts";
  query?: string;
}

/** 两端共用的供应商网格：左侧图标和名称，右侧提示多个版本。 */
export default function PresetGrid({ presets, selectedKind, onSelect, category, query = "" }: PresetGridProps) {
  const { t } = useTranslation("profiles");
  const { t: tCommon } = useTranslation("common");
  // 组内按名称排序（zh 拼音序，显式 locale 保证各环境一致）。
  const all = presetEntries(presets);
  const entries = [
    ...all.filter((entry) => entry.preset.kind === "custom"),
    ...all.filter((entry) => entry.preset.kind !== "custom").sort((a, b) => a.name.localeCompare(b.name, "zh")),
  ];
  const search = query.trim().toLowerCase();
  const visibleEntries = entries.filter(({ name, preset }) =>
    (category === "all" || presetEntryCategory(preset) === category)
    && [name, preset.plan, preset.admin_url, ...preset.endpoints.flatMap((endpoint) => [endpoint.label, endpoint.admin_url])].join(" ").toLowerCase().includes(search));
  const groupIds: readonly (keyof typeof GROUP_LABEL_KEYS | null)[] = category === "all" ? ["accounts", "plans", "api", null] : [category];
  const groups = groupIds
    .map((group) => ({ key: group ?? "custom", entries: visibleEntries.filter(({ preset }) => presetEntryCategory(preset) === group) }))
    .filter((group) => group.entries.length > 0);
  return (
    <div className="apple-panel-section">
      {!visibleEntries.length ? <p className="muted text-sm" role="status">{tCommon("select.noResults")}</p> : null}
      {groups.map((group, index) => <section key={group.key} className={index ? "mt-6" : undefined}>
      {category === "all" ? <h2 className="field-subtitle mb-3">{t(GROUP_LABEL_KEYS[group.key], { count: group.entries.length })}</h2> : null}
      <div className="apple-tile-grid apple-tile-grid--labeled apple-tile-grid--providers">
        {group.entries.map(({ key, name, preset }) => {
          const subtitle = preset.kind === "custom" ? t("edit.customProviderSubtitle") : preset.admin_url;
          return (
            <button
              key={key}
              type="button"
              className={`flex items-center gap-2 rounded-xl px-2.5 py-2 text-left transition-colors ${selectedKind === key ? "shadow-[0_0_0_1px_var(--accent)] bg-(--active-bg)" : "shadow-(--card-edge-shadow) hover:bg-(--hover-bg)"}`}
              aria-pressed={selectedKind === key}
              onClick={() => onSelect(key)}
            >
              <ProfileIconTile name={preset.name} icon={preset.icon} size="sm" />
              <span className="min-w-0 flex-1">
                <span className="setting-title block truncate font-medium!" title={name}>{name}</span>
                {subtitle ? (
                  <span className="meta-xs muted mt-0.5 block truncate font-normal" title={subtitle}>{subtitle}</span>
                ) : null}
              </span>
              <span className={preset.endpoints.length > 1 ? "shrink-0" : "invisible shrink-0"} aria-hidden={preset.endpoints.length <= 1}>
                <span className="plan-badge font-normal!">{t("edit.multipleVersions")}</span>
              </span>
              <ChevronRight size={16} strokeWidth={2} className="muted shrink-0" aria-hidden="true" />
            </button>
          );
        })}
      </div>
      </section>)}
    </div>
  );
}
