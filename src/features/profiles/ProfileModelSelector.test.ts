import { createInstance } from "i18next";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { expect, it, vi } from "vitest";
import { FeedbackProvider } from "../../app/Feedback";
import { setOneMillionModelSuffix } from "../claude/profileEnvText";
import ProfileModelSelector, { getModelChoices, resolveEffortLevels, resolveModelSelection, selectModel } from "./ProfileModelSelector";
import { codexEffortLevels, ReasoningEffortSlider } from "../../components/ReasoningEffortSlider";

it.each([undefined, {}, { fixture: [] }, { fixture: ["low", "high"] }])("旧缓存缺少档位时仍可调节，目录明确的空档位仍禁用：%j", (efforts) => {
  const levels = resolveEffortLevels("fixture", efforts, codexEffortLevels);
  expect(levels).toEqual(efforts?.fixture ?? codexEffortLevels);
  const markup = renderToStaticMarkup(createElement(ReasoningEffortSlider, { value: "high", levels, onChange: () => {} }));
  const input = markup.match(/<input[^>]*>/)?.[0];
  expect(input).toContain(`max="${Math.max(0, levels.length - 1)}"`);
  expect(input?.includes('disabled=""')).toBe(levels.length === 0);
});

it("默认跟随目录推荐顺序和模型默认档位，切换到不支持 ultra 的模型回落其默认档位", () => {
  const options = { model: "second", effort: "ultra", models: ["first", "second"],
    defaults: { first: "low", second: "high" }, efforts: { first: ["low", "max"], second: ["high", "ultra"] } };
  expect(resolveModelSelection({ model: "", effort: "" }, options)).toEqual({ model: "first", effort: "low" });
  expect(resolveModelSelection({ model: "", effort: "" }, { ...options, models: ["second", "first"] }))
    .toEqual({ model: "second", effort: "high" });
  expect(resolveModelSelection(selectModel("first", "ultra", options.efforts.first), options))
    .toEqual({ model: "first", effort: "low" });
  expect(resolveModelSelection({ model: "second", effort: "ultra" }, options)).toEqual({ model: "second", effort: "ultra" });
  expect(resolveModelSelection({ model: "", effort: "" }, { model: "", effort: "", models: ["first"] }))
    .toEqual({ model: "", effort: "" });
});

it("Claude 模型按隐藏 1M 后的名称去重，保留当前请求值；Codex 保留不同原值", () => {
  const model = "fixture[1M]";
  const models = ["fixture", "fixture[1m]", "other", "other", ""];
  expect(getModelChoices(model, models, (value) => setOneMillionModelSuffix(value, false))).toEqual([model, "other"]);
  expect(getModelChoices(model, models)).toEqual(["fixture", "fixture[1m]", "other", model]);
  expect(getModelChoices("", [])).toEqual([]);
});

it("当前模型保留目录位置，未收录的手输模型追加到末尾", () => {
  expect(getModelChoices("second", ["first", "second", "third"])).toEqual(["first", "second", "third"]);
  expect(getModelChoices("manual", ["first", "second"])).toEqual(["first", "second", "manual"]);
  expect(getModelChoices("replacement", ["first", "second"])).toEqual(["first", "second", "replacement"]);
  expect(getModelChoices("", ["first", "second"])).toEqual(["first", "second"]);
  expect(getModelChoices("second[1M]", ["first", "second", "third"], (value) => setOneMillionModelSuffix(value, false)))
    .toEqual(["first", "second[1M]", "third"]);
});

it("切换模型保留可用或未知档位，仅对目录明确不支持的档位恢复默认", () => {
  expect(selectModel("next", "high", ["low", "high"])).toEqual({ model: "next", effort: "high" });
  expect(selectModel("next", "max", ["low", "high"])).toEqual({ model: "next", effort: "" });
  expect(selectModel("next", "future-level")).toEqual({ model: "next", effort: "future-level" });
  expect(selectModel("next", "high", [])).toEqual({ model: "next", effort: "" });
});

it("模型名与快速模式图标直接显示在关闭的配置卡片上，无需读取详情", () => {
  const i18n = createInstance();
  void i18n.init({ lng: "en", resources: { en: { profiles: { modelSelection: { fastMode: "Fast mode" } } } }, initAsync: false });
  const onLoad = vi.fn(async () => ({ model: "fixture", effort: "high", models: [] }));
  const render = (fast: boolean, supportsFastMode = true, model = "fixture", formatModelLabel?: (value: string) => string) => renderToStaticMarkup(
    createElement(I18nextProvider, { i18n }, createElement(FeedbackProvider, {
      children: createElement(ProfileModelSelector, {
        model, effort: "high", levels: ["high"], disabled: false,
        fast, supportsFastMode, formatModelLabel, onLoad, onSave: async () => {},
      }),
    })),
  );
  expect(render(true)).toContain('aria-label="Fast mode"');
  expect(render(true)).toContain('aria-expanded="false"');
  expect(render(false)).not.toContain('aria-label="Fast mode"');
  expect(render(true, false)).not.toContain('aria-label="Fast mode"');
  const claude = render(false, false, "glm-5.3[1m]", (value) => setOneMillionModelSuffix(value, false));
  expect(claude).toContain(">glm-5.3</span>");
  expect(claude).not.toContain("[1m]");
  expect(render(false, false, "fixture[1m]")).toContain(">fixture[1m]</span>");
  expect(onLoad).not.toHaveBeenCalled();
});
