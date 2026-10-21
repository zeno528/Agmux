import { lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import { LoaderCircle } from "lucide-react";
import { AppDialog } from "../../components/AppDialog";
import { HeroDoodles } from "../../components/HeroDoodles";
import { useAppUpdate } from "./AppUpdateProvider";

// 与 SkillsView 一致：markdown 渲染懒加载，不进主 bundle
const loadMarkdownPreview = () => import("react-markdown");
const MarkdownPreview = lazy(loadMarkdownPreview);

/** 更新检查发现日志后预热渲染模块；失败不影响更新检查，弹窗打开时仍会按原逻辑重试。 */
export function preloadUpdateNotesRenderer() {
  void loadMarkdownPreview().catch(() => undefined);
}

interface UpdateNotesDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** 确认式更新弹窗：先展示本次版本更新日志，用户在弹窗内确认后才下载安装。 */
export function UpdateNotesDialog({ open, onOpenChange }: UpdateNotesDialogProps) {
  const { update, installing, install } = useAppUpdate();
  const { t } = useTranslation("updates");
  if (!update) return null;
  return (
    <AppDialog
      open={open}
      onOpenChange={onOpenChange}
      title={`v${update.version}`}
      hero={
        <>
          <HeroDoodles patternId="cg-update-hero-doodles" />
          <img src="/logo.svg" alt="" className="app-logo h-14 w-14 shrink-0" aria-hidden="true" />
        </>
      }
      footer={
        <>
          <button type="button" className="apple-action-button" onClick={() => onOpenChange(false)}>
            {t("notice.later")}
          </button>
          <button type="button" className="apple-action-button app-button--primary" disabled={installing} onClick={() => void install()}>
            {installing ? <LoaderCircle className="h-4 w-4 animate-spin" strokeWidth={2} aria-hidden="true" /> : null}
            {installing ? t("notice.installing") : t("notice.updateNow")}
          </button>
        </>
      }
    >
      <Suspense fallback={<div className="grid place-items-center py-8"><LoaderCircle className="h-4 w-4 animate-spin text-[var(--text-secondary)]" strokeWidth={2} aria-hidden="true" /></div>}>
        <div className="skill-markdown-preview">
          {update.notes ? <MarkdownPreview>{update.notes}</MarkdownPreview> : <p className="muted">{t("notice.noNotes")}</p>}
        </div>
      </Suspense>
    </AppDialog>
  );
}
