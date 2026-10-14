import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../../../api";
import { setupI18n } from "../../../i18n";
import { useProfileAdvancedPatches, type ProfileAdvancedPatches } from "./useProfileAdvancedPatches";

vi.mock("../../../app/Feedback", () => ({ useFeedback: () => ({ error: vi.fn() }) }));
setupI18n("zh-CN");
afterEach(() => vi.restoreAllMocks());

function captureControls() {
  let controls!: ProfileAdvancedPatches;
  const onPatched = vi.fn();
  const setConfigText = vi.fn();
  function Capture() {
    controls = useProfileAdvancedPatches({
      configText: 'model = "example"\n', setConfigText,
      initialized: { current: true }, showLongContextOverride: true, onPatched,
    });
    return null;
  }
  renderToStaticMarkup(<Capture />);
  return { controls, onPatched, setConfigText };
}

describe("工具栏配置写入后的定位请求", () => {
  it.each([
    ["toggleLongContext", "codexPatchChatgptContextConfig", "model_context_window"],
    ["toggleSystemProxy", "codexPatchSystemProxyConfig", "respect_system_proxy"],
    ["toggleContextManagement", "codexPatchContextManagementConfig", "experimental_mode"],
  ] as const)("%s 成功后才传递返回文本和目标字段，取消时不定位", async (action, method, field) => {
    const next = `${field} = true\n`;
    vi.spyOn(api, method).mockResolvedValue(next);
    const { controls, onPatched, setConfigText } = captureControls();
    await controls[action](true);
    expect(setConfigText).toHaveBeenCalledWith(next);
    expect(onPatched).toHaveBeenCalledExactlyOnceWith(next, field);
    onPatched.mockClear();
    await controls[action](false);
    expect(onPatched).not.toHaveBeenCalled();
  });

  it("补丁失败时不修改文本，也不请求定位", async () => {
    vi.spyOn(api, "codexPatchSystemProxyConfig").mockRejectedValue(new Error("patch failed"));
    const { controls, onPatched, setConfigText } = captureControls();
    await controls.toggleSystemProxy(true);
    expect(setConfigText).not.toHaveBeenCalled();
    expect(onPatched).not.toHaveBeenCalled();
  });
});
