import { Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api, isTauri, type UpdateLogEvent } from "../../api";
import i18n from "../../i18n";

export interface AppUpdate {
  version: string;
  /** 本次版本更新日志（latest.json 的 notes 段落）；旧清单或空日志发版时为 null */
  notes: string | null;
  install: () => Promise<void>;
}

/** 旧版本把更新标记存 localStorage 的键：启动时兜底消费一次，覆盖升级过渡期 */
export const UPDATED_VERSION_KEY = "budtty.updated-version";

async function logUpdateEvent(event: UpdateLogEvent, version?: string) {
  await api.logUpdateEvent(event, version).catch(() => undefined);
}

function webUpdate(): AppUpdate {
  return {
    version: "99.0.0", // ponytail: 哨兵版本号，永远高于真实迭代，无需跟进维护
    notes: i18n.t("webMock.notes", { ns: "updates" }),
    install: async () => undefined,
  };
}

export function toAppUpdate(update: Pick<Update, "version" | "body" | "download" | "install">): AppUpdate {
  return {
    version: update.version,
    notes: update.body ?? null,
    install: async () => {
      await logUpdateEvent("download_start", update.version);
      try {
        await update.download();
      } catch (error) {
        await logUpdateEvent("download_failure", update.version);
        throw error;
      }
      await logUpdateEvent("download_complete", update.version);
      // Windows 的 install 成功启动安装器后会立即退出当前进程，标记必须先原子落盘——
      // localStorage 由 WebView 异步提交，进程被杀时可能来不及写盘导致升级通知丢失
      await api.setUpdateMarker(update.version);
      try {
        await update.install();
        await logUpdateEvent("install_complete", update.version);
        // macOS / Linux 安装后不会自动重启，手动 relaunch；Windows 不会走到这里。
        await relaunch();
      } catch (error) {
        await logUpdateEvent("install_failure", update.version);
        await api.takeUpdateMarker(true).catch(() => undefined);
        throw error;
      }
    },
  };
}

export async function checkForAppUpdate(): Promise<AppUpdate | null> {
  if (!isTauri) return webUpdate();
  try {
    const metadata = await api.checkAppUpdate();
    if (!metadata) {
      await logUpdateEvent("check_latest");
      return null;
    }
    await logUpdateEvent("check_available", metadata.version);
    return toAppUpdate(new Update(metadata));
  } catch (error) {
    await logUpdateEvent("check_failure");
    throw error;
  }
}
