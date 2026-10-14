import { Check, ChevronDown, MoreHorizontal, Search, X } from "lucide-react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import { useFixedMenuPosition } from "./useFixedMenuPosition";
import { useMenuDismiss } from "./useMenuDismiss";

interface SelectOption<T extends string | number = string> {
  label: string;
  value: T;
}

interface AppSelectProps<T extends string | number> {
  value: T | null | undefined;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  placeholder?: string;
  disabled?: boolean;
  renderLabel?: (option: SelectOption<T>) => ReactNode;
  iconOnly?: boolean;
  menuWidth?: CSSProperties["width"];
  menuAlign?: "match" | "end";
  compact?: boolean;
  searchable?: boolean;
  /** 传入后使用复选菜单；onChange 返回被切换的键，菜单保持展开。 */
  checkedValues?: readonly T[];
}

export function selectedRowScrollTop(itemTop: number, itemHeight: number, viewportHeight: number, rowStep: number) {
  const rowsAbove = Math.max(0, Math.floor((viewportHeight - itemHeight) / (2 * rowStep)));
  return Math.max(0, itemTop - rowsAbove * rowStep);
}

export function AppSelect<T extends string | number>({
  value,
  options,
  onChange,
  placeholder,
  disabled,
  renderLabel,
  iconOnly = false,
  menuWidth,
  menuAlign,
  compact = false,
  searchable = false,
  checkedValues,
}: AppSelectProps<T>) {
  const { t } = useTranslation();
  const selected = options.find((option) => String(option.value) === String(value));
  const hasOptions = options.length > 0;
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [searchFocused, setSearchFocused] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const filteredOptions = searchable ? options.filter((option) => option.label.toLowerCase().includes(search.trim().toLowerCase())) : options;
  const rootRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const optionsRef = useRef<HTMLDivElement>(null);
  // 定位（向下/向上自适应翻转）与行内 ⋯ 菜单共用同一套逻辑；收起（外点/滚动/Escape）同样复用全局基建
  const menuStyle = useFixedMenuPosition(open, rootRef.current, menuRef, menuAlign ?? (iconOnly ? "end" : "match"), menuWidth);
  useMenuDismiss(open, rootRef, menuRef, setOpen);

  useLayoutEffect(() => {
    if (!open) { setSearch(""); setSearchFocused(false); return; }
    if (!searchable) return;
    const menu = menuRef.current;
    // 锁定展开时的宽高，展开搜索和过滤结果都不改变菜单尺寸。
    // 用 offsetWidth/Height 而非 getBoundingClientRect：菜单带等比缩放进场，
    // getBoundingClientRect 会读到动画首帧的缩放尺寸，把菜单永久锁成 80% 大小。
    if (menu) {
      Object.assign(menu.style, { width: `${menu.offsetWidth}px`, height: `${menu.offsetHeight}px` });
    }
    return () => { if (menu) { menu.style.width = ""; menu.style.height = ""; } };
  }, [open, searchable]);

  // 打开时定位到当前选中项；可搜索列表按完整行滚动，避免上下露出半截模型名。
  useLayoutEffect(() => {
    if (!open) return;
    const menu = searchable ? optionsRef.current : menuRef.current;
    if (searchable && search.trim() && menu) { menu.scrollTop = 0; return; }
    const current = menu?.querySelector<HTMLButtonElement>('[data-selected="true"]');
    if (!menu || !current) return;
    if (searchable) {
      const sibling = (current.nextElementSibling ?? current.previousElementSibling) as HTMLElement | null;
      const rowStep = sibling ? Math.abs(sibling.offsetTop - current.offsetTop) : current.offsetHeight;
      menu.scrollTop = selectedRowScrollTop(current.offsetTop, current.offsetHeight, menu.clientHeight, rowStep);
      return;
    }
    menu.scrollTop = Math.max(0, current.offsetTop - (menu.clientHeight - current.offsetHeight) / 2);
  }, [open, options.length, searchable, search, menuStyle]);

  // 菜单自身的滚轮不穿透：内容不满或已滚到边界时拦下，背景纹丝不动
  //   （React 的 onWheel 是 passive 的，preventDefault 必须用原生 non-passive 监听）
  useEffect(() => {
    if (!open) return;
    const menu = menuRef.current;
    const scrollContainer = searchable ? optionsRef.current : menu;
    if (!menu || !scrollContainer) return;
    const onMenuWheel = (event: WheelEvent) => {
      const { scrollTop, scrollHeight, clientHeight } = scrollContainer;
      const canScroll = scrollHeight > clientHeight;
      const atEdge = event.deltaY < 0 ? scrollTop <= 0 : scrollTop + clientHeight >= scrollHeight;
      if (!scrollContainer.contains(event.target as Node) || !canScroll || atEdge) event.preventDefault();
    };
    menu.addEventListener("wheel", onMenuWheel, { passive: false });
    return () => menu.removeEventListener("wheel", onMenuWheel);
  }, [open, searchable]);

  const selectOption = (option: SelectOption<T>) => {
    onChange(option.value);
    if (!checkedValues) setOpen(false);
  };

  // iconOnly 触发器图标跟随语义：复选菜单（⋯ 更多操作）用 MoreHorizontal，列表选择用箭头
  const TriggerGlyph = iconOnly && checkedValues ? MoreHorizontal : ChevronDown;

  const menu = (
    <div ref={menuRef} className={`app-select-menu app-popover ${iconOnly ? "app-select-menu--arrow" : ""} ${searchable ? "app-select-menu--searchable" : ""}`} data-open={open} data-popover-in={open ? "" : undefined} style={menuStyle} role={searchable ? undefined : checkedValues ? "menu" : "listbox"} aria-label={placeholder ?? t("select.optionsLabel")} aria-hidden={!open}>
      {searchable ? <div className="app-select-search">
        <Search className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-[var(--text-secondary)]" size={16} strokeWidth={2} aria-hidden="true" />
        <input ref={searchRef} type="search" aria-label={t("select.search")} placeholder={searchFocused ? t("select.search") : `${t("select.search")} ${t("select.availableCount", { count: options.length })}`} value={search} tabIndex={open ? 0 : -1} onFocus={() => setSearchFocused(true)} onBlur={() => setSearchFocused(false)} onChange={(event) => setSearch(event.target.value)} />
        {search ? <button
          type="button"
          className="apple-icon-button absolute right-1 top-1/2 -translate-y-1/2 !h-7 !w-7"
          aria-label={t("select.clearSearch")}
          tabIndex={open ? 0 : -1}
          onClick={() => { setSearch(""); searchRef.current?.focus({ preventScroll: true }); }}
        >
          <X size={16} strokeWidth={2} aria-hidden="true" />
        </button> : null}
      </div> : null}
      <div ref={optionsRef} className={searchable ? "app-select-options" : "contents"} role={searchable ? checkedValues ? "menu" : "listbox" : undefined} aria-label={searchable ? placeholder ?? t("select.optionsLabel") : undefined}>
      {searchable && filteredOptions.length === 0 ? <div className="app-select-option muted">{t("select.noResults")}</div> : null}
      {filteredOptions.map((option) => checkedValues ? <label
        key={String(option.value)}
        className="app-select-option app-selection-state justify-start! gap-2!"
      >
        <input
          type="checkbox"
          role="menuitemcheckbox"
          aria-checked={checkedValues.includes(option.value)}
          checked={checkedValues.includes(option.value)}
          disabled={disabled}
          tabIndex={open ? 0 : -1}
          onChange={() => selectOption(option)}
        />
        <span>{renderLabel?.(option) ?? option.label}</span>
      </label> : <button
        key={String(option.value)}
        type="button"
        role="option"
        tabIndex={open ? 0 : -1}
        aria-selected={selected?.value === option.value}
        className="app-select-option app-selection-state"
        data-active={selected?.value === option.value ? "true" : undefined}
        data-selected={selected?.value === option.value}
        onClick={() => selectOption(option)}
      >
        <span>{renderLabel?.(option) ?? option.label}</span>
        {selected?.value === option.value ? <Check className="app-select-option__check" size={16} strokeWidth={2.5} aria-hidden="true" /> : null}
      </button>)}
      </div>
    </div>
  );
  const menuContent = typeof document === "undefined" ? menu : createPortal(menu, document.body);

  return (
    <div ref={rootRef} className={`app-select-wrap ${iconOnly ? "app-select-wrap--icon" : ""} ${compact ? "app-select-wrap--compact" : ""}`} data-open={open}>
      <button
        type="button"
        className={`app-select ${iconOnly ? "app-select--icon" : ""} ${compact ? "app-select--compact" : ""}`}
        disabled={disabled}
        aria-haspopup={checkedValues ? "menu" : "listbox"}
        aria-expanded={open}
        aria-label={iconOnly ? (selected ? `${placeholder ?? t("select.placeholder")}: ${selected.label}` : placeholder ?? t("select.placeholder")) : placeholder}
        title={iconOnly ? (selected?.label ?? placeholder) : undefined}
        onClick={() => hasOptions && setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (hasOptions && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        {iconOnly ? <span className="sr-only">{selected ? renderLabel?.(selected) ?? selected.label : placeholder ?? t("select.placeholder")}</span> : <span className="app-select__label">{selected ? renderLabel?.(selected) ?? selected.label : placeholder ?? t("select.placeholder")}</span>}
        <TriggerGlyph className="app-select__icon" size={compact ? 14 : 16} strokeWidth={2} aria-hidden="true" />
      </button>
      {menuContent}
    </div>
  );
}
