import { Fragment, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, Terminal } from "lucide-react";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { clearCachedCliUpdate, getCachedCliStatus, getCachedCliUpdate, runCliUpdateCheck, setCliStatusCache, subscribeCliUpdate, touchCliUpdateCheckedAt } from "../../app/managementDataCache";
import { CliUpgradePill, cliFailure, cliFailureMessage } from "../../components/CliUpgradePill";
import { LoadingSpinner } from "../../components/LoadingSpinner";
import type { CliStatus } from "../../types";

const clients = {
  codex: {
    label: "Codex", icon: "/codex.svg", installer: "https://chatgpt.com/codex/install",
    homepage: "https://openai.com/codex/", homepageLabel: "openai.com/codex",
    description: "cli.description.codex" as const,
    status: api.codexGetCliStatus, check: api.codexCheckCliUpdate, install: api.codexInstallCli,
  },
  claude: {
    label: "Claude Code", icon: "/claude-code.svg", installer: "https://claude.ai/install",
    homepage: "https://claude.com/product/claude-code", homepageLabel: "claude.com/product/claude-code",
    description: "cli.description.claude" as const,
    status: api.claudeGetCliStatus, check: api.claudeCheckCliUpdate, install: api.claudeInstallCli,
  },
};
type Client = keyof typeof clients;
type CliOperation = "refresh" | "check" | "install" | null;

/// 官方安装命令：macOS 走系统 shell 的 curl 管道，Windows 走 PowerShell 的 irm 管道。
function installCommand(client: Client, platform: string) {
  const installer = clients[client].installer;
  if (platform.startsWith("win32-")) {
    return client === "codex"
      ? `powershell -ExecutionPolicy ByPass -c "irm ${installer}.ps1 | iex"`
      : `irm ${installer}.ps1 | iex`;
  }
  return `curl -fsSL ${installer}.sh | ${client === "codex" ? "sh" : "bash"}`;
}

/// 标注安装命令实际使用的 Shell。
const shellLabel = (client: Client, platform: string) => (platform.startsWith("win32-") ? "PowerShell" : client === "codex" ? "sh" : "Bash");

// 在 SettingsView 中调用；切分区只隐藏卡片，不丢失任务和错误。
export function useCliManagement(client: Client, active: boolean) {
  const { t } = useTranslation("settings");
  const { success, error: reportError, info, confirm } = useFeedback();
  // 缓存直出：进分区首帧就渲染上次检测结果，骨架闪帧由静默刷新消除（同备份列表的取缓存写法）。
  const [status, setStatus] = useState<CliStatus | null>(getCachedCliStatus(client));
  const [busy, setBusy] = useState(false);
  const [operation, setOperation] = useState<CliOperation>(null);
  const running = useRef(false);
  const commands = clients[client];
  // 升级结论走共享缓存（与供应商页的胶囊同源），卡片只订阅不另存一份。
  const update = useSyncExternalStore(subscribeCliUpdate, () => getCachedCliUpdate(client), () => getCachedCliUpdate(client));
  // 卡片的 DOM 状态也归这里：卡片是纯展示，长高动画与成功描边由本 hook 驱动。
  const bodyRef = useRef<HTMLDivElement>(null);
  const growFrom = useRef<number | null>(null);
  const [flash, setFlash] = useState(false);

  // 缓存写穿：检测/安装/升级的每个落点都同步进缓存，下次进页直出最新状态。
  const applyStatus = (next: CliStatus) => { setCliStatusCache(client, next); setStatus(next); };

  const refresh = async (): Promise<CliStatus | null> => {
    if (running.current) return null;
    running.current = true;
    setBusy(true);
    setOperation("refresh");
    try {
      const next = await commands.status();
      applyStatus(next);
      return next;
    }
    catch {
      return null;
    }
    finally {
      running.current = false;
      setBusy(false); setOperation(null);
    }
  };

  // 其他页面发起的任务也要跟踪到结束；离页或任务结束即停止检测。
  useEffect(() => {
    if (!active || !status?.busy) return;
    const timer = setInterval(() => void refresh(), 2000);
    return () => clearInterval(timer);
  }, [active, status?.busy]);

  // 安装开始时先钉住当前高度：装完换成「详情」时卡片从旧高连续长过去，不闪跳。
  useEffect(() => {
    if (operation !== "install") return;
    const el = bodyRef.current;
    if (!el) return;
    const height = el.scrollHeight;
    if (height < 1) return;
    growFrom.current = height;
    el.style.height = `${height}px`;
    el.style.overflow = "hidden";
  }, [operation]);

  // 形态变化后按新高度过渡，动画结束交还自适应高度。
  // 只跟形态走：安装开始那一刻形态没变，钉住的高度要留到安装完成再放。
  const shape = `${status?.installation ?? "none"}:${status?.version ?? ""}`;
  useEffect(() => {
    const el = bodyRef.current;
    const from = growFrom.current;
    if (!el || from === null) return;
    growFrom.current = null;
    const to = el.scrollHeight;
    if (Math.abs(from - to) < 1) {
      el.style.transition = "";
      el.style.height = "";
      el.style.overflow = "";
      return;
    }
    el.style.height = `${from}px`;
    void el.offsetHeight; // 强制回流：浏览器先认下旧高度，再过渡到新高度
    el.style.transition = "height 280ms cubic-bezier(0.22, 1, 0.36, 1)";
    el.style.height = `${to}px`;
    const timer = window.setTimeout(() => {
      el.style.transition = "";
      el.style.height = "";
      el.style.overflow = "";
    }, 300);
    return () => window.clearTimeout(timer);
  }, [shape]);

  // 装完的瞬间给一圈成功描边，提示这一步已经完成。
  const previous = useRef(status?.installation);
  useEffect(() => {
    const before = previous.current;
    previous.current = status?.installation;
    if (!before || before === "native" || status?.installation !== "native") return;
    setFlash(true);
    const timer = window.setTimeout(() => setFlash(false), 700);
    return () => window.clearTimeout(timer);
  }, [status?.installation]);

  // 每次重新进入 Agent 工具分区都检测一次，纠正外部安装/升级造成的本地缓存过期；
  // 检测成功且为原生安装时顺带静默检查线上更新。
  // 离开分区时丢弃"当前没有更新"这类一次性反馈（常驻只会变成视觉噪音）；
  // 升级胶囊是可行动状态，保留到真正升级或版本变化。
  useEffect(() => {
    if (!active) return;
    void (async () => {
      const next = await refresh();
      if (next?.installation === "native" && !next.busy) await check(false);
    })();
  }, [active, client]);

  // 静默和主动检查共用状态；静默检查仅更新缓存，不弹通知。
  const check = async (notify = true) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setOperation("check");
    try {
      const result = await runCliUpdateCheck(client);
      applyStatus(result.status);
      if (!notify) return;
      if (result.available) {
        info(t("cli.updateAvailable", { version: result.latest_version, channel: result.channel }));
      } else {
        success(t("cli.noUpdate", { version: result.status.version ?? result.latest_version }));
      }
    } catch (error) {
      touchCliUpdateCheckedAt(client); // 静默和手动失败都推进冷却，定时器不立刻重试
      if (notify) reportError(cliFailureMessage(cliFailure(error, "fetch_version"), client, t));
    }
    finally { running.current = false; setBusy(false); setOperation(null); }
  };

  const run = async () => {
    if (running.current || !status) return;
    running.current = true;
    try {
      if (!await confirm({
        title: t("cli.installConfirmTitle", { client: commands.label }),
        // 命令已经在卡片上展示，弹窗只做确认，不再重复输出终端命令。
        description: t("cli.installConfirmDescription"),
        confirmText: t("cli.install"),
      })) return;
      setBusy(true);
      setOperation("install");
      const result = await commands.install();
      applyStatus(result);
      // 升级/安装成功的权威落点：本机版本已追平官方（后端 verify_update 校验过），
      // 更新缓存立即翻转，供应商页胶囊同步消失，不等下一次检查。
      clearCachedCliUpdate(client);
      success(t("cli.installComplete", {
        client: commands.label,
        version: result.version ?? "",
      }));
    } catch (error) {
      // 失败时 shape 不变，钉住的高度等不到恢复动画，这里手动放掉。
      growFrom.current = null;
      const el = bodyRef.current;
      if (el) { el.style.height = ""; el.style.overflow = ""; }
      reportError(cliFailureMessage(cliFailure(error, "run_cli"), client, t, "install"));
    }
    finally { running.current = false; setBusy(false); setOperation(null); }
  };

  return { status, busy, operation, update, flash, bodyRef, refresh, check, run };
}

export function CliCard({ client, management }: { client: Client; management: ReturnType<typeof useCliManagement> }) {
  const { t } = useTranslation("settings");
  const { success, error: reportError } = useFeedback();
  const { status, busy, operation, update, flash, bodyRef, check, refresh, run } = management;
  const native = status?.installation === "native";
  const unavailable = !status || status.busy || !["missing", "broken", "native"].includes(status.installation);
  // 检查进度放在按钮上；原生安装的外部任务由升级胶囊反馈。
  const showProgress = busy && operation === "install";
  const command = status ? installCommand(client, status.platform) : "";
  const stateText = status && !["missing", "native"].includes(status.installation)
    ? t(status.installation === "broken" ? "cli.broken" : status.installation === "other" ? "cli.other" : "cli.conflict")
    : null;

  const copyCommand = async () => {
    try { await navigator.clipboard.writeText(command); success(t("cli.commandCopied")); }
    catch { reportError(t("cli.copyFailed")); }
  };

  const showUpgrade = native && !busy && (update?.available || status?.busy);
  const upToDate = !update?.available;

  // 中部：原生安装看详情，其余看安装命令（命令块只在需要装的时候出现）。
  const body = !status ? null : native || status.installation === "other" || status.installation === "conflict" ? (
    <section className="cli-card__section">
      <h2 className="setting-title cli-card__section-title">{t("cli.detailsTitle")}</h2>
      {/* dt/dd 直接做 grid 子项：包裹 div 会各自占一列，错开标签与值。 */}
      <dl className="cli-details">
        {status.version ? (<>
          <dt>{t("cli.installed")}</dt>
          <dd className="cli-card__version">
            {status.version}
            {upToDate ? <Check className="cli-card__status" size={16} strokeWidth={3} role="img" aria-label={t("cli.upToDate")}><title>{t("cli.upToDate")}</title></Check> : null}
          </dd>
        </>) : null}
        <dt>{t("cli.pathLabel")}</dt>
        <dd title={status.path ?? undefined}>{status.path ?? t("cli.pathUnavailable")}</dd>
        {status.other_paths.map((path) => (
          <Fragment key={path}><dt /><dd title={path}>{path}</dd></Fragment>
        ))}
        <dt>{t("cli.homepage")}</dt>
        <dd>
          <button type="button" className="cli-card__link" onClick={() => void api.openUrl(clients[client].homepage).catch((error) => reportError(String(error)))}>
            {clients[client].homepageLabel} ↗
          </button>
        </dd>
      </dl>
    </section>
  ) : (
    <section className="cli-card__section">
      <h2 className="setting-title cli-card__section-title">{t("cli.installCommandTitle")}<span className="plan-badge cli-command__platform">{shellLabel(client, status.platform)}</span></h2>
      <div className="cli-command">
        <Terminal size={16} strokeWidth={2} aria-hidden="true" />
        <code className="cli-command__code">{command}</code>
        <button type="button" className="apple-icon-button" title={t("cli.copyCommand")} aria-label={t("cli.copyCommand")} onClick={() => void copyCommand()}>
          <Copy size={16} strokeWidth={2} aria-hidden="true" />
        </button>
      </div>
      <p className="setting-description cli-command__note">{t("cli.commandHint")}</p>
    </section>
  );

  return (
    <div className={`apple-group cli-card${flash ? " cli-card--flash" : ""}`}>
      <div className="cli-card__head">
        <span className="cli-card__icon"><img src={clients[client].icon} alt="" draggable={false} /></span>
        <div className="cli-card__main">
          <div className="cli-card__name-row">
            <span className="setting-title">{clients[client].label}</span>
            {status?.source && status.source !== "embedded" ? (
              <span className="plan-badge cli-card__source">{t(`cli.sources.${client}.${status.source}`, { defaultValue: status.source })}</span>
            ) : null}
            {status?.busy && !native && !showProgress ? (
              <span className="inline-flex items-center gap-1.5 text-(--text-secondary)" role="status" aria-live="polite">
                <LoadingSpinner size="md" />
                <span className="meta-xs">{t("cli.working")}</span>
              </span>
            ) : null}
          </div>
          <p className="setting-description cli-card__desc">{stateText ?? t(clients[client].description)}</p>
        </div>
      </div>
      <div className="cli-card__body" ref={bodyRef}>{body}</div>
      <div className="cli-card__actions">
        {showUpgrade ? <CliUpgradePill client={client} className="apple-action-button app-button--primary" busy={status.busy || busy} onUpdated={() => void refresh()} /> : <button type="button" className={native ? "apple-action-button" : "apple-action-button app-button--primary"}
          style={busy && operation === "refresh" && !unavailable ? { opacity: 1 } : undefined}
          disabled={busy || unavailable} onClick={() => native ? void check() : void run()}>
          {showProgress
            ? <span className="inline-flex items-center gap-1.5" role="status" aria-live="polite"><LoadingSpinner size="md" />{t("cli.installing")}</span>
            : busy && operation === "check"
            ? <span role="status" aria-live="polite">{t("cli.checkingUpdate")}</span>
            : t(native ? "cli.checkUpdate" : "cli.install")}
        </button>}
      </div>
    </div>
  );
}
