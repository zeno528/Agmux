import { ExternalLink, HardDrive, ShieldCheck, UsersRound } from "lucide-react";
import { useTranslation } from "react-i18next";
import { AppModal } from "../../components/AppModal";
import { chatgptLogo } from "../../icons";
import type { BrowserLoginStart } from "../../types";

interface AddAccountDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  busy: boolean;
  /** 非 null 表示浏览器授权进行中，弹窗切换到等待视图 */
  browserLogin: BrowserLoginStart | null;
  onStartLogin: () => void;
  onReopen: () => void;
  onCancel: () => void;
}

/** 添加 ChatGPT 账号的卡片式 OAuth 弹窗（视觉源：work/demo1.html 一比一复刻，样式见 style.css 的 oauth- 段）。 */
export function AddAccountDialog({ open, onOpenChange, busy, browserLogin, onStartLogin, onReopen, onCancel }: AddAccountDialogProps) {
  const { t } = useTranslation("settings");
  const features = [
    { icon: ShieldCheck, title: t("account.officialSignIn"), description: t("account.officialSignInDescription") },
    { icon: HardDrive, title: t("account.localCredentials"), description: t("account.localCredentialsDescription") },
    { icon: UsersRound, title: t("account.accountManagement"), description: t("account.accountManagementDescription") },
  ];

  return (
    <AppModal
      open={open}
      onOpenChange={onOpenChange}
      label={t("account.addAccountTitle")}
      dismissible={false}
      className="oauth-modal"
      closeClassName="app-dialog-close oauth-modal-close"
    >
      <section className="oauth-hero" aria-hidden="true">
        <div className="oauth-glow oauth-glow-left" />
        <div className="oauth-glow oauth-glow-right" />
        <div className="oauth-wave" />
        <div className="oauth-hero-visual">
          {chatgptLogo ? <img className="oauth-hero-logo" src={chatgptLogo} alt="" /> : null}
        </div>
      </section>
      <section className="oauth-content">
        {browserLogin ? (
          <div className="oauth-pending" aria-live="polite">
            <h2 className="oauth-content-title">
              <svg className="oauth-pending-spinner" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.25" strokeWidth="2.5" />
                <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
              </svg>
              <span>{t("account.waitingForAuthorization")}</span>
            </h2>
            <p>{t("account.authorizationPendingDescription")}</p>
            <div className="oauth-pending-actions">
              <button type="button" className="oauth-reopen-btn" onClick={onReopen}>
                <ExternalLink size={16} strokeWidth={2} aria-hidden="true" />
                <span>{t("account.reopenBrowser")}</span>
              </button>
              <button type="button" className="oauth-cancel-btn" onClick={onCancel}>{t("account.cancelLogin")}</button>
            </div>
          </div>
        ) : (
          <div className="oauth-intro">
            <h2 className="oauth-content-title">{t("account.addAccountTitle")}</h2>
            <div className="oauth-feature-list">
              {features.map(({ icon: Icon, title, description }) => (
                <div key={title} className="oauth-feature">
                  <div className="oauth-feature-icon" aria-hidden="true">
                    <Icon size={22} strokeWidth={2} />
                  </div>
                  <div className="oauth-feature-copy">
                    <h4>{title}</h4>
                    <p>{description}</p>
                  </div>
                </div>
              ))}
            </div>
            <div className="oauth-actions">
              <button type="button" className="oauth-auth-btn" disabled={busy} onClick={onStartLogin}>{t("account.signIn")}</button>
            </div>
          </div>
        )}
      </section>
    </AppModal>
  );
}
