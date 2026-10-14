import { describe, expect, it } from "vitest";
import i18next from "i18next";
import { setupI18n } from "../../i18n";
import { updateFailureMessage } from "./updateText";

/** 按指定语言取 updates 命名空间的固定 t。 */
const fixedT = (language: "zh-CN" | "en-US") => {
  setupI18n(language);
  return i18next.getFixedT(language, "updates");
};

describe("updateFailureMessage", () => {
  it("将网络层错误转成系统代理提示", () => {
    expect(updateFailureMessage(new Error("error sending request for url"), fixedT("zh-CN"))).toBe(
      "无法连接 GitHub，请检查网络代理后重试",
    );
  });

  it("保留非网络错误的可读信息", () => {
    expect(updateFailureMessage(new Error("更新包签名无效"), fixedT("zh-CN"))).toBe("更新包签名无效");
  });

  it("英文界面下网络层错误给出英文提示", () => {
    expect(updateFailureMessage(new Error("network timeout"), fixedT("en-US"))).toBe(
      "Cannot reach GitHub. Check your system proxy and try again",
    );
  });
});
