import { ChevronLeft, BookOpenText, FolderOpen, PackagePlus, PackageSearch, Trash2 } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, isTauri } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { getCachedSkills, loadSkills, setSkillsCache } from "../../app/managementDataCache";
import { AppDialog } from "../../components/AppDialog";
import { AppSearchInput } from "../../components/AppSearchInput";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { SkillTargetLogo } from "../../components/SkillTargetLogo";
import { matchesQuery } from "../plugins/pluginMeta";
import type { SkillCandidate, SkillSummary, SkillTool } from "../../types";

let savedScrollTop = 0;
const MarkdownPreview = lazy(() => import("react-markdown"));

export function selectableSkillPaths(candidates: SkillCandidate[]) {
  return candidates.filter((candidate) => !candidate.has_content_conflict).map((candidate) => candidate.store_path);
}

export default function SkillsView({ activationEpoch, popEpoch }: { activationEpoch: number; popEpoch: number }) {
  const feedback = useFeedback();
  const { t } = useTranslation("skills");
  const cached = getCachedSkills();
  const [skills, setSkills] = useState<SkillSummary[]>(cached ?? []);
  const [loaded, setLoaded] = useState(cached !== null);
  const [importing, setImporting] = useState(false);
  const [candidates, setCandidates] = useState<SkillCandidate[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [previewName, setPreviewName] = useState<string | null>(null);
  const [previewContent, setPreviewContent] = useState("");
  const [loadError, setLoadError] = useState("");
  const [query, setQuery] = useState("");
  const updateScanInFlight = useRef(false);

  // 侧栏重复点击本视图 = 弹回一级；只清子页状态，导入完成后有各自的刷新链路。
  useEffect(() => {
    if (!popEpoch) return;
    setImporting(false);
  }, [popEpoch]);
  const availableCount = candidates.length;

  const refresh = async (force = false) => {
    try { const next = await loadSkills(force); setSkills(next); setLoadError(""); }
    catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
  };
  const openImport = async () => {
    if (busy) return;
    setImporting(true);
    if (candidates.length) return;
    setBusy("scan");
    try { setCandidates(await api.scanUnmanagedSkills()); }
    catch (error) { feedback.error(String(error)); }
    finally { setBusy(null); }
  };
  const importCandidates = async (paths: string[]) => {
    if (busy || !paths.length) return;
    setBusy("import");
    try {
      let imported = 0;
      for (const path of paths) {
        imported += await api.importSkill(path);
      }
      const importedNames = new Set(candidates.filter((candidate) => paths.includes(candidate.store_path)).map((candidate) => candidate.name));
      setCandidates((current) => current.filter((candidate) => !importedNames.has(candidate.name)));
      if (imported) feedback.success(t("importedToast", { count: imported }));
      else feedback.warning(t("importedIdenticalToast"));
    } catch (error) { feedback.error(String(error)); }
    finally {
      await refresh(true);
      void scanForUpdates(); // 导入改变候选集，角标立即重扫，不等窗口重新聚焦
      setBusy(null);
    }
  };
  const importFromFolder = async () => {
    if (busy) return;
    try {
      let picked: string | null = null;
      if (isTauri) {
        const result = await openDialog({ title: t("importFolderDialogTitle"), directory: true, multiple: false });
        picked = typeof result === "string" ? result : null;
        if (!picked) return;
      }
      setBusy("folder");
      const imported = await api.importSkill(picked ?? "mock-folder-skill");
      if (imported) feedback.success(t("importedToast", { count: imported }));
      else feedback.warning(t("importedIdenticalToast"));
      setImporting(false);
      await refresh(true);
      void scanForUpdates(); // 文件夹导入改变候选集，角标立即重扫，不等窗口重新聚焦
    } catch (error) { feedback.error(String(error)); }
    finally { setBusy(null); }
  };
  const run = async (name: string, action: "enable" | "disable" | "delete", tool: SkillTool = "codex") => {
    if (busy) return;
    if (action === "delete" && !await feedback.confirm({ title: t("deleteDialogTitle"), description: t("deleteDialogDescription", { name }), confirmText: t("delete"), destructive: true })) return;
    setBusy(`${action}:${name}`);
    const previous = skills;
    if (action !== "delete") setSkills((current) => { const next = current.map((skill) => skill.name === name ? { ...skill, [tool === "claude" ? "claude_enabled" : "enabled"]: action === "enable" } : skill); setSkillsCache(next); return next; });
    try { if (action === "enable") await api.enableSkill(name, tool); if (action === "disable") await api.disableSkill(name, tool); if (action === "delete") { await api.deleteSkill(name); feedback.success(t("deletedToast")); await refresh(true); void scanForUpdates(); } }
    catch (error) { if (action !== "delete") { setSkillsCache(previous); setSkills(previous); } feedback.error(String(error)); }
    finally { setBusy(null); }
  };
  const openPreview = async (name: string) => {
    setPreviewName(name); setPreviewContent("");
    try { setPreviewContent(await api.getSkillContent(name)); }
    catch (error) { setPreviewContent(String(error)); }
  };
  const scanForUpdates = async () => {
    if (updateScanInFlight.current) return;
    updateScanInFlight.current = true;
    try { setCandidates(await api.scanUnmanagedSkills()); }
    catch { /* 后台扫描失败保留上次结果，导入按钮仍可手动触发完整扫描。 */ }
    finally { updateScanInFlight.current = false; }
  };

  // 缓存直出（localStorage 恢复）后静默强刷；无缓存时两次调用共享同一在途请求。
  useEffect(() => { void refresh(); void refresh(true); }, []);
  useEffect(() => { void scanForUpdates(); }, [activationEpoch]);
  useEffect(() => { const main = document.querySelector("main"); if (!main) return; main.scrollTop = savedScrollTop; return () => { savedScrollTop = main.scrollTop; }; }, []);

  if (importing) return <ImportPage candidates={candidates} busy={busy} onBack={() => setImporting(false)} onImport={(paths) => void importCandidates(paths)} onImportFolder={() => void importFromFolder()} />;
  const enabledCount = skills.filter((skill) => skill.enabled).length;
  const claudeEnabledCount = skills.filter((skill) => skill.claude_enabled).length;
  const visibleSkills = skills.filter((skill) => matchesQuery(skill, query));
  return <><section className="apple-scroll-page mx-auto w-full max-w-none"><header className="apple-page-bar flex-wrap justify-between gap-4"><ManagementPageTitle icon={<span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><BookOpenText className="h-[18px] w-[18px]" strokeWidth={2} /></span>} title="Skill" count={loaded ? skills.length : undefined} countLabel={loaded ? t("installedCount", { count: skills.length }) : undefined} /><div className="flex items-center gap-2"><AppSearchInput value={query} onChange={setQuery} placeholder={t("searchPlaceholder")} /><button type="button" className="apple-action-button app-button--primary relative" aria-label={availableCount ? t("importSkillAria", { count: availableCount }) : t("importSkill")} title={availableCount ? t("importSkillTitle", { count: availableCount }) : undefined} onClick={() => void openImport()}><PackagePlus className="h-4 w-4" />{t("importSkill")}{availableCount ? <span className="apple-count-badge" aria-hidden="true">{availableCount > 9 ? "9+" : availableCount}</span> : null}</button></div></header><div className="apple-edit-content">{loadError ? <p className="muted mt-4 text-sm">{loadError}</p> : null}{!skills.length ? <EmptyStateCard loading={!loaded} icon={<BookOpenText className="h-5 w-5" strokeWidth={1.8} />}><p className="muted">{t("empty")}</p><button type="button" className="apple-inline-btn" onClick={() => void openImport()}>{t("importFromLocal")}</button></EmptyStateCard> : null}{skills.length ? <div className="apple-group apple-list-card">
    <table className="skills-table" aria-label={t("installedCount", { count: skills.length })}>
      <colgroup><col /><col className="skills-table-client" /><col className="skills-table-client" /><col className="skills-table-action" /></colgroup>
      <thead>
        <tr>
          <th scope="col" className="field-label">{t("nameColumn")}</th>
          <th scope="col" className="field-label" aria-label={t("claudeEnabledCount", { count: claudeEnabledCount })}>
            <span className="plan-badge gap-1.5"><SkillTargetLogo target="claude" variant="title" /><span>{claudeEnabledCount}</span></span>
          </th>
          <th scope="col" className="field-label" aria-label={t("codexEnabledCount", { count: enabledCount })}>
            <span className="plan-badge gap-1.5"><SkillTargetLogo target="codex" variant="title" /><span>{enabledCount}</span></span>
          </th>
          <th scope="col" className="field-label"><span className="sr-only">{t("delete")}</span></th>
        </tr>
      </thead>
      <tbody>{visibleSkills.map((skill) => <SkillRow key={skill.name} skill={skill} busy={busy !== null} onRun={run} onPreview={openPreview} />)}</tbody>
    </table>
  </div> : null}</div></section><AppDialog open={previewName !== null} onOpenChange={(open) => { if (!open) setPreviewName(null); }} title={previewName ?? "Skill"} className="app-dialog-content--wide" footer={<button type="button" className="apple-action-button app-button--primary" onClick={() => setPreviewName(null)}>{t("done")}</button>}><Suspense fallback={<div className="flex h-[var(--dialog-preview-height)] flex-col items-center justify-center gap-2"><LoadingSpinner size="md" /><span className="muted meta-xs">{t("loadingPreview")}</span></div>}><div className="skill-markdown-preview h-[var(--dialog-preview-height)] overflow-auto">{previewContent ? <MarkdownPreview>{previewContent}</MarkdownPreview> : <div className="flex h-full flex-col items-center justify-center gap-2"><LoadingSpinner size="md" /><span className="muted meta-xs">{t("loading")}</span></div>}</div></Suspense></AppDialog></>;
}

function ImportPage({ candidates, busy, onBack, onImport, onImportFolder }: { candidates: SkillCandidate[]; busy: string | null; onBack: () => void; onImport: (paths: string[]) => void; onImportFolder: () => void }) {
  const { t, i18n } = useTranslation("skills");
  const [ignoredPaths, setIgnoredPaths] = useState<string[]>([]);
  const visibleCandidates = candidates.filter((candidate) => !ignoredPaths.includes(candidate.store_path));
  const importablePaths = selectableSkillPaths(visibleCandidates);
  const [preview, setPreview] = useState<SkillCandidate | null>(null);
  const [previewContent, setPreviewContent] = useState("");
  const openPreview = async (skill: SkillCandidate) => {
    setPreview(skill);
    setPreviewContent("");
    try { setPreviewContent(await api.getImportSkillContent(skill.store_path)); }
    catch (error) { setPreviewContent(String(error)); }
  };
  return (
    <section className="apple-edit-page mx-auto flex w-full max-w-none flex-col">
      <div className="apple-page-bar apple-edit-toolbar apple-edit-toolbar--header justify-between">
        <button type="button" className="apple-page-header apple-back-button" disabled={busy !== null} onClick={onBack}>
          <ManagementPageTitle className="management-page-title--compact" icon={<ChevronLeft className="h-4 w-4 shrink-0 text-accent" />} title={t("importPageTitle")} count={visibleCandidates.length} />
        </button>
        <button type="button" className="apple-action-button" disabled={busy !== null} onClick={onImportFolder}>
          <FolderOpen className="h-4 w-4" />{t("importFromFolder")}
        </button>
      </div>
      <div className="apple-edit-content">
        {busy === "scan" ? (
          <EmptyStateCard loading icon={<PackageSearch className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("scanning")}</p>
          </EmptyStateCard>
        ) : visibleCandidates.length ? (
          <div className="apple-group apple-list-card">
            {visibleCandidates.map((skill) => (
              <div key={skill.store_path} className="apple-list-row gap-4">
                <span className="group min-w-0 flex-1">
                  <button type="button" className="max-w-full truncate align-bottom font-semibold transition-colors group-hover:text-accent" onClick={() => void openPreview(skill)}>{skill.name}</button>
                  <span className={`apple-chip ml-2 ${skill.is_update ? "text-accent" : ""}`}>{skill.is_update ? t("updateFound") : t("newCandidate")}</span>
                  <span className="apple-chip ml-2">{t("source", { source: skill.source })}</span>
                  {skill.has_content_conflict ? <span className="meta-xs ml-2 font-semibold text-[var(--warning)]">{t("conflict")}</span> : null}
                  {skill.description ? <span className="muted meta-xs mt-1 block truncate">{skill.description}</span> : null}
                  {skill.has_content_conflict ? <span className="muted meta-xs mt-1 block">{t("modifiedAt", { date: new Date(skill.modified_at * 1000).toLocaleString(i18n.language) })}</span> : null}
                </span>
                <div className="flex shrink-0 items-center gap-2">
                  <button type="button" className="apple-action-button mcp-diff-action-button" disabled={busy !== null} onClick={() => setIgnoredPaths((current) => [...current, skill.store_path])}>{t("ignore")}</button>
                  <button type="button" className="apple-action-button mcp-diff-action-button app-button--primary" disabled={busy !== null} onClick={() => onImport([skill.store_path])}>{t("importOne")}</button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <EmptyStateCard icon={<PackageSearch className="h-5 w-5" strokeWidth={1.8} />}>
            <p className="muted">{t("noCandidates")}</p>
          </EmptyStateCard>
        )}
      </div>
      <div className="apple-edit-toolbar apple-edit-toolbar--footer">
        {visibleCandidates.some((skill) => skill.has_content_conflict) ? <span className="muted meta-xs mr-auto">{t("bulkSkipsConflicts")}</span> : null}
        <button type="button" className="apple-action-button" disabled={busy !== null} onClick={onBack}>{t("cancel")}</button>
        <button type="button" className="apple-action-button app-button--primary" disabled={busy !== null || !importablePaths.length} onClick={() => onImport(importablePaths)}>
          {busy === "import" ? t("importing") : t("importAll")}
        </button>
      </div>
      <AppDialog open={preview !== null} onOpenChange={(open) => { if (!open) setPreview(null); }} title={preview?.name ?? "Skill"} className="app-dialog-content--wide" footer={<button type="button" className="apple-action-button app-button--primary" onClick={() => setPreview(null)}>{t("done")}</button>}>
        <Suspense fallback={<div className="flex h-[var(--dialog-preview-height)] flex-col items-center justify-center gap-2"><LoadingSpinner size="md" /><span className="muted meta-xs">{t("loadingPreview")}</span></div>}>
          <div className="skill-markdown-preview h-[var(--dialog-preview-height)] overflow-auto">{previewContent ? <MarkdownPreview>{previewContent}</MarkdownPreview> : <div className="flex h-full flex-col items-center justify-center gap-2"><LoadingSpinner size="md" /><span className="muted meta-xs">{t("loading")}</span></div>}</div>
        </Suspense>
      </AppDialog>
    </section>
  );
}

function SkillRow({ skill, busy, onRun, onPreview }: { skill: SkillSummary; busy: boolean; onRun: (name: string, action: "enable" | "disable" | "delete", tool?: SkillTool) => Promise<void>; onPreview: (name: string) => void }) {
  const { t } = useTranslation("skills");
  return (
    <tr>
      <td><button type="button" className="group w-full min-w-0 cursor-pointer text-left" title={t("previewSkill")} onClick={() => void onPreview(skill.name)}><div className="font-semibold transition-colors group-hover:text-accent">{skill.name}</div>{skill.description ? <div className="muted meta-xs truncate">{skill.description}</div> : null}</button></td>
      <td><button type="button" role="switch" className={`apple-icon-button ${skill.claude_enabled ? "app-button--primary" : "hover:bg-transparent"}`} aria-label={skill.claude_enabled ? t("toggleClaudeRemoveAria", { name: skill.name }) : t("toggleClaudeAddAria", { name: skill.name })} aria-checked={skill.claude_enabled} disabled={busy} onClick={() => void onRun(skill.name, skill.claude_enabled ? "disable" : "enable", "claude")}><SkillTargetLogo target="claude" active={skill.claude_enabled} /></button></td>
      <td><button type="button" role="switch" className={`apple-icon-button ${skill.enabled ? "app-button--primary" : "hover:bg-transparent"}`} aria-label={skill.enabled ? t("toggleRemoveAria", { name: skill.name }) : t("toggleAddAria", { name: skill.name })} aria-checked={skill.enabled} disabled={busy} onClick={() => void onRun(skill.name, skill.enabled ? "disable" : "enable")}><SkillTargetLogo target="codex" active={skill.enabled} /></button></td>
      <td><button type="button" className="apple-icon-button text-[var(--danger)]/70 hover:bg-(--danger)/10 hover:text-[var(--danger)]" title={t("delete")} aria-label={t("deleteAria", { name: skill.name })} disabled={busy} onClick={() => void onRun(skill.name, "delete")}><Trash2 className="h-4 w-4" /></button></td>
    </tr>
  );
}
