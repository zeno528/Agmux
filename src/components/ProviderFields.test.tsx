import { Children, isValidElement, type ElementType, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { setupI18n } from "../i18n";
import { patchModelMappings, readModelMappings, patchModelDisplayNames, readModelDisplayNames, hasOneMillionModelSuffix } from "../features/claude/profileEnvText";
import { ProviderModelFields } from "./ProviderFields";
import { AppSelect } from "./AppSelect";

setupI18n("zh-CN");

type ControlProps = { children?: ReactNode; "aria-label"?: string; disabled?: boolean; checked?: boolean; onClick?: () => void;
  value?: string; options?: Array<{ value: string }> };
function findControls(node: ReactNode, type: ElementType, label?: string): ControlProps[] {
  const found: ControlProps[] = [];
  for (const child of Children.toArray(node)) {
    if (!isValidElement<ControlProps>(child)) continue;
    if (child.type === type && (label === undefined || child.props["aria-label"] === label)) found.push(child.props);
    else found.push(...findControls(child.props.children, type, label));
  }
  return found;
}
const findControl = (node: ReactNode, type: ElementType, label: string) => findControls(node, type, label)[0];

function mappingFields(value: string, onChange: (value: string) => void, displayValue = "", onDisplayChange: (value: string) => void = () => {}) {
  return ProviderModelFields({
    models: [], fetching: false, disabled: false, onFetch: () => {},
    mappingFields: [{ key: "ANTHROPIC_DEFAULT_OPUS_MODEL", label: "Opus", value, onChange, displayValue, onDisplayChange, oneMillion: hasOneMillionModelSuffix(value), onToggleOneMillion: () => {} }],
    labels: { model: "模型", models: "模型列表", fetch: "获取", available: "", select: "选择", fetchFirst: "先获取", requestModel: "请求模型", displayName: "显示名", oneMillion: "1M", clear: "清空" },
  });
}

describe("模型映射行操作", () => {
  it("候选列表只来自获取结果并去重，手输映射保留但不混入候选", () => {
    const onChange = vi.fn();
    const tree = ProviderModelFields({
      models: ["fetched-model", "manual-model", "fetched-model"],
      fetching: false, disabled: false, onFetch: vi.fn(),
      mappingFields: [{ key: "opus", label: "Opus", value: "other-model[1M]", onChange }],
      labels: { model: "模型", models: "模型列表", fetch: "获取", available: "", select: "选择", fetchFirst: "先获取" },
    });
    const selects = findControls(tree, AppSelect);
    expect(selects).toHaveLength(1);
    expect(selects[0].options!.map((option) => option.value)).toEqual(["fetched-model", "manual-model"]);
    expect(selects[0].value).toBeNull();
    expect(findControl(tree, "input", "Opus 模型").value).toBe("other-model");
  });

  it("未获取列表时不以手输模型填充下拉，选择器禁用", () => {
    const tree = mappingFields("manual-model", vi.fn());
    expect(findControls(tree, AppSelect)[0]).toMatchObject({ options: [], value: null, disabled: true });
    expect(findControl(tree, "input", "Opus 请求模型").value).toBe("manual-model");
  });

  it.each(["", "   ", "[1M]"])("请求模型为空（%j）时清空与 1M 都禁用且不选中", (value) => {
    const tree = mappingFields(value, vi.fn());
    expect(findControl(tree, "button", "清空 Opus")?.disabled).toBe(true);
    expect(findControl(tree, "input", "Opus 1M")).toMatchObject({ disabled: true, checked: false });
    const html = renderToStaticMarkup(tree);
    expect(html).toContain('aria-disabled="true"');
    expect(html).not.toMatch(/<label class="[^"]*\bon\b/);
    expect(html).not.toContain(">清空<");
  });

  it("有请求模型时启用两项操作，1M 正确回显", () => {
    const tree = mappingFields("gateway-model[1M]", vi.fn());
    expect(findControl(tree, "button", "清空 Opus")?.disabled).toBe(false);
    expect(findControl(tree, "input", "Opus 1M")).toMatchObject({ disabled: false, checked: true });
    const html = renderToStaticMarkup(tree);
    expect(html).toContain("2.5rem_max-content_2rem");
    expect(html).toContain("justify-self-center");
  });

  it.each([["gateway-model[1M]", "显示名"], ["gateway-model", ""], ["", "显示名"]])("清空整行（模型 %j，显示名 %j），保留其他行和配置", (value, displayValue) => {
    let raw = JSON.stringify({ permissions: { allow: ["Read"] }, env: { ANTHROPIC_MODEL: "other-model", ANTHROPIC_DEFAULT_OPUS_MODEL: value, ANTHROPIC_DEFAULT_OPUS_MODEL_NAME: displayValue } });
    const mappings = readModelMappings(raw);
    const names = readModelDisplayNames(raw);
    const onChange = (value: string) => {
      mappings.ANTHROPIC_DEFAULT_OPUS_MODEL = value;
      raw = patchModelMappings(raw, mappings);
    };
    const onDisplayChange = (value: string) => {
      names.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME = value;
      raw = patchModelDisplayNames(raw, names);
    };
    const tree = mappingFields(value, onChange, displayValue, onDisplayChange);
    const clear = findControl(tree, "button", "清空 Opus");
    expect(clear?.disabled).toBe(false);
    expect(findControl(tree, "input", "Opus 1M")?.disabled).toBe(!value);
    expect(clear?.onClick).toBeTypeOf("function");
    clear!.onClick!();
    expect(JSON.parse(raw)).toEqual({ permissions: { allow: ["Read"] }, env: { ANTHROPIC_MODEL: "other-model" } });
    const next = mappingFields(mappings.ANTHROPIC_DEFAULT_OPUS_MODEL, onChange, names.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME, onDisplayChange);
    expect(findControl(next, "button", "清空 Opus")?.disabled).toBe(true);
    expect(findControl(next, "input", "Opus 1M")).toMatchObject({ disabled: true, checked: false });
  });
});
