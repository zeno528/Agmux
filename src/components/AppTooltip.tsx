import { CircleQuestionMark } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** 出场动画时长，与 style.css 的 --motion-popover 保持一致。 */
export const POPOVER_EXIT_MS = 200;

/** 提示卡位置与缩放原点的解算结果。 */
interface PlacedPosition extends CSSProperties {
  "--app-popover-origin"?: string;
}

interface AppTooltipProps {
  label: string;
  children: ReactNode;
}

export function tooltipPosition(trigger: Pick<DOMRect, "left" | "top" | "bottom">, panel: { width: number; height: number }, viewport: { width: number; height: number }) {
  const gap = 8;
  return {
    left: Math.max(gap, Math.min(trigger.left, viewport.width - panel.width - gap)),
    top: trigger.bottom + gap + panel.height <= viewport.height
      ? trigger.bottom + gap
      : Math.max(gap, trigger.top - panel.height - gap),
  };
}

/** 提示卡纵向展开方向：卡片在按钮下方贴上缘，在按钮上方贴下缘。 */
export function tooltipOriginY(trigger: Pick<DOMRect, "top" | "bottom">, panelTop: number) {
  return panelTop >= trigger.bottom ? "0%" : "100%";
}

/** 提示卡横向展开方向：取卡片左右边缘中离按钮中心更近的一侧。 */
export function tooltipOriginX(trigger: Pick<DOMRect, "left" | "right">, panelLeft: number, panelWidth: number) {
  const triggerCenter = (trigger.left + trigger.right) / 2;
  const panelCenter = panelLeft + panelWidth / 2;
  return panelCenter >= triggerCenter ? "0%" : "100%";
}

/** 与 MCP 差异说明同款的悬停卡片；Portal 避免被卡片的 overflow: hidden 裁切。 */
export function AppTooltip({ label, children }: AppTooltipProps) {
  const id = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [position, setPosition] = useState<PlacedPosition | null>(null);
  // 退出中仍保持节点在场，让 .app-popover 的出场动画播完再卸载；
  // 关闭途中重新悬停/聚焦会取消退出（下方 effect 依赖 open）。
  const [exiting, setExiting] = useState(false);
  const open = hovered || focused;
  const mounted = open || exiting;
  // 位置未解算完成时不播进场动画，避免首帧退回兜底原点
  const placed = position !== null;

  useEffect(() => {
    // 只管退出的存在态，不要在这里清 position：
    // 本 effect 是被动 effect，跑在上面的定位 useLayoutEffect 之后，
    // 清掉会把刚解算好的位置抹成 null，而定位 effect 依赖 [open] 不会重跑，
    // 卡片就永远停在 visibility: hidden。旧位置由定位 effect 每次打开时覆盖。
    if (open) { setExiting(false); return; }
    setExiting(true);
    const timer = setTimeout(() => setExiting(false), POPOVER_EXIT_MS);
    return () => clearTimeout(timer);
  }, [open]);

  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const trigger = triggerRef.current?.getBoundingClientRect();
      const panel = panelRef.current;
      if (!trigger || !panel) return;
      const pos = tooltipPosition(trigger, { width: panel.offsetWidth, height: panel.offsetHeight }, { width: window.innerWidth, height: window.innerHeight });
      // 原点必须按"解算后的落位"算，不能读 panel.offsetLeft/offsetTop：
      // 卡片此刻还没拿到 left/top（正是这次 setPosition 要写进去的），
      // 读自身偏移只会拿到未定位的值，横向原点因此算反、从错误的一侧展开。
      const box = { width: panel.offsetWidth, height: panel.offsetHeight };
      setPosition({
        ...pos,
        "--app-popover-origin": `${tooltipOriginX(trigger, pos.left, box.width)} ${tooltipOriginY(trigger, pos.top)}`,
      });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  return <>
    <button
      ref={triggerRef}
      type="button"
      className="grid h-6 w-6 shrink-0 cursor-help place-items-center text-(--text-secondary) transition-colors hover:text-accent focus:text-accent"
      title=""
      aria-label={label}
      aria-describedby={open ? id : undefined}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      onClick={(event) => { event.currentTarget.blur(); event.stopPropagation(); }}
    >
      <CircleQuestionMark className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
    </button>
    {mounted && typeof document !== "undefined" ? createPortal(
      <div
        ref={panelRef}
        id={id}
        role="tooltip"
        className="app-popover fixed z-50 w-80 max-w-[calc(100vw-1rem)] space-y-1.5 rounded-[var(--radius-control)] border border-(--panel-border) bg-(--panel-bg) p-3 shadow-lg"
        data-popover-in={open && placed ? "" : undefined}
        data-popover-out={open ? undefined : (placed ? "" : undefined)}
        style={position ?? { visibility: "hidden" }}
      >{children}</div>,
      document.body,
    ) : null}
  </>;
}
