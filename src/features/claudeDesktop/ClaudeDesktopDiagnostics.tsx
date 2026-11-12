import { Activity, ArrowRight, ArrowUpRight, ChartNoAxesColumn, ChevronLeft, Monitor, Route, Server } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { AppDisclosure } from "../../components/AppDisclosure";
import { AppTabBar } from "../../components/AppTabBar";
import { AppTooltip } from "../../components/AppTooltip";
import { EmptyStateCard } from "../../components/EmptyStateCard";
import type { ClaudeDesktopDiagnostics as Diagnostics, ClaudeDesktopModelUsage, ClaudeDesktopTokenUsage } from "../../types";

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

export default function ClaudeDesktopDiagnostics({ onBack }: { onBack: () => void }) {
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
  const hosts = [...new Set(data?.routes.map((route) => route.host) ?? [])];
  const connection = !data?.active_name ? "notApplied" : !data.live_matches ? "mismatched" : direct ? "directReady" : !data.gateway_running ? "stopped" : totals.completed > 0 ? "observed" : "waiting";
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
          <section className="desktop-diagnostics__connection" aria-label={t("diagnostics.flow")}>
            <div className="desktop-diagnostics__section-heading">
              <h2 className="title-sm">{t(`modes.${data.mode}`)}<span className="desktop-diagnostics__meta"> · {data.active_name ?? t("diagnostics.notApplied")}</span></h2>
              <span className={`apple-chip ${connection === "observed" ? "chip-success" : ["mismatched", "stopped"].includes(connection) ? "chip-warn" : ""}`}>{t(`diagnostics.${connection}`)}</span>
            </div>
            <div className="desktop-diagnostics__flow" data-direct={direct || undefined}>
              <div className="desktop-diagnostics__node"><div className="desktop-diagnostics__node-title"><Monitor size={17} aria-hidden="true" /><span>Claude Desktop</span></div><span className={`desktop-diagnostics__node-state ${data.live_matches ? "text-success" : ""}`}><span className="desktop-diagnostics__dot" />{t(!data.active_name ? "diagnostics.notApplied" : data.live_matches ? "diagnostics.matched" : "diagnostics.mismatched")}</span></div>
              <ArrowRight className="desktop-diagnostics__arrow" size={18} aria-hidden="true" />
              {!direct ? <><div className="desktop-diagnostics__node"><div className="desktop-diagnostics__node-title"><Route size={17} aria-hidden="true" /><span>{t("diagnostics.gatewayNode")}</span></div><span className={`desktop-diagnostics__node-state ${data.gateway_running ? "text-success" : ""}`}><span className="desktop-diagnostics__dot" />{t(data.gateway_running ? "diagnostics.running" : "diagnostics.stopped")}</span><code className="desktop-diagnostics__meta">{data.gateway_url ?? "—"}</code></div><ArrowRight className="desktop-diagnostics__arrow" size={18} aria-hidden="true" /></> : null}
              <div className="desktop-diagnostics__node"><div className="desktop-diagnostics__node-title"><Server size={17} aria-hidden="true" /><span>{direct ? data.active_name ?? t("diagnostics.targetNode") : t("diagnostics.routeCount", { routes: data.routes.length })}</span></div><span className="desktop-diagnostics__meta">{direct ? t("diagnostics.bypassed") : hosts.join(" · ") || t("diagnostics.noRoutes")}</span></div>
            </div>
            <div className="desktop-diagnostics__flow-footer"><span>{t(direct ? "diagnostics.directCaption" : data.mode === "route" ? "diagnostics.routeCaption" : "diagnostics.aggregateCaption")}</span><AppTooltip label={t("diagnostics.flow")}><p className="setting-title">{t("diagnostics.flow")}</p><p className="setting-description">{t(direct ? "diagnostics.directScope" : data.mode === "route" ? "diagnostics.routeFlow" : "diagnostics.aggregateFlow")}</p></AppTooltip></div>
          </section>
          {!direct ? <>
            <section aria-label={t("diagnostics.usage")}>
              <div className="desktop-diagnostics__section-heading"><div className="flex items-center gap-1"><h2 className="title-sm">{t("diagnostics.session")}</h2><AppTooltip label={t("diagnostics.usageScopeLabel")}><p className="setting-title">{t("diagnostics.usageScopeLabel")}</p><p className="setting-description">{t("diagnostics.usageScope")}</p></AppTooltip></div><span className="desktop-diagnostics__meta">{data.activity.started_at_ms !== null ? t("diagnostics.since", { time: new Date(data.activity.started_at_ms).toLocaleTimeString() }) : t("diagnostics.notStarted")}</span></div>
              <dl className="desktop-diagnostics__metrics">
                <div><dt>{t("diagnostics.totalCalls")}</dt><dd>{number(totals.requests)}</dd><span>{t("diagnostics.activeCalls", { pending: totals.requests - totals.completed - totals.failed - totals.cancelled })}</span></div>
                <div><dt>{t("diagnostics.completedCalls")}</dt><dd>{number(totals.completed)}</dd><span className={totals.failed ? "chip-warn" : ""}>{t("diagnostics.otherCalls", { failed: totals.failed, cancelled: totals.cancelled })}</span></div>
                <div><dt>{t("diagnostics.input_tokens")}</dt><dd className={totals.input === null ? "desktop-diagnostics__unavailable" : undefined}>{number(totals.input)}</dd><span>{t("diagnostics.reported", { reported: totals.reported })}</span></div>
                <div><dt>{t("diagnostics.output_tokens")}</dt><dd className={totals.output === null ? "desktop-diagnostics__unavailable" : undefined}>{number(totals.output)}</dd><span>{t("diagnostics.reportedOnly")}</span></div>
              </dl>
            </section>
            <section>
              <div className="settings-tab-bar desktop-diagnostics__tab-row"><AppTabBar items={[{ id: "usage", label: t("diagnostics.usage"), icon: <ChartNoAxesColumn size={16} /> }, { id: "requests", label: t("diagnostics.requests"), icon: <Activity size={16} /> }]} selected={tab} onSelect={setTab} label={t("diagnostics.details")} /><AppTooltip label={t("diagnostics.requestScopeLabel")}><p className="setting-title">{t("diagnostics.requestScopeLabel")}</p><p className="setting-description">{t("diagnostics.requestScope")}</p></AppTooltip></div>
              {tab === "usage" ? data.activity.models.length ? <div className="desktop-diagnostics__table-scroll"><table className="desktop-diagnostics__table" aria-label={t("diagnostics.usage")}>
                <thead><tr><th scope="col">{t("diagnostics.model")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.calls")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.inputOutput")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.cachePair")}</th></tr></thead>
                <tbody>{data.activity.models.map((model) => <tr key={model.route.desktop_model}>
                  <td><div className="desktop-diagnostics__identity">{model.route.label}</div><code className="desktop-diagnostics__meta block">{model.route.desktop_model}</code></td>
                  <td className="desktop-diagnostics__numeric"><div>{number(model.requests)}</div><div className="desktop-diagnostics__meta">{t("diagnostics.modelOutcome", { completed: model.completed, failed: model.failed })}</div></td>
                  <td className="desktop-diagnostics__numeric">{tokenPair(model.tokens)}</td><td className="desktop-diagnostics__numeric">{tokenPair(model.tokens, true)}</td>
                </tr>)}</tbody>
              </table></div> : <div className="desktop-diagnostics__empty"><ChartNoAxesColumn size={24} aria-hidden="true" /><h3>{t("diagnostics.noUsageTitle")}</h3><p>{t("diagnostics.noUsage")}</p></div>
                : data.activity.requests.length ? <div className="desktop-diagnostics__table-scroll"><table className="desktop-diagnostics__table" aria-label={t("diagnostics.requests")}>
                  <thead><tr><th scope="col">{t("diagnostics.time")}</th><th scope="col">{t("diagnostics.model")}</th><th scope="col">{t("diagnostics.result")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.latency")}</th><th scope="col" className="desktop-diagnostics__numeric">{t("diagnostics.inputOutput")}</th></tr></thead>
                  <tbody>{data.activity.requests.map((event, index) => <tr key={`${event.at_ms}-${index}`}>
                    <td className="desktop-diagnostics__time">{new Date(event.at_ms).toLocaleTimeString()}</td>
                    <td><div className="desktop-diagnostics__identity">{event.route?.label ?? t("diagnostics.local")}</div><div className="desktop-diagnostics__meta">{event.route?.upstream_model ?? event.path}</div></td>
                    <td><span className={`apple-chip ${event.outcome === "completed" ? "chip-success" : event.outcome === "pending" ? "" : "chip-warn"}`}>{t(`diagnostics.outcomes.${event.outcome}`)}</span><div className="desktop-diagnostics__meta">{event.status !== null ? t(event.upstream_status === null ? "diagnostics.localStatus" : "diagnostics.upstreamStatus", { status: event.status }) : "—"}</div></td>
                    <td className="desktop-diagnostics__numeric"><div>{number(event.first_chunk_ms)}</div><div className="desktop-diagnostics__meta">{number(event.duration_ms)}</div></td><td className="desktop-diagnostics__numeric">{tokenPair(event.usage)}</td>
                  </tr>)}</tbody>
                </table></div> : <div className="desktop-diagnostics__empty"><Activity size={24} aria-hidden="true" /><h3>{t("diagnostics.noRequestsTitle")}</h3><p>{t("diagnostics.noRequests")}</p></div>}
            </section>
            <AppDisclosure open={routesOpen} onOpenChange={setRoutesOpen} className="desktop-diagnostics__routes" summary={<><Route size={16} aria-hidden="true" /><span className="setting-title">{t("diagnostics.routes")}</span><span className="desktop-diagnostics__meta">{data.routes.length}</span></>}>
              {data.routes.length ? <div className="desktop-diagnostics__route-list">{data.routes.map((route) => <div key={route.desktop_model} className="desktop-diagnostics__route"><code>{route.desktop_model}</code><ArrowRight size={16} aria-hidden="true" /><div><div>{route.label}</div><div className="desktop-diagnostics__meta">{route.upstream_model} · {route.host}</div></div></div>)}</div> : <p className="setting-description">{t("diagnostics.noRoutes")}</p>}
            </AppDisclosure>
          </> : <div className="desktop-diagnostics__direct-note"><ArrowUpRight size={20} aria-hidden="true" /><div><h2 className="setting-title">{t("diagnostics.directUsageTitle")}</h2><p className="setting-description">{t("diagnostics.directScope")}</p></div></div>}
        </> : null}
      </div>
    </div>
  </section>;
}
