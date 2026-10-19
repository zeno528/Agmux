import { Activity, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ClientTargetSwitch, type ClientTarget } from "../../components/ClientTargetSwitch";
import ClaudePluginsView from "./ClaudePluginsView";
import CodexPluginsView from "./CodexPluginsView";
import PluginSearchInput from "./components/PluginSearchInput";
import type { AppState } from "../../types";

export type PluginHeaderState = { title: ReactNode; editing?: boolean };

/// 与 MCP 一样把页头留在父层，切换客户端时保留滑块实例。
/// 搜索框通栏放在正文首行（照看板层级：客户端维度 → 搜索 → 过滤 chips → 分区），作用于当前客户端的看板。
export default function PluginsView({ state, activationEpoch, popEpoch }: { state: AppState; activationEpoch: number; popEpoch: number }) {
  const { t } = useTranslation("plugins");
  const [target, setTarget] = useState<ClientTarget>("codex");
  const [claudeVisited, setClaudeVisited] = useState(false);
  const [query, setQuery] = useState("");
  const [header, setHeader] = useState<PluginHeaderState | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  return (
    <section className={header?.editing ? "contents" : "apple-scroll-page mx-auto w-full max-w-none"}>
      {!header?.editing ? (
        <header className="apple-page-bar flex-wrap justify-between gap-4">
          {header?.title}
          <div className="flex items-center gap-2">
            <ClientTargetSwitch
              key="target-switch"
              value={target}
              onChange={(next) => {
                if (next === target) return;
                if (contentRef.current) contentRef.current.scrollTop = 0;
                if (next === "claude") setClaudeVisited(true);
                setTarget(next);
              }}
              label={t("target.label")}
              labels={{ codex: t("target.codex"), claude: t("target.claude") }}
            />
          </div>
        </header>
      ) : null}
      <div ref={contentRef} className={header?.editing ? "contents" : "apple-edit-content"}>
        {/* 搜索只属于看板：详情/添加等子页（editing）不挂载 */}
        {header?.editing ? null : <PluginSearchInput value={query} onChange={setQuery} className="mb-4 w-full" />}
        <Activity mode={target === "codex" ? "visible" : "hidden"}>
          <CodexPluginsView state={state} activationEpoch={activationEpoch} popEpoch={popEpoch} query={query} onHeaderChange={setHeader} />
        </Activity>
        {claudeVisited ? (
          <Activity mode={target === "claude" ? "visible" : "hidden"}>
            <ClaudePluginsView popEpoch={popEpoch} query={query} onHeaderChange={setHeader} />
          </Activity>
        ) : null}
      </div>
    </section>
  );
}
