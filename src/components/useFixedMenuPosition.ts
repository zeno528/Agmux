import { useLayoutEffect, useState, type RefObject } from "react";
import type { CSSProperties } from "react";

/** 菜单展示高度封顶（20rem），须与 style.css 里 .app-select-menu 的 max-height 保持一致。 */
export const MENU_MAX_HEIGHT = 320;

/// AppSelect 下拉与行内 ⋯ 菜单共用的 fixed 弹层定位：打开后按菜单真实展示高度
/// 判定下方空间，不够且上方更大时向上翻转，够则向下展开。
/// align "match" = 左缘对齐且默认同宽（下拉框）；"end" = 右缘对齐、宽度自适应（行菜单）。
/// 返回值里带 --app-popover-origin：浮层等比缩放的变换原点，贴着触发按钮那一侧的边角，
/// 菜单从按钮所在的角"胀开"，而不是一律从左上角。对齐方式与上下翻转共同决定该原点。
export function useFixedMenuPosition(
  open: boolean,
  trigger: HTMLElement | null,
  menuRef: RefObject<HTMLDivElement | null>,
  align: "match" | "end",
  menuWidth?: CSSProperties["width"],
): CSSProperties | undefined {
  // 定位结果除标准属性外还注入 --app-popover-origin（浮层缩放原点），
  // 该自定义属性不在 CSSProperties 里，用扩展类型承载，避免 setStyle 处断言
  type MenuPositionStyle = CSSProperties & { "--app-popover-origin"?: string };
  const [style, setStyle] = useState<MenuPositionStyle | undefined>(undefined);
  useLayoutEffect(() => {
    if (!open || !trigger) return;
    const updatePosition = () => {
      const menu = menuRef.current;
      if (!menu) return;
      const rect = trigger.getBoundingClientRect();
      const gap = 6;
      // 翻转判定用实际会展示的高度（CSS max-height 封顶后的值），而不是内容
      // 完整高度 scrollHeight：后者会高估需求，导致下方空间明明够却向上翻转
      const effectiveHeight = Math.min(menu.scrollHeight, MENU_MAX_HEIGHT);
      const below = window.innerHeight - rect.bottom - gap;
      const above = rect.top - gap;
      const openUp = below < effectiveHeight && above > below;
      const available = Math.max(0, openUp ? above : below);
      // 原点取"靠近触发按钮的角"：右缘对齐时贴右边，向下展开时贴上边。
      // 两者任一成立都说明菜单在按钮的另一侧展开，原点必须跟着换。
      const originY = openUp ? "100%" : "0%";
      const originX = align === "end" ? "100%" : "0%";
      setStyle({
        ...(align === "end"
          ? { left: "auto", right: `${window.innerWidth - rect.right}px` }
          : { left: `${rect.left}px`, width: menuWidth ?? `${rect.width}px` }),
        ...(openUp
          ? { top: "auto", bottom: `${window.innerHeight - rect.top + gap}px` }
          : { top: `${rect.bottom + gap}px`, bottom: "auto" }),
        maxHeight: `${Math.min(MENU_MAX_HEIGHT, available)}px`,
        "--app-popover-origin": `${originX} ${originY}`,
      });
    };
    updatePosition();
    window.addEventListener("resize", updatePosition);
    return () => window.removeEventListener("resize", updatePosition);
  }, [open, trigger, align, menuWidth]);
  return style;
}
