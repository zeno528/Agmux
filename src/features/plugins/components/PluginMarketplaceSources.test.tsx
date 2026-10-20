import { Children, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, expect, it, vi } from "vitest";
import { setupI18n } from "../../../i18n";
import PluginMarketplaceSources, { PluginMarketplaceRow } from "./PluginMarketplaceSources";

const feedback = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("../../../app/Feedback", () => ({ useFeedback: () => feedback }));

function capture(target: "codex" | "claude", source: string, onAdd: (value: string) => Promise<{ name: string }>, busy = false) {
  let submit!: (event: { preventDefault: () => void }) => Promise<void>;
  function Capture() {
    const tree = PluginMarketplaceSources({ target, marketplaces: [], loaded: true, busy, onAdd, children: null });
    const form = Children.toArray(tree.props.children)[0] as ReactElement<{ children: ReactNode; onSubmit: typeof submit }>;
    const input = Children.toArray(form.props.children)[0] as ReactElement<{ value: string; onChange: (event: { target: { value: string } }) => void }>;
    if (input.props.value !== source) input.props.onChange({ target: { value: source } });
    submit = form.props.onSubmit;
    return tree;
  }
  const html = renderToStaticMarkup(<Capture />);
  return { html, submit: () => submit({ preventDefault: vi.fn() }) };
}

beforeEach(() => {
  setupI18n("zh-CN");
  vi.clearAllMocks();
});

it("本地市场路径只显示一次，GitHub 市场保留来源文字与链接", () => {
  const path = "C:\\Users\\fixture\\.codex\\marketplaces\\sample";
  const local = renderToStaticMarkup(<PluginMarketplaceRow name="sample" source={path} />);
  expect(local.split(path)).toHaveLength(2);
  expect(local).not.toContain("<button");
  const github = renderToStaticMarkup(<PluginMarketplaceRow name="sample" source="https://github.com/example/sample.git" />);
  expect(github).toContain(">example/sample</span>");
  expect(github).toMatch(/<button[^>]*>.*example\/sample.*<\/button>/);
  expect(github).toContain('aria-label="打开 GitHub"');
});

it.each(["codex", "claude"] as const)("%s 共用表单去除首尾空白后添加，成功显示反馈", async (target) => {
  const onAdd = vi.fn().mockResolvedValue({ name: "sample-market" });
  const form = capture(target, "  example/sample-market  ", onAdd);
  expect(form.html).toContain(">添加</button>");
  await form.submit();
  expect(onAdd).toHaveBeenCalledExactlyOnceWith("example/sample-market");
  expect(feedback.success).toHaveBeenCalledWith("已添加插件市场「sample-market」");
  expect(feedback.error).not.toHaveBeenCalled();
});

it("添加失败显示错误，不报告成功；空输入和操作繁忙时不提交", async () => {
  const onAdd = vi.fn().mockRejectedValue(new Error("source unavailable"));
  await capture("claude", "example/sample-market", onAdd).submit();
  expect(feedback.error).toHaveBeenCalledWith("Error: source unavailable");
  expect(feedback.success).not.toHaveBeenCalled();
  onAdd.mockClear();
  await capture("codex", "  ", onAdd).submit();
  const busy = capture("codex", "example/sample-market", onAdd, true);
  expect(busy.html).toMatch(/<fieldset[^>]*disabled=""/);
  await busy.submit();
  expect(onAdd).not.toHaveBeenCalled();
});
