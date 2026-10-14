import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { useFeedback } from "../../app/Feedback";
import { authQuotaErrorKind, getAuthQuotaError, getVisibleAuthQuota, setAuthQuotaFailure, setAuthQuotaSuccess } from "../../app/authQuotaCache";
import type { ProfileBalanceInfo } from "../../types";

const balanceInfoCache = new Map<string, ProfileBalanceInfo>();
const balanceErrorCache = new Map<string, string>();

export function getCachedProfileBalance(profileId: string, fallback: ProfileBalanceInfo | null = null, authQuotaKey?: string | null) {
  if (authQuotaKey) return getVisibleAuthQuota(authQuotaKey, balanceInfoCache.get(profileId) ?? fallback);
  if (balanceErrorCache.has(profileId)) return null;
  return balanceInfoCache.get(profileId) ?? fallback;
}

export function getCachedProfileBalanceError(profileId: string, authQuotaKey?: string | null) {
  if (authQuotaKey) return getAuthQuotaError(authQuotaKey);
  return balanceErrorCache.get(profileId) ?? "";
}

interface UseProfileBalanceOptions {
  profileId: string;
  showBalance: boolean;
  supportsBalance: boolean;
  hasCredential: boolean;
  active: boolean;
  activationEpoch: number;
  coldStart: boolean;
  cachedBalance?: ProfileBalanceInfo | null;
  authQuotaKey?: string | null;
  source: "codex" | "claude";
}

export function useProfileBalance({ profileId, showBalance, supportsBalance, hasCredential, active, activationEpoch, coldStart, cachedBalance, authQuotaKey, source }: UseProfileBalanceOptions) {
  const feedback = useFeedback();
  const { t } = useTranslation("profiles");
  const restoreCachedBalance = () => {
    const error = getCachedProfileBalanceError(profileId, authQuotaKey);
    const info = getCachedProfileBalance(profileId, cachedBalance ?? null, authQuotaKey);
    return { error, info: error || !info ? null : info };
  };
  const initial = restoreCachedBalance();
  const [balanceInfos, setBalanceInfos] = useState<ProfileBalanceInfo[]>(() => initial.info ? [initial.info] : []);
  const [balanceError, setBalanceError] = useState(() => initial.error);
  const [balanceRefreshing, setBalanceRefreshing] = useState(false);
  const balanceInFlightRef = useRef<Promise<void> | null>(null);
  const invalidateBalance = (message: string) => {
    setBalanceInfos([]);
    balanceInfoCache.delete(profileId);
    setBalanceError(message);
    balanceErrorCache.set(profileId, message);
    if (authQuotaKey) setAuthQuotaFailure(authQuotaKey, message);
  };

  const fetchBalance = async (manual = false): Promise<void> => {
    if (balanceInFlightRef.current) return balanceInFlightRef.current;
    if (!supportsBalance || !showBalance) return;
    if (!hasCredential) {
      invalidateBalance(t("balance.missingApiKey"));
      return;
    }
    const request = (async () => {
      try {
        const result = source === "claude" ? await api.claudeGetProfileBalance(profileId) : await api.codexGetProfileBalance(profileId);
        const infos = result.balance_infos;
        if (!infos[0]) throw new Error("查询未返回余额/用量数据"); // i18n-exempt: 该消息只被当布尔用，界面渲染的是固定文案 balance.queryFailed
        setBalanceError("");
        balanceErrorCache.delete(profileId);
        setBalanceInfos(infos);
        balanceInfoCache.set(profileId, infos[0]);
        void api.setProfileBalance(profileId, infos[0]);
        if (authQuotaKey) {
          setAuthQuotaSuccess(authQuotaKey, infos[0]);
          void api.setProfileBalance(authQuotaKey, infos[0]);
        }
      } catch (error) {
        const message = String(error);
        invalidateBalance(message);
        if (manual) {
          const authInvalid = authQuotaErrorKind(message) === "auth_expired";
          feedback.error(t(authInvalid ? "balance.authInvalidToast" : "balance.queryFailedToast"));
        }
      } finally {
        balanceInFlightRef.current = null;
      }
    })();
    balanceInFlightRef.current = request;
    return request;
  };

  const refreshBalance = (manual = true) => {
    setBalanceRefreshing(true);
    void fetchBalance(manual).finally(() => setBalanceRefreshing(false));
  };

  useEffect(() => {
    if (!supportsBalance) return;
    const { error, info } = restoreCachedBalance();
    setBalanceInfos((current) => current.length === (info ? 1 : 0) && (info ? current[0] === info : true) ? current : info ? [info] : []);
    setBalanceError(error);
    const timer = window.setTimeout(() => void fetchBalance(), coldStart ? (active ? 900 : 1200) : 0);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activationEpoch, profileId, showBalance, supportsBalance, authQuotaKey]);

  useEffect(() => {
    if (!active || !supportsBalance || !showBalance) return;
    const timer = window.setInterval(() => void fetchBalance(), 5 * 60 * 1000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, profileId, showBalance, supportsBalance]);

  return { balanceInfos, balanceError, balanceRefreshing, refreshBalance };
}
