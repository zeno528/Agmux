import { describe, expect, it } from "vitest";
import i18next from "i18next";
import { setupI18n } from "../../i18n";
import { isWeeklyWindowLabel, localizeBalanceLabel } from "./balanceLabel";

/** 按指定语言取 profiles 命名空间的固定 t。 */
const fixedT = (language: "zh-CN" | "en-US") => {
  setupI18n(language);
  return i18next.getFixedT(language, "profiles");
};

describe("localizeBalanceLabel", () => {
  it("中文界面下后端下发的窗口标签逐字不变", () => {
    const t = fixedT("zh-CN");
    expect(localizeBalanceLabel("5小时", t)).toBe("5小时");
    expect(localizeBalanceLabel("7天", t)).toBe("7天");
    expect(localizeBalanceLabel("30天", t)).toBe("30天");
    expect(localizeBalanceLabel("额度", t)).toBe("用量");
    expect(localizeBalanceLabel("用量", t)).toBe("用量");
    expect(localizeBalanceLabel("周期", t)).toBe("周期");
  });

  it("英文界面下换成对应写法", () => {
    const t = fixedT("en-US");
    expect(localizeBalanceLabel("5小时", t)).toBe("5h");
    expect(localizeBalanceLabel("7天", t)).toBe("Weekly");
    expect(localizeBalanceLabel("30天", t)).toBe("Monthly");
    expect(localizeBalanceLabel("额度", t)).toBe("Usage");
    expect(localizeBalanceLabel("用量", t)).toBe("Usage");
    expect(localizeBalanceLabel("周期", t)).toBe("Period");
  });

  it("未知取值原样返回，空值返回 null", () => {
    const t = fixedT("en-US");
    expect(localizeBalanceLabel("12天", t)).toBe("12天");
    expect(localizeBalanceLabel("", t)).toBeNull();
    expect(localizeBalanceLabel(null, t)).toBeNull();
    expect(localizeBalanceLabel(undefined, t)).toBeNull();
  });
});

describe("isWeeklyWindowLabel", () => {
  it("只认后端下发的每周窗口值", () => {
    expect(isWeeklyWindowLabel("7天")).toBe(true);
    expect(isWeeklyWindowLabel("5小时")).toBe(false);
    expect(isWeeklyWindowLabel(null)).toBe(false);
  });
});
