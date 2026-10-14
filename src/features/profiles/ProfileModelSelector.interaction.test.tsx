import { isValidElement, type ReactElement, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import ProfileModelSelector, { type ModelSelectionOptions } from "./ProfileModelSelector";
import { ReasoningEffortSlider } from "../../components/ReasoningEffortSlider";
import { setOneMillionModelSuffix } from "../claude/profileEnvText";

// 沿用仓库的 Node hook 调度方式，执行真实组件回调；不模拟浏览器排版。
const hooks = vi.hoisted(() => ({ cells: [] as unknown[], index: 0, effects: [] as (() => void)[], changed: false, close: () => {}, error: vi.fn() }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useCallback: <T,>(callback: T) => callback,
  useLayoutEffect: (effect: () => void) => { hooks.effects.push(effect); },
  useRef: (initial: unknown) => {
    const index = hooks.index++;
    hooks.cells[index] ??= { current: initial };
    return hooks.cells[index];
  },
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.cells)) hooks.cells[index] = initial;
    return [hooks.cells[index], (value: unknown) => {
      const next = typeof value === "function" ? value(hooks.cells[index]) : value;
      hooks.changed ||= !Object.is(next, hooks.cells[index]);
      hooks.cells[index] = next;
    }];
  },
}));
vi.mock("react-dom", () => ({ createPortal: (node: ReactNode) => node }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ error: hooks.error }) }));
vi.mock("../../components/useFixedMenuPosition", () => ({ useFixedMenuPosition: () => undefined }));
vi.mock("../../components/useMenuDismiss", () => ({ useMenuDismiss: (_open: boolean, _trigger: unknown, _menu: unknown, close: () => void) => { hooks.close = close; } }));

type Element = ReactElement<Record<string, unknown>>;
function elements(node: ReactNode): Element[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  return isValidElement<Record<string, unknown>>(node) ? [node, ...elements(node.props.children as ReactNode)] : [];
}
const find = (tree: ReactNode, predicate: (node: Element) => boolean) => elements(tree).find(predicate)!;
const trigger = (tree: ReactNode) => find(tree, (node) => node.props.className === "profile-card-action-meta profile-model-trigger");
const label = (tree: ReactNode) => find(tree, (node) => node.props.className === "profile-card-action-meta__model").props.children;

beforeEach(() => {
  hooks.cells = []; hooks.index = 0; hooks.effects = []; hooks.error.mockClear();
  vi.stubGlobal("document", { body: {} });
});
afterEach(() => vi.unstubAllGlobals());

function setup(onSave: (changes: unknown) => Promise<void>) {
  const props = { model: "old-model", effort: "high", fast: false, supportsFastMode: true,
    levels: ["low", "high"], disabled: false, onSave,
    formatModelLabel: undefined as ((value: string) => string) | undefined,
    onLoad: async (): Promise<ModelSelectionOptions> => ({ model: "old-model", effort: "high", fast: false, models: ["recommended", "longer-new-model"],
      defaults: { recommended: "low" }, efforts: { "longer-new-model": ["low", "high"] } }),
  };
  const render = () => {
    let tree: ReactNode;
    do {
      hooks.index = 0; hooks.effects = []; hooks.changed = false;
      tree = ProfileModelSelector(props);
      hooks.effects.forEach((effect) => effect());
    } while (hooks.changed);
    return tree;
  };
  const open = async () => {
    const button = trigger(render());
    (button.props.ref as { current: unknown }).current = { getBoundingClientRect: () => ({ width: 200 }) };
    (button.props.onClick as () => void)();
    await Promise.resolve();
    return render();
  };
  return { props, render, open };
}

it.each(["longer-new-model", ""])("浮卡宽度固定，关闭同步显示新值，保存后等待父级确认：%j", async (model) => {
  let finish = () => {};
  const onSave = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
  const { props, render, open } = setup(onSave);
  let tree = await open();
  const slider = find(tree, (node) => node.type === ReasoningEffortSlider);
  (slider.props.onChange as (value: string) => void)("low");
  tree = render();
  expect(trigger(tree).props.style).toEqual({ width: "200px" });
  expect(onSave).not.toHaveBeenCalled();
  (find(tree, (node) => node.props.className === "profile-model-current field-label").props.onClick as () => void)();
  (find(render(), (node) => node.props.role === "option" && node.key === model).props.onClick as () => void)();
  if (!model) await Promise.resolve();
  tree = render();
  expect(trigger(tree).props.style).toEqual({ width: "200px" });
  hooks.close();
  const expected = model || "recommended";
  tree = render();
  expect(trigger(tree).props["aria-expanded"]).toBe(false);
  expect(trigger(tree).props.style).toBeUndefined();
  expect(label(tree)).toBe(expected);
  expect(onSave).toHaveBeenCalledExactlyOnceWith({ model, effort: model ? "low" : "" });
  finish();
  await vi.waitFor(() => expect(trigger(render()).props["aria-busy"]).toBe(false));
  expect(label(render())).toBe(expected); // 父级仍是旧值，不能闪回。
  props.model = expected; props.effort = "low";
  expect(label(render())).toBe(expected);
  props.model = "external-model";
  expect(label(render())).toBe("external-model"); // 确认后释放预览，后续外部更新照常显示。
});

it("模型列表名字与 1M 标签拆开展示，选中保存仍用原值", async () => {
  const { props, render, open } = setup(vi.fn(async () => {}));
  props.onLoad = async () => ({ model: "fixture[1M]", effort: "high", fast: false, models: ["fixture[1M]", "plain"] });
  props.formatModelLabel = (value: string) => setOneMillionModelSuffix(value, false);
  const tree = await open();
  (find(tree, (node) => node.props.className === "profile-model-current field-label").props.onClick as () => void)();
  const list = render();
  const optionPart = (key: string, className: string) => find(
    find(list, (node) => node.props.role === "option" && node.key === key),
    (node) => node.props.className === className,
  ).props.children;
  expect(optionPart("fixture[1M]", "min-w-0 truncate")).toBe("fixture");
  expect(optionPart("fixture[1M]", "meta-xs muted shrink-0")).toBe("1M");
  expect(optionPart("plain", "min-w-0 truncate")).toBe("plain");
  expect(label(list)).toBe("fixture");
});

it("保存失败回退父级值并保留错误反馈", async () => {
  let fail = (_error: Error) => {};
  const { render, open } = setup(() => new Promise<void>((_resolve, reject) => { fail = reject; }));
  const tree = await open();
  (find(tree, (node) => node.type === ReasoningEffortSlider).props.onChange as (value: string) => void)("low");
  render(); hooks.close();
  expect(find(render(), (node) => node.type === "span" && node.props.style !== undefined).props.children).toBe("low");
  fail(new Error("save failed"));
  await vi.waitFor(() => expect(trigger(render()).props["aria-busy"]).toBe(false));
  expect(find(render(), (node) => node.type === "span" && node.props.style !== undefined).props.children).toBe("high");
  expect(hooks.error).toHaveBeenCalledExactlyOnceWith("Error: save failed");
});
