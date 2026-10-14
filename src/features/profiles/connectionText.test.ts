import { expect, it } from "vitest";
import i18next from "i18next";
import { setupI18n } from "../../i18n";
import { connectionErrorMessage, connectionExceptionMessage, connectionFailureMessage } from "./connectionText";

it("连通提示跟随界面语言，保留状态码与供应商详情", () => {
  setupI18n("en-US");
  for (const language of ["zh-CN", "en-US"] as const) {
    const t = i18next.getFixedT(language, "profiles");
    for (const [status, kind] of [
      [400, "badRequest"], [401, "unauthorized"], [402, "billing"], [403, "forbidden"],
      [404, "notFound"], [405, "methodNotAllowed"], [408, "requestTimeout"],
      [415, "unsupportedMedia"], [422, "unprocessable"], [429, "rateLimited"],
      [500, "serverError"], [502, "badGateway"], [503, "unavailable"], [504, "gatewayTimeout"], [529, "overloaded"],
    ] as const) {
      const message = connectionErrorMessage({ ok: false, status, error: "provider detail", latency_ms: 1 }, t);
      expect(message).toBe(`HTTP ${status}: ${t(`connection.errors.${kind}`)}; provider detail`);
      if (language === "en-US") expect(message).not.toMatch(/[\u4e00-\u9fff]/);
    }
    for (const error of ["invalidEndpoint", "timeout", "unreachable", "requestFailed"] as const) {
      expect(connectionErrorMessage({ ok: false, status: null, error, latency_ms: null }, t)).toBe(t(`connection.errors.${error}`));
    }
    for (const status of [409, 413, 499]) {
      expect(connectionErrorMessage({ ok: false, status, error: "provider detail", latency_ms: 1 }, t)).toBe(`HTTP ${status}: ${t("connection.errors.requestFailed")}; provider detail`);
    }
    expect(connectionErrorMessage({ ok: false, status: 200, error: "body code 401", latency_ms: 1 }, t)).toBe(`HTTP 200: ${t("connection.errors.requestFailed")}; body code 401`);
    expect(connectionFailureMessage("auth_expired", t, 401)).toBe(t("connection.testFailed", { error: `HTTP 401: ${t("balance.authInvalidToast")}` }));
    expect(connectionFailureMessage("HTTP 429: limited", t, 429)).toBe(t("connection.failed", { error: "HTTP 429: limited" }));
  }
});

it("英文获取模型的失败通知不拼接后端中文", () => {
  setupI18n("en-US");
  const t = i18next.getFixedT("en-US", "profiles");
  for (const error of ["请求失败: error sending request for url", "timeout", "HTTP 404:", "HTTP 401: unauthorized"]) {
    const message = connectionExceptionMessage(error, t);
    expect(message).not.toMatch(/[\u4e00-\u9fff]/);
    expect(message).not.toBe(error);
  }
  expect(connectionExceptionMessage("HTTP 401: unauthorized", t)).toContain("HTTP 401");
  expect(connectionExceptionMessage("HTTP 404:", t)).toBe(`HTTP 404: ${t("connection.errors.notFound")}`);
});
