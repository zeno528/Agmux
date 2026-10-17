import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ClientTargetSwitch, type ClientTarget } from "../../components/ClientTargetSwitch";
import ClaudePluginsView from "./ClaudePluginsView";
import CodexPluginsView from "./CodexPluginsView";
import PluginSearchInput from "./components/PluginSearchInput";
import type { AppState } from "../../types";

export type PluginHeaderState = { title: ReactNode; actions: ReactNode; editing?: boolean };

/// 与 MCP 一样把页头留在父层，切换客户端时保留滑块实例。
export default function PluginsView({ state, popEpoch }: { state: AppState; popEpoch: number }) {
  const { t } = useTranslation("plugins");
  const [target, setTarget] = useState<ClientTarget>("codex");
  const [query, setQuery] = useState("");
  const [header, setHeader] = useState<PluginHeaderState | null>(null);
  return (
    <section className={header?.editing ? "contents" : "apple-scroll-page mx-auto w-full max-w-none"}>
      {!header?.editing ? (
        <header className="apple-page-bar flex-wrap justify-between gap-4">
          {header?.title}
          <div className="flex items-center gap-2">
            <PluginSearchInput value={query} onChange={setQuery} />
            <ClientTargetSwitch
              key="target-switch"
              value={target}
              onChange={setTarget}
              label={t("target.label")}
              labels={{ codex: t("target.codex"), claude: t("target.claude") }}
            />
            {header?.actions}
          </div>
        </header>
      ) : null}
      <div className={header?.editing ? "contents" : "apple-edit-content"}>
        {target === "codex"
          ? <CodexPluginsView state={state} popEpoch={popEpoch} query={query} onHeaderChange={setHeader} />
          : <ClaudePluginsView popEpoch={popEpoch} query={query} onHeaderChange={setHeader} />}
      </div>
    </section>
  );
}
