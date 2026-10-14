import { useEffect, useState } from "react";
import { getCachedProxyStatus, loadProxyStatus, type ProxyStatus } from "./managementDataCache";

/// 代理走向的共享订阅：缓存直出首帧，挂载与窗口激活时静默强刷。
/// 设置页代理卡片与 CLI 卡片共用这一个 hook——网络走向只有一个事实来源，
/// 各组件禁止自己保存快照（快照会随外部代理变化而失真）。
export function useProxyStatus(enabled = true): ProxyStatus | null {
  const [status, setStatus] = useState<ProxyStatus | null>(getCachedProxyStatus);
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let request = 0;
    const refresh = () => {
      const current = ++request;
      // 刷新期间保留旧结果；共享缓存合并重入请求，失败也返回可显示的状态。
      void loadProxyStatus(true).then((next) => {
        if (active && request === current) setStatus(next);
      });
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => { active = false; window.removeEventListener("focus", refresh); };
  }, [enabled]);
  return status;
}
