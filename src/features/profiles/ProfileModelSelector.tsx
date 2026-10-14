import { Check, ChevronRight, Zap } from "lucide-react";
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useFeedback } from "../../app/Feedback";
import { ReasoningEffortSlider } from "../../components/ReasoningEffortSlider";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { useMenuDismiss } from "../../components/useMenuDismiss";

interface ModelSelection { model: string; effort: string; fast?: boolean }
export interface ModelSelectionOptions extends ModelSelection {
  models: string[];
  efforts?: Record<string, string[] | undefined>;
  defaults?: Record<string, string>;
}

export function resolveModelSelection(selection: ModelSelection, options: ModelSelectionOptions | null): ModelSelection {
  const model = selection.model || (options?.defaults ? options.models[0] ?? "" : "");
  return { ...selection, model, effort: selection.effort || options?.defaults?.[model] || "" };
}

export function selectModel(model: string, effort: string, advertised?: readonly string[]): ModelSelection {
  return { model, effort: advertised && !advertised.includes(effort) ? "" : effort };
}

export function resolveEffortLevels(model: string, efforts: ModelSelectionOptions["efforts"], fallback: readonly string[]) {
  return efforts?.[model] ?? fallback;
}

const modelLabel = (value: string, format?: (value: string) => string) => (format ? format(value) : value).replace(/^gpt-/i, "GPT-")
  .replace(/-(sol|astra|luna|terra)$/i, (_, name: string) => " " + name[0].toUpperCase() + name.slice(1));

export function getModelChoices(model: string, models: readonly string[], format?: (value: string) => string) {
  const choices = new Map<string, string>();
  for (const value of [...models, model].filter(Boolean)) {
    const label = format ? format(value) : value;
    if (!choices.has(label) || value === model) choices.set(label, value);
  }
  return [...choices.values()];
}

export default function ProfileModelSelector({ model, effort, levels, disabled, supportsFastMode = false, fast = false, formatModelLabel, onLoad, onSave }: {
  model: string | null;
  effort: string | null;
  levels: readonly string[];
  disabled: boolean;
  supportsFastMode?: boolean;
  fast?: boolean;
  formatModelLabel?: (value: string) => string;
  onLoad: (refreshDefaults?: boolean) => Promise<ModelSelectionOptions>;
  onSave: (changes: Partial<ModelSelection>) => Promise<void>;
}) {
  const { t } = useTranslation("profiles");
  const feedback = useFeedback();
  const [view, setView] = useState<"effort" | "models" | null>(null);
  const [triggerWidth, setTriggerWidth] = useState<number | null>(null);
  const [options, setOptions] = useState<ModelSelectionOptions | null>(null);
  const [draft, setDraft] = useState<ModelSelection>({ model: "", effort: "" });
  const [pendingSelection, setPendingSelection] = useState<ModelSelection | null>(null);
  const [busy, setBusy] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuWidth = view === "models" ? "17rem" : "16rem";
  const menuStyle = useFixedMenuPosition(view !== null, triggerRef.current, menuRef, "end", menuWidth);
  useLayoutEffect(() => {
    if (!view) return;
    const target = menuRef.current?.querySelector<HTMLElement>(view === "models" ? '[aria-selected="true"]' : 'input[type="range"]');
    target?.focus({ preventScroll: true });
    if (view === "models") target?.scrollIntoView({ block: "nearest" });
  }, [view]);
  const open = async () => {
    if (view) { close(); return; }
    const width = triggerRef.current?.getBoundingClientRect().width ?? null;
    setOptions(null);
    setBusy(true);
    try {
      const loaded = await onLoad();
      setDraft({ model: loaded.model, effort: loaded.effort, fast: loaded.fast });
      setOptions(loaded);
      setTriggerWidth(width);
      setView("effort");
    } catch (error) { feedback.error(String(error)); }
    finally { setBusy(false); }
  };
  const apply = (next: ModelSelection) => {
    if (!options || disabled) return;
    setDraft(next);
  };
  // 交互期间用 draft 预览，关闭浮卡时才保存；一次会话只写一次库和实时文件。
  const flush = async () => {
    if (!options || busy) return;
    const changes = {
      ...(draft.model !== options.model ? { model: draft.model } : {}),
      ...(draft.effort !== options.effort ? { effort: draft.effort } : {}),
      ...(supportsFastMode && draft.fast !== options.fast ? { fast: draft.fast } : {}),
    };
    if (!Object.keys(changes).length) return;
    setPendingSelection(resolveModelSelection(draft, options));
    setBusy(true);
    try {
      await onSave(changes);
      setOptions({ ...options, ...draft });
    } catch (error) { setPendingSelection(null); feedback.error(String(error)); }
    finally { setBusy(false); }
  };
  const flushRef = useRef(flush);
  flushRef.current = flush;
  // useMenuDismiss 要求稳定 close；经 ref 转发最新 flush，避免监听器随草稿反复挂卸。
  const close = useCallback(() => { setView(null); void flushRef.current(); }, []);
  useMenuDismiss(view !== null, triggerRef, menuRef, close);
  const reset = async () => {
    if (!options || busy || disabled) return;
    setBusy(true);
    try {
      // 旧缓存仅在用户主动恢复默认时补齐元数据，不阻塞每次打开。
      const loaded = Object.keys(options.defaults ?? {}).length ? options : await onLoad(true);
      if (!loaded.models.length) throw new Error(t("modelSelection.noModels"));
      setOptions(loaded);
      apply({ ...draft, model: "", effort: "" });
    } catch (error) { feedback.error(String(error)); }
    finally { setBusy(false); }
  };
  const pickModel = (value: string) => {
    if (!value && options?.defaults) {
      void reset();
      setView((current) => current === "models" ? "effort" : current);
      return;
    }
    apply({ ...draft, ...selectModel(value, draft.effort, options?.efforts?.[value]) });
    setView((current) => current === "models" ? "effort" : current);
  };
  const effective = resolveModelSelection(draft, options);
  const savedSelection = resolveModelSelection({ model: model ?? "", effort: effort ?? "", fast }, options);
  // 保存结束不代表父级已更新：显示新值直到回传匹配，避免旧文字与宽度闪回。
  useLayoutEffect(() => {
    if (pendingSelection && savedSelection.model === pendingSelection.model
      && savedSelection.effort === pendingSelection.effort && !!savedSelection.fast === !!pendingSelection.fast) {
      setPendingSelection(null);
    }
  }, [pendingSelection, savedSelection.model, savedSelection.effort, savedSelection.fast]);
  const displayedSelection = view ? effective : pendingSelection ?? savedSelection;
  const availableLevels = resolveEffortLevels(effective.model, options?.efforts, levels);
  const models = getModelChoices(options?.model ?? "", options?.models ?? [], formatModelLabel);
  return (
    <div className="contents" onClick={(event) => event.stopPropagation()} onMouseDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => { if (event.key === "Enter") event.stopPropagation(); }}>
      <button ref={triggerRef} type="button" className="profile-card-action-meta profile-model-trigger" style={view && triggerWidth ? { width: `${triggerWidth}px` } : undefined} disabled={disabled || busy}
        title={t("modelSelection.title")} aria-haspopup="dialog" aria-expanded={view !== null} aria-busy={busy} onClick={() => void open()}>
        {supportsFastMode && displayedSelection.fast && <Zap size={14} fill="currentColor" className="shrink-0" role="img" aria-label={t("modelSelection.fastMode")} />}
        <span className="profile-card-action-meta__model">{modelLabel(displayedSelection.model || (supportsFastMode ? "" : t("modelSelection.default")), formatModelLabel)}</span>
        {(!supportsFastMode || (displayedSelection.model && displayedSelection.effort)) && <span aria-hidden="true">·</span>}
        <span style={{ textTransform: "capitalize", color: displayedSelection.effort === "ultra" ? "var(--reasoning-ultra)" : undefined }}>
          {displayedSelection.effort || (supportsFastMode ? "" : t("modelSelection.default"))}</span>
        {view && <span className="profile-model-trigger__prompt muted">{t("modelSelection.chooseModel")}</span>}
      </button>
      {view && options && createPortal(
        <div ref={menuRef} className="app-select-menu app-popover profile-model-popover" data-open="true" data-popover-in
          role="dialog" aria-label={t("modelSelection.title")} aria-busy={busy} style={{ ...menuStyle, width: menuWidth }}>
          <div key={view} className="profile-model-content">
          {view === "effort" ? <ReasoningEffortSlider value={effective.effort} levels={availableLevels} disabled={busy || disabled}
            onReset={options.defaults ? () => void reset() : undefined}
            fast={supportsFastMode && !!draft.fast}
            leading={supportsFastMode ? <button type="button" className="apple-icon-button profile-model-fast"
              style={{ color: draft.fast && effective.effort === "ultra" ? "var(--reasoning-ultra)" : undefined }}
              disabled={busy || disabled} aria-pressed={!!draft.fast} title={t("modelSelection.fastModeHint")} aria-label={t("modelSelection.fastMode")}
              onClick={() => apply({ ...draft, fast: !draft.fast })}><Zap size={18} fill={draft.fast ? "currentColor" : "none"} aria-hidden="true" /></button> : undefined}
            onChange={(value) => setDraft({ ...draft, effort: value })}>
            <button type="button" className="profile-model-current field-label" disabled={busy || disabled}
              aria-haspopup="listbox" onClick={() => setView("models")}>
              <span className={effective.model ? undefined : "profile-model-default"}>{modelLabel(effective.model || t("modelSelection.default"), formatModelLabel)}</span><ChevronRight size={14} aria-hidden="true" />
            </button>
          </ReasoningEffortSlider> : <>
            <div role="listbox" aria-label={t("modelSelection.chooseModel")} onKeyDown={(event) => {
              if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
              event.preventDefault();
              const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="option"]')];
              const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
              buttons[event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
                : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length]?.focus();
            }}>
              {["", ...models].map((value) => {
                const name = value.replace(/\[1m\]\s*$/i, "");
                return (
                  <button key={value} type="button" role="option" aria-selected={draft.model === value} disabled={busy || disabled}
                    className="app-select-option app-selection-state profile-model-option" onClick={() => pickModel(value)}>
                    {/* 选中与保存用原值（所选即所写）；显示层把 [1M] 尾缀从名字里拆成右侧小标签，只改排版。 */}
                    <span className="min-w-0">
                      <span className="flex items-center gap-1.5">
                        <span className="min-w-0 truncate">{modelLabel(name || t("modelSelection.default"))}</span>
                        {name !== value && <span className="meta-xs muted shrink-0">1M</span>}
                      </span>
                    </span>
                    {draft.model === value && <Check size={18} strokeWidth={2.5} className="text-(--text-primary)" aria-hidden="true" />}
                  </button>
                );
              })}
            </div>
          </>}
          </div>
        </div>, document.body,
      )}
    </div>
  );
}
