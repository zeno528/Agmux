import { Minus, Plus, Trash2 } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AppSelect } from "../../components/AppSelect";
import type { KVPair } from "./mcpFormData";

export function PairEditor({ pairs, onChange, label, addLabel, keyPlaceholder, valuePlaceholder }: { pairs: KVPair[]; onChange: (pairs: KVPair[]) => void; label: string; addLabel?: string; keyPlaceholder: string; valuePlaceholder: string }) {
  const { t } = useTranslation("mcp");
  const addRow = () => onChange([...pairs, { key: "", value: "" }]);
  // 空列表时兜底显示一行占位空行，与 ArgsEditor 的单行起始行为一致。
  const rows = pairs.length ? pairs : [{ key: "", value: "" }];
  return <div className="app-dynamic-input">
    <div className="mb-1.5 flex min-h-6 items-center justify-between gap-2">
      <div className="field-label">{label}</div>
      {addLabel ? <button type="button" className="editor-ghost shrink-0 !h-7 !px-1.5" onClick={addRow}><Plus size={16} strokeWidth={2} aria-hidden="true" />{addLabel}</button> : null}
    </div>
    {rows.map((pair, index) => <div key={index} className="app-dynamic-input__item">
      <div className="app-dynamic-input__pair">
        <div className="app-input-focus-frame"><input className="app-input app-dynamic-input__input mono" placeholder={keyPlaceholder} value={pair.key} onChange={(event) => onChange(rows.map((current, currentIndex) => currentIndex === index ? { ...current, key: event.target.value } : current))} /></div>
        <div className="app-input-focus-frame min-w-0 flex-1"><input className="app-input app-dynamic-input__input mono" placeholder={valuePlaceholder} value={pair.value} onChange={(event) => onChange(rows.map((current, currentIndex) => currentIndex === index ? { ...current, value: event.target.value } : current))} /></div>
        {/* 只剩一行且为空时没有可删的内容才禁用；多行时空行也按行删除 */}
        <button type="button" className="apple-icon-button shrink-0 text-[var(--danger)]/60 enabled:hover:bg-(--danger)/10 enabled:hover:text-[var(--danger)] disabled:cursor-not-allowed disabled:opacity-40" disabled={rows.length === 1 && !pair.key.trim() && !pair.value.trim()} aria-label={t("edit.removeRow")} onClick={() => onChange(pairs.filter((_current, currentIndex) => currentIndex !== index))}><Trash2 size={16} strokeWidth={2} aria-hidden="true" /></button>
      </div>
    </div>)}
  </div>;
}

export function ArgsEditor({ text, onChange, label, addLabel, placeholder }: { text: string; onChange: (value: string) => void; label: string; addLabel: string; placeholder: string }) {
  const { t } = useTranslation("mcp");
  const args = text ? text.split(/\r?\n/) : [""];
  const update = (next: string[]) => onChange(next.join("\n"));
  return <div className="app-dynamic-input">
    <div className="mb-1.5 flex min-h-6 items-center justify-between gap-2">
      <div className="field-label">{label}</div>
      <button type="button" className="editor-ghost shrink-0 !h-7 !px-1.5" onClick={() => update([...args, ""])}><Plus size={16} strokeWidth={2} aria-hidden="true" />{addLabel}</button>
    </div>
    {args.map((arg, index) => <div key={index} className="app-dynamic-input__item">
      <div className="app-dynamic-input__pair">
        <div className="app-input-focus-frame"><input className="app-input app-dynamic-input__input mono" placeholder={placeholder} value={arg} onChange={(event) => update(args.map((current, currentIndex) => currentIndex === index ? event.target.value : current))} /></div>
        <button type="button" className="apple-icon-button shrink-0 text-[var(--danger)]/60 enabled:hover:bg-(--danger)/10 enabled:hover:text-[var(--danger)] disabled:cursor-not-allowed disabled:opacity-40" disabled={args.length === 1 && !arg.trim()} aria-label={t("edit.removeRow")} onClick={() => update(args.filter((_current, currentIndex) => currentIndex !== index))}><Trash2 size={16} strokeWidth={2} aria-hidden="true" /></button>
      </div>
    </div>)}
  </div>;
}

export function TimeoutInput({ value, onChange, placeholder, min = 1, step = 1 }: { value: number | null; onChange: (value: number | null) => void; placeholder: string; min?: number; step?: number }) {
  const { t } = useTranslation("mcp");
  return <div className="app-input-stepper app-input-focus-frame"><input className="app-input app-input-stepper__input" type="number" min={min} step={step} placeholder={placeholder} value={value ?? ""} onChange={(event) => onChange(event.target.value ? Number(event.target.value) : null)} /><div className="app-input-stepper__actions"><button type="button" className="app-input-stepper__action" aria-label={t("edit.decreaseValue")} onClick={() => onChange(Math.max(min, (value ?? min) - step))}><Minus size={16} strokeWidth={2} aria-hidden="true" /></button><button type="button" className="app-input-stepper__action" aria-label={t("edit.increaseValue")} onClick={() => onChange((value ?? (min - step)) + step)}><Plus size={16} strokeWidth={2} aria-hidden="true" /></button></div></div>;
}


interface McpConnectionFormProps {
  name: string;
  setName: (value: string) => void;
  nameReadOnly?: boolean;
  transport: string;
  setTransport: (value: string) => void;
  command: string;
  setCommand: (value: string) => void;
  argsText: string;
  setArgsText: (value: string) => void;
  url: string;
  setUrl: (value: string) => void;
  envPairs: KVPair[];
  setEnvPairs: (value: KVPair[]) => void;
  headerPairs: KVPair[];
  setHeaderPairs: (value: KVPair[]) => void;
  disabled?: boolean;
  extraTransports?: { label: string; value: string }[];
  httpFields?: ReactNode;
  httpAdvancedFields?: ReactNode;
  timeoutFields: ReactNode;
}

export default function McpConnectionForm({
  name, setName, nameReadOnly = false, transport, setTransport, command, setCommand, argsText, setArgsText,
  url, setUrl, envPairs, setEnvPairs, headerPairs, setHeaderPairs,
  disabled = false, extraTransports = [],
  httpFields, httpAdvancedFields, timeoutFields,
}: McpConnectionFormProps) {
  const { t } = useTranslation("mcp");
  return (
    <>
      <fieldset className="apple-panel-section" disabled={disabled}>
        <div className="grid gap-4 sm:grid-cols-2">
          <div>
            <div className="field-label mb-1.5">{t("edit.name")}</div>
            {nameReadOnly ? (
              <div className="app-input flex min-w-0 items-center gap-2">
                <span className="truncate font-medium text-(--text-secondary)">{name}</span>
              </div>
            ) : <input className="app-input mono" maxLength={64} placeholder={t("edit.namePlaceholder")} value={name} onChange={(event) => setName(event.target.value)} />}
          </div>
          <div>
            <div className="field-label mb-1.5">{t("edit.transport")}</div>
            <AppSelect value={transport} options={[{ label: t("edit.transportStdio"), value: "stdio" }, { label: t("edit.transportHttp"), value: "http" }, ...extraTransports]} onChange={setTransport} disabled={disabled} />
          </div>
        </div>
      </fieldset>

      <fieldset className="apple-panel-section apple-panel-section--compact" disabled={disabled}>
        {transport === "stdio" ? <>
          <div>
            <div className="field-label mb-1.5">{t("edit.command")}</div>
            <input className="app-input mono" placeholder={t("edit.commandPlaceholder")} value={command} onChange={(event) => setCommand(event.target.value)} />
          </div>
          <div className="mt-4">
            <ArgsEditor text={argsText} onChange={setArgsText} label={t("edit.args")} addLabel={t("edit.addArgument")} placeholder={t("edit.argsPlaceholder")} />
          </div>
        </> : <>
          <div>
            <div className="field-label mb-1.5">{t("edit.url")}</div>
            <input className="app-input mono" placeholder="https://mcp.example.com/mcp" value={url} onChange={(event) => setUrl(event.target.value)} />
          </div>
          {httpFields}
        </>}
        <div className="mt-4 grid gap-4">
          {transport === "stdio" ? <>
            <PairEditor label={t("edit.env")} addLabel={t("edit.addVariable")} pairs={envPairs} onChange={setEnvPairs} keyPlaceholder={t("edit.envKeyPlaceholder")} valuePlaceholder={t("edit.valuePlaceholder")} />
          </> : <>
            <PairEditor label={t("edit.headerFixed")} addLabel={t("edit.addHeader")} pairs={headerPairs} onChange={setHeaderPairs} keyPlaceholder={t("edit.headerKeyPlaceholder")} valuePlaceholder={t("edit.valuePlaceholder")} />
            {httpAdvancedFields}
          </>}
          {timeoutFields}
        </div>
      </fieldset>
    </>
  );
}
