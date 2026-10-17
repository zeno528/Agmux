import { AppSegmentedControl } from "./AppSegmentedControl";
import { SkillTargetLogo } from "./SkillTargetLogo";

export type ClientTarget = "codex" | "claude";

/** MCP 与插件页共用的客户端页签，滑块过渡由 AppSegmentedControl 管理。 */
export function ClientTargetSwitch({ value, onChange, label, labels, badges, className = "" }: {
  value: ClientTarget;
  onChange: (target: ClientTarget) => void;
  label: string;
  labels: Record<ClientTarget, string>;
  badges?: Partial<Record<ClientTarget, string | null>>;
  className?: string;
}) {
  return (
    <AppSegmentedControl
      className={`${className} h-[var(--toolbar-control-height)] shrink-0`.trim()}
      selectedIndex={value === "codex" ? 0 : 1}
      role="tablist"
      label={label}
    >
      {(["codex", "claude"] as const).map((target) => (
        <button
          key={target}
          type="button"
          role="tab"
          aria-selected={value === target}
          className="apple-action-button"
          title={label}
          onClick={() => onChange(target)}
        >
          <SkillTargetLogo target={target} variant="title" />
          {labels[target]}
          {badges?.[target] ? <span className="apple-count-badge" aria-hidden="true">{badges[target]}</span> : null}
        </button>
      ))}
    </AppSegmentedControl>
  );
}
