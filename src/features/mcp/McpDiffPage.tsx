import { ChevronLeft, Check, CircleAlert, GitCompare, Undo2 } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { AppTooltip } from "../../components/AppTooltip";
import { SkillTargetLogo } from "../../components/SkillTargetLogo";
import type { McpSyncDiffEntry, McpSyncPreview } from "../../types";
import type { McpDiffVerb, McpTarget } from "./McpView";

interface McpDiffPageProps {
  target: McpTarget;
  preview: McpSyncPreview | null;
  previewError: string;
  resolving: boolean;
  onBack: () => void;
  onResolve: (entry: McpSyncDiffEntry, verb: McpDiffVerb) => void;
  onResolveAll: (verb: McpDiffVerb) => void;
  onRebuild?: () => void;
}

const kindText = (entry: McpSyncDiffEntry, t: TFunction<"mcp">) => entry.kind === "live_only" ? t("diff.kind.liveOnly") : entry.kind === "db_only" ? t("diff.kind.dbOnly") : t("diff.kind.changed");
const detailFallback = (entry: McpSyncDiffEntry, t: TFunction<"mcp">) => entry.kind === "live_only" ? t("diff.detail.liveOnly") : entry.kind === "db_only" ? t("diff.detail.dbOnly") : t("diff.detail.changed");

export type McpDiffLineType = "same" | "del" | "add";

export interface McpDiffLine {
  type: McpDiffLineType;
  text: string;
}

/// 行级 diff（LCS）：对比数据库片段与 live 片段的每一行，按代码变更差异的粒度展示。
/// 片段首尾的空行直接忽略，避免机器格式化抖动产生伪变更行和顶部空档。
export function mcpDiffLines(dbToml: string, liveToml: string): McpDiffLine[] {
  const db = dbToml.replace(/\r\n/g, "\n").split("\n");
  const live = liveToml.replace(/\r\n/g, "\n").split("\n");
  while (db.length && db[db.length - 1] === "") db.pop();
  while (live.length && live[live.length - 1] === "") live.pop();
  while (db.length && db[0] === "") db.shift();
  while (live.length && live[0] === "") live.shift();
  const lcs: number[][] = Array.from({ length: db.length + 1 }, () => new Array<number>(live.length + 1).fill(0));
  for (let r = db.length - 1; r >= 0; r--) {
    for (let c = live.length - 1; c >= 0; c--) {
      lcs[r][c] = db[r] === live[c] ? lcs[r + 1][c + 1] + 1 : Math.max(lcs[r + 1][c], lcs[r][c + 1]);
    }
  }
  const lines: McpDiffLine[] = [];
  let r = 0;
  let c = 0;
  while (r < db.length && c < live.length) {
    if (db[r] === live[c]) { lines.push({ type: "same", text: db[r] }); r++; c++; }
    else if (lcs[r + 1][c] >= lcs[r][c + 1]) { lines.push({ type: "del", text: db[r] }); r++; }
    else { lines.push({ type: "add", text: live[c] }); c++; }
  }
  while (r < db.length) { lines.push({ type: "del", text: db[r] }); r++; }
  while (c < live.length) { lines.push({ type: "add", text: live[c] }); c++; }
  return lines;
}

/** 行级变更行：红行=数据库旧内容，绿行=config.toml 新内容，未涉及行保持正文样式。 */
function McpDiffDetail({ entry }: { entry: McpSyncDiffEntry }) {
  const { t } = useTranslation("mcp");
  let content: ReactNode;
  if (entry.kind === "changed") {
    content = (
      <div className="mcp-diff-block">
        {mcpDiffLines(entry.db_toml ?? "", entry.live_toml ?? "").map((line, index) => (
          <div key={index} className={`${line.type === "same" ? "" : `mcp-diff-line--${line.type} `}mono meta-xs whitespace-pre-wrap break-all`}>{line.text || " "}</div>
        ))}
      </div>
    );
  } else if (entry.live_toml?.trim() || entry.db_toml?.trim()) {
    content = <pre className={`mcp-diff-line--${entry.kind === "live_only" ? "add" : "del"} mono meta-xs m-0 whitespace-pre-wrap break-all`}>{entry.live_toml ?? entry.db_toml}</pre>;
  } else {
    content = <p className="muted m-0 text-sm">{detailFallback(entry, t)}</p>;
  }
  return <div className="px-3 pb-2">{content}</div>;
}

interface McpDiffRowProps {
  entry: McpSyncDiffEntry;
  busy: boolean;
  onResolve: McpDiffPageProps["onResolve"];
}

/** 与 MCP 列表卡片同构，差异内容始终展开。 */
function McpDiffRow({ entry, busy, onResolve }: McpDiffRowProps) {
  const { t } = useTranslation("mcp");
  return (
    <div className="apple-group">
      <div className="apple-list-row mcp-expanded-card__header">
        <span className={`apple-chip ${entry.kind === "live_only" ? "chip-warn" : "chip-danger"}`}>{kindText(entry, t)}</span>
        <span className="min-w-0 flex-1 truncate font-semibold">{entry.name}</span>
        <div className="flex shrink-0 items-center gap-1.5">
          <button type="button" className="apple-action-button mcp-diff-action-button text-[var(--text-secondary)] enabled:hover:text-accent disabled:cursor-not-allowed disabled:opacity-40" disabled={busy} title={t("diff.revertAria", { name: entry.name })} aria-label={t("diff.revertAria", { name: entry.name })} onClick={() => onResolve(entry, "revert")}>
            <Undo2 className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
            {t("diff.revert")}
          </button>
          <button type="button" className="apple-action-button mcp-diff-action-button app-button--primary enabled:hover:text-accent disabled:cursor-not-allowed disabled:opacity-40" disabled={busy} title={t("diff.adoptAria", { name: entry.name })} aria-label={t("diff.adoptAria", { name: entry.name })} onClick={() => onResolve(entry, "adopt")}>
            <Check className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
            {t("diff.adopt")}
          </button>
        </div>
      </div>
      <McpDiffDetail entry={entry} />
    </div>
  );
}

export default function McpDiffPage({ target, preview, previewError, resolving, onBack, onResolve, onResolveAll, onRebuild }: McpDiffPageProps) {
  const { t } = useTranslation("mcp");
  const entries = preview?.entries ?? [];
  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-edit-toolbar apple-edit-toolbar--header justify-between">
        <div className="flex min-w-0 items-center gap-2">
          <button type="button" className="apple-page-header apple-back-button" onClick={onBack}>
            <ChevronLeft className="h-4 w-4 shrink-0 text-accent" />
            <SkillTargetLogo target={target} variant="title" />
            <span className="apple-title">{t("diff.title")}</span>
            {!previewError ? <span className="apple-chip">{entries.length}</span> : null}
          </button>
          <AppTooltip label={t("diff.help.title")}>
              <div className="field-subtitle">{t("diff.help.title")}</div>
              <p className="muted meta-xs"><strong className="font-semibold text-(--danger)">{t("diff.help.red.keyword")}</strong>{t("diff.help.red.text")}</p>
              <p className="muted meta-xs"><strong className="font-semibold text-(--success)">{t("diff.help.green.keyword")}</strong>{t("diff.help.green.text")}</p>
              <p className="muted meta-xs"><strong className="font-semibold">{t("diff.help.adopt.keyword")}</strong>{t("diff.help.adopt.text")}</p>
              <p className="muted meta-xs"><strong className="font-semibold">{t("diff.help.revert.keyword")}</strong>{t("diff.help.revert.text")}</p>
          </AppTooltip>
        </div>
      </div>
      <div className="apple-edit-content">
        {previewError ? (
          <div className="apple-group p-4">
            {/* 图标放标题右侧：标题、描述、按钮、报错盒子左边缘对齐，不留悬挂缩进 */}
            <div className="flex items-center gap-2">
              <div className="setting-title">{t("diff.parseFailedTitle")}</div>
              <CircleAlert className="h-[18px] w-[18px] shrink-0 text-[var(--danger)]" strokeWidth={2} aria-hidden="true" />
            </div>
            <p className="setting-description mt-1">{t(onRebuild ? "diff.parseFailedImpact" : "diff.claudeParseFailedImpact")}</p>
            {onRebuild ? <button type="button" className="apple-action-button app-button--primary mt-3" disabled={resolving} onClick={onRebuild}>{t("diff.rebuild")}</button> : null}
            {/* 报错原文是唯一能定位到行的信息，默认摊开——这页就是为它存在的 */}
            <div className="field-label mt-3">{t("diff.rawError")}</div>
            <pre className="mono meta-xs muted m-0 mt-1.5 whitespace-pre-wrap break-all rounded-[var(--radius-control)] border border-[var(--panel-border)] bg-(--profile-chip-bg) px-2.5 py-2">{previewError}</pre>
          </div>
        ) : entries.length ? (
          <div className="space-y-2">
            {entries.map((entry) => (
              <McpDiffRow key={entry.name} entry={entry} busy={resolving} onResolve={onResolve} />
            ))}
          </div>
        ) : (
          <EmptyStateCard icon={<GitCompare className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("diff.resolved")}</p>
          </EmptyStateCard>
        )}
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        <button type="button" className="apple-action-button" disabled={resolving || !entries.length} onClick={() => onResolveAll("revert")}>{t("diff.revertAll")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={resolving || !entries.length} onClick={() => onResolveAll("adopt")}>{t("diff.adoptAll")}</button>
      </div>
    </section>
  );
}
