import { Bot } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { AppSwitch } from "../../components/AppSwitch";
import type { ClientSidebarView } from "../../types";
import { AgentToolHeader, CliCard, useCliManagement } from "./CliManagement";

export default function AgentToolsView({ active, desktopInstalled, hiddenClients, onClientVisibilityChange }: {
  active: boolean;
  desktopInstalled: boolean;
  hiddenClients: ReadonlySet<string>;
  onClientVisibilityChange: (client: ClientSidebarView, visible: boolean) => void;
}) {
  const { t } = useTranslation("common");
  const { t: tSettings } = useTranslation("settings");
  const codexCli = useCliManagement("codex", active);
  const claudeCli = useCliManagement("claude", active);
  const [desktopVersion, setDesktopVersion] = useState<string | null>(null);
  useEffect(() => {
    if (!active || !desktopInstalled) return;
    let cancelled = false;
    void api.claudeDesktopGetStatus().then(({ version }) => {
      if (!cancelled) setDesktopVersion(version);
    }).catch((error) => console.warn("Claude Desktop version read failed", error));
    return () => { cancelled = true; };
  }, [active, desktopInstalled]);
  const sidebarControl = (client: ClientSidebarView, name: string) => (
    <AppSwitch checked={!hiddenClients.has(client)} onCheckedChange={(visible) => onClientVisibilityChange(client, visible)} label={tSettings("cli.showInSidebar", { client: name })} />
  );
  const desktopName = t("nav.groupClaudeDesktop");
  const tools = [
    { installed: codexCli.status ? codexCli.status.installation !== "missing" : null, row: <CliCard key="codex" client="codex" management={codexCli} sidebarControl={sidebarControl("codexProfiles", t("nav.groupCodex"))} /> },
    { installed: claudeCli.status ? claudeCli.status.installation !== "missing" : null, row: <CliCard key="claude" client="claude" management={claudeCli} sidebarControl={sidebarControl("claudeProfiles", t("nav.groupClaude"))} /> },
    { installed: desktopInstalled, row: (
      <div key="claude-desktop" className="cli-card">
        <AgentToolHeader name={desktopName} icon="/claude.svg" />
        <div className="cli-card__revision"><span className="cli-card__version">{desktopInstalled && desktopVersion ? desktopVersion : "—"}</span></div>
        <div className="cli-card__controls">{sidebarControl("claudeDesktopProfiles", desktopName)}</div>
      </div>
    ) },
  ];
  const groups = [
    { installed: true, label: "cli.installed" },
    { installed: false, label: "cli.notInstalled" },
    { installed: null, label: "cli.installationUnknown" },
  ] as const;
  return <section className="apple-scroll-page mx-auto w-full max-w-none">
    <header className="apple-page-bar">
      <div className="management-page-title">
        <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><Bot className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" /></span>
        <h1 className="apple-title management-page-title__label"><span>{t("nav.agentTools")}</span></h1>
      </div>
    </header>
    <div className="apple-edit-content">
      <div className="apple-group agent-tools-list">
        <div className="cli-card cli-card--header meta-xs muted font-medium">
          <span>{tSettings("cli.name")}</span>
          <span>{tSettings("cli.version")}</span>
          <span className="cli-card__actions">{tSettings("cli.actions")}</span>
          <span className="cli-card__controls">{tSettings("cli.show")}</span>
        </div>
        <div className="apple-list-card">
          {groups.flatMap(({ installed, label }) =>
            tools.filter((tool) => tool.installed === installed).map(({ row }, index) => (
              <div key={row.key}>
                {index === 0 ? <h2 className="agent-tools-group-title setting-title">{tSettings(label)}</h2> : null}
                {row}
              </div>
            )),
          )}
        </div>
      </div>
    </div>
  </section>;
}
