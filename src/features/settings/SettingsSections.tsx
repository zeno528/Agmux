import { AppWindowMac, Blocks, BookOpenText, Bot, ChevronRight, CircleUserRound, Clock, Database, Download, FileCog, FileText, FolderOpen, History, Languages, LayoutDashboard, LoaderCircle, Moon, MoonStar, Monitor, MoreHorizontal, Palette, PanelBottomClose, Pencil, Power, RefreshCw, RotateCcw, Save, Settings as SettingsIcon, Sun, Trash2, Upload } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { createPortal } from "react-dom";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api, isMacWindow, isTauri } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedDatabaseBackups, loadDatabaseBackups } from "../../app/managementDataCache";
import { AppDialog } from "../../components/AppDialog";
import { AppDisclosure } from "../../components/AppDisclosure";
import { GithubMark } from "../../components/GithubMark";
import { McpIcon } from "../../components/McpIcon";
import { AppSelect } from "../../components/AppSelect";
import { AppSegmentedControl } from "../../components/AppSegmentedControl";
import { AppSwitch } from "../../components/AppSwitch";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { updateFailureMessage } from "../updates/updateText";
import { useAppUpdate, releaseNotesUrl } from "../updates/AppUpdateProvider";
import { UpdateNotesDialog } from "../updates/UpdateNotesDialog";
import type { DatabaseBackupInfo, PathInfo, Settings } from "../../types";
import version from "../../../VERSION?raw";

// as const 必需：labelKey 若推断为 string，typed t() 会拒绝（键名无法在编译期校验）。
const themeOptions = [{ labelKey: "theme.system", value: "system" }, { labelKey: "theme.light", value: "light" }, { labelKey: "theme.dark", value: "dark" }] as const;
const languageOptions = [{ labelKey: "language.system", value: "system" }, { labelKey: "language.zh", value: "zh-CN" }, { labelKey: "language.en", value: "en-US" }] as const;
// 启动页选项 = 侧栏全部页面平铺（与侧栏清单同序同文案 key，新增页面两处同步）
const startupViewOptions = [
  { labelKey: "nav.groupCodex", value: "codexProfiles", icon: <img src="/codex.svg" alt="" className="h-4 w-4 shrink-0" /> },
  { labelKey: "nav.groupClaude", value: "claudeProfiles", icon: <img src="/claude-code.svg" alt="" className="h-4 w-4 shrink-0" /> },
  { labelKey: "nav.plugins", value: "plugins", icon: <Blocks className="h-4 w-4 shrink-0" aria-hidden="true" /> },
  { labelKey: "nav.accounts", value: "accounts", icon: <CircleUserRound className="h-4 w-4 shrink-0" aria-hidden="true" /> },
  { labelKey: "nav.mcp", value: "mcp", icon: <McpIcon className="h-4 w-4 shrink-0" /> },
  { labelKey: "nav.skills", value: "skills", icon: <BookOpenText className="h-4 w-4 shrink-0" aria-hidden="true" /> },
  { labelKey: "nav.agentTools", value: "agentTools", icon: <Bot className="h-4 w-4 shrink-0" aria-hidden="true" /> },
  { labelKey: "nav.settings", value: "settings", icon: <SettingsIcon className="h-4 w-4 shrink-0" aria-hidden="true" /> },
] as const;
// Rust 下发的 PathInfo.label（i18n key）数据值，用于比对而非展示
const BACKUP_DIR_LABEL = "about.paths.backups"; // i18n-exempt: 数据标签比较值，不是界面文案

// PathInfo.label 约定为 about.paths.* i18n key（Rust path_info 下发）。强类型 t 只收
// 已知 key，动态值经此处受控断言；约定外的取值原样回退，不把脏数据渲染成字面 key
const pathLabel = (t: (key: "about.paths.appData") => string, label: string): string =>
  label.startsWith("about.paths.") ? t(label as "about.paths.appData") : label;

const pad2 = (value: number) => String(value).padStart(2, "0");
export const backupTitle = (name: string) => name.replace(/^(?:agmux-(?:backup-)?|cg-backup-|budtty-export-)/, "").replace(/\.db$/, "");
// 自动备份文件名剥前缀后是纯时间戳（20260822-120000-000），手动「立即备份」带 manual- 标记，
// 重命名过的是自由文本；行标题的"自动/手动"前缀据此区分
export const isAutoBackupName = (name: string) => /^\d{8}-\d{6}-\d{3}$/.test(backupTitle(name));
export const isManualBackupName = (name: string) => /^manual-\d{8}-\d{6}-\d{3}$/.test(backupTitle(name));
export const formatSize = (bytes: number) => bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(2)} MB`;
// 备份记录折叠状态的会话内记忆：切分页重挂载不丢，重启回到默认折叠（不持久化）。
// 默认折叠：记录列表随自动备份只增不减，展开态会把设置页越顶越长
let recordsOpenSession = false;
export const formatTimestamp = (seconds: number) => { const date = new Date(seconds * 1000); return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} ${pad2(date.getHours())}:${pad2(date.getMinutes())}`; };
// 自动备份行标题里的短时间（MM-DD HH:mm）：完整时间在行的 meta 里，标题只做扫读锚点
export const formatShortTimestamp = (seconds: number) => { const date = new Date(seconds * 1000); return `${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} ${pad2(date.getHours())}:${pad2(date.getMinutes())}`; };

// 设置页各分区统一的左上角标题包装：标题 id 与 aria-labelledby 成对生成
export function SettingsPanelSection({ id, label, children }: { id: string; label: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2" aria-labelledby={`settings-content-${id}`}>
      <h2 id={`settings-content-${id}`} className="title-sm px-1">
        {label}
      </h2>
      {children}
    </section>
  );
}

interface SettingsGeneralProps { form: Settings; onPatch: (patch: Partial<Settings>) => void; }

export function SettingsGeneral({ form, onPatch }: SettingsGeneralProps) {
  const { t } = useTranslation("settings");
  // 启动页选项文案住在 common/nav（与侧栏同一份），单挂 settings ns 取不到
  const { t: tNav } = useTranslation("common");
  return (
    <div className="flex flex-col gap-[var(--gap-section)]">
      <SettingsPanelSection id="appearance-language" label={t("general.appearanceGroupTitle")}>
        <div className="apple-group px-[var(--gap-card-inline)]">
          <div className="flex flex-col divide-y divide-[var(--panel-divider)]">
            <div className="flex items-center justify-between gap-4 py-4">
              <div className="flex min-w-0 items-start gap-3">
                <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                  <Palette className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
                </span>
                <div className="min-w-0">
                  <div className="setting-title">{t("appearance.title")}</div>
                  <div className="setting-description mt-0.5">{t("appearance.description")}</div>
                </div>
              </div>
              <AppSegmentedControl
                className="h-9 w-72 shrink-0"
                selectedIndex={themeOptions.findIndex((option) => option.value === form.theme)}
                label={t("appearance.title")}
              >
                {themeOptions.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    className="inline-flex h-full items-center justify-center gap-1.5 text-sm font-normal"
                    aria-pressed={form.theme === option.value}
                    onClick={() => onPatch({ theme: option.value })}
                  >
                    {option.value === "system" ? <Monitor className="h-4 w-4" strokeWidth={2} /> : option.value === "light" ? <Sun className="h-4 w-4" strokeWidth={2} /> : <Moon className="h-4 w-4" strokeWidth={2} />}
                    {t(option.labelKey)}
                  </button>
                ))}
              </AppSegmentedControl>
            </div>
            <div className="flex items-center justify-between gap-4 py-4">
              <div className="flex min-w-0 items-start gap-3">
                <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                  <Languages className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
                </span>
                <div className="min-w-0">
                  <div className="setting-title">{t("language.title")}</div>
                  <div className="setting-description mt-0.5">{t("language.description")}</div>
                </div>
              </div>
              <div className="w-72 shrink-0">
                <AppSelect
                  value={form.language}
                  options={languageOptions.map((option) => ({ label: t(option.labelKey), value: option.value }))}
                  onChange={(value) => onPatch({ language: value })}
                />
              </div>
            </div>
          </div>
        </div>
      </SettingsPanelSection>
      <SettingsPanelSection id="startup" label={t("general.startupGroupTitle")}>
        <div className="apple-group px-[var(--gap-card-inline)]">
          <div className="flex flex-col divide-y divide-[var(--panel-divider)]">
            {[
              ["autostart_enabled", t("startup.autostartTitle"), t("startup.autostartDescription"), Power],
              ["silent_start", t("startup.silentTitle"), t("startup.silentDescription"), MoonStar],
              ["minimize_to_tray", t("startup.minimizeTitle"), t("startup.minimizeDescription"), PanelBottomClose],
            ].map(([key, label, description, Icon]) => (
              <div key={String(key)} className="flex items-center justify-between gap-4 py-4">
                <div className="flex items-start gap-3">
                  <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                    <Icon className="h-[18px] w-[18px]" strokeWidth={2} />
                  </span>
                  <div>
                    <div className="setting-title">{String(label)}</div>
                    <div className="setting-description mt-0.5">{String(description)}</div>
                  </div>
                </div>
                <AppSwitch
                  checked={Boolean(form[key as keyof Settings])}
                  onCheckedChange={(value) => onPatch({ [String(key)]: value })}
                />
              </div>
            ))}
            <div className="flex items-center justify-between gap-4 py-4">
              <div className="flex min-w-0 items-start gap-3">
                <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                  <LayoutDashboard className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
                </span>
                <div className="min-w-0">
                  <div className="setting-title">{t("startup.startupViewTitle")}</div>
                  <div className="setting-description mt-0.5">{t("startup.startupViewDescription")}</div>
                </div>
              </div>
              <div className="w-72 shrink-0">
                <AppSelect
                  value={form.startup_view}
                  options={startupViewOptions.map((option) => ({ label: tNav(option.labelKey), value: option.value }))}
                  renderLabel={(option) => (
                    <span className="inline-flex min-w-0 items-center gap-2">
                      {startupViewOptions.find((item) => item.value === option.value)?.icon}
                      <span className="truncate">{option.label}</span>
                    </span>
                  )}
                  onChange={(value) => onPatch({ startup_view: value })}
                />
              </div>
            </div>
            {isMacWindow ? (
              <div className="flex items-center justify-between gap-4 py-4">
                <div className="flex min-w-0 items-start gap-3">
                  <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                    <AppWindowMac className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
                  </span>
                  <div className="min-w-0">
                    <div className="setting-title">{t("startup.terminalAppTitle")}</div>
                    <div className="setting-description mt-0.5">{t("startup.terminalAppDescription")}</div>
                  </div>
                </div>
                <div className="w-72 shrink-0">
                  <AppSelect
                    value={form.macos_terminal}
                    options={[{ label: "Terminal.app", value: "terminal" }, { label: "Ghostty", value: "ghostty" }]}
                    renderLabel={(option) => (
                      <span className="inline-flex min-w-0 items-center gap-2">
                        <img src={option.value === "ghostty" ? "/ghostty.png" : "/terminal-app.png"} alt="" className="h-5 w-5 shrink-0 object-contain" />
                        <span className="truncate">{option.label}</span>
                      </span>
                    )}
                    onChange={(value) => onPatch({ macos_terminal: value })}
                  />
                </div>
              </div>
            ) : null}
          </div>
        </div>
      </SettingsPanelSection>
    </div>
  );
}

interface SettingsAdvancedProps { form: Settings; onPatch: (patch: Partial<Settings>) => void; paths: PathInfo[]; backupsEpoch: number; onOpenPath: (item: PathInfo) => void; openingPath: string | null; onRefresh: () => Promise<void>; }

export function SettingsAdvanced({ form, onPatch, paths, backupsEpoch, onOpenPath, openingPath, onRefresh }: SettingsAdvancedProps) {
  const feedback = useFeedback();
  const { t } = useTranslation("settings");
  // 开关负责自动备份的启停（关 = interval 0），下拉只管间隔，不再提供"关闭"档
  const autoBackupOptions = [{ label: t("backup.interval6h"), value: 6 }, { label: t("backup.interval12h"), value: 12 }, { label: t("backup.interval24h"), value: 24 }, { label: t("backup.interval48h"), value: 48 }, { label: t("backup.interval7d"), value: 168 }];
  const keepOptions = [3, 5, 10, 15, 20, 30].map((value) => ({ label: t("backup.keepCount", { count: value }), value }));
  // null = 尚未加载过：首帧无缓存时不渲染"还没有备份记录"空态，防切分页闪现
  const [backups, setBackups] = useState<DatabaseBackupInfo[] | null>(getCachedDatabaseBackups);
  const [exporting, setExporting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [renameTarget, setRenameTarget] = useState<DatabaseBackupInfo | null>(null);
  const [renameText, setRenameText] = useState("");
  const [renaming, setRenaming] = useState(false);
  const [recordsOpen, setRecordsOpen] = useState(recordsOpenSession);

  // 进页先缓存直出再静默刷新（CLAUDE.md 管理页数据约定）；本地增删改/epoch 变更走 force
  const loadBackups = async (force = false) => { try { setBackups(await loadDatabaseBackups(force)); } catch { setBackups(null); } };
  useEffect(() => { void loadBackups(true); }, [backupsEpoch]);

  const exportBackupToFile = async () => {
    if (exporting) return;
    setExporting(true);
    try { let directory: string | null = null; if (isTauri) { const result = await openDialog({ title: t("backup.pickExportDir"), directory: true, multiple: false }); directory = typeof result === "string" ? result : null; if (!directory) return; } await api.exportDatabaseTo(directory ?? "mock-export"); feedback.success(t("backup.toastExported")); }
    catch (error) { feedback.error(String(error)); }
    finally { setExporting(false); }
  };
  const importBackupFromFile = async () => {
    if (importing) return;
    try { let picked: string | null = null; if (isTauri) { const result = await openDialog({ title: t("backup.pickImportFile"), multiple: false, filters: [{ name: t("backup.sqliteFilter"), extensions: ["db"] }] }); picked = typeof result === "string" ? result : null; if (!picked) return; } setImporting(true); await api.importDatabase(picked ?? "mock-backup.db"); feedback.success(t("backup.toastImported")); await onRefresh(); await loadBackups(true); }
    catch (error) { feedback.error(String(error)); }
    finally { setImporting(false); }
  };
  const createImmediateBackup = async () => { if (exporting) return; setExporting(true); try { await api.exportDatabase(); feedback.success(t("backup.toastCreated")); await loadBackups(true); } catch (error) { feedback.error(String(error)); } finally { setExporting(false); } };
  const openRename = (backup: DatabaseBackupInfo) => { setRenameTarget(backup); setRenameText(backupTitle(backup.name)); };
  const submitRename = async () => { const target = renameTarget; const text = renameText.trim(); if (!target || renaming || !text || text === backupTitle(target.name)) { setRenameTarget(null); return; } setRenaming(true); try { await api.renameDatabaseBackup(target.name, text); feedback.success(t("backup.toastRenamed")); await loadBackups(true); setRenameTarget(null); } catch (error) { feedback.error(String(error)); } finally { setRenaming(false); } };
  const restoreBackup = async (backup: DatabaseBackupInfo) => { if (!await feedback.confirm({ title: t("backup.restoreConfirmTitle"), description: t("backup.restoreConfirm", { name: backup.name }), confirmText: t("backup.restore"), destructive: true })) return; try { await api.restoreDatabase(backup.name); feedback.success(t("backup.toastRestored")); await onRefresh(); await loadBackups(true); } catch (error) { feedback.error(String(error)); } };
  const deleteBackup = async (backup: DatabaseBackupInfo) => { if (!await feedback.confirm({ title: t("backup.deleteConfirmTitle"), description: <Trans ns="settings" i18nKey="backup.deleteConfirm" values={{ name: backup.name }} components={{ strong: <strong /> }} />, confirmText: t("backup.delete"), destructive: true })) return; try { await api.deleteDatabaseBackup(backup.name); feedback.success(t("backup.toastDeleted")); await loadBackups(true); } catch (error) { feedback.error(String(error)); } };

  // ⋯ 菜单：apple-group 是 overflow:hidden，行内 absolute 弹层会被裁掉，
  // 走 portal + fixed 定位，向下/向上自适应翻转复用 AppSelect 的共享逻辑；
  // 外点 / 滚动即收起
  const [menuTarget, setMenuTarget] = useState<{ backup: DatabaseBackupInfo; trigger: HTMLElement } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuStyle = useFixedMenuPosition(menuTarget !== null, menuTarget?.trigger ?? null, menuRef, "end");
  const openMenu = (backup: DatabaseBackupInfo, trigger: HTMLElement) => {
    setMenuTarget({ backup, trigger });
  };
  useEffect(() => {
    if (!menuTarget) return;
    const close = (event: Event) => {
      if (event.target instanceof Element && event.target.closest("[data-row-menu], [data-row-menu-trigger]")) return;
      setMenuTarget(null);
    };
    document.addEventListener("pointerdown", close);
    window.addEventListener("scroll", close, true);
    return () => { document.removeEventListener("pointerdown", close); window.removeEventListener("scroll", close, true); };
  }, [menuTarget]);
  const rowMenu = menuTarget ? createPortal(
    <div ref={menuRef} className="app-select-menu app-popover" data-open="true" data-popover-in data-row-menu role="menu" aria-label={t("backup.moreTooltip")} style={{ ...menuStyle, minWidth: "10rem" }} onKeyDown={(event) => { if (event.key === "Escape") setMenuTarget(null); }}>
      <button type="button" role="menuitem" className="app-select-option app-selection-state" onClick={() => { const target = menuTarget.backup; setMenuTarget(null); openRename(target); }}>
        <span className="flex items-center gap-2"><Pencil className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{t("backup.renameAction")}</span>
      </button>
      <button type="button" role="menuitem" className="app-select-option app-selection-state app-select-option--danger" onClick={() => { const target = menuTarget.backup; setMenuTarget(null); void deleteBackup(target); }}>
        <span className="flex items-center gap-2"><Trash2 className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{t("backup.deleteAction")}</span>
      </button>
    </div>,
    document.body,
  ) : null;
  const recordsSummary = backups !== null && backups.length > 0 ? t("backup.recordSummary", { count: backups.length, size: formatSize(backups.reduce((total, backup) => total + backup.size_bytes, 0)) }) : "";

  return (
    <div className="flex flex-col gap-[var(--gap-section)]">
      <SettingsPanelSection id="backup" label={t("backup.sectionTitle")}>
        <div className="apple-group px-[var(--gap-card-inline)]">
          <div className="flex flex-col divide-y divide-[var(--panel-divider)]">
            {/* 数据备份：标题 + 整排操作按钮 */}
            <div className="flex flex-col gap-4 py-4">
              <div className="flex items-start gap-3">
                <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                  <Database className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
                </span>
                <div className="min-w-0">
                  <div className="setting-title">{t("backup.title")}</div>
                  <div className="setting-description mt-0.5">{t("backup.description")}</div>
                </div>
              </div>
              <div className="flex flex-wrap gap-2">
                <button type="button" className="apple-action-button app-button--primary flex-1" disabled={exporting} onClick={() => void createImmediateBackup()}>
                  <Save className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                  {t("backup.createNow")}
                </button>
                <button type="button" className="apple-action-button flex-1" disabled={importing} onClick={() => void importBackupFromFile()}>
                  <Download className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                  {t("backup.import")}
                </button>
                <button type="button" className="apple-action-button flex-1" disabled={exporting} onClick={() => void exportBackupToFile()}>
                  <Upload className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                  {t("backup.export")}
                </button>
                <button type="button" className="apple-action-button flex-1" onClick={() => { const item = paths.find((path) => path.label === BACKUP_DIR_LABEL); if (item) onOpenPath(item); else feedback.warning(t("backup.toastFolderMissing")); }}>
                  <FolderOpen className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                  {t("backup.folder")}
                </button>
              </div>
            </div>
            {/* 自动备份：开关在右侧控制启停，关掉时频率/保留两个下拉都不可用 */}
            <div className="flex flex-col gap-4 py-4">
              <div className="flex items-center justify-between gap-4">
                <div className="min-w-0">
                  <div className="setting-title">{t("backup.autoLabel")}</div>
                  <div className="setting-description mt-0.5">{t("backup.autoDescription")}</div>
                </div>
                <AppSwitch checked={form.auto_backup_interval_hours > 0} onCheckedChange={(on) => onPatch({ auto_backup_interval_hours: on ? 6 : 0 })} label={t("backup.autoLabel")} />
              </div>
              <div className="flex flex-wrap gap-3">
                <div className="flex min-w-0 flex-1 flex-col gap-2 rounded-xl border border-[var(--panel-border)] p-3">
                  <div className="flex items-center gap-2">
                    <Clock className="h-4 w-4 shrink-0 text-[var(--text-secondary)]" strokeWidth={2} aria-hidden="true" />
                    <span className="field-label muted">{t("backup.frequencyLabel")}</span>
                  </div>
                  {/* 关闭时展示即将启用的默认间隔（与开关打开写入的 6 一致），避免显示裸 0 */}
                  <AppSelect value={form.auto_backup_interval_hours || 6} options={autoBackupOptions} disabled={form.auto_backup_interval_hours === 0} onChange={(value) => onPatch({ auto_backup_interval_hours: value })} />
                </div>
                <div className="flex min-w-0 flex-1 flex-col gap-2 rounded-xl border border-[var(--panel-border)] p-3">
                  <div className="flex items-center gap-2">
                    <Database className="h-4 w-4 shrink-0 text-[var(--text-secondary)]" strokeWidth={2} aria-hidden="true" />
                    <span className="field-label muted">{t("backup.keepLabel")}</span>
                  </div>
                  <AppSelect value={form.database_backup_keep_count} options={keepOptions} disabled={form.auto_backup_interval_hours === 0} onChange={(value) => onPatch({ database_backup_keep_count: value })} />
                </div>
              </div>
            </div>
            {/* 备份记录：摘要行可点折叠（复用 AppDisclosure）；行独立成卡，来源用高亮药丸标记 */}
            <AppDisclosure
              className="backup-records-disclosure py-4"
              open={recordsOpen}
              onOpenChange={(open) => { recordsOpenSession = open; setRecordsOpen(open); }}
              summary={(
                <>
                  <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">
                    <History className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />
                  </span>
                  <span className="min-w-0">
                    <span className="setting-title block">{t("backup.recordsTitle")}</span>
                    <span className="setting-description mt-0.5 block">{recordsSummary}</span>
                  </span>
                </>
              )}
            >
              {backups !== null && backups.length > 0 ? (
                <div className="flex flex-col gap-2">
                  {backups.map((backup) => {
                    const menuOpen = menuTarget?.backup.name === backup.name;
                    const auto = isAutoBackupName(backup.name);
                    return (
                      <div key={backup.name} className="apple-list-row">
                        <div className="flex min-w-0 items-center gap-3">
                          <span className={`apple-chip apple-chip--roomy shrink-0 ${auto ? "apple-chip--accent" : "apple-chip--success"}`}>{auto ? t("backup.badgeAuto") : t("backup.badgeManual")}</span>
                          <div className="min-w-0">
                            <div className="field-label truncate">{auto || isManualBackupName(backup.name) ? formatShortTimestamp(backup.created_at) : backupTitle(backup.name)}</div>
                            <div className="muted meta-xs mt-0.5 truncate">{formatTimestamp(backup.created_at)} · {formatSize(backup.size_bytes)}</div>
                          </div>
                        </div>
                        <div className="flex shrink-0 items-center gap-1.5">
                          <button type="button" className="apple-inline-btn apple-inline-btn--quiet" onClick={() => void restoreBackup(backup)}><RotateCcw className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{t("backup.restore")}</button>
                          <button type="button" className="apple-icon-button text-[var(--text-secondary)] hover:text-accent" data-row-menu-trigger aria-haspopup="menu" aria-expanded={menuOpen} aria-label={t("backup.moreTooltip")} onClick={(event) => (menuOpen ? setMenuTarget(null) : openMenu(backup, event.currentTarget))} onKeyDown={(event) => { if (event.key === "Escape") setMenuTarget(null); }}>
                            <MoreHorizontal className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : backups !== null ? (
                <div className="setting-description flex items-center gap-2">
                  <Database className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
                  {t("backup.empty")}
                </div>
              ) : null}
            </AppDisclosure>
          </div>
          {rowMenu}
          <AppDialog open={renameTarget !== null} onOpenChange={(open) => { if (!open) setRenameTarget(null); }} title={t("backup.renameDialogTitle")} footer={<><button type="button" className="apple-action-button" onClick={() => setRenameTarget(null)}>{t("backup.cancel")}</button><button type="button" className="apple-action-button app-button--primary" disabled={renaming || !renameText.trim()} onClick={() => void submitRename()}>{t("backup.save")}</button></>}><input className="app-input" maxLength={80} placeholder={t("backup.renamePlaceholder")} value={renameText} onChange={(event) => setRenameText(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) void submitRename(); }} /></AppDialog>
        </div>
      </SettingsPanelSection>
      <SettingsPanelSection id="data-paths" label={t("about.dataAndPaths")}>
        <div className="apple-group divide-y divide-[var(--panel-divider)] px-[var(--gap-card-inline)]">
          {paths.filter((item) => item.label !== BACKUP_DIR_LABEL).map((item) => {
            const Icon = item.label === "about.paths.codexConfig" ? FileCog : item.label === "about.paths.logs" ? FileText : Database;
            const description = item.label === "about.paths.codexConfig" ? t("about.pathDescriptions.codexConfig") : item.label === "about.paths.logs" ? t("about.pathDescriptions.logs") : t("about.pathDescriptions.appData");
            return (
              <SettingsRowLink
                key={item.label}
                icon={<Icon className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />}
                title={pathLabel(t, item.label)} description={description}
                tooltip={t("about.openPath", { label: pathLabel(t, item.label) })}
                onClick={() => onOpenPath(item)}
                disabled={Boolean(openingPath)}
                trailing={openingPath === item.path ? <LoaderCircle className="h-4 w-4 shrink-0 animate-spin text-[var(--text-secondary)]" strokeWidth={2} aria-hidden="true" /> : undefined}
              />
            );
          })}
        </div>
      </SettingsPanelSection>
    </div>
  );
}

// 设置页「整行可点」行链接：图标座 + 标题/描述 + 尾部箭头，About 与「数据与路径」共用同一行语言
function SettingsRowLink({ icon, title, description, tooltip, onClick, disabled, trailing }: {
  icon: ReactNode; title: string; description: string; tooltip: string; onClick: () => void; disabled?: boolean; trailing?: ReactNode;
}) {
  return (
    <button type="button" className="group flex w-full items-center justify-between gap-4 py-4 text-left" disabled={disabled} title={tooltip} onClick={onClick}>
      <span className="flex min-w-0 items-center gap-3">
        <span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-xl">{icon}</span>
        <span className="min-w-0">
          <span className="setting-title block transition-colors group-hover:text-accent">{title}</span>
          <span className="setting-description mt-0.5 block">{description}</span>
        </span>
      </span>
      {trailing ?? <ChevronRight className="h-4 w-4 shrink-0 text-[var(--text-secondary)]" strokeWidth={2} aria-hidden="true" />}
    </button>
  );
}

export function SettingsAbout() {
  const feedback = useFeedback();
  const { t } = useTranslation("settings");
  // 更新失败提示的文案归 updates 命名空间，故另取一个对应的 t
  const { t: tUpdate } = useTranslation("updates");
  const { update, checking, installing, check } = useAppUpdate();
  // 与状态栏悬浮卡片一致：升级走确认式弹窗，先看更新日志再安装
  const [confirming, setConfirming] = useState(false);
  const openRepository = () => void api.openUrl("https://github.com/zeno528/Agmux").catch((error) => feedback.error(String(error)));
  // 检查只负责发现并展示版本号，升级必须由用户点击「立即重启更新」触发
  const checkUpdate = async () => {
    try {
      const found = await check();
      if (found) {
        setConfirming(true);
      } else {
        feedback.success(t("about.upToDate"));
      }
    } catch (error) {
      feedback.error(updateFailureMessage(error, tUpdate));
    }
  };

  return (
    <div className="flex flex-col gap-[var(--gap-section)]">
      <div className="apple-group divide-y divide-[var(--panel-divider)] px-[var(--gap-card-inline)]">
        <div className="relative flex flex-wrap items-center justify-between gap-4 py-4">
          <div className="flex items-center gap-3">
            <img src="/logo.svg" alt="Agmux" className="app-logo h-12 w-12 shrink-0" />
            <div className="flex flex-col items-start gap-1">
              <span className="apple-wordmark">Agmux</span>
              <span className="flex items-center gap-2">
                <span className="app-version">v{version.trim()}</span>
                {update ? <span className="apple-chip apple-chip--success" style={{ color: "var(--text-primary)" }}>{t("about.updateAvailable")}</span> : null}
              </span>
            </div>
          </div>
          <div className="settings-about__actions flex flex-wrap items-center gap-[var(--gap-actions)]">
            {update ? (
              <button type="button" className="apple-action-button app-button--primary" disabled={installing} onClick={() => setConfirming(true)}>
                {installing ? <LoaderCircle className="h-4 w-4 animate-spin" strokeWidth={2} aria-hidden="true" /> : <Download className="h-4 w-4" strokeWidth={2} aria-hidden="true" />}
                {installing ? t("about.installing") : t("about.updateTo")} v{update.version}
              </button>
            ) : (
              <button type="button" className="apple-action-button app-button--primary" disabled={checking} onClick={() => void checkUpdate()}>
                <RefreshCw className={`h-4 w-4 ${checking ? "animate-spin" : ""}`} strokeWidth={2} />
                {t("about.checkUpdate")}
              </button>
            )}
          </div>
        </div>
        {/* 参考 Folo / AFFiNE：About 区拆成与「数据与路径」同语言的行列表 */}
        <SettingsRowLink
          icon={<GithubMark className="h-[18px] w-[18px]" />}
          title={t("about.repository")} description={t("about.repositoryDescription")}
          tooltip={t("about.openRepo")}
          onClick={openRepository}
        />
        <SettingsRowLink
          icon={<History className="h-[18px] w-[18px]" strokeWidth={2} aria-hidden="true" />}
          title={t("about.changelog")} description={t("about.changelogDescription")}
          tooltip={t("about.viewReleases")}
          onClick={() => void api.openUrl(releaseNotesUrl(update?.version ?? version.trim())).catch((error) => feedback.error(String(error)))}
        />
      </div>
      <UpdateNotesDialog open={confirming} onOpenChange={setConfirming} />
    </div>
  );
}
