import { CircleDashed, GitCompare, Globe, MoreHorizontal, Pencil, Plus, Terminal, Wifi, Wrench } from "lucide-react";
import { createPortal } from "react-dom";
import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { deleteCachedMcpProbe, getCachedClaudeMcpServers, getCachedMcpProbe, getCachedMcpServers, getMcpDiffBadge, loadClaudeMcpServers, loadMcpServers, mcpDiffBadgeText, setCachedMcpProbe, setClaudeMcpServersCache, setMcpDiffBadge, setMcpServersCache, subscribeMcpDiffBadge, type McpProbeScope } from "../../app/managementDataCache";
import { AppSwitch } from "../../components/AppSwitch";
import { AppSegmentedControl } from "../../components/AppSegmentedControl";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import { TrashIcon } from "../../components/TrashIcon";
import { ManagementPageTitle } from "../../components/ManagementPageTitle";
import { McpIcon } from "../../components/McpIcon";
import { SkillTargetLogo } from "../../components/SkillTargetLogo";
import { useFixedMenuPosition } from "../../components/useFixedMenuPosition";
import { useMenuDismiss } from "../../components/useMenuDismiss";
import type { McpDiffEntryAction, McpProbeResult, McpServerSpec, McpSyncDiffEntry, McpSyncPreview } from "../../types";
import ClaudeMcpEdit from "./ClaudeMcpEdit";
import McpDiffPage from "./McpDiffPage";
import McpEdit from "./McpEdit";

type Transport = "http" | "stdio" | "unknown";

export type McpDiffVerb = "adopt" | "revert";
export type McpTarget = "codex" | "claude";

interface McpHeaderState {
  count?: number;
  editing: boolean;
  onCreate: () => void;
  onDiff?: () => void;
}

interface McpClientViewProps {
  activationEpoch: number;
  /// 侧栏重复点击本视图时递增；视图据此弹回一级。一级时已是初始值，setState 同值跳过，零反应。
  popEpoch: number;
  onHeaderChange: (header: McpHeaderState) => void;
}

/// 差异动作：只指向单个条目的单侧（mirror=数据库镜像，live=config.toml）；
/// fragment 为空表示删除该侧的条目。
export type McpDiffAction = { side: "mirror" | "live" } & McpDiffEntryAction;

export function McpTargetSwitch({ value, onChange }: { value: McpTarget; onChange: (target: McpTarget) => void }) {
  const { t } = useTranslation("mcp");
  const codexBadge = useSyncExternalStore(subscribeMcpDiffBadge, getMcpDiffBadge, getMcpDiffBadge);
  const claudeBadge = useSyncExternalStore(subscribeMcpDiffBadge, () => getMcpDiffBadge("claude"), () => getMcpDiffBadge("claude"));
  const badges = { codex: mcpDiffBadgeText(codexBadge), claude: mcpDiffBadgeText(claudeBadge) };
  return (
    <AppSegmentedControl
      className="mcp-target-switch h-[var(--toolbar-control-height)] shrink-0"
      selectedIndex={value === "codex" ? 0 : 1}
      role="tablist"
      label={t("target.label")}
    >
      {(["codex", "claude"] as const).map((target) => (
        <button
          key={target}
          type="button"
          role="tab"
          aria-selected={value === target}
          className="apple-action-button"
          title={t("target.label")}
          onClick={() => onChange(target)}
        >
          <SkillTargetLogo target={target} variant="title" />
          {t(`target.${target}`)}
          {badges[target] ? <span className="apple-count-badge" aria-hidden="true">{badges[target]}</span> : null}
        </button>
      ))}
    </AppSegmentedControl>
  );
}

/// 差异×动词到外科手术原语的唯一映射——每条命令只碰单个条目的单侧（镜像或 live），
/// 不得使用 codexSaveMcpServer/codexDeleteMcpServer（它们会同时修改 live，无法表达只采纳镜像）：
/// 同步 = 采纳外部修改（live 片段写入镜像；外部已删除的删除镜像条目）；
/// 撤销 = 回退外部修改（数据库片段写回 live；外部新增的从 live 移除）。
export function mcpEntryAction(entry: McpSyncDiffEntry, verb: McpDiffVerb): McpDiffAction | null {
  if (verb === "adopt") {
    if (entry.kind === "db_only") return { side: "mirror", name: entry.name, fragment: null };
    return entry.live_toml ? { side: "mirror", name: entry.name, fragment: entry.live_toml } : null;
  }
  if (entry.kind === "live_only") return { side: "live", name: entry.name, fragment: null };
  return entry.db_toml ? { side: "live", name: entry.name, fragment: entry.db_toml } : null;
}

function mcpServerFingerprint(server: McpServerSpec) {
  const { enabled, startup_timeout_sec, tool_timeout_sec, ...connection } = server;
  let hash = 2166136261;
  for (const char of JSON.stringify(connection)) {
    hash ^= char.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  // ponytail: 非加密哈希只用于缓存键；若未来要求抗碰撞，再换 Web Crypto。
  return (hash >>> 0).toString(36);
}

function cachedProbeState(servers: McpServerSpec[], scope: McpProbeScope = "codex") {
  const results: Record<string, McpProbeResult> = {};
  const toolsLoaded: Record<string, boolean> = {};
  for (const server of servers) {
    const entry = getCachedMcpProbe(server.name, mcpServerFingerprint(server), scope);
    if (!entry) continue;
    results[server.name] = entry.result;
    if (entry.toolsLoaded) toolsLoaded[server.name] = true;
  }
  return { results, toolsLoaded };
}

/// MCP 探测状态机：Codex 与 Claude 两页共用同一套探测/工具清单/缓存逻辑，缓存按 scope 隔离。
/// probe 一条函数覆盖两条路径：includeTools=false 只验连通（结果连同细节入库缓存），
/// includeTools=true 拉工具清单（失败不覆盖既有连通结论，只报错——灯反映的是连通性，不是清单拉取）。
/// manual 控制是否给用户反馈与后端 manual 标记，静默重探传 false；showLoading 默认跟随 manual。
function useMcpProbes(scope: McpProbeScope, initialServers: McpServerSpec[]) {
  const feedback = useFeedback();
  const { t } = useTranslation("mcp");
  const [probingNames, setProbingNames] = useState<Record<string, boolean>>({});
  const [probeResults, setProbeResults] = useState<Record<string, McpProbeResult>>(() => cachedProbeState(initialServers, scope).results);
  const [toolsOpen, setToolsOpen] = useState<Record<string, boolean>>({});
  const [toolsLoading, setToolsLoading] = useState<Record<string, boolean>>({});
  const [toolsLoaded, setToolsLoaded] = useState<Record<string, boolean>>(() => cachedProbeState(initialServers, scope).toolsLoaded);

  const notifyFailure = (name: string, message: string) => {
    if (/(超时|timeout|timed out)/i.test(message)) feedback.warning(t("list.connectionTimeout", { name })); // i18n-exempt: 匹配后端错误原文
    else feedback.error(t("list.connectionFailed", { name }));
  };

  /// 列表刷新后调用：用缓存重铺探测结果（进页不重新点亮状态灯），收起所有工具面板。
  const applyCache = (servers: McpServerSpec[]) => {
    const cached = cachedProbeState(servers, scope);
    setProbeResults(cached.results);
    setToolsOpen({});
    setToolsLoading({});
    setToolsLoaded(cached.toolsLoaded);
  };

  const probe = async (server: McpServerSpec, { includeTools = false, manual = true, showLoading = manual }: { includeTools?: boolean; manual?: boolean; showLoading?: boolean } = {}) => {
    const name = server.name;
    const fingerprint = mcpServerFingerprint(server);
    // 连通探测不带工具清单：之前取到的工具列表跨探测保留，不因一次重探清空
    const keepTools = (result: McpProbeResult) => {
      const cached = getCachedMcpProbe(name, fingerprint, scope);
      return cached?.toolsLoaded
        ? { ...result, tools: cached.result.tools, tools_truncated: cached.result.tools_truncated, tools_error: cached.result.tools_error }
        : result;
    };
    const remember = (result: McpProbeResult, toolsLoadedNow: boolean) => {
      setProbeResults((current) => ({ ...current, [name]: result }));
      setCachedMcpProbe(name, { fingerprint, result, toolsLoaded: toolsLoadedNow }, scope);
      if (toolsLoadedNow) setToolsLoaded((current) => ({ ...current, [name]: true }));
    };
    if (includeTools) {
      setToolsOpen((current) => ({ ...current, [name]: true }));
      if (showLoading) setToolsLoading((current) => ({ ...current, [name]: true }));
    } else if (showLoading) {
      setProbingNames((current) => ({ ...current, [name]: true }));
    }
    try {
      const result = await api.probeMcpServer(name, includeTools, manual, scope);
      if (includeTools) {
        if (!result.ok) throw new Error(result.error ?? t("list.connectionFailed", { name }));
        if (result.tools_error) throw new Error(result.tools_error);
        remember(result, true);
      } else {
        const next = keepTools(result);
        remember(next, getCachedMcpProbe(name, fingerprint, scope)?.toolsLoaded ?? false);
        if (manual) {
          if (next.ok) feedback.success(t("list.connectionSuccess", { name, ms: next.latency_ms ?? "-" }));
          else notifyFailure(name, next.error ?? "");
        }
      }
    } catch (error) {
      if (includeTools) {
        if (manual) feedback.error(String(error));
      } else {
        const result = keepTools({ ok: false, latency_ms: null, status: null, protocol_version: null, server_info: null, tools: [], tools_truncated: false, error: String(error), tools_error: null });
        remember(result, getCachedMcpProbe(name, fingerprint, scope)?.toolsLoaded ?? false);
        if (manual) notifyFailure(name, String(error));
      }
    } finally {
      if (includeTools) {
        if (showLoading) setToolsLoading((current) => { const next = { ...current }; delete next[name]; return next; });
      } else if (showLoading) {
        setProbingNames((current) => { const next = { ...current }; delete next[name]; return next; });
      }
    }
  };

  const toggleTools = (server: McpServerSpec) => {
    if (server.enabled === false) return;
    const name = server.name;
    if (toolsOpen[name]) {
      setToolsOpen((current) => ({ ...current, [name]: false }));
      return;
    }
    if (toolsLoading[name]) {
      setToolsOpen((current) => ({ ...current, [name]: true }));
      return;
    }
    void probe(server, { includeTools: true });
  };

  /// 应用级开关：两页只差刷新策略（refreshAfter）与乐观回滚（optimistic）。
  /// optimistic 点击即翻列表与缓存，失败用其返回的回滚函数还原（模式对齐拆分前的旧实现）。
  const [togglingName, setTogglingName] = useState("");
  const toggleEnabled = async (server: McpServerSpec, enabled: boolean, refreshAfter: () => Promise<unknown>, optimistic?: (server: McpServerSpec, enabled: boolean) => () => void) => {
    if (togglingName) return;
    setTogglingName(server.name);
    const rollback = optimistic?.(server, enabled);
    try {
      await api.setMcpServerEnabled(server.name, scope, enabled);
      if (!enabled) deleteCachedMcpProbe(server.name, scope);
      await refreshAfter();
      feedback.success(t("feedback.updated"));
    } catch (error) {
      rollback?.();
      feedback.error(String(error));
    } finally {
      setTogglingName("");
    }
  };

  return { probingNames, probeResults, toolsOpen, toolsLoading, toolsLoaded, probe, toggleTools, toggleEnabled, applyCache };
}

/// 开关的乐观翻转（Codex/Claude 两页共用，只差缓存写入口）：点击即翻列表与列表缓存，
/// 失败由 toggleEnabled 调用返回的回滚函数还原到点击前的 enabled 值（模式对齐拆分前旧实现）。
function optimisticToggleEnabled(setServers: (update: (current: McpServerSpec[]) => McpServerSpec[]) => void, writeCache: (items: McpServerSpec[]) => void) {
  return (server: McpServerSpec, enabled: boolean): () => void => {
    const previous = server.enabled;
    const apply = (value: boolean | null) => setServers((current) => {
      const next = current.map((item) => item.name === server.name ? { ...item, enabled: value } : item);
      writeCache(next);
      return next;
    });
    apply(enabled ? null : false);
    return () => apply(previous);
  };
}

function transportOf(server: McpServerSpec): Transport { return server.url ? "http" : server.command ? "stdio" : "unknown"; }

/// 列表排序：类型分组（stdio 本地 → http 远程 → unknown 兜底）优先，组内按名称。
const transportOrder: Record<Transport, number> = { stdio: 0, http: 1, unknown: 2 };

export function compareMcpServers(left: McpServerSpec, right: McpServerSpec): number {
  return transportOrder[transportOf(left)] - transportOrder[transportOf(right)] || left.name.localeCompare(right.name);
}
function transportIcon(server: McpServerSpec) { const current = transportOf(server); return current === "http" ? Globe : current === "stdio" ? Terminal : CircleDashed; }
function metaOf(server: McpServerSpec) { return server.command ? [server.command, ...server.args.slice(0, 2)].join(" ") : server.url ?? ""; }

function McpToolsPanel({ result, pending }: { result: McpProbeResult; pending: boolean }) {
  const { t } = useTranslation("mcp");

  return (
    <div className="px-3 pb-3 pt-1">
      <div className="flex flex-wrap gap-1.5">
        {result.tools.map((tool) => (
          <code key={tool.name} className="apple-chip mono">{tool.name}</code>
        ))}
        {/* 工具清单还在路上（或压根没取过）时不下结论：此刻的 result 是连通探测留下的，
            空列表只说明"还没取"，不说明"服务端没有工具" */}
        {!result.tools.length && !pending ? <p className="muted meta-xs">{t("list.noTools")}</p> : null}
      </div>
    </div>
  );
}

/** 行内元信息小 chip：传输类型 / 版本 / 工具数共用同一形态。 */
function MetaChip({ children }: { children: ReactNode }) {
  return <span className="shrink-0 rounded-md bg-black/5 px-1.5 py-px text-[10px] font-medium tracking-wide muted dark:bg-white/10">{children}</span>;
}

type McpServerRowProps = {
  server: McpServerSpec;
  result: McpProbeResult | undefined;
  probing: boolean;
  detailsVisible: boolean;
  toolsBusy: boolean;
  toolsLoaded: boolean;
  onEdit: (server: McpServerSpec) => void;
  onDelete: (server: McpServerSpec) => void;
  onProbe: (server: McpServerSpec) => void;
  onToggleTools: (server: McpServerSpec) => void;
  onToggleEnabled?: (server: McpServerSpec, enabled: boolean) => void;
};

export function McpServerRow({ server, result, probing, detailsVisible, toolsBusy, toolsLoaded, onEdit, onDelete, onProbe, onToggleTools, onToggleEnabled }: McpServerRowProps) {
  const { t } = useTranslation("mcp");
  const testTitle = t(server.enabled === false ? "list.testConnectionDisabled" : "list.testConnection");
  const [menuOpen, setMenuOpen] = useState(false);
  const menuTriggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuStyle = useFixedMenuPosition(menuOpen, menuTriggerRef.current, menuRef, "end");
  useMenuDismiss(menuOpen, menuTriggerRef, menuRef, setMenuOpen);
  const Icon = transportIcon(server);
  const toolsTitle = t(detailsVisible ? "list.collapseTools" : "list.toolsButton");
  const menu = menuOpen ? createPortal(
    <div ref={menuRef} className="app-select-menu app-popover" data-open="true" data-popover-in role="menu" aria-label={t("list.moreTooltip")} style={{ ...menuStyle, minWidth: "10rem" }}>
      <button type="button" role="menuitem" className="app-select-option app-selection-state" onClick={() => { setMenuOpen(false); onEdit(server); }}>
        <span className="flex items-center gap-2"><Pencil className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{t("list.editTooltip")}</span>
      </button>
      <button type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={probing || server.enabled === false} onClick={() => { setMenuOpen(false); onProbe(server); }}>
        <span className="flex items-center gap-2"><Wifi className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{testTitle}</span>
      </button>
      <button type="button" role="menuitem" className="app-select-option app-selection-state disabled:cursor-not-allowed disabled:opacity-40" disabled={server.enabled === false} onClick={() => { setMenuOpen(false); onToggleTools(server); }}>
        <span className="flex items-center gap-2"><Wrench className="h-4 w-4" strokeWidth={2} aria-hidden="true" />{toolsTitle}</span>
      </button>
      <button type="button" role="menuitem" className="app-select-option app-selection-state app-select-option--danger" onClick={() => { setMenuOpen(false); onDelete(server); }}>
        <span className="flex items-center gap-2"><TrashIcon />{t("edit.uninstall")}</span>
      </button>
    </div>,
    document.body,
  ) : null;

  return (
    <div onClick={(event) => {
      if (!detailsVisible || !(event.target instanceof HTMLElement) || event.target.closest('button, input, code, [role="switch"]')) return;
      onToggleTools(server);
    }}>
      <div className="apple-list-row mcp-expanded-card__header">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="settings-icon-tile grid h-8 w-8 shrink-0 place-items-center rounded-lg text-accent">
            <Icon className="h-4 w-4" strokeWidth={2} />
          </span>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span
                className={`h-2 w-2 shrink-0 rounded-full ${server.enabled !== false && result?.ok ? "bg-success" : "bg-(--text-secondary)"}`}
                role="img"
                aria-label={t(server.enabled === false ? "list.testConnectionDisabled" : probing ? "list.connectionStateChecking" : result?.ok ? "list.connectionStateConnected" : "list.connectionStateUnavailable")}
              />
              <span className="min-w-0 truncate font-semibold">{server.name}</span>
              <MetaChip>{server?.url ? "HTTP" : server?.command ? "STDIO" : t("transport.unknown")}</MetaChip>
              {result?.server_info?.version ? <MetaChip>{result.server_info.version}</MetaChip> : null}
              {toolsLoaded ? <MetaChip>{t("list.toolCount", { count: result?.tools.length ?? 0 })}</MetaChip> : null}
              {toolsBusy ? <span className="muted shrink-0"><LoadingSpinner /></span> : null}
            </div>
            <div className="mono muted meta-xs truncate">{metaOf(server)}</div>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <button
            ref={menuTriggerRef}
            type="button"
            className="apple-icon-button text-[var(--text-secondary)] enabled:hover:text-accent"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            aria-label={t("list.moreTooltip")}
            onClick={() => setMenuOpen((open) => !open)}
          >
            <MoreHorizontal className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
          </button>
          {onToggleEnabled ? <AppSwitch size="sm" checked={server.enabled !== false} label={t("list.enableServer", { name: server.name })} onCheckedChange={(value) => onToggleEnabled(server, value)} /> : null}
        </div>
      </div>
      {result ? (
        <div className={`apple-disclosure mcp-tools-disclosure ${detailsVisible ? "apple-disclosure--open" : ""}`}>
          <div className="apple-disclosure__content" aria-hidden={!detailsVisible} inert={!detailsVisible}>
            <div className="apple-disclosure__body">
              <McpToolsPanel result={result} pending={toolsBusy || !toolsLoaded} />
            </div>
          </div>
        </div>
      ) : null}
      {menu}
    </div>
  );
}

function McpClientView({ target, activationEpoch, popEpoch, onHeaderChange }: McpClientViewProps & { target: McpTarget }) {
  const feedback = useFeedback();
  const { t } = useTranslation("mcp");
  const cachedServers = target === "codex" ? getCachedMcpServers() : getCachedClaudeMcpServers();
  const [servers, setServers] = useState<McpServerSpec[]>(() => cachedServers ?? []);
  const [loaded, setLoaded] = useState(cachedServers !== null);
  const [loadError, setLoadError] = useState("");
  const [editingServer, setEditingServer] = useState<McpServerSpec | null>(null);
  const [creatingServer, setCreatingServer] = useState(false);
  const { probingNames, probeResults, toolsOpen, toolsLoading, toolsLoaded, probe, toggleTools, toggleEnabled, applyCache } = useMcpProbes(target, cachedServers ?? []);
  const optimisticToggle = optimisticToggleEnabled(setServers, target === "codex" ? setMcpServersCache : setClaudeMcpServersCache);
  const [syncPreview, setSyncPreview] = useState<McpSyncPreview | null>(null);
  const [previewError, setPreviewError] = useState("");
  const [diffOpen, setDiffOpen] = useState(false);
  const [resolving, setResolving] = useState(false);
  const previewInFlight = useRef(false);

  // 侧栏重复点击本视图 = 弹回一级；只清子页状态，不刷新（保存流程自带刷新，弹回不产生数据变化）。
  useEffect(() => {
    if (!popEpoch) return;
    setEditingServer(null);
    setCreatingServer(false);
    setDiffOpen(false);
  }, [popEpoch]);

  const loadPreview = async () => {
    if (previewInFlight.current) return;
    previewInFlight.current = true;
    try {
      const preview = await (target === "codex" ? api.codexMcpSyncPreview() : api.claudeMcpSyncPreview());
      setSyncPreview(preview);
      setPreviewError("");
      if (!preview.entries.length) setDiffOpen(false);
      // 侧栏角标与页面同源：页面查到就写回共享缓存
      setMcpDiffBadge({ count: preview.entries.length, error: false }, target);
    }
    catch (error) {
      setPreviewError(String(error));
      setSyncPreview(null);
      // 差异算不出来本身就是一种状态：写回 error 让侧栏跟着亮，
      // 否则用户不点进 MCP 页就不知道 config.toml 坏了
      setMcpDiffBadge({ count: 0, error: true }, target);
    }
    finally { previewInFlight.current = false; }
  };
  /// 重新读列表 + 查差异，顺带决定要不要重连 MCP。
  /// 规则只有一条：**只有配置真的被改过的那几台才值得重连**——重连意味着把本机
  /// stdio 服务真的拉起来再发网络请求，不能因为"数据变了"就无脑全量重探。
  /// probe 不传 = 全连（挂载、整段重建这类配置整体变了的场景）；
  /// `[]` = 一台都不连；`[name]` = 只连这几台。
  /// 调用方若还要单独处理某一台（比如刚保存的要验一次连通），传 `[]`
  /// 再自己调 probeServer——不要用"跳过它"来绕，那会把其余几台也全探一遍。
  const refresh = async (force = false, only?: string[]) => {
    let next: McpServerSpec[] | null = null;
    try {
      next = await (target === "codex" ? loadMcpServers(force) : loadClaudeMcpServers(force));
      setServers(next);
      applyCache(next);
      setLoadError("");
    } catch (error) { setLoadError(String(error)); }
    finally { setLoaded(true); }
    if (next) {
      const targets = only ? next.filter((server) => only.includes(server.name)) : next;
      const enabledTargets = targets.filter((server) => server.enabled !== false);
      if (enabledTargets.length) void Promise.all(enabledTargets.map((server) => probe(server, { manual: false })));
    }
    await loadPreview();
    return next;
  };
  const probedOnceRef = useRef(false);
  useEffect(() => {
    // StrictMode 下 effect 双跑共用同一实例，必须用 ref 防重入，
    // 否则每次进页全量探测两遍（stdio 服务器会被拉起两次）
    if (probedOnceRef.current) return;
    probedOnceRef.current = true;
    void refresh(true);
  }, []);

  // 窗口激活时刷新当前客户端的差异预览。
  // epoch=0 表示尚未激活过（含首次挂载，此时上面的 refresh 已经取过预览），不重复请求。
  useEffect(() => { if (activationEpoch === 0) return; if (target === "claude") void refresh(true, []); else void loadPreview(); }, [activationEpoch]);

  const removeServer = async (server: McpServerSpec) => {
    const confirmed = await feedback.confirm({ title: t("confirm.deleteTitle"), description: <Trans ns="mcp" i18nKey="confirm.deleteDescription" values={{ name: server.name }} components={{ strong: <strong /> }} />, confirmText: t("confirm.delete"), destructive: true });
    if (!confirmed) return;
    try {
      await (target === "codex" ? api.codexDeleteMcpServer(server.name) : api.claudeDeleteMcpServer(server.name));
      deleteCachedMcpProbe(server.name, target);
      setEditingServer(null);
      feedback.success(t("feedback.deleted"));
      // 删掉的那台已经不存在了，其余几台的配置一个字没动 → 一台都不用重连
      await refresh(true, []);
    }
    catch (error) { feedback.error(String(error)); }
  };

  const applyDiffActions = (actions: McpDiffEntryAction[], verb: McpDiffVerb) =>
    target === "claude" ? api.claudeResolveMcpEntries(actions, verb === "adopt")
      : verb === "adopt" ? api.setMcpMirrorEntries(actions) : api.revertMcpLiveEntries(actions);

  const resolveEntry = async (entry: McpSyncDiffEntry, verb: McpDiffVerb) => {
    if (resolving) return;
    const action = mcpEntryAction(entry, verb);
    if (!action) { feedback.error(t("diff.resolveFailed", { name: entry.name })); return; }
    setResolving(true);
    try {
      await applyDiffActions([{ name: action.name, fragment: action.fragment }], verb);
      feedback.success(t(verb === "adopt" ? "diff.adoptedToast" : "diff.revertedToast", { name: entry.name }));
      // 同步只写数据库镜像，配置文件没被碰过 → 一台都不用重连；
      // 撤回写回了 live，只有这一台的内容变了 → 只重连它
      await refresh(true, verb === "revert" ? [entry.name] : []);
    } catch (error) { feedback.error(String(error)); }
    finally { setResolving(false); }
  };

  const resolveAll = async (verb: McpDiffVerb) => {
    if (resolving || !syncPreview?.entries.length) return;
    const entries = syncPreview.entries;
    // 整批一次提交：后端在一个文档里逐条原地改写，只备份并写盘一次；
    // 逐条调用会各备份一次，把备份保留池里操作前的那份挤掉，且中途失败会留下半完成状态
    const actions = entries
      .map((entry) => mcpEntryAction(entry, verb))
      .filter((action): action is McpDiffAction => action !== null)
      .map(({ name, fragment }) => ({ name, fragment }));
    if (!actions.length) return;
    const confirmed = await feedback.confirm({
      title: t(verb === "adopt" ? "diff.adoptAll" : "diff.revertAll"),
      description: t(verb === "adopt" ? "diff.confirmAdoptAll" : "diff.confirmRevertAll", { count: actions.length }),
      confirmText: verb === "adopt" ? t("diff.adoptAll") : t("diff.revertAll"),
      destructive: verb === "revert",
    });
    if (!confirmed) return;
    setResolving(true);
    try {
      const count = await applyDiffActions(actions, verb);
      feedback.success(t("diff.resolvedAllToast", { count }));
      // 同 resolveEntry：同步不碰配置文件，撤回只动被处理的那几台
      await refresh(true, verb === "revert" ? actions.map((action) => action.name) : []);
    } catch (error) { feedback.error(String(error)); }
    finally { setResolving(false); }
  };

  const rebuildFromDatabase = async () => {
    if (resolving) return;
    setResolving(true);
    try {
      const count = await api.restoreMcpFromDatabase();
      feedback.success(t("feedback.restoredToLive", { count }));
      setDiffOpen(false);
      await refresh(true);
    } catch (error) { feedback.error(String(error)); }
    finally { setResolving(false); }
  };

  const orderedServers = [...servers].sort(compareMcpServers);
  const editing = diffOpen || Boolean(editingServer) || creatingServer;
  useLayoutEffect(() => {
    onHeaderChange({
      count: loaded ? servers.length : undefined, editing,
      onCreate: () => setCreatingServer(true), onDiff: () => setDiffOpen(true),
    });
  }, [onHeaderChange, loaded, servers.length, editing]);

  if (diffOpen) {
    return (
      <McpDiffPage
        target={target}
        preview={syncPreview}
        previewError={previewError}
        resolving={resolving}
        onBack={() => setDiffOpen(false)}
        onResolve={(entry, verb) => void resolveEntry(entry, verb)}
        onResolveAll={(verb) => void resolveAll(verb)}
        onRebuild={target === "codex" ? () => void rebuildFromDatabase() : undefined}
      />
    );
  }
  if (editingServer || creatingServer) {
    if (target === "claude") return (
      <ClaudeMcpEdit
        server={editingServer}
        create={creatingServer}
        onBack={() => { setEditingServer(null); setCreatingServer(false); }}
        onSaved={(name) => { setEditingServer(null); setCreatingServer(false); void refresh(true, [name]); }}
      />
    );
    return (
      <McpEdit
        server={editingServer}
        create={creatingServer}
        onBack={(savedServer) => {
          setEditingServer(null);
          setCreatingServer(false);
          void (async () => {
            // 保存后必须单独验一次这台的连通性——保存的最终结论就是"它还能不能用"。
            // 只探这一台：其余几台的配置一个字没动，没有理由重连。
            // 顺序不能反：refresh 结尾会拿缓存整体重铺一遍探测结果，而刚改过的这台
            // 指纹已变、缓存里没有它，先探会被刷掉，灯反而灭。所以先等列表对齐再探，
            // 且用表单保存下来的 spec 直接探，不依赖 refresh 的返回值（它可能为 null）。
            try { await refresh(true, []); } catch { /* 列表刷新失败不拖累连通性验证 */ }
            if (savedServer) await probe(savedServer, { manual: false });
          })();
        }}
      />
    );
  }
  return (
    <div className="apple-edit-content">
      {loadError ? (
        <p className="muted mt-4 text-sm">
          {loadError}
          {loaded && target === "codex" ? t("list.loadErrorHint") : ""}
        </p>
      ) : null}
      <div>
        {!servers.length ? (
          <EmptyStateCard loading={!loaded} icon={<McpIcon className="h-5 w-5" />}>
            <p className="muted">{t("empty.description")}</p>
          </EmptyStateCard>
        ) : servers.length ? (
          <div className="apple-group apple-list-card">
            {orderedServers.map((server) => (
              <McpServerRow
                key={server.name}
                server={server}
                result={probeResults[server.name]}
                probing={Boolean(probingNames[server.name])}
                detailsVisible={Boolean(toolsOpen[server.name])}
                toolsBusy={Boolean(toolsLoading[server.name])}
                toolsLoaded={Boolean(toolsLoaded[server.name])}
                onEdit={setEditingServer}
                onDelete={(target) => void removeServer(target)}
                onProbe={(target) => void probe(target)}
                onToggleTools={(target) => void toggleTools(target)}
                onToggleEnabled={(target, enabled) => void toggleEnabled(target, enabled, () => refresh(true, enabled ? [target.name] : []), optimisticToggle)}
              />
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

export default function McpView({ activationEpoch, popEpoch }: { activationEpoch: number; popEpoch: number }) {
  const { t } = useTranslation("mcp");
  const [target, setTarget] = useState<McpTarget>("codex");
  const [header, setHeader] = useState<McpHeaderState | null>(null);
  const targetBadge = useSyncExternalStore(subscribeMcpDiffBadge, () => getMcpDiffBadge(target), () => getMcpDiffBadge(target));

  // 客户端仅上报页头数据；切换器在父层保持同一实例，CSS 才能从旧位置过渡到新位置。
  return (
    <section className={header?.editing ? "contents" : "apple-scroll-page mx-auto w-full max-w-none"}>
      {!header?.editing ? (
        <header className="apple-page-bar flex-wrap justify-between gap-4">
          <ManagementPageTitle
            icon={<span className="settings-icon-tile grid h-9 w-9 shrink-0 place-items-center rounded-[10px] text-accent"><McpIcon className="h-[22px] w-[22px]" /></span>}
            title={t("list.title")}
            count={header?.count}
            countLabel={header?.count !== undefined ? t("list.serverCount", { count: header.count }) : undefined}
          />
          <div className="flex w-full max-w-2xl flex-wrap items-center justify-end gap-2">
            <McpTargetSwitch key="target-switch" value={target} onChange={setTarget} />
            <button type="button" className="apple-action-button app-button--primary" disabled={!header} onClick={header?.onCreate}>
              <Plus className="h-4 w-4" strokeWidth={2} />
              {t("list.addServer")}
            </button>
          </div>
        </header>
      ) : null}
      {!header?.editing && mcpDiffBadgeText(targetBadge) ? (
        <div className="flex items-center gap-3 rounded-[var(--radius-control)] bg-(--selection-bg) px-3 py-2">
          <GitCompare className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} />
          <span className="setting-description min-w-0 flex-1">
            {targetBadge?.error ? t("list.diffUnavailable") : t("list.diffNotice", { count: targetBadge?.count ?? 0 })}
          </span>
          <button type="button" className="apple-inline-btn shrink-0" disabled={!header} onClick={header?.onDiff}>{t("list.viewDiff")}</button>
        </div>
      ) : null}
      <McpClientView key={target} target={target} activationEpoch={activationEpoch} popEpoch={popEpoch} onHeaderChange={setHeader} />
    </section>
  );
}
