import { beforeEach, expect, it, vi } from "vitest";
import { Children, isValidElement, type ReactNode } from "react";
import { api } from "../../api";
import ConfigTextEditor from "../../components/ConfigTextEditor";
import ClaudeCommonTemplateEdit from "./ClaudeCommonTemplateEdit";

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

function findEditor(node: ReactNode): string | undefined {
  for (const child of Children.toArray(node)) {
    if (!isValidElement<{ children?: ReactNode; value?: string }>(child)) continue;
    if (child.type === ConfigTextEditor) return child.props.value;
    const value = findEditor(child.props.children);
    if (value !== undefined) return value;
  }
}

it.each([null, '{"env":{"API_TIMEOUT_MS":"1000000"}}'])("模板页只在读取完成后显示编辑器：%s", async (saved) => {
  let finish!: (text: string | null) => void;
  vi.spyOn(api, "claudeGetCommonSettings").mockReturnValue(new Promise((resolve) => { finish = resolve; }));
  const render = () => {
    hooks.cursor = 0;
    return ClaudeCommonTemplateEdit({ sourceText: "{}", onBack: vi.fn() });
  };
  expect(findEditor(render())).toBeUndefined();
  hooks.effect();
  finish(saved);
  await Promise.resolve();
  expect(findEditor(render())).toBe(saved ?? "{}");
});
