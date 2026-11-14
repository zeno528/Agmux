import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";

interface AppModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 无标题弹窗的无障碍名称：以 aria-label 代替 Dialog.Title */
  label: string;
  /** 是否允许 ESC / 点击空白处关闭；有流程进行中的弹窗传 false，只能走关闭按钮（与确认弹窗的 AlertDialog 语义一致） */
  dismissible?: boolean;
  className?: string;
  closeClassName?: string;
  children: ReactNode;
}

/** 自由布局卡片弹窗基座：Radix 只负责遮罩、焦点圈定和 ESC 关闭，视觉完全交给 className 定制（区别于 AppDialog 的标题式骨架）。 */
export function AppModal({ open, onOpenChange, label, dismissible = true, className = "", closeClassName = "", children }: AppModalProps) {
  const { t } = useTranslation();
  const blockDismiss = (event: Event) => event.preventDefault();
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="app-dialog-overlay" />
        <Dialog.Content
          className={`app-dialog-panel ${className}`}
          aria-label={label}
          onInteractOutside={dismissible ? undefined : blockDismiss}
          onEscapeKeyDown={dismissible ? undefined : blockDismiss}
        >
          {children}
          <Dialog.Close aria-label={t("window.close")} className={closeClassName}>
            <X className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
          </Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
