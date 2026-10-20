import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

/** 设置页与供应商分类共用的顶部导航，滑动指示线沿用全局样式。 */
export function AppTabBar<T extends string>({ items, selected, onSelect, label }: {
  items: readonly { id: T; label: ReactNode; icon?: ReactNode; count?: ReactNode }[];
  selected: T;
  onSelect: (id: T) => void;
  label: string;
}) {
  const tabBar = useRef<HTMLElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null);
  useLayoutEffect(() => {
    const button = tabBar.current?.querySelector<HTMLElement>('[aria-current="page"]');
    const text = button?.querySelector<HTMLElement>("span");
    if (button && text) setIndicator({ left: button.offsetLeft + text.offsetLeft + text.offsetWidth * 0.075, width: text.offsetWidth * 0.85 });
  }, [selected, items]);

  return <nav ref={tabBar} className="settings-tab-bar relative flex items-center gap-1" aria-label={label}>
    {indicator && <span className="settings-tab-indicator" style={{ transform: `translateX(${indicator.left}px)`, width: indicator.width }} aria-hidden="true" />}
    {items.map((item) => <button
      key={item.id}
      type="button"
      className={`settings-tab relative flex h-10 items-center gap-1.5 rounded-md px-3 transition-colors ${selected === item.id ? "text-[var(--text-primary)]" : "text-[var(--text-secondary)] hover:text-[var(--text-primary)]"}`}
      aria-current={selected === item.id ? "page" : undefined}
      onClick={() => onSelect(item.id)}
    >
      {item.icon}
      <span>{item.label}</span>
      {item.count}
    </button>)}
  </nav>;
}
