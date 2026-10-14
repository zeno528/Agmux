import type { ReactNode } from "react";
import { LoadingSpinner } from "./LoadingSpinner";

interface ManagementPageTitleProps {
  icon: ReactNode;
  title: string;
  count?: number;
  countLabel?: string;
  loadingLabel?: string;
  className?: string;
}

/** 插件、MCP、Skill 入口统一的标题 + 总数。 */
export function ManagementPageTitle({ icon, title, count, countLabel, loadingLabel, className = "" }: ManagementPageTitleProps) {
  // count 未就绪即在加载，调用方不必各自推导 loading。
  const loading = count === undefined;
  return (
    <span className={`management-page-title ${className}`}>
      {icon}
      <span className="apple-title management-page-title__label">
        <span>{title}</span>
        {loading ? <span className="text-accent" role="status" aria-label={loadingLabel}><LoadingSpinner size="md" /></span> : count !== undefined ? <span className="management-page-title__count" title={countLabel} aria-label={countLabel}><span aria-hidden="true">·</span><span>{count}</span></span> : null}
      </span>
    </span>
  );
}
