import { useLayoutEffect, useRef, type RefObject } from "react";

export function useMenuExit(open: boolean, menuRef: RefObject<HTMLDivElement | null>) {
  const wasOpen = useRef(false);

  // Activity also runs layout-effect cleanup when a page is hidden.
  useLayoutEffect(() => () => { wasOpen.current = false; }, []);

  useLayoutEffect(() => {
    const closing = wasOpen.current && !open;
    wasOpen.current = open;
    const menu = menuRef.current;
    if (!closing || !menu || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    menu.setAttribute("data-popover-out", "");
    const clear = () => menu.removeAttribute("data-popover-out");
    const onAnimationEnd = (event: AnimationEvent) => {
      if (event.target === menu && event.animationName === "app-select-menu-out") clear();
    };
    menu.addEventListener("animationend", onAnimationEnd);
    menu.addEventListener("animationcancel", onAnimationEnd);
    return () => {
      clear();
      menu.removeEventListener("animationend", onAnimationEnd);
      menu.removeEventListener("animationcancel", onAnimationEnd);
    };
  }, [open, menuRef]);
}
