import { Folder, FolderPlus, Terminal, X } from "lucide-react";
import { useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, isTauri } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { useMenuDismiss } from "../../components/useMenuDismiss";
import type { Settings } from "../../types";

/** 列表上限：菜单按 MENU_MAX_HEIGHT 封顶，超出部分对用户没有价值。 */
const DIR_LIMIT = 8;

/** 只显示末级目录名，完整路径走 title —— 菜单宽度自适应，不适合铺长路径。 */
export function dirName(dir: string) {
  const parts = dir.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? dir;
}

/** 记为最近使用：去重后前插到最前，超出上限挤掉最旧的。 */
export function nextDirs(current: string[], dir: string) {
  return [dir, ...current.filter((item) => item !== dir)].slice(0, DIR_LIMIT);
}

/**
 * 供应商卡片上的终端入口：点开一个最近目录列表，选中即在该目录起一个绑定本供应商的
 * Claude Code 终端。列表存在应用设置里（全局一份，所有卡片共用），删除不确认。
 *
 * 列表只在首次展开时读取，不进冷启动路径。
 */
export default function ProfileTerminalMenu({ profileId }: { profileId: string }) {
  const { t } = useTranslation("profiles");
  const feedback = useFeedback();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [settings, setSettings] = useState<Settings | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuStyle = useFixedMenuPosition(open, triggerRef.current, menuRef, "end");
  useMenuDismiss(open, triggerRef, menuRef, setOpen);
  const dirs = settings?.claude_terminal_dirs ?? [];

  const saveDirs = async (next: string[]) => {
    if (!settings) return;
    const updated = { ...settings, claude_terminal_dirs: next };
    setSettings(updated);
    await api.saveSettings(updated);
  };

  const launch = async (dir: string) => {
    if (busy) return;
    setOpen(false);
    setBusy(true);
    try {
      await api.claudeOpenTerminal(profileId, dir);
      await saveDirs(nextDirs(dirs, dir));
    } catch (error) {
      feedback.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const pickAndLaunch = async () => {
    if (busy) return;
    setOpen(false);
    if (!isTauri) {
      feedback.error(t("terminal.desktopOnly"));
      return;
    }
    // 从上次用过的目录打开；目录已失效时插件会自行回落到父目录。
    const picked = await openDialog({
      title: t("terminal.pickDir"),
      directory: true,
      multiple: false,
      defaultPath: dirs[0],
    });
    const dir = typeof picked === "string" ? picked : null;
    if (!dir) return;
    await launch(dir);
  };

  const toggle = async () => {
    const next = !open;
    if (next && !settings) {
      try {
        setSettings(await api.getSettings());
      } catch (error) {
        feedback.error(String(error));
      }
    }
    setOpen(next);
  };

  const menu = open ? createPortal(
    <div ref={menuRef} className="app-select-menu app-popover" data-open="true" data-popover-in role="menu" aria-label={t("terminal.menuLabel")} style={{ ...menuStyle, minWidth: "14rem" }}>
      <button type="button" role="menuitem" className="app-select-option app-selection-state" onClick={() => void pickAndLaunch()}>
        <span className="flex items-center gap-2"><FolderPlus className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{t("terminal.new")}</span>
      </button>
      {dirs.length === 0 ? <p className="muted meta-xs px-2 py-1.5">{t("terminal.empty")}</p> : dirs.map((dir) => (
        <div key={dir} className="app-select-option app-selection-state min-w-0 gap-0.5! p-0! focus-within:bg-(--hover-bg)">
          <button type="button" role="menuitem" className="app-select-option min-w-0 flex-1 justify-start! gap-2!" title={dir} onClick={() => void launch(dir)}>
            <Folder className="h-4 w-4 shrink-0" strokeWidth={2} aria-hidden="true" />
            <span className="truncate">{dirName(dir)}</span>
          </button>
          <button type="button" className="apple-icon-button shrink-0 text-[var(--text-secondary)] hover:bg-transparent! focus-visible:bg-transparent! enabled:hover:text-[var(--danger)]" aria-label={t("terminal.remove")} title={t("terminal.remove")} onClick={() => void saveDirs(dirs.filter((item) => item !== dir)).catch((error) => feedback.error(String(error)))}>
            <X className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>,
    document.body,
  ) : null;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="apple-icon-button text-[var(--text-secondary)] enabled:hover:text-accent disabled:cursor-not-allowed disabled:opacity-40"
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={t("terminal.open")}
        title={t("terminal.open")}
        onClick={() => void toggle()}
      >
        <Terminal className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
      </button>
      {menu}
    </>
  );
}
