import { Network } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { isTauri } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { useProxyStatus } from "../../app/useProxyStatus";
import { AppSegmentedControl } from "../../components/AppSegmentedControl";
import type { Settings } from "../../types";
import { SettingsPanelSection } from "./SettingsSections";

const modes = ["auto", "off", "custom"] as const;

export function ProxySettings({ settings, saving, onSave }: {
  settings: Settings;
  saving: boolean;
  onSave: (patch: Partial<Settings>) => Promise<boolean>;
}) {
  const { t } = useTranslation("settings");
  const feedback = useFeedback();
  const [custom, setCustom] = useState(settings.proxy_mode === "custom");
  const [address, setAddress] = useState(settings.proxy_url);
  // 自动模式才订阅检测；共享 hook 带挂载/窗口激活刷新，缓存直出不闪。
  const status = useProxyStatus(isTauri && settings.proxy_mode === "auto");
  const input = useRef<HTMLInputElement>(null);
  const submitting = useRef(false);
  const addressOpen = custom || settings.proxy_mode === "custom";

  // Activity 重新显示会重建 effects，在绘制前丢弃未保存的草稿。
  useLayoutEffect(() => {
    setCustom(settings.proxy_mode === "custom");
    setAddress(settings.proxy_url);
  }, [settings.proxy_mode, settings.proxy_url]);

  useEffect(() => {
    if (custom && settings.proxy_mode !== "custom") input.current?.focus({ preventScroll: true });
  }, [custom, settings.proxy_mode]);

  const save = async (mode: Settings["proxy_mode"]) => {
    if (saving || submitting.current) return;
    const patch: Partial<Settings> = { proxy_mode: mode };
    if (mode === "custom") {
      const value = address.trim();
      try {
        const url = new URL(value);
        if (!["http:", "https:"].includes(url.protocol) || !url.hostname) throw new Error();
      } catch {
        feedback.error(t("proxy.invalid"));
        input.current?.focus();
        return;
      }
      patch.proxy_url = value;
    }
    if (mode === settings.proxy_mode && (mode !== "custom" || patch.proxy_url === settings.proxy_url)) return;
    submitting.current = true;
    try {
      if (await onSave(patch)) setCustom(mode === "custom");
    } finally { submitting.current = false; }
  };

  const selected = custom ? "custom" : settings.proxy_mode;
  const addressSaved = settings.proxy_mode === "custom" && address.trim() === settings.proxy_url;
  const description = custom && !addressSaved
    ? t("proxy.customHint")
    : settings.proxy_mode === "off" ? t("proxy.offDescription")
    : settings.proxy_mode === "custom" ? t("proxy.customDescription")
    : !isTauri ? t("proxy.previewDescription")
    : status?.error ? t("proxy.statusUnavailable")
    : !status ? t("proxy.autoDescription")
    : status.proxy ? t("proxy.autoProxy", { address: status.proxy }) : t("proxy.autoDirect");

  return (
    <SettingsPanelSection id="network" label={t("proxy.sectionTitle")}>
      <div className="apple-group px-[var(--gap-card-inline)]" aria-busy={saving}>
        <div className="flex items-center justify-between gap-4 py-4">
          <div className="flex min-w-0 flex-1 items-start gap-3">
            <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
              <Network size={18} strokeWidth={2} aria-hidden="true" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="setting-title">{t("proxy.title")}</div>
              <div className="setting-description mt-0.5 truncate" role="status" title={description}>{description}</div>
            </div>
          </div>
          <AppSegmentedControl
            className="h-9 w-52 shrink-0"
            selectedIndex={modes.indexOf(selected)}
            label={t("proxy.title")}
          >
            {modes.map((mode) => (
              <button
                key={mode}
                type="button"
                data-proxy-mode={mode}
                className="inline-flex h-full items-center justify-center text-sm font-normal"
                aria-pressed={selected === mode}
                disabled={saving}
                onClick={() => {
                  if (mode === "custom") {
                    setCustom(true);
                  } else {
                    setCustom(false);
                    setAddress(settings.proxy_url);
                    void save(mode);
                  }
                }}
              >{t(`proxy.${mode}`)}</button>
            ))}
          </AppSegmentedControl>
        </div>
        <div className={`apple-disclosure proxy-address-disclosure ${addressOpen ? "apple-disclosure--open" : ""}`}>
          <div className="apple-disclosure__content" aria-hidden={!addressOpen} inert={!addressOpen}>
            <div className="apple-disclosure__body">
              <div className="border-t border-[var(--panel-divider)] py-4">
                <label className="field-label mb-2 block" htmlFor="settings-proxy-address">{t("proxy.address")}</label>
                <div className="flex items-center gap-3">
                  <input
                    id="settings-proxy-address"
                    ref={input}
                    type="url"
                    className="app-input min-w-0 flex-1"
                    value={address}
                    placeholder="http://127.0.0.1:7890"
                    aria-label={t("proxy.address")}
                    title={t("proxy.customHint")}
                    spellCheck={false}
                    autoComplete="off"
                    disabled={saving || !addressOpen}
                    onChange={(event) => setAddress(event.target.value)}
                  />
                  <button
                    type="button"
                    className="apple-action-button app-button--primary shrink-0"
                    disabled={saving || !addressOpen || !address.trim() || addressSaved}
                    onClick={() => void save("custom")}
                  >{t(addressSaved ? "proxy.saved" : "proxy.save")}</button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </SettingsPanelSection>
  );
}
