import { Download, Eye, EyeOff, Pencil, Trash2, Wifi } from "lucide-react";
import type { ReactNode } from "react";
import { AppSelect } from "./AppSelect";
import { LoadingSpinner } from "./LoadingSpinner";
import { ProfileIconTile } from "./ProfileIconTile";

const displayModelLabel = (value: string) => value.replace(/\[1m\]\s*$/i, "");

export function ProviderIdentityFields({ idPrefix, name, description, icon, iconName, onIcon, onName, onDescription, labels }: {
  idPrefix: string;
  name: string;
  description: string;
  icon: string | null;
  iconName?: string;
  onIcon: () => void;
  onName: (value: string) => void;
  onDescription: (value: string) => void;
  labels: {
    changeIcon: string;
    changeIconLabel: string;
    name: string;
    namePlaceholder: string;
    description: string;
    descriptionPlaceholder: string;
  };
}) {
  return (
    <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
      <div className="flex min-w-0 items-center gap-4">
        <button
          type="button"
          className="relative grid h-[61px] w-[61px] shrink-0 place-items-center rounded-[16px] transition-opacity hover:opacity-80"
          title={labels.changeIcon}
          aria-label={labels.changeIconLabel}
          onClick={onIcon}
        >
          <ProfileIconTile name={iconName ?? name} icon={icon} size="fill" />
          <span className="absolute -bottom-1 -right-1 grid h-5 w-5 place-items-center rounded-full bg-accent text-white shadow" aria-hidden="true">
            <Pencil className="h-2.5 w-2.5" strokeWidth={2} />
          </span>
        </button>
        <div className="min-w-0 flex-1">
          <label className="field-label mb-1.5 block" htmlFor={`${idPrefix}-name`}>{labels.name}</label>
          <input id={`${idPrefix}-name`} className="app-input" maxLength={50} placeholder={labels.namePlaceholder} value={name} onChange={(event) => onName(event.target.value)} />
        </div>
      </div>
      <div className="min-w-0">
        <label className="field-label mb-1.5 block" htmlFor={`${idPrefix}-description`}>{labels.description}</label>
        <input
          id={`${idPrefix}-description`} className="app-input" autoComplete="off" maxLength={200}
          placeholder={labels.descriptionPlaceholder} value={description}
          onChange={(event) => onDescription(event.target.value)}
        />
      </div>
    </div>
  );
}

export function ProviderSecretField({ label, placeholder, value, onChange, visible, onToggle, showLabel, hideLabel, testLabel, testTitle, testing, onTest, help }: {
  label: string;
  placeholder: string;
  value: string;
  onChange: (value: string) => void;
  visible: boolean;
  onToggle: () => void;
  showLabel: string;
  hideLabel: string;
  testLabel: string;
  testTitle?: string;
  testing: boolean;
  onTest: () => void;
  help?: ReactNode;
}) {
  return <>
    <div className="mb-1.5 mt-4 flex items-center gap-2">
      <span className="field-label">{label}</span>
      {help}
      <button
        type="button" className="apple-inline-btn apple-inline-btn--quiet !h-5"
        disabled={testing || Boolean(testTitle)} title={testTitle} onClick={onTest}
      >
        {testing ? <LoadingSpinner /> : <Wifi className="h-3 w-3" strokeWidth={2} aria-hidden="true" />}
        {testLabel}
      </button>
    </div>
    <div className="app-input-action">
      <input className="app-input app-input--action" type={visible ? "text" : "password"} placeholder={placeholder} value={value} onChange={(event) => onChange(event.target.value)} />
      <button
        type="button" className="app-input-action__button" aria-label={visible ? hideLabel : showLabel}
        title={visible ? hideLabel : showLabel} aria-pressed={visible} onClick={onToggle}
      >
        {visible ? <EyeOff className="h-4 w-4" strokeWidth={2} aria-hidden="true" /> : <Eye className="h-4 w-4" strokeWidth={2} aria-hidden="true" />}
      </button>
    </div>
  </>;
}

export function ProviderModelFields({ models: fetchedModels, fetching, disabled, onFetch, onApplyModel, mappingFields, labels }: {
  models: string[];
  fetching: boolean;
  disabled: boolean;
  onFetch: () => void;
  onApplyModel?: (value: string) => void;
  mappingFields: Array<{
    key: string;
    label: string;
    value: string;
    onChange: (value: string) => void;
    /** 请求模型列悬停提示（环境变量名）：常显会被截断成半截标识符，只走原生 title */
    hint?: string;
    displayValue?: string;
    displayPlaceholder?: string;
    displayDisabled?: boolean;
    onDisplayChange?: (value: string) => void;
    oneMillion?: boolean;
    onToggleOneMillion?: (enabled: boolean) => void;
  }>;
  labels: {
    model: string;
    models: string;
    fetch: string;
    available: string;
    select: string;
    fetchFirst: string;
    quickSet?: string;
    mappingTitle?: string;
    mappingDescription?: string;
    role?: string;
    displayName?: string;
    requestModel?: string;
    oneMillion?: string;
    oneMillionColumn?: string;
    oneMillionTitle?: string;
    clear?: string;
  };
}) {
  const models = [...new Set(fetchedModels.map((model) => model.trim()))].filter(Boolean);
  // 去重后的模型下拉项建一次给每一行用，别在 map 里重建。
  const optionsByLabel = new Map<string, string>();
  for (const item of models) {
    const label = displayModelLabel(item);
    if (!optionsByLabel.has(label)) optionsByLabel.set(label, item);
  }
  const modelOptions = [...optionsByLabel].map(([label, value]) => ({ label, value }));
  return (
    <div>
      <div className="mb-3 flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="title-sm">{labels.mappingTitle ?? labels.models}</div>
          {labels.mappingDescription ? <div className="setting-description mt-1">{labels.mappingDescription}</div> : null}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {onApplyModel && labels.quickSet ? (
            <div className="w-28 shrink-0">
              <AppSelect
                value={null}
                options={models.map((item) => ({ label: item, value: item }))}
                onChange={onApplyModel}
                placeholder={labels.quickSet}
                disabled={disabled || fetching || models.length === 0}
                menuAlign="end"
                compact
                searchable
              />
            </div>
          ) : null}
          <div className="w-28 shrink-0">
            <button type="button" className="apple-inline-btn apple-inline-btn--quiet !h-7 w-full justify-center" disabled={fetching || disabled} onClick={onFetch}>
              {fetching ? <LoadingSpinner /> : <Download className="h-3 w-3" strokeWidth={2} aria-hidden="true" />}
              {labels.fetch}
            </button>
          </div>
        </div>
      </div>
      <div className="overflow-hidden rounded-xl border border-(--panel-border) bg-(--main-surface-bg)">
        <div className="hidden grid-cols-[7rem_minmax(0,1fr)_minmax(0,1fr)_2.5rem_max-content_2rem] items-center gap-2 bg-(--tile-bg) px-3 py-2 sm:grid">
          <span className="meta-xs muted font-medium">{labels.role ?? labels.model}</span>
          <span className="meta-xs muted font-medium">{labels.displayName}</span>
          <span className="meta-xs muted flex min-w-0 items-center gap-2 whitespace-nowrap font-medium">
            <span>{labels.requestModel}</span>
            {models.length > 0 ? <span aria-hidden="true">·</span> : null}
            {models.length > 0 ? <span>{labels.available}</span> : null}
          </span>
          <span aria-hidden="true" />
          <span className="meta-xs muted whitespace-nowrap text-center font-medium">{labels.oneMillionColumn ?? labels.oneMillion}</span>
          <span aria-hidden="true" />
        </div>
        {mappingFields.map((field) => {
          const empty = !displayModelLabel(field.value).trim();
          const selected = modelOptions.find((option) => option.label === displayModelLabel(field.value.trim()))?.value ?? null;
          return (
            <div key={field.key} className="grid grid-cols-1 gap-2 border-t border-(--panel-divider) px-3 py-2 sm:grid-cols-[7rem_minmax(0,1fr)_minmax(0,1fr)_2.5rem_max-content_2rem] sm:items-center">
              <div className="flex min-w-0 items-center">
                <span className="inline-flex min-h-8 w-full min-w-0 items-center rounded-lg bg-(--tile-bg) px-3 text-sm font-medium text-(--text-secondary)" title={field.label}>{field.label}</span>
              </div>
              <input
                className="app-input app-input--compact min-w-0 disabled:bg-(--tile-bg) disabled:text-(--text-secondary)"
                aria-label={`${field.label} ${labels.displayName ?? ""}`}
                placeholder={field.displayPlaceholder ?? ""}
                value={field.displayValue ?? ""}
                disabled={field.displayDisabled ?? !field.onDisplayChange}
                onChange={(event) => field.onDisplayChange?.(event.target.value)}
              />
              <input className="app-input app-input--compact min-w-0" aria-label={`${field.label} ${labels.requestModel ?? labels.model}`} title={field.hint} value={displayModelLabel(field.value)} onChange={(event) => field.onChange(event.target.value)} />
              <AppSelect
                value={selected}
                options={modelOptions}
                onChange={field.onChange}
                placeholder={models.length ? labels.select : labels.fetchFirst}
                disabled={disabled || fetching || !models.length}
                iconOnly
                searchable
              />
              {field.onToggleOneMillion && labels.oneMillion ? (
                <label className={`editor-ghost !h-8 shrink-0 !px-2 justify-self-center ${!empty && field.oneMillion ? "on" : ""}`} title={labels.oneMillionTitle} aria-disabled={empty}>
                  <input type="checkbox" checked={!empty && (field.oneMillion ?? false)} disabled={empty} aria-label={`${field.label} ${labels.oneMillion}`} onChange={(event) => field.onToggleOneMillion?.(event.target.checked)} />
                  <span className="meta-xs font-medium">{labels.oneMillion}</span>
                </label>
              ) : null}
              {labels.clear ? (
                <button type="button" className="apple-icon-button shrink-0 text-[var(--danger)]/60 enabled:hover:bg-(--danger)/10 enabled:hover:text-[var(--danger)] disabled:text-(--text-secondary) disabled:opacity-40" disabled={empty && !field.displayValue?.trim()} aria-label={`${labels.clear} ${field.label}`} onClick={() => { field.onChange(""); field.onDisplayChange?.(""); }}>
                  <Trash2 size={16} strokeWidth={2} aria-hidden="true" />
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
