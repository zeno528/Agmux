import { Check, ChevronDown } from "lucide-react";
import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { presetRegionName, type PresetEndpoint } from "../presets";
import { useFixedMenuPosition } from "./useFixedMenuPosition";
import { useMenuDismiss } from "./useMenuDismiss";

interface EndpointFieldProps {
  value: string;
  onChange: (url: string) => void;
  /** 官方区域端点档。 */
  endpoints: readonly PresetEndpoint[];
  /** 选中某档后的附加动作（如同步控制台地址）。 */
  onPick?: (endpoint: PresetEndpoint) => void;
  placeholder: string;
  /** 输入框 aria 菜单名称（调用方 i18n 解析后的文案）。 */
  label: string;
}

/** 区域端点 combobox：地址可自由输入，菜单只是档位快捷入口；
 *  弹层复用全局 app-select-menu 基建（定位 useFixedMenuPosition、收起 useMenuDismiss），
 *  菜单行带区域徽标（与可选的计费通道 label）。Codex / Claude Code 编辑页共用。 */
export default function EndpointField({ value, onChange, endpoints, onPick, placeholder, label }: EndpointFieldProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuStyle = useFixedMenuPosition(open, rootRef.current, menuRef, "match");
  useMenuDismiss(open, rootRef, menuRef, setOpen);
  return (
    <div ref={rootRef}>
      <div className="app-input-action">
        <input className="app-input app-input--action" placeholder={placeholder} value={value} onChange={(event) => onChange(event.target.value)} />
        <button type="button" className="app-input-action__button" aria-label={label} aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen((prev) => !prev)}>
          <ChevronDown className={open ? "h-4 w-4 rotate-180 transition-transform" : "h-4 w-4 transition-transform"} strokeWidth={2} aria-hidden="true" />
        </button>
      </div>
      {createPortal(
        <div ref={menuRef} className="app-select-menu app-popover" data-open={open} data-popover-in={open ? "" : undefined} role="listbox" aria-label={label} style={menuStyle}>
          {endpoints.map((endpoint) => {
            const selected = endpoint.base_url === value;
            return (
              <button key={endpoint.base_url} type="button" role="option" aria-selected={selected} className="app-select-option app-selection-state" data-active={selected ? "true" : undefined} data-selected={selected} onClick={() => { onChange(endpoint.base_url); onPick?.(endpoint); setOpen(false); }}>
                <span className="flex min-w-0 items-center gap-2">
                  <span className="meta-xs inline-flex w-13 shrink-0 items-center justify-center rounded-md bg-(--tile-bg) py-0.5 font-medium text-accent">
                    {presetRegionName(endpoint.region)}
                  </span>
                  {endpoint.label ? <span className="meta-xs shrink-0 rounded-md bg-(--tile-bg) px-1.5 py-0.5 font-medium text-[var(--text-secondary)]">{endpoint.label}</span> : null}
                  <span className="min-w-0 truncate">{endpoint.base_url}</span>
                </span>
                {selected ? <Check className="app-select-option__check" size={16} strokeWidth={2.5} aria-hidden="true" /> : null}
              </button>
            );
          })}
        </div>,
        document.body,
      )}
    </div>
  );
}
