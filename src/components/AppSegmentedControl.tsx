import { Children, type CSSProperties, type ReactNode } from "react";

/** 全局等宽分段控件：selectedIndex 与按钮的 aria-pressed（单选）或 aria-selected（页签）保持一致。 */
export function AppSegmentedControl({ selectedIndex, label, role = "group", className = "", children }: {
  selectedIndex: number;
  label: string;
  role?: "group" | "tablist";
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      className={`app-segmented-control ${className}`}
      role={role}
      aria-label={label}
      style={{ "--segment-count": Children.count(children), "--segment-index": Math.max(0, selectedIndex) } as CSSProperties}
    >
      {children}
    </div>
  );
}
