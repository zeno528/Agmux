import type { ProfileBalanceInfo, CodexProfileSummary } from "../types";

type AuthSource = "desktop" | "oauth";

const quotaCache = new Map<string, ProfileBalanceInfo>();
const errorCache = new Map<string, string>();

export function authQuotaCacheKey(source: AuthSource, accountId: string) {
  return `auth:${source}:${accountId}`;
}

export function profileAuthQuotaCacheKey(profile: CodexProfileSummary) {
  if (profile.kind !== "official") return null;
  const source = profile.auth_source ?? (profile.account_id ? "oauth" : "desktop");
  const accountId = profile.account_id ?? profile.auth_account_id;
  return accountId ? authQuotaCacheKey(source, accountId) : null;
}

export function getAuthQuotaBalance(cacheKey: string) {
  return quotaCache.get(cacheKey) ?? null;
}

export function getAuthQuotaError(cacheKey: string) {
  return errorCache.get(cacheKey) ?? "";
}

export function getVisibleAuthQuota(cacheKey: string, fallback: ProfileBalanceInfo | null) {
  if (getAuthQuotaError(cacheKey)) return null;
  return getAuthQuotaBalance(cacheKey) ?? fallback;
}

export function setAuthQuotaFailure(cacheKey: string, message: string) {
  errorCache.set(cacheKey, message);
}

/** 重新授权成功后调用：该账号的旧失败态即刻作废，下次挂载按无错误路径自动刷新 */
export function clearAuthQuotaError(cacheKey: string) {
  errorCache.delete(cacheKey);
}

export function setAuthQuotaSuccess(cacheKey: string, balance: ProfileBalanceInfo) {
  quotaCache.set(cacheKey, balance);
  errorCache.delete(cacheKey);
}

export function authQuotaErrorKind(message: string): "auth_missing" | "auth_expired" | "query_failed" {
  const normalized = message.trim().toLowerCase();
  if (normalized === "该 codex 配置尚未保存有效登录" // i18n-exempt: Match backend auth errors.
    || normalized === "未检测到有效的 codex 登录" // i18n-exempt: Match backend auth errors.
    || normalized === "oauth 配置未绑定订阅账号" // i18n-exempt: Match backend auth errors.
    || normalized === "官方配置缺少登录方式" // i18n-exempt: Match backend auth errors.
    || normalized === "官方配置缺少认证来源") return "auth_missing"; // i18n-exempt: Match backend auth errors.
  const refreshTokenRejected = normalized.includes("refresh_token")
    && ["拒绝", "invalid", "revok", "expired", "失效", "过期"].some((part) => normalized.includes(part)); // i18n-exempt: Match backend auth error text.
  const expired = normalized === "auth_expired"
    || normalized.includes("refresh token invalid")
    || normalized.includes("refresh token expired")
    || normalized.includes("refresh token 失效") // i18n-exempt: Match backend auth error text.
    || refreshTokenRejected
    || normalized.includes("invalid_grant")
    || normalized.includes("登录已失效") // i18n-exempt: Match backend auth error text.
    || normalized.includes("登录凭证已失效"); // i18n-exempt: Match backend auth error text.
  return expired ? "auth_expired" : "query_failed";
}
