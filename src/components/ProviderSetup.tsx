import { ArrowLeft, ChevronRight, Search } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { presetConnectionName, presetConnections, presetEntries, presetEntryCategory, presetRegionName, type ClientPreset, type PresetConnection } from "../presets";
import { AppSegmentedControl } from "./AppSegmentedControl";
import { ProfileIconTile } from "./ProfileIconTile";
import PresetGrid from "./PresetGrid";
import { AppTabBar } from "./AppTabBar";

export function ProviderCreate({ presets, onBack, children }: {
  presets: readonly ClientPreset[];
  onBack: () => void;
  children: (preset: ClientPreset, onChangeProvider: () => void) => ReactNode;
}) {
  const { t } = useTranslation("profiles");
  const entries = useMemo(() => presetEntries(presets), [presets]);
  const [visited, setVisited] = useState<string[]>([]);
  const [selectedKind, setSelectedKind] = useState("");
  const [picking, setPicking] = useState(true);
  const [category, setCategory] = useState<"all" | "api" | "plans" | "accounts">("all");
  const [query, setQuery] = useState("");
  const backLabel = t("edit.createTitle");
  return <>
    {picking ? <section className="apple-scroll-page mx-auto flex w-full max-w-none flex-col">
      <header className="apple-page-bar">
        <button type="button" className="apple-page-header apple-back-button" aria-label={backLabel} onClick={onBack}>
          <ArrowLeft size={16} strokeWidth={2} className="shrink-0 text-accent" aria-hidden="true" />
          <span className="apple-title">{backLabel}</span>
        </button>
        <div className="relative ml-auto w-64 shrink-0">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-(--text-secondary)" strokeWidth={2} aria-hidden="true" />
          <input type="search" className="app-input app-input--pill" value={query}
            placeholder={t("edit.searchProviders")} aria-label={t("edit.searchProviders")}
            onChange={(event) => setQuery(event.target.value)} />
        </div>
      </header>
      <AppTabBar selected={category} onSelect={setCategory} label={t("edit.selectProvider")} items={[
        { id: "all", label: t("edit.providerFilterAll", { count: entries.length }) },
        { id: "accounts", label: t("edit.providerFilterAccounts", { count: entries.filter(({ preset }) => presetEntryCategory(preset) === "accounts").length }) },
        { id: "plans", label: t("edit.providerFilterPlans", { count: entries.filter(({ preset }) => presetEntryCategory(preset) === "plans").length }) },
        { id: "api", label: t("edit.providerFilterApi", { count: entries.filter(({ preset }) => presetEntryCategory(preset) === "api").length }) },
      ]} />
      <div className="apple-edit-content">
        <div className="apple-edit-surface">
          <PresetGrid presets={presets} category={category} query={query} selectedKind={selectedKind} onSelect={(kind) => {
            setVisited((current) => current.includes(kind) ? current : [...current, kind]);
            setSelectedKind(kind);
            setPicking(false);
          }} />
        </div>
      </div>
    </section> : null}
    {/* 按套餐入口保留完整草稿，切回时不重挂载，也不把密钥带到另一入口。 */}
    {visited.map((kind) => <div key={kind} className={picking || selectedKind !== kind ? "hidden" : "contents"}>
      {children(entries.find((entry) => entry.key === kind)!.preset, () => setPicking(true))}
    </div>)}
  </>;
}

export function ProviderPresetControls({ preset, baseUrl, disabled, onChangeProvider, onSelect }: {
  preset: ClientPreset;
  baseUrl: string;
  disabled: boolean;
  onChangeProvider: () => void;
  onSelect: (connection: PresetConnection) => void;
}) {
  const { t } = useTranslation("profiles");
  const connections = presetConnections(preset, [preset]);
  const selected = connections.find((item) => item.base_url === baseUrl);
  const current = selected ?? connections[0];
  const plans = [...new Set(connections.map((item) => item.label))];
  const multiplePlans = plans.length > 1;
  const samePlan = connections.filter((item) => item.label === current.label);
  const versions = multiplePlans
    ? plans.map((plan) => connections.find((item) => item.label === plan && item.region === current.region) ?? connections.find((item) => item.label === plan)!)
    : samePlan;
  const controls = [
    { label: t("edit.versionLabel"), description: t(multiplePlans ? "edit.billingDescription" : "edit.versionDescription"), versions, billing: multiplePlans },
    ...(multiplePlans && samePlan.length > 1 ? [{ label: t("edit.regionLabel"), description: t("edit.versionDescription"), versions: samePlan, billing: false }] : []),
  ];
  return <div className="apple-panel-section" onKeyDown={(event) => { if (event.key === "Enter") event.stopPropagation(); }}>
    <div className="flex items-center gap-3">
      <ProfileIconTile name={preset.name} icon={preset.icon} />
      <span className="title-sm min-w-0 flex-1 truncate">{presetConnectionName(preset)}</span>
      <button type="button" className="apple-action-button" disabled={disabled} onClick={onChangeProvider}>
        {t("edit.changeProvider")}<ChevronRight size={16} strokeWidth={2} aria-hidden="true" />
      </button>
    </div>
    <div className="provider-preset-controls" data-custom={!selected}>
      {controls.filter((control) => control.versions.length > 1).map((control) => <div key={control.label} className="mt-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <span className="field-label">{control.label}</span>
          <p className="setting-description mt-1">{control.description}</p>
        </div>
        <AppSegmentedControl className="h-9 w-72 max-w-full" label={control.label} selectedIndex={control.versions.findIndex((version) => version.base_url === selected?.base_url)}>
          {control.versions.map((version) => <button key={version.base_url} type="button" className="text-sm" aria-pressed={selected?.base_url === version.base_url} disabled={disabled}
            onClick={() => onSelect(version)}>
            {control.billing ? version.label === "API" ? t("edit.payAsYouGo") : version.label : presetRegionName(version.region)}
          </button>)}
        </AppSegmentedControl>
      </div>)}
      {!selected && baseUrl ? <p className="setting-description mt-3">{t("edit.customEndpoint")}</p> : null}
    </div>
  </div>;
}
