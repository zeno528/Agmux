import { Activity, ArrowRight, ArrowUpRight, ChartNoAxesColumn, ChevronLeft, Route, Server } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppDisclosure } from "../../components/AppDisclosure";
import { AppTabBar } from "../../components/AppTabBar";
import { AppTooltip } from "../../components/AppTooltip";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import { ProfileIconTile } from "../../components/ProfileIconTile";
import type { ClaudeDesktopDiagnostics as Diagnostics, ClaudeDesktopModelUsage, ClaudeDesktopProfileSummary, ClaudeDesktopTokenUsage } from "../../types";

const number = (value: number | null) => value === null ? "—" : value.toLocaleString();

export function summarizeModelUsage(models: ClaudeDesktopModelUsage[]) {
  return models.reduce((total, model) => ({
    requests: total.requests + model.requests,
    completed: total.completed + model.completed,
    failed: total.failed + model.failed,
    cancelled: total.cancelled + model.cancelled,
    reported: total.reported + model.usage_reported,
    input: model.tokens.input_tokens === null ? total.input : (total.input ?? 0) + model.tokens.input_tokens,
    output: model.tokens.output_tokens === null ? total.output : (total.output ?? 0) + model.tokens.output_tokens,
  }), { requests: 0, completed: 0, failed: 0, cancelled: 0, reported: 0, input: null as number | null, output: null as number | null });
}

export default function ClaudeDesktopDiagnostics({ activeProfile, onBack }: { activeProfile?: ClaudeDesktopProfileSummary; onBack: () => void }) {
  const { t } = useTranslation("claudeDesktop");
  const feedback = useFeedback();
  const [data, setData] = useState<Diagnostics | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<"usage" | "requests">("usage");
  const [routesOpen, setRoutesOpen] = useState(false);
  const [refreshEpoch, setRefreshEpoch] = useState(0);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  useEffect(() => {
    let alive = true;
    let pending = false;
    let reported = false;
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const next = await api.claudeDesktopGetDiagnostics();
        if (alive) { setData(next); setError(""); setUpdatedAt(new Date()); }
      } catch (error) {
        if (alive) {
          setError(String(error));
          if (!reported) { feedback.error(String(error)); reported = true; }
        }
      } finally { pending = false; }
    };
    void load();
    const timer = window.setInterval(() => void load(), 2000);
    return () => { alive = false; window.clearInterval(timer); };
  }, [refreshEpoch]);

  const totals = summarizeModelUsage(data?.activity.models ?? []);
  const direct = data?.mode === "direct";
  const pendingCalls = totals.requests - totals.completed - totals.failed - totals.cancelled;
  const providerName = data?.mode === "aggregate" ? null : data?.active_name;
  const providerIcon = providerName === activeProfile?.name ? activeProfile?.icon ?? null : null;
  const hosts = [...new Set(data?.routes.map((route) => route.host) ?? [])];
  const upstreamModels = [...new Set(data?.routes.map((route) => route.upstream_model) ?? [])];
  const connection = !data?.active_name ? "notApplied"
    : !data.live_matches ? "mismatched"
    : direct ? "directReady"
    : !data.gateway_running ? "stopped"
    : !error && pendingCalls > 0 ? "processing"
    : totals.completed > 0 ? "observed"
    : "waiting";
  const flowLink = (label: string) => (
    <div className="desktop-diagnostics__link">
      <span>{label}</span>
      <span className="desktop-diagnostics__wire" aria-hidden="true" />
    </div>
  );
  const tokenPair = (usage: ClaudeDesktopTokenUsage, cache = false) => <>
    <div className="desktop-diagnostics__number">{number(cache ? usage.cache_read_input_tokens : usage.input_tokens)}</div>
    <div className="desktop-diagnostics__meta desktop-diagnostics__number">{number(cache ? usage.cache_creation_input_tokens : usage.output_tokens)}</div>
  </>;

  return <section className="apple-edit-page desktop-diagnostics mx-auto flex w-full max-w-none flex-col">
    <div className="apple-page-bar apple-edit-toolbar apple-edit-toolbar--header">
      <h1><button type="button" className="apple-page-header apple-back-button" aria-label={t("edit.back", { ns: "profiles" })} onClick={onBack}>
        <ChevronLeft className="h-4 w-4 shrink-0 text-accent" strokeWidth={2} aria-hidden="true" />
        <span className="apple-title">{t("diagnostics.title")}</span>
      </button></h1>
      <div className="desktop-diagnostics__freshness">
        <span>{t(error ? "diagnostics.stale" : "diagnostics.autoRefresh")}</span>
        <AppTooltip label={t("diagnostics.about")}><p className="setting-title">{t("diagnostics.title")}</p><p className="setting-description">{t("diagnostics.scope")}</p></AppTooltip>
      </div>
    </div>
    <div className="apple-edit-content">
      <div className="apple-edit-surface desktop-diagnostics__content">
        {error ? <div className="desktop-diagnostics__error" role="alert"><div><p>{error}</p>{updatedAt ? <p className="desktop-diagnostics__meta">{t("diagnostics.lastUpdated", { time: updatedAt.toLocaleTimeString() })}</p> : null}</div><button type="button" className="apple-action-button" onClick={() => setRefreshEpoch((value) => value + 1)}>{t("diagnostics.retry")}</button></div> : null}
        {!data && !error ? <EmptyStateCard loading icon={<Activity size={20} />}>{null}</EmptyStateCard> : null}
        {data ? <>
          <section aria-labelledby="desktop-connection-title">
            <div className="desktop-diagnostics__section-heading desktop-diagnostics__connection-heading">
              <div>
                <h2 className="title-md" id="desktop-connection-title" aria-live="polite">{t(`diagnostics.${connection}`)}</h2>
                {connection === "waiting" ? <p className="setting-description">{t("diagnostics.verifyRequest")}</p> : null}
              </div>
              <div className="flex items-center gap-1.5">
                <span className="apple-chip">{t(`modes.${data.mode}`)}</span>
                <AppTooltip label={t("diagnostics.flow")}><p className="setting-title">{t("diagnostics.flow")}</p><p className="setting-description">{t(direct ? "diagnostics.directScope" : data.mode === "route" ? "diagnostics.routeFlow" : "diagnostics.aggregateFlow")}</p></AppTooltip>
              </div>
            </div>
            <div className="desktop-diagnostics__flow-frame">
              <div className="desktop-diagnostics__flow" role="group" aria-label={t("diagnostics.flow")} data-direct={direct || undefined} data-state={connection}>
                <div className="panel desktop-diagnostics__node">
                  <h3 className="desktop-diagnostics__node-title"><span className="desktop-diagnostics__node-icon"><img src="/claude.svg" alt="" className="size-4" draggable="false" /></span>Claude Desktop</h3>
                  <span className={data.live_matches ? "text-success" : "muted"}>{t(!data.active_name ? "diagnostics.notApplied" : data.live_matches ? "diagnostics.matched" : "diagnostics.mismatched")}</span>
                </div>
                {flowLink(t(direct ? "modes.direct" : "diagnostics.local"))}
                {!direct ? <>
                  <div className="panel desktop-diagnostics__node desktop-diagnostics__node--gateway">
                    <h3 className="desktop-diagnostics__node-title"><span className="desktop-diagnostics__node-icon"><Route size={17} aria-hidden="true" /></span>{t("diagnostics.gatewayNode")}</h3>
                    <span className={data.gateway_running ? "text-success" : "muted"}>{t(data.gateway_running ? "diagnostics.running" : "diagnostics.stopped")}</span>
                    {data.gateway_url ? <code className="desktop-diagnostics__meta">{data.gateway_url}</code> : null}
                  </div>
                  {flowLink(t("diagnostics.forward"))}
                </> : null}
                <div className="panel desktop-diagnostics__node">
                  <h3 className="desktop-diagnostics__node-title">
                    {providerName ? <ProfileIconTile name={providerName} icon={providerIcon} size="xs" /> : <span className="desktop-diagnostics__node-icon"><Server size={17} aria-hidden="true" /></span>}
                    {providerName ?? t("diagnostics.targetNode")}
                  </h3>
                  <span className={connection === "observed" ? "text-success" : "muted"}>
                    {t(direct ? "diagnostics.bypassed" : connection === "processing" ? "diagnostics.awaitingUpstream" : connection === "observed" ? "diagnostics.observed" : data.routes.length ? "diagnostics.routeCount" : "diagnostics.noRoutes", { routes: data.routes.length })}
                  </span>
                  {providerName && upstreamModels.length === 1 ? <code className="desktop-diagnostics__meta">{upstreamModels[0]}</code> : null}
                  {hosts.length === 1 ? <code className="desktop-diagnostics__meta">{hosts[0]}</code> : null}
                </div>
              </div>
            </div>
            {!direct ? (
              <AppDisclosure open={routesOpen} onOpenChange={setRoutesOpen} className="desktop-diagnostics__routes" summary={<><Route size={16} aria-hidden="true" /><span className="setting-title">{t("diagnostics.routes")}</span><span className="desktop-diagnostics__meta">{data.routes.length}</span></>}>
                {data.routes.length ? <div className="desktop-diagnostics__route-list">{data.routes.map((route) => <div key={route.desktop_model} className="desktop-diagnostics__route"><code>{route.desktop_model}</code><ArrowRight size={16} aria-hidden="true" /><div><div>{route.label}</div><div className="desktop-diagnostics__meta">{route.upstream_model} · {route.host}</div></div></div>)}</div> : <p className="setting-description">{t("diagnostics.noRoutes")}</p>}
              </AppDisclosure>
            ) : null}
          </section>
          {!direct ? (
            <section aria-labelledby="desktop-activity-title">
              <div className="desktop-diagnostics__section-heading mb-2">
                <div className="flex items-center gap-1">
                  <h2 className="title-sm" id="desktop-activity-title">{t("diagnostics.session")}</h2>
                  <AppTooltip label={t("diagnostics.requestScopeLabel")}>
                    <p className="setting-description">{t("diagnostics.usageScope")}</p>
                    <p className="setting-description mt-2">{t("diagnostics.requestScope")}</p>
                  </AppTooltip>
                </div>
                <span className="desktop-diagnostics__meta">{data.activity.started_at_ms !== null ? t("diagnostics.since", { time: new Date(data.activity.started_at_ms).toLocaleTimeString() }) : t("diagnostics.notStarted")}</span>
              </div>
              <div className="panel desktop-diagnostics__activity">
                <dl className="desktop-diagnostics__metrics">
                  <div><dt>{t("diagnostics.totalCalls")}</dt><dd>{number(totals.requests)}</dd>{pendingCalls > 0 ? <span>{t("diagnostics.activeCalls", { pending: pendingCalls })}</span> : null}</div>
                  <div><dt>{t("diagnostics.completedCalls")}</dt><dd>{number(totals.completed)}</dd>{totals.failed || totals.cancelled ? <span className={totals.failed ? "chip-warn" : ""}>{t("diagnostics.otherCalls", { failed: totals.failed, cancelled: totals.cancelled })}</span> : null}</div>
                  <div><dt>{t("diagnostics.input_tokens")}</dt><dd className={totals.input === null ? "desktop-diagnostics__unavailable" : undefined}>{number(totals.input)}</dd>{totals.reported > 0 ? <span>{t("diagnostics.reported", { reported: totals.reported })}</span> : null}</div>
                  <div><dt>{t("diagnostics.output_tokens")}</dt><dd className={totals.output === null ? "desktop-diagnostics__unavailable" : undefined}>{number(totals.output)}</dd></div>
                </dl>
                <AppTabBar items={[{ id: "usage", label: t("diagnostics.usage"), icon: <ChartNoAxesColumn size={16} /> }, { id: "requests", label: t("diagnostics.requests"), icon: <Activity size={16} /> }]} selected={tab} onSelect={setTab} label={t("diagnostics.details")} />
                {tab === "usage" ? data.activity.models.length ? <div className="desktop-diagnostics__table-scroll"><table className="desktop-diagnostics__table" aria-label={t("diagnostics.usage")}>
                  <thead><tr><th scope="col">{t("diagnostics.model")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.calls")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.inputOutput")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.cachePair")}</th></tr></thead>
                  <tbody>{data.activity.models.map((model) => <tr key={model.route.desktop_model}>
                    <td><div className="desktop-diagnostics__identity">{model.route.label}</div><code className="desktop-diagnostics__meta block">{model.route.desktop_model}</code></td>
                    <td className="desktop-diagnostics__numeric"><div>{number(model.requests)}</div><div className="desktop-diagnostics__meta">{t("diagnostics.modelOutcome", { completed: model.completed, failed: model.failed })}</div></td>
                    <td className="desktop-diagnostics__numeric">{tokenPair(model.tokens)}</td><td className="desktop-diagnostics__numeric">{tokenPair(model.tokens, true)}</td>
                  </tr>)}</tbody>
                </table></div> : <div className="desktop-diagnostics__empty"><h3>{t("diagnostics.noUsageTitle")}</h3><p>{t("diagnostics.noUsage")}</p></div>
                  : data.activity.requests.length ? <div className="desktop-diagnostics__table-scroll"><table className="desktop-diagnostics__table" aria-label={t("diagnostics.requests")}>
                    <thead><tr><th scope="col">{t("diagnostics.time")}</th><th scope="col">{t("diagnostics.model")}</th><th scope="col">{t("diagnostics.result")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.latency")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.inputOutput")}</th></tr></thead>
                    <tbody>{data.activity.requests.map((event, index) => <tr key={`${event.at_ms}-${index}`}>
                      <td className="desktop-diagnostics__time">{new Date(event.at_ms).toLocaleTimeString()}</td>
                      <td><div className="desktop-diagnostics__identity">{event.route?.label ?? t("diagnostics.local")}</div><div className="desktop-diagnostics__meta">{event.route?.upstream_model ?? event.path}</div></td>
                      <td><span className={`apple-chip ${event.outcome === "completed" ? "chip-success" : event.outcome === "pending" ? "" : "chip-warn"}`}>{t(`diagnostics.outcomes.${event.outcome}`)}</span><div className="desktop-diagnostics__meta">{event.status !== null ? t(event.upstream_status === null ? "diagnostics.localStatus" : "diagnostics.upstreamStatus", { status: event.status }) : "—"}</div></td>
                      <td className="desktop-diagnostics__numeric"><div>{number(event.first_chunk_ms)}</div><div className="desktop-diagnostics__meta">{number(event.duration_ms)}</div></td><td className="desktop-diagnostics__numeric">{tokenPair(event.usage)}</td>
                    </tr>)}</tbody>
                  </table></div> : <div className="desktop-diagnostics__empty"><h3>{t("diagnostics.noRequestsTitle")}</h3></div>}
              </div>
            </section>
          ) : <div className="desktop-diagnostics__direct-note"><ArrowUpRight size={20} aria-hidden="true" /><div><h2 className="setting-title">{t("diagnostics.directUsageTitle")}</h2><p className="setting-description">{t("diagnostics.directScope")}</p></div></div>}
        </> : null}
      </div>
    </div>
  </section>;
}
