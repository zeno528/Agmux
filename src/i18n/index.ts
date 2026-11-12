import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import { resolveLanguage, type AppLanguage } from "./resolve";
import zhCommon from "./locales/zh-CN/common";
import zhClaude from "./locales/zh-CN/claude";
import zhClaudeDesktop from "./locales/zh-CN/claudeDesktop";
import zhMcp from "./locales/zh-CN/mcp";
import zhPlugins from "./locales/zh-CN/plugins";
import zhProfiles from "./locales/zh-CN/profiles";
import zhSettings from "./locales/zh-CN/settings";
import zhSkills from "./locales/zh-CN/skills";
import zhUpdates from "./locales/zh-CN/updates";
import enCommon from "./locales/en-US/common";
import enClaude from "./locales/en-US/claude";
import enClaudeDesktop from "./locales/en-US/claudeDesktop";
import enMcp from "./locales/en-US/mcp";
import enPlugins from "./locales/en-US/plugins";
import enProfiles from "./locales/en-US/profiles";
import enSettings from "./locales/en-US/settings";
import enSkills from "./locales/en-US/skills";
import enUpdates from "./locales/en-US/updates";

export const defaultNS = "common";

export const resources = {
  "zh-CN": { common: zhCommon, claude: zhClaude, claudeDesktop: zhClaudeDesktop, mcp: zhMcp, plugins: zhPlugins, profiles: zhProfiles, settings: zhSettings, skills: zhSkills, updates: zhUpdates },
  "en-US": { common: enCommon, claude: enClaude, claudeDesktop: enClaudeDesktop, mcp: enMcp, plugins: enPlugins, profiles: enProfiles, settings: enSettings, skills: enSkills, updates: enUpdates },
} as const;

/**
 * 按设置值（可为 "system"）与系统语言切换界面语言。
 * 资源是静态打包的，`init` 同步完成，可在首绘前调用（见 AppShell 显示窗口前的调用点）。
 */
export function setupI18n(setting: string | undefined): AppLanguage {
  const lng = resolveLanguage(setting, navigator.language);
  if (!i18next.isInitialized) {
    void i18next.use(initReactI18next).init({
      lng,
      fallbackLng: "zh-CN",
      supportedLngs: ["zh-CN", "en-US"],
      resources,
      defaultNS,
      interpolation: { escapeValue: false },
    });
  } else if (i18next.language !== lng) {
    void i18next.changeLanguage(lng);
  }
  return lng;
}

export default i18next;
