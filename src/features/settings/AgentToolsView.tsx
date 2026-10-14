import { Bot } from "lucide-react";
import { useTranslation } from "react-i18next";
import { CliCard, useCliManagement } from "./CliManagement";

/// 侧栏顶层的 Agent 工具页：从设置页签迁出，内容与检测时机（进页才跑）不变。
export default function AgentToolsView({ active }: { active: boolean }) {
  const { t } = useTranslation("common");
  const codexCli = useCliManagement("codex", active);
  const claudeCli = useCliManagement("claude", active);
  return <section className="apple-scroll-page mx-auto w-full max-w-none">
    <header className="apple-page-bar">
      <div className="management-page-title">
        <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><Bot className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" /></span>
        <h1 className="apple-title management-page-title__label"><span>{t("nav.agentTools")}</span></h1>
      </div>
    </header>
    <div className="apple-edit-content flex flex-col gap-[var(--gap-section)]">
      <CliCard client="codex" management={codexCli} />
      <CliCard client="claude" management={claudeCli} />
    </div>
  </section>;
}
