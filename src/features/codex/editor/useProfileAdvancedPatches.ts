import { useEffect, useState, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api";
import { useFeedback } from "../../../app/Feedback";

const defaultCompactTokenLimit = "900000";

function hasLongContextOverride(text: string) {
  return /^\s*model_context_window\s*=/m.test(text) && /^\s*model_auto_compact_token_limit\s*=/m.test(text);
}

// 阈值行格式：readCompactTokenLimit 解析与 replaceCompactTokenLimit 写入共用同一 pattern，二者不可漂移
const compactTokenLimitLine = /^(\s*model_auto_compact_token_limit\s*=\s*)(\d+)\s*$/m;

function readCompactTokenLimit(text: string) {
  return compactTokenLimitLine.exec(text)?.[2] ?? defaultCompactTokenLimit;
}

// 输入框实时联动：把配置文本里的阈值行替换为新值；键不存在时原样返回（由 blur 时的后端补丁兜底补写）
function replaceCompactTokenLimitText(text: string, limit: number) {
  return text.replace(compactTokenLimitLine, `$1${limit}`);
}

function hasSystemProxyOverride(text: string) {
  return /^\s*respect_system_proxy\s*=/m.test(text);
}

// 仅匹配 [features.context_management] 段内的 experimental_mode = true；[^[] 保证不跨入下一个段头
function hasContextManagementOverride(text: string) {
  return /^\s*\[features\.context_management\]\s*\n[^[]*?^\s*experimental_mode\s*=\s*true\s*$/m.test(text);
}

export interface ProfileAdvancedPatches {
  /** 官方档才显示长上下文控件（渲染条件透传）。 */
  showLongContextOverride: boolean;
  longContextEnabled: boolean;
  compactTokenLimit: string;
  patchingLongContext: boolean;
  patchingSystemProxy: boolean;
  patchingContextMgmt: boolean;
  systemProxyEnabled: boolean;
  contextMgmtEnabled: boolean;
  toggleLongContext: (enabled: boolean) => Promise<void>;
  updateCompactTokenLimit: () => Promise<void>;
  /** 阈值输入框 onChange：更新输入值，合法时实时联动写回 config 文本。 */
  updateCompactTokenLimitInput: (next: string) => void;
  toggleSystemProxy: (enabled: boolean) => Promise<void>;
  toggleContextManagement: (enabled: boolean) => Promise<void>;
  /** 从 config 文本重新推导四个开关（详情加载完成与切换预设时调用）。 */
  syncFromConfig: (text: string, longContextAllowed: boolean) => void;
}

/**
 * 编辑器高级配置补丁：长上下文 / 压缩阈值 / 系统代理 / 上下文管理。
 * 四个开关读写同一段 config 文本：开关翻转走后端补丁写回 configText，
 * configText 变化（非补丁中）时反向同步开关状态。
 */
export function useProfileAdvancedPatches(options: {
  configText: string;
  setConfigText: Dispatch<SetStateAction<string>>;
  initialized: MutableRefObject<boolean>;
  showLongContextOverride: boolean;
  onPatched?: (text: string, field: string) => void;
}): ProfileAdvancedPatches {
  const { configText, setConfigText, initialized, showLongContextOverride, onPatched } = options;
  const feedback = useFeedback();
  const { t } = useTranslation("profiles");
  const [longContextEnabled, setLongContextEnabled] = useState(() => showLongContextOverride && hasLongContextOverride(configText));
  const [compactTokenLimit, setCompactTokenLimit] = useState(() => readCompactTokenLimit(configText));
  const [patchingLongContext, setPatchingLongContext] = useState(false);
  const [systemProxyEnabled, setSystemProxyEnabled] = useState(() => hasSystemProxyOverride(configText));
  const [patchingSystemProxy, setPatchingSystemProxy] = useState(false);
  const [contextMgmtEnabled, setContextMgmtEnabled] = useState(() => hasContextManagementOverride(configText));
  const [patchingContextMgmt, setPatchingContextMgmt] = useState(false);

  useEffect(() => {
    if (showLongContextOverride && !patchingLongContext) {
      setLongContextEnabled(hasLongContextOverride(configText));
      setCompactTokenLimit(readCompactTokenLimit(configText));
    }
  }, [configText, patchingLongContext, showLongContextOverride]);

  useEffect(() => {
    if (!initialized.current) return;
    if (!patchingSystemProxy) setSystemProxyEnabled(hasSystemProxyOverride(configText));
  }, [configText, patchingSystemProxy]);

  useEffect(() => {
    if (!initialized.current) return;
    if (!patchingContextMgmt) setContextMgmtEnabled(hasContextManagementOverride(configText));
  }, [configText, patchingContextMgmt]);

  const toggleLongContext = async (enabled: boolean) => {
    if (patchingLongContext) return;
    setPatchingLongContext(true);
    try {
      const next = await api.codexPatchChatgptContextConfig(configText, enabled, Number(compactTokenLimit));
      if (enabled) onPatched?.(next, "model_context_window");
      setConfigText(next);
      setLongContextEnabled(enabled);
    } catch (error) {
      feedback.error(t("edit.errorLongContext", { error: String(error) }));
    } finally {
      setPatchingLongContext(false);
    }
  };

  const updateCompactTokenLimit = async () => {
    const limit = Number(compactTokenLimit);
    if (patchingLongContext || !Number.isInteger(limit) || limit < 1 || limit > 1_000_000) {
      feedback.error(t("edit.compactLimitRange"));
      setCompactTokenLimit(readCompactTokenLimit(configText));
      return;
    }
    setPatchingLongContext(true);
    try {
      const next = await api.codexPatchChatgptContextConfig(configText, true, limit);
      onPatched?.(next, "model_auto_compact_token_limit");
      setConfigText(next);
    } catch (error) {
      feedback.error(t("edit.errorCompactLimit", { error: String(error) }));
    } finally {
      setPatchingLongContext(false);
    }
  };

  const toggleSystemProxy = async (enabled: boolean) => {
    if (patchingSystemProxy) return;
    setPatchingSystemProxy(true);
    try {
      const next = await api.codexPatchSystemProxyConfig(configText, enabled);
      if (enabled) onPatched?.(next, "respect_system_proxy");
      setConfigText(next);
      setSystemProxyEnabled(enabled);
    } catch (error) {
      feedback.error(t("edit.errorSystemProxy", { error: String(error) }));
    } finally {
      setPatchingSystemProxy(false);
    }
  };

  const toggleContextManagement = async (enabled: boolean) => {
    if (patchingContextMgmt) return;
    setPatchingContextMgmt(true);
    try {
      const next = await api.codexPatchContextManagementConfig(configText, enabled);
      if (enabled) onPatched?.(next, "experimental_mode");
      setConfigText(next);
      setContextMgmtEnabled(enabled);
    } catch (error) {
      feedback.error(t("edit.errorContextMgmt", { error: String(error) }));
    } finally {
      setPatchingContextMgmt(false);
    }
  };

  const syncFromConfig = (text: string, longContextAllowed: boolean) => {
    setLongContextEnabled(longContextAllowed && hasLongContextOverride(text));
    setCompactTokenLimit(readCompactTokenLimit(text));
    setSystemProxyEnabled(hasSystemProxyOverride(text));
    setContextMgmtEnabled(hasContextManagementOverride(text));
  };

  // 阈值输入框实时联动：合法输入立即写入 configText，最终校验与格式化仍由 blur 时的后端补丁完成
  const updateCompactTokenLimitInput = (next: string) => {
    setCompactTokenLimit(next);
    const limit = Number(next);
    if (!longContextEnabled || patchingLongContext || !Number.isInteger(limit) || limit < 1 || limit > 1_000_000) return;
    setConfigText((current) => replaceCompactTokenLimitText(current, limit));
  };

  return {
    showLongContextOverride,
    longContextEnabled,
    compactTokenLimit,
    patchingLongContext,
    patchingSystemProxy,
    patchingContextMgmt,
    systemProxyEnabled,
    contextMgmtEnabled,
    toggleLongContext,
    updateCompactTokenLimit,
    updateCompactTokenLimitInput,
    toggleSystemProxy,
    toggleContextManagement,
    syncFromConfig,
  };
}
