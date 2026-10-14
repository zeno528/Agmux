import type { TFunction } from "i18next";
import type { CodexProfileConnectionResult } from "../../types";
import { authQuotaErrorKind } from "../../app/authQuotaCache";

const httpErrorKinds = {
  400: "badRequest", 401: "unauthorized", 402: "billing", 403: "forbidden",
  404: "notFound", 405: "methodNotAllowed", 408: "requestTimeout",
  415: "unsupportedMedia", 422: "unprocessable", 429: "rateLimited",
  500: "serverError", 502: "badGateway", 503: "unavailable", 504: "gatewayTimeout", 529: "overloaded",
} as const;

/** 两端共用状态码与通知译文；供应商详情保持原文。 */
export function connectionErrorMessage(result: CodexProfileConnectionResult, t: TFunction<"profiles">) {
  const status = result.status;
  const kind = (status != null ? httpErrorKinds[status as keyof typeof httpErrorKinds] : undefined)
    ?? (status != null && status >= 500 ? "serverError"
    : result.error === "invalidEndpoint" ? "invalidEndpoint"
    : result.error === "timeout" ? "timeout"
    : result.error === "unreachable" ? "unreachable" : "requestFailed");
  const hint = t(`connection.errors.${kind}`);
  const detail = status != null && result.error && result.error !== "requestFailed" ? `; ${result.error}` : "";
  return `${status != null ? `HTTP ${status}: ` : ""}${hint}${detail}`;
}

/** 获取模型沿用 IPC 错误字符串，将 HTTP 状态和网络错误代码送入同一解析。 */
export function connectionExceptionMessage(error: unknown, t: TFunction<"profiles">) {
  const message = error instanceof Error ? error.message : String(error);
  const match = /^HTTP (\d{3}):\s*(.*)$/s.exec(message);
  return connectionErrorMessage({ ok: false, latency_ms: null,
    status: match ? Number(match[1]) : null, error: match ? match[2] : message }, t);
}

/** Codex 登录失效提示也必须保留本次响应的真实状态码。 */
export function connectionFailureMessage(error: string, t: TFunction<"profiles">, status: number | null = null) {
  const expired = authQuotaErrorKind(error) === "auth_expired";
  const message = expired ? t("balance.authInvalidToast") : error;
  const prefix = status != null && !message.startsWith(`HTTP ${status}:`) ? `HTTP ${status}: ` : "";
  return t(expired ? "connection.testFailed" : "connection.failed", { error: `${prefix}${message}` });
}
