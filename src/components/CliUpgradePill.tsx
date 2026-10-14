import { Download } from "lucide-react";
import type { TFunction } from "i18next";
import { useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../api";
import { useFeedback } from "../app/Feedback";
import { clearCachedCliUpdate, getCachedCliStatus, getCachedCliUpdate, runCliUpdateCheck, setCliStatusCache, subscribeCliUpdate, type CliClient } from "../app/managementDataCache";
import { LoadingSpinner } from "./LoadingSpinner";
import type { CliFailure, CliStatus } from "../types";

const clientLabels: Record<CliClient, string> = {
  codex: "Codex",
  claude: "Claude Code",
};

type CliUpgradePillProps = {
  client: CliClient;
  onUpdated?: (status: CliStatus) => void | Promise<unknown>;
};

export function cliFailure(error: unknown, stage: string): CliFailure {
  if (error && typeof error === "object" && "stage" in error && typeof error.stage === "string"
    && "kind" in error && typeof error.kind === "string" && "message" in error && typeof error.message === "string") {
    return error as CliFailure;
  }
  return { stage, kind: "internal", message: String(error) };
}

export function cliFailureMessage(error: CliFailure, client: CliClient, t: TFunction<"settings">, action?: "install" | "update") {
  const reason = t(`cli.errors.${error.stage}_${error.kind}`, {
    defaultValue: t(`cli.errors.${error.kind}`, { defaultValue: t("cli.errors.internal") }),
  });
  return t(action === "install" ? "cli.installFailed" : action === "update" ? "cli.updateFailed" : "cli.failed", {
    client: client === "codex" ? "Codex" : "Claude Code",
    reason,
  });
}

/// 供应商页标题旁的 CLI 升级胶囊（各客户端同款）：读跨页共享的更新缓存（全局
/// 懒计时器 / Agent 页静默检查写入），原生安装且发现新版本才出现；点击直接走
/// 官方升级链路——用户主动动作，失败照常透出。升级完成立即静默复检一次，
/// 版本追平后胶囊自然消失。未安装/非原生安装的客户端不显示（没有升级链路）。
export function CliUpgradePill({ client, onUpdated }: CliUpgradePillProps) {
  const { t: tCommon } = useTranslation("common");
  const { t: tSettings } = useTranslation("settings");
  const feedback = useFeedback();
  const update = useSyncExternalStore(subscribeCliUpdate, () => getCachedCliUpdate(client), () => getCachedCliUpdate(client));
  const native = getCachedCliStatus(client)?.installation === "native";
  const [upgrading, setUpgrading] = useState(false);
  if (!update?.available || !native) return null;
  const label = clientLabels[client];
  const upgrade = async () => {
    if (upgrading) return;
    setUpgrading(true);
    try {
      // 胶囊结果跨重启持久化，但后端只保留会话内的检查结果——点击先静默补检，
      // 既满足升级前置（require_update），又确认更新仍然可用：已不可升级则翻转
      // 缓存让胶囊消失，不打扰；检查失败才透出。守卫文案只兜真正的异常路径。
      const checked = await runCliUpdateCheck(client);
      if (!checked.available) {
        clearCachedCliUpdate(client);
        return;
      }
      const result = await (client === "codex" ? api.codexUpdateCli : api.claudeUpdateCli)();
      feedback.success(tSettings("cli.updateComplete", { client: label, version: result.version ?? "" }));
      setCliStatusCache(client, result);
      // 升级成功即权威翻转：版本已追平官方（后端校验过），胶囊立即消失
      clearCachedCliUpdate(client);
      await onUpdated?.(result);
    } catch (error) {
      feedback.error(cliFailureMessage(cliFailure(error, "run_cli"), client, tSettings, "update"));
    } finally {
      setUpgrading(false);
    }
  };
  return (
    <button
      type="button"
      className="plan-badge gap-1"
      disabled={upgrading}
      title={tCommon("cliUpdate.upgradeTitle", { client: label, version: update.latest_version, channel: update.channel })}
      onClick={() => void upgrade()}
    >
      {upgrading ? <><LoadingSpinner /><span className="meta-xs">{tSettings("cli.updating")}</span></> : <><Download size={12} strokeWidth={2} aria-hidden="true" />{tCommon("cliUpdate.upgrade", { version: update.latest_version })}</>}
    </button>
  );
}
