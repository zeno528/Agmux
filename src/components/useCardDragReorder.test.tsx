import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import { useCardDragReorder } from "./useCardDragReorder";
import { CardDragPreview } from "./SortableCard";

// Node 中模拟 React 调度与浏览器 DOM 边界，执行真实拖拽处理函数。
const hooks = vi.hoisted(() => ({ cells: [] as unknown[], index: 0, effects: [] as (() => (() => void))[] }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useEffect: () => undefined,
  useLayoutEffect: (effect: () => (() => void)) => { hooks.effects.push(effect); },
  useRef: (initial: unknown) => {
    const index = hooks.index++;
    hooks.cells[index] ??= { current: initial };
    return hooks.cells[index];
  },
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.cells)) hooks.cells[index] = initial;
    return [hooks.cells[index], (value: unknown) => { hooks.cells[index] = value; }];
  },
}));
vi.mock("@dnd-kit/core", () => ({ KeyboardSensor: class {}, PointerSensor: class {}, useSensor: () => null, useSensors: () => [] }));

beforeEach(() => { hooks.cells = []; hooks.index = 0; hooks.effects = []; });
afterEach(() => { vi.unstubAllGlobals(); });

it.each(["cancel", "drop"])("浮层直接使用当前整卡副本，%s 后清除且不改变源卡片", (finish) => {
  const preview = { style: { transform: "translate(0px, 8px)", transition: "transform 200ms" }, classList: { add: vi.fn(), remove: vi.fn() }, removeAttribute: vi.fn() };
  const source = { dataset: { profileId: "a" }, style: { ...preview.style }, cloneNode: vi.fn(() => preview), getBoundingClientRect: () => ({ width: 800, height: 90 }) };
  const original = { ...source.style };
  const bodyClasses = { add: vi.fn(), remove: vi.fn() };
  class DragHandle { classList = { contains: (name: string) => name === "drag-handle" }; blur = vi.fn(); }
  const handle = new DragHandle();
  vi.stubGlobal("document", { querySelectorAll: () => [source], activeElement: handle, body: { classList: bodyClasses } });
  vi.stubGlobal("HTMLElement", DragHandle);
  const items = [{ id: "a" }, { id: "b" }];
  const setItems = vi.fn();
  const persist = vi.fn(async () => undefined);
  const render = () => { hooks.index = 0; return useCardDragReorder(items, setItems, persist); };
  const idle = render();
  expect(bodyClasses.add).not.toHaveBeenCalled();
  idle.onDragStart({ active: { id: "a", rect: { current: { initial: null } } } } as DragStartEvent);
  expect(bodyClasses.add).toHaveBeenCalledWith("drag-active");
  expect(source.cloneNode).toHaveBeenCalledWith(true);
  expect(render().dragPreview).toBe(preview);
  expect(preview.style).toEqual({ transform: "", transition: "", width: "800px", height: "90px" });
  expect(preview.classList.remove).toHaveBeenCalledWith("opacity-0", "opacity-100");
  expect(preview.classList.add).toHaveBeenCalledWith("drag-dragging", "profile-drag-preview");
  expect(preview.removeAttribute).toHaveBeenCalledWith("data-profile-id");
  expect(source.style).toEqual(original);
  expect(handle.blur).not.toHaveBeenCalled();
  if (finish === "cancel") render().onDragCancel();
  else render().onDragEnd({ active: { id: "a" }, over: { id: "b" } } as DragEndEvent);
  expect(render().dragPreview).toBeNull();
  expect(bodyClasses.remove).toHaveBeenCalledWith("drag-active");
  expect(handle.blur).toHaveBeenCalledOnce();
  expect(persist).toHaveBeenCalledTimes(finish === "drop" ? 1 : 0);
  if (finish === "drop") expect(setItems).toHaveBeenCalledWith([items[1], items[0]]);
});

it("浮层保留列表样式作用域、禁止交互并在卸载时清除副本", () => {
  const card = {} as HTMLElement;
  const element = CardDragPreview({ card });
  expect(element.props).toMatchObject({ className: "profile-list", inert: true });
  const host = { replaceChildren: vi.fn() };
  (hooks.cells[0] as { current: unknown }).current = host;
  const cleanup = hooks.effects[0]();
  expect(host.replaceChildren).toHaveBeenCalledWith(card);
  cleanup();
  expect(host.replaceChildren).toHaveBeenLastCalledWith();
});
