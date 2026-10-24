import packageJson from "../../package.json";
import profileEditSource from "../features/codex/CodexProfileEdit.tsx?raw";
import editorSource from "./ConfigTextEditor.tsx?raw";
import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { collectJsonDiagnostics, computeTextChange, connectEditorScrollbar, editorToolbarScrollMargins, findConfigFieldPosition, syncEditorScrollbar } from "./ConfigTextEditor";
import { patchEnvValue } from "../features/claude/profileEnvText";

describe("ConfigTextEditor runtime", () => {
  it("uses the native CodeMirror runtime instead of a duplicate wrapper runtime", () => {
    const dependencies = packageJson.dependencies as Record<string, string>;
    expect(dependencies["@uiw/react-codemirror"]).toBeUndefined();
    expect(dependencies.codemirror).toBeUndefined();
    expect(dependencies["@codemirror/state"]).toBeDefined();
    expect(dependencies["@codemirror/view"]).toBeDefined();
  });

  it("binds diagnostic focus to every profile editor variant", () => {
    expect(profileEditSource.match(/<ConfigTextEditor[^>]*ref=\{editorRef\}/g)).toHaveLength(3);
  });

  it("在首帧绘制前同步创建编辑器，内容与页面同帧呈现", () => {
    expect(editorSource).toMatch(/useLayoutEffect\(\(\) => \{\r?\n\s+const parent = hostRef\.current;/);
  });

  it("uses the longest profile document as the shared editor minimum", () => {
    expect(editorSource).toContain("minLines?: number;");
    expect(editorSource).toContain('className="apple-editor-surface"');
    expect(editorSource).toContain("style={{ minHeight: editorMinHeight }}");
    expect(profileEditSource.match(/minLines=\{editorMinLines\}/g)).toHaveLength(3);
  });

  it("共享最小占位按正文可用高度伸展，初次挂载和保活复显均监听正文尺寸", () => {
    expect(editorSource).toContain('const scrollContent = parent.closest(".apple-edit-content");');
    expect(editorSource).toContain("if (scrollContent) resizeObserver.observe(scrollContent);");
    expect(editorSource).toContain("if (scrollContent) reattachedObserver.observe(scrollContent);");
    expect(editorSource).toContain("window.innerHeight - content.clientHeight + chromeHeight + bottomGap");
    expect(editorSource).toContain('shell.style.setProperty("--editor-available-height",');
    expect(editorSource).toContain("max(0px, calc(100vh - ${reservedHeight}px))");
    expect(editorSource).toContain("px, var(--editor-available-height))");
  });

  it("主题判定取当下 DOM 真值：Activity 隐藏期切主题后复显不会挂回配色过期的实例", () => {
    // 隐藏期 effects 被拆掉，setDark 不会跑；判等与 oneDark 都必须用 DOM 现读值
    expect(editorSource).toContain('const currentDark = document.documentElement.classList.contains("dark")');
    expect(editorSource).toContain("const creationDeps = [currentDark, language, placeholder, validateToml, t]");
    expect(editorSource).toContain("...(currentDark ? [oneDark] : [])");
    expect(editorSource).not.toContain("...(dark ? [oneDark] : [])");
  });

  it("打开/复显/改文档时补齐整棵语法树：快速滚动不再落到未解析区间（无高亮的白字）", () => {
    // Language.state 初始化只解析前 3000 字符，其余靠 ParseWorker 空闲补，滚动更快时无树可用
    expect(editorSource).toContain("forceParsing(editor, editor.state.doc.length, 200);");
  });

  it("only synchronizes the changed config fragment", () => {
    const current = 'model = "gpt-5.6"\nmodel_reasoning_effort = "medium"\n[features]\n';
    const next = `${current}respect_system_proxy = true\n`;
    expect(computeTextChange(current, next)).toEqual({
      from: current.length,
      to: current.length,
      insert: "respect_system_proxy = true\n",
    });
    expect(editorSource).toContain("const change = computeTextChange(editor.state.doc.toString(), value);");
    expect(editorSource).toContain("editor.dispatch({ changes: change });");
  });

  it("定位新写入字段，跳过注释和变更前的同名字段", () => {
    const current = '# experimental_mode = true\n[other]\nexperimental_mode = true\n[features.context_management]\nexperimental_mode = false\n';
    const next = current.replace("experimental_mode = false", "experimental_mode = true");
    const change = computeTextChange(current, next);
    expect(findConfigFieldPosition(next, "experimental_mode", change.from)).toBe(next.lastIndexOf("experimental_mode"));
    expect(findConfigFieldPosition('# respect_system_proxy = true\n', "respect_system_proxy", 0)).toBeNull();
    expect(findConfigFieldPosition('"a.b" = true\naXb = false\n', "a.b", 0)).toBe(0);
  });

  it("Claude 工具栏写入重排 JSON 后定位实际 env 字段", () => {
    const current = '{"env":{"UNRELATED":"keep"}}';
    const next = patchEnvValue(current, "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
    const change = computeTextChange(current, next);
    expect(findConfigFieldPosition(next, "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", change.from)).toBe(next.indexOf('"CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"'));
    expect(findConfigFieldPosition(next, "MISSING_FIELD", change.from)).toBeNull();
  });

  it("只对成功写入的对应文本定位，其他外部更新继续恢复滚动且不抢输入焦点", () => {
    expect(editorSource).toContain("const reveal = pendingRevealRef.current;\n    pendingRevealRef.current = null;");
    expect(editorSource).toContain("reveal?.text === value ? findConfigFieldPosition(value, reveal.field, change.from) : null");
    expect(editorSource).toContain('const scrollContent = editor.dom.closest<HTMLElement>(".apple-edit-content");');
    expect(editorSource).toContain('scrollContent.scrollTo({ top: targetScrollTop, behavior: "smooth" });');
    expect(editorSource).toContain("editor.dispatch({ selection: { anchor: revealPosition }, scrollIntoView: true });");
    expect(editorSource).toContain("} else {\n        restoreScrollPosition();\n        restoreFrame = requestAnimationFrame(restoreScrollPosition);");
  });

  it("横向使用吸底滑块，不拦截滚轮或改变纵向滚动", () => {
    expect(editorSource).toContain('className="apple-editor-scrollbar"');
    expect(editorSource).toContain('type="range"');
    expect(editorSource).toContain("scrollbar.hidden = !content || editor.scrollDOM.scrollWidth <= editor.scrollDOM.clientWidth;");
    expect(editorSource).not.toContain("onEditorWheel");
    expect(editorSource).toContain("const previousScrollTop = editor.scrollDOM.scrollTop");
    expect(editorSource).toContain("editor.scrollDOM.scrollTop = previousScrollTop");
    expect(editorSource).toContain("if (scrollContent) scrollContent.scrollTop = previousContentScrollTop;");
  });

  it("按整个文档的最长行在首屏预设 contentDOM 最小宽度（虚拟化下原生滚动才能到底）", () => {
    expect(editorSource).toContain('document.createElement("canvas").getContext("2d")');
    expect(editorSource).toContain("editor.state.doc.lines; number += 1");
    expect(editorSource).toContain("editor.contentDOM.style.minWidth");
    // canvas 估算偏小时真实文本溢出内容盒，滚到最右行背景会露缝：读回 scrollWidth 二次校正
    expect(editorSource).toContain("if (renderedWidth > editor.contentDOM.clientWidth) {");
    expect(editorSource).toContain("syncEditorLayout(editor);");
  });

  it("页面保活复显时编辑器实例原样重挂，不销毁重建（切页不闪编辑器、滚动不丢）", () => {
    // cleanup 只摘 DOM 不销毁：Activity 隐藏与真卸载共用 cleanup，销毁交给微任务裁决
    expect(editorSource.match(/return \(\) => detachEditor\(/g)).toHaveLength(2);
    // 复显走挂回存活实例的分支，不 new EditorView
    expect(editorSource).toContain("parent.appendChild(alive.dom)");
    expect(editorSource).toContain("alive.requestMeasure()");
    // 真卸载裁决：hostRef 已被 React 置空才销毁，实例未销毁过才补刀
    expect(editorSource).toContain("if (!hostRef.current && !destroyedRef.current)");
    // 创建参数变化（主题/语言）仍走重建，保活不吞掉合法重建
    expect(editorSource).toContain("alive.destroy();");
  });

  it("用错误行高亮和红色粗体行号替代独立错误 gutter", () => {
    expect(editorSource).not.toContain("lintGutter()");
    expect(editorSource).toContain("setDiagnosticsEffect");
    expect(editorSource).toContain("gutterLineClass");
    expect(editorSource).toContain("value.gutters.map(transaction.changes)");
    expect(editorSource).toContain("cm-diagnostic-error-line");
    expect(editorSource).toContain("cm-diagnostic-error-gutter");
  });
});

describe("connectEditorScrollbar", () => {
  it("双向同步水平位置，保持纵向位置，并在卸载后移除监听", () => {
    const scroller = Object.assign(new EventTarget(), { scrollLeft: 120, scrollTop: 75, scrollWidth: 1000, clientWidth: 500 }) as unknown as HTMLElement;
    const properties: Record<string, string> = {};
    const scrollbar = Object.assign(new EventTarget(), { value: "0", max: "0", style: {
      setProperty: (key: string, value: string) => { properties[key] = value; },
    } }) as unknown as HTMLInputElement;
    const disconnect = connectEditorScrollbar(scroller, scrollbar);
    expect(scrollbar.value).toBe("120");
    expect(scrollbar.max).toBe("500");
    expect(properties["--editor-scrollbar-thumb-width"]).toBe("250px");
    scrollbar.value = "340";
    scrollbar.dispatchEvent(new Event("input"));
    expect(scroller.scrollLeft).toBe(340);
    expect(scroller.scrollTop).toBe(75);
    scroller.scrollLeft = 60;
    scroller.dispatchEvent(new Event("scroll"));
    expect(scrollbar.value).toBe("60");
    disconnect();
    scrollbar.value = "500";
    scrollbar.dispatchEvent(new Event("input"));
    expect(scroller.scrollLeft).toBe(60);
    scroller.scrollLeft = 800;
    scroller.dispatchEvent(new Event("scroll"));
    expect(scrollbar.value).toBe("500");
  });

  it.each([{ scrollWidth: 500, clientWidth: 500, width: "500px" }, { scrollWidth: 0, clientWidth: 0, width: "0px" }])("无溢出或隐藏时范围归零，不产生无效宽度：%j", ({ scrollWidth, clientWidth, width }) => {
    const properties: Record<string, string> = {};
    const scrollbar = { style: { setProperty: (key: string, value: string) => { properties[key] = value; } } } as HTMLInputElement;
    syncEditorScrollbar({ scrollWidth, clientWidth, scrollLeft: 0 } as HTMLElement, scrollbar);
    expect(scrollbar.max).toBe("0");
    expect(properties["--editor-scrollbar-thumb-width"]).toBe(width);
  });
});

describe("collectJsonDiagnostics", () => {
  const stateOf = (doc: string) => EditorState.create({ doc });

  it("valid JSON never reports errors, even with huge single-line strings", () => {
    // 真实回归样本：模型目录里 4 万字符的超长 base_instructions 曾被语法树误报
    const catalog = JSON.stringify({
      models: [{ slug: "deepseek-flash", base_instructions: "You are Codex. ".repeat(2800) }],
    });
    expect(collectJsonDiagnostics(stateOf(catalog))).toEqual([]);
  });

  it("blank documents report nothing", () => {
    expect(collectJsonDiagnostics(stateOf("   \n  "))).toEqual([]);
  });
});

describe("编辑页统一滚动", () => {
  it("不再监听滚轮分配页面与编辑器的纵向滚动", () => {
    expect(editorSource).not.toContain('addEventListener("wheel"');
    expect(editorSource).not.toContain("routeEditorWheel");
  });

  const readMargins = EditorState.create({ extensions: editorToolbarScrollMargins }).facet(EditorView.scrollMargins)[0]!;

  it("定位时留出整个吸顶栏高度，并跟随控件换行后的高度", () => {
    let height = 80;
    const toolbar = { getBoundingClientRect: () => ({ height }) };
    const view = { dom: { closest: () => ({ querySelector: () => toolbar }) } } as unknown as EditorView;
    expect(readMargins(view)).toEqual({ top: 80 });
    height = 118;
    expect(readMargins(view)).toEqual({ top: 118 });
  });

  it.each([null, { querySelector: () => null }])("没有吸顶栏的编辑器不增加定位留白：%j", (section) => {
    const view = { dom: { closest: () => section } } as unknown as EditorView;
    expect(readMargins(view)).toEqual({ top: 0 });
  });
});
