import { beforeEach, expect, it, vi } from "vitest";
import { Children, isValidElement } from "react";
import { api } from "../../api";
import ConfigTextEditor from "../../components/ConfigTextEditor";
import ClaudeCommonTemplateDialog from "./ClaudeCommonTemplateDialog";

const hooks = vi.hoisted(() => ({ values: [] as unknown[], cursor: 0, effect: () => {} }));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useState: (initial: unknown) => {
    const index = hooks.cursor++;
    if (!(index in hooks.values)) hooks.values[index] = initial;
    return [hooks.values[index], (value: unknown) => { hooks.values[index] = value; }];
  },
  useEffect: (effect: () => void) => { hooks.effect = effect; },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => ({ error: vi.fn() }) }));
vi.mock("../../components/ConfigTextEditor", () => ({ default: () => null }));
beforeEach(() => { hooks.values = []; hooks.cursor = 0; vi.restoreAllMocks(); });

it.each([null, '{"env":{"API_TIMEOUT_MS":"1000000"}}'])("模板读取完成前不打开弹窗，首次打开即使用已读取内容：%s", async (saved) => {
  let finish!: (text: string | null) => void;
  vi.spyOn(api, "claudeGetCommonSettings").mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const render = () => { hooks.cursor = 0; return ClaudeCommonTemplateDialog({ onClose: vi.fn() }); };
  expect(render().props.open).toBe(false);
  hooks.effect();
  expect(render().props.open).toBe(false);
  finish(saved);
  await Promise.resolve();
  const dialog = render();
  expect(dialog.props.open).toBe(true);
  const group = Children.toArray(dialog.props.children).find(isValidElement)!;
  const editor = Children.toArray((group.props as { children: React.ReactNode }).children)
    .find((child) => isValidElement(child) && child.type === ConfigTextEditor);
  expect(isValidElement(editor) && (editor.props as { value: string }).value).toBe(saved ?? "{}");
});
