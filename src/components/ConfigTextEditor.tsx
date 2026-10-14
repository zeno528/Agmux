import { closeBrackets, closeBracketsKeymap, autocompletion, completionKeymap } from "@codemirror/autocomplete";
import { history, defaultKeymap, historyKeymap } from "@codemirror/commands";
import { bracketMatching, defaultHighlightStyle, ensureSyntaxTree, foldGutter, foldKeymap, forceParsing, indentOnInput, indentUnit as indentUnitFacet, StreamLanguage, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import { json } from "@codemirror/lang-json";
import { forEachDiagnostic, lintKeymap, linter, setDiagnosticsEffect, type Diagnostic } from "@codemirror/lint";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, RangeSet, RangeSetBuilder, StateField } from "@codemirror/state";
import { crosshairCursor, Decoration, drawSelection, EditorView, gutterLineClass, GutterMarker, highlightActiveLine, highlightActiveLineGutter, highlightSpecialChars, keymap, lineNumbers, placeholder as editorPlaceholder, rectangularSelection, dropCursor, ViewPlugin, WidgetType, type ViewUpdate } from "@codemirror/view";
import { oneDark } from "@codemirror/theme-one-dark";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { detectIndentUnit, indentGuideLayout, visualIndentColumns } from "./editorIndentUnit";
import i18next from "i18next";
import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../api";
import type { EditorDiagnosticSummary, TomlDiagnostic } from "../types";

class IndentGuideFill extends WidgetType {
  constructor(readonly segments: readonly number[]) { super(); }

  toDOM() {
    const fill = document.createElement("span");
    fill.className = "cm-indent-guide-fill";
    for (const spaces of this.segments) {
      const segment = fill.appendChild(document.createElement("span"));
      segment.className = "cm-indent-guide";
      segment.textContent = " ".repeat(spaces);
    }
    return fill;
  }

  eq(other: WidgetType) {
    return other instanceof IndentGuideFill && this.segments.length === other.segments.length && this.segments.every((spaces, index) => spaces === other.segments[index]);
  }

  ignoreEvent() { return true; }
}

const indentGuideMark = Decoration.mark({ class: "cm-indent-guide" });

function nearestIndentLevel(doc: EditorState["doc"], lineNumber: number, direction: -1 | 1, unitColumns: number, tabSize: number) {
  for (let number = lineNumber; number >= 1 && number <= doc.lines; number += direction) {
    const line = doc.line(number);
    if (line.text.trim()) {
      const prefix = /^[ \t]*/.exec(line.text)?.[0] ?? "";
      return { number, level: Math.floor(visualIndentColumns(prefix, tabSize) / unitColumns) };
    }
  }
  return null;
}

function inheritedBlankLineLevel(doc: EditorState["doc"], lineNumber: number, unitColumns: number, tabSize: number) {
  if (lineNumber === 1) return 0;
  const previous = nearestIndentLevel(doc, lineNumber - 1, -1, unitColumns, tabSize);
  if (!previous) return 0;
  if (lineNumber === doc.lines) return previous.level;
  const next = nearestIndentLevel(doc, lineNumber + 1, 1, unitColumns, tabSize);
  if (!next || previous.level >= next.level) return previous.level;
  return Math.min(next.level, previous.level + lineNumber - previous.number);
}

const indentationGuidePlugin = ViewPlugin.fromClass(class {
  decorations: RangeSet<Decoration>;
  private indentUnit: string;

  constructor(view: EditorView) {
    this.indentUnit = view.state.facet(indentUnitFacet);
    this.decorations = this.build(view);
  }

  update(update: ViewUpdate) {
    const nextIndentUnit = update.state.facet(indentUnitFacet);
    if (!update.docChanged && nextIndentUnit === this.indentUnit) return;
    this.indentUnit = nextIndentUnit;
    this.decorations = this.build(update.view);
  }

  private build(view: EditorView) {
    const builder = new RangeSetBuilder<Decoration>();
    const { state } = view;
    const tabSize = state.tabSize;
    const unitColumns = visualIndentColumns(this.indentUnit, tabSize);
    if (!unitColumns) return builder.finish();
    for (let number = 1; number <= state.doc.lines; number += 1) {
      const line = state.doc.line(number);
      const empty = !line.text.trim();
      const guideCount = empty
        ? Math.max(0, inheritedBlankLineLevel(state.doc, line.number, unitColumns, tabSize) - 1)
        : undefined;
      const layout = indentGuideLayout(line.text, this.indentUnit, tabSize, guideCount);
      for (const markEnd of layout.markEnds) {
        builder.add(line.from + markEnd - 1, line.from + markEnd, indentGuideMark);
      }
      if (layout.fillSegments.length) {
        builder.add(line.to, line.to, Decoration.widget({ widget: new IndentGuideFill(layout.fillSegments), side: 1 }));
      }
    }
    return builder.finish();
  }
}, { decorations: (plugin) => plugin.decorations });

const basicSetup = [
  lineNumbers(),
  highlightActiveLineGutter(),
  highlightSpecialChars(),
  history(),
  // Keep the editor's baseline behavior aligned with CodeMirror's public basicSetup.
  // All extensions are imported directly so Vite cannot embed a second state runtime.
  foldGutter(),
  drawSelection(),
  dropCursor(),
  EditorState.allowMultipleSelections.of(true),
  indentOnInput(),
  syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
  indentationGuidePlugin,
  bracketMatching(),
  closeBrackets(),
  autocompletion(),
  rectangularSelection(),
  crosshairCursor(),
  highlightActiveLine(),
  highlightSelectionMatches(),
  keymap.of([
    ...closeBracketsKeymap,
    ...defaultKeymap,
    ...searchKeymap,
    ...historyKeymap,
    ...foldKeymap,
    ...completionKeymap,
    ...lintKeymap,
  ]),
];

const diagnosticErrorGutterMarker = new (class extends GutterMarker {
  elementClass = "cm-diagnostic-error-gutter";
})();

function diagnosticLineDecorations(doc: EditorState["doc"], diagnostics: readonly Diagnostic[]) {
  const lineStarts = [...new Set(diagnostics.filter(({ severity }) => severity === "error").map(({ from }) => doc.lineAt(from).from))];
  return {
    content: Decoration.set(lineStarts.map((from) => Decoration.line({ class: "cm-diagnostic-error-line" }).range(from)), true),
    gutters: RangeSet.of(lineStarts.map((from) => diagnosticErrorGutterMarker.range(from)), true),
  };
}

const diagnosticLineDecorationsField = StateField.define<ReturnType<typeof diagnosticLineDecorations>>({
  create: () => ({ content: Decoration.none, gutters: RangeSet.empty }),
  update(value, transaction) {
    const diagnosticEffect = transaction.effects.find((effect) => effect.is(setDiagnosticsEffect));
    if (diagnosticEffect?.is(setDiagnosticsEffect)) return diagnosticLineDecorations(transaction.state.doc, diagnosticEffect.value);
    return { content: value.content.map(transaction.changes), gutters: value.gutters.map(transaction.changes) };
  },
  provide: (field) => [
    EditorView.decorations.from(field, (value) => value.content),
    gutterLineClass.from(field, (value) => value.gutters),
  ],
});

export interface ConfigTextEditorHandle {
  focusFirstDiagnostic: () => void;
  revealField: (text: string, field: string) => void;
}

/**
 * JSON.parse 是唯一裁决：解析通过绝不报错（部分语法树/错误恢复会对合法大文档误报）；
 * 确认损坏后才用 ensureSyntaxTree 取完整语法树定位。
 */
export function collectJsonDiagnostics(state: EditorState): Diagnostic[] {
  const text = state.doc.toString();
  if (!text.trim()) return [];
  try {
    JSON.parse(text);
    return [];
  } catch {
    // 已确认损坏，继续用语法树定位
  }
  const diagnostics: Diagnostic[] = [];
  // 1 秒预算内强制解析完整棵树；超时则退回当前可用的树（文档已确认损坏，只影响定位精度）
  const tree = ensureSyntaxTree(state, state.doc.length, 1000) ?? syntaxTree(state);
  tree.iterate({
    enter(node) {
      if (!node.type.isError) return;
      diagnostics.push({
        from: node.from,
        to: Math.min(state.doc.length, Math.max(node.to, node.from + 1)),
        severity: "error",
        source: "JSON",
        message: i18next.t("editor.jsonSyntaxError"),
      });
    },
  });
  return diagnostics;
}

export function computeTextChange(current: string, next: string) {
  let from = 0;
  while (from < current.length && from < next.length && current.charCodeAt(from) === next.charCodeAt(from)) from += 1;

  let currentTo = current.length;
  let nextTo = next.length;
  while (currentTo > from && nextTo > from && current.charCodeAt(currentTo - 1) === next.charCodeAt(nextTo - 1)) {
    currentTo -= 1;
    nextTo -= 1;
  }

  return { from, to: currentTo, insert: next.slice(from, nextTo) };
}

/** 从实际变更所在行向后定位字段，跳过前面同名字段和注释。 */
export function findConfigFieldPosition(text: string, field: string, from: number): number | null {
  const escaped = field.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`^[\\t ]*(?:"${escaped}"|'${escaped}'|${escaped})[\\t ]*[:=]`, "m");
  const start = from === 0 ? 0 : text.lastIndexOf("\n", from - 1) + 1;
  const match = pattern.exec(text.slice(start));
  return match ? start + match.index + match[0].search(/\S/) : null;
}

/**
 * 滚轮路由决策：只有向下滚且页面未到底时才把滚动交给页面；上滑永远滚编辑器。
 * 页面距底 ≤1px 视为已到底。
 */
export function routeEditorWheel(deltaY: number, pageDistanceToBottom: number): boolean {
  return deltaY > 0 && pageDistanceToBottom > 1;
}

// canvas 文本测量上下文只读共享：多个编辑器、多次显隐重挂复用，不随 effect 重建
let measureContext: CanvasRenderingContext2D | null | undefined;

/// 长文档按视口虚拟化，渲染宽度 ≠ 全文宽度：按 canvas 测量预设 contentDOM 的
/// min-width，原生横向滚动条才能滚到最后一列（tab 展开宽度也要计入）。
function syncEditorLayout(editor: EditorView) {
  const shell = editor.dom.closest<HTMLElement>(".apple-editor-shell");
  const section = shell?.closest<HTMLElement>(".apple-panel-section");
  const content = shell?.closest<HTMLElement>(".apple-edit-content");
  if (shell && section && content) {
    // 滚到底后，分区顶部保持原位；窗口新增高度全部用于代码区，不挤动页面滚动位置。
    const chromeHeight = shell.getBoundingClientRect().top - section.getBoundingClientRect().top;
    const bottomGap = parseFloat(getComputedStyle(content).paddingBottom);
    const reservedHeight = window.innerHeight - content.clientHeight + chromeHeight + bottomGap;
    shell.style.setProperty("--editor-max-height", `max(0px, calc(100vh - ${reservedHeight}px))`);
  }
  // 语法树预热：CM 初始化只解析前 3000 字符，其余在空闲时补，快速滚动会先跑到未解析区间（无高亮）
  forceParsing(editor, editor.state.doc.length, 200);
  if (measureContext === undefined) measureContext = document.createElement("canvas").getContext("2d");
  if (!measureContext) return;
  const style = getComputedStyle(editor.contentDOM);
  measureContext.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const tabWidth = measureContext.measureText(" ").width * editor.state.tabSize;
  let documentWidth = 0;
  for (let number = 1; number <= editor.state.doc.lines; number += 1) {
    const parts = editor.state.doc.line(number).text.split("\t");
    let lineWidth = 0;
    for (let index = 0; index < parts.length; index += 1) {
      lineWidth += measureContext.measureText(parts[index]!).width;
      if (index < parts.length - 1 && tabWidth > 0) {
        const remainder = lineWidth % tabWidth;
        lineWidth += remainder === 0 ? tabWidth : tabWidth - remainder;
      }
    }
    documentWidth = Math.max(documentWidth, lineWidth);
  }
  editor.contentDOM.style.minWidth = `${Math.ceil(documentWidth)}px`;
}

interface ConfigTextEditorProps {
  value: string;
  language: "toml" | "json";
  minLines?: number;
  placeholder?: string;
  readOnly?: boolean;
  validateToml?: (text: string) => Promise<TomlDiagnostic[]>;
  onChange: (value: string) => void;
  onDiagnostics: (summary: EditorDiagnosticSummary) => void;
}

const ConfigTextEditor = forwardRef<ConfigTextEditorHandle, ConfigTextEditorProps>(function ConfigTextEditor(
  { value, language, minLines = 1, placeholder, readOnly = false, validateToml = api.validateToml, onChange, onDiagnostics },
  ref,
) {
  const { t } = useTranslation();
  const [dark, setDark] = useState(() => document.documentElement.classList.contains("dark"));
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const valueRef = useRef(value);
  const onChangeRef = useRef(onChange);
  const onDiagnosticsRef = useRef(onDiagnostics);
  const lastSummary = useRef<EditorDiagnosticSummary | null>(null);
  const syncingValueRef = useRef(false);
  const pendingRevealRef = useRef<{ text: string; field: string } | null>(null);
  const editingCompartment = useRef(new Compartment());
  const indentUnitCompartment = useRef(new Compartment());
  const appliedIndentUnitRef = useRef<string | null>(null);
  const syncFrameRef = useRef(0);
  const destroyCheckRef = useRef(0);
  const destroyedRef = useRef(false);
  const creationDepsRef = useRef<readonly unknown[] | null>(null);
  const editorMinHeight = `min(${Math.max(1, minLines) * 19.2 + 8}px, var(--editor-max-height))`;

  valueRef.current = value;
  onChangeRef.current = onChange;
  onDiagnosticsRef.current = onDiagnostics;

  const scheduleContentWidthSync = (editor: EditorView) => {
    cancelAnimationFrame(syncFrameRef.current);
    syncFrameRef.current = requestAnimationFrame(() => syncEditorLayout(editor));
  };

  useEffect(() => {
    const observer = new MutationObserver(() => setDark(document.documentElement.classList.contains("dark")));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);

  useImperativeHandle(ref, () => ({
    revealField: (text, field) => {
      pendingRevealRef.current = viewRef.current?.state.doc.toString() !== text ? { text, field } : null;
    },
    focusFirstDiagnostic: () => {
      const view = viewRef.current;
      if (!view) return;
      let firstFrom: number | null = null;
      let firstTo: number | null = null;
      forEachDiagnostic(view.state, (_diagnostic, from, to) => {
        if (firstFrom === null) {
          firstFrom = from;
          firstTo = to;
        }
      });
      if (firstFrom === null || firstTo === null) return;
      view.dispatch({ selection: { anchor: firstFrom, head: firstTo }, scrollIntoView: true });
      view.focus();
    },
  }), []);

  // 编辑器实例在首帧绘制前同步创建（useLayoutEffect）：CodeMirror DOM 与页面同帧呈现，
  // 避免 useEffect（绘后执行）导致编辑器区域晚一帧出现的空壳闪烁。
  // 页面保活（AppShell 的 Activity）会把隐藏页的 effects 拆掉、复显时重装，而 React 不区分
  // "隐藏"与"真卸载"——两者共用同一段 cleanup。因此 cleanup 只摘除 DOM、不销毁实例，
  // 复显时把存活实例原样挂回（滚动位置、撤销栈、语法树、诊断全部保留，不闪重建帧）；
  // 真卸载交给微任务后裁决：届时 React 已把 hostRef 置空，补一次 destroy 防泄漏。
  useLayoutEffect(() => {
    const parent = hostRef.current;
    if (!parent) return;
    const scrollContent = parent.closest(".apple-edit-content");

    const reportDiagnostics = (view: EditorView) => {
      let count = 0;
      let firstLine: number | null = null;
      forEachDiagnostic(view.state, (_diagnostic, from) => {
        count += 1;
        if (firstLine === null) firstLine = view.state.doc.lineAt(from).number;
      });
      if (lastSummary.current?.count === count && lastSummary.current.firstLine === firstLine) return;
      lastSummary.current = { count, firstLine };
      onDiagnosticsRef.current({ count, firstLine });
    };

    const detachEditor = (view: EditorView, observer: ResizeObserver) => {
      cancelAnimationFrame(syncFrameRef.current);
      observer.disconnect();
      view.dom.remove();
      window.clearTimeout(destroyCheckRef.current);
      destroyCheckRef.current = window.setTimeout(() => {
        if (!hostRef.current && !destroyedRef.current) {
          view.destroy();
          destroyedRef.current = true;
          if (viewRef.current === view) viewRef.current = null;
        }
      }, 0);
    };

    window.clearTimeout(destroyCheckRef.current);
    // 主题取当下 DOM 真值，不信 dark state：Activity 隐藏期会拆掉本页 effects（连带
    // MutationObserver），隐藏中切主题时 setDark 不会跑。复显时 state 仍是旧主题，
    // 下面按 state 判等会走保活挂回分支，留下一个配色过期的实例（浅色高亮 + 深色底）。
    const currentDark = document.documentElement.classList.contains("dark");
    const creationDeps = [currentDark, language, placeholder, validateToml, t] as const;
    const previousDeps = creationDepsRef.current;
    const alive = viewRef.current;
    if (alive && !destroyedRef.current && previousDeps !== null && creationDeps.every((dep, index) => dep === previousDeps[index])) {
      // 保活复显：实例存活且创建参数没变，挂回原 DOM 即可
      parent.appendChild(alive.dom);
      alive.requestMeasure();
      scheduleContentWidthSync(alive);
      const reattachedObserver = new ResizeObserver(() => scheduleContentWidthSync(alive));
      reattachedObserver.observe(alive.dom);
      if (scrollContent) reattachedObserver.observe(scrollContent);
      return () => detachEditor(alive, reattachedObserver);
    }
    if (alive) {
      // 创建参数变了（主题/语言/占位文案）：旧实例彻底销毁后重建
      alive.destroy();
      destroyedRef.current = true;
      viewRef.current = null;
    }

    const jsonDiagnostics = linter((view) => collectJsonDiagnostics(view.state));
    const tomlDiagnostics = linter(async (view) => {
      const diagnostics = await validateToml(view.state.doc.toString());
      return diagnostics.map(({ from, to, message }) => ({
        from,
        to,
        severity: "error" as const,
        source: "TOML",
        message,
      }));
    });

    // 缩进单位按当前文档内容探测，供参考线按实际空白字符生成；
    // 编辑器可能先于异步详情挂载，value 到达/变化时由下方 [value] 效应重探并热重配
    const docIndentUnit = detectIndentUnit(valueRef.current);
    appliedIndentUnitRef.current = docIndentUnit;

    const editor = new EditorView({
      state: EditorState.create({
        doc: valueRef.current,
        extensions: [
          basicSetup,
          indentUnitCompartment.current.of(indentUnitFacet.of(docIndentUnit)),
          editingCompartment.current.of([
            EditorState.readOnly.of(readOnly),
            EditorView.editable.of(!readOnly),
          ]),
          editorPlaceholder(placeholder ?? t("editor.placeholder")),
          language === "toml" ? StreamLanguage.define(toml) : json(),
          language === "toml" ? tomlDiagnostics : jsonDiagnostics,
          diagnosticLineDecorationsField,
          ...(currentDark ? [oneDark] : []),
          EditorView.updateListener.of((update: ViewUpdate) => {
            if (update.docChanged && !syncingValueRef.current) onChangeRef.current(update.state.doc.toString());
            reportDiagnostics(update.view);
            if (update.docChanged || update.geometryChanged) scheduleContentWidthSync(update.view);
          }),
        ],
      }),
      parent,
    });
    viewRef.current = editor;
    destroyedRef.current = false;
    creationDepsRef.current = creationDeps;
    // 滚轮路由：下滚页面优先（页面没到底先滚页面露出完整编辑器），上滚编辑器优先
    // （编辑器没到顶先滚内容，到顶后滚页面）——合起来就是"滚轮滚还没到边的那个容器"。
    // cm-scroller 自带边界链式滚动拦截（见 style.css 滚动容器规则），越界方向必须在这里
    // 显式转发；横向滚动与缩放手势照常原生。直接写入页面滚动位置，避免反向滚动时
    // 旧的平滑目标继续追赶造成跳动。监听挂在实例的 scrollDOM 上，保活复显原样挂回时随实例一起存活。
    editor.scrollDOM.addEventListener("wheel", (event) => {
      if (event.ctrlKey) return; // 缩放手势（触控板捏合）放行原生
      const page = editor.dom.closest<HTMLElement>(".apple-edit-content");
      if (!page) return;
      const routeDown = routeEditorWheel(event.deltaY, page.scrollHeight - page.clientHeight - page.scrollTop);
      const routeUp = event.deltaY < 0 && editor.scrollDOM.scrollTop <= 0 && page.scrollTop > 1;
      if (!routeDown && !routeUp) return;
      event.preventDefault();
      page.scrollTop = Math.max(0, Math.min(page.scrollTop + event.deltaY, page.scrollHeight - page.clientHeight));
    }, { passive: false });
    const resizeObserver = new ResizeObserver(() => scheduleContentWidthSync(editor));
    resizeObserver.observe(editor.dom);
    if (scrollContent) resizeObserver.observe(scrollContent);
    syncEditorLayout(editor);
    reportDiagnostics(editor);

    return () => detachEditor(editor, resizeObserver);
  }, [dark, language, placeholder, validateToml, t]);

  useEffect(() => {
    const editor = viewRef.current;
    if (!editor) return;
    editor.dispatch({
      effects: editingCompartment.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    });
  }, [readOnly]);

  useEffect(() => {
    const editor = viewRef.current;
    if (!editor) return;
    const reveal = pendingRevealRef.current;
    pendingRevealRef.current = null;
    // 外部灌入的内容可能换了缩进风格（如异步详情晚于编辑器挂载），重探并热重配参考线
    const nextIndentUnit = detectIndentUnit(value);
    if (nextIndentUnit !== appliedIndentUnitRef.current) {
      appliedIndentUnitRef.current = nextIndentUnit;
      editor.dispatch({
        effects: indentUnitCompartment.current.reconfigure(indentUnitFacet.of(nextIndentUnit)),
      });
    }
    if (editor.state.doc.toString() === value) return;
    const change = computeTextChange(editor.state.doc.toString(), value);
    const revealPosition = reveal?.text === value ? findConfigFieldPosition(value, reveal.field, change.from) : null;
    const scrollContent = editor.dom.closest<HTMLElement>(".apple-edit-content");
    const previousContentScrollTop = scrollContent?.scrollTop ?? 0;
    const previousScrollTop = editor.scrollDOM.scrollTop;
    const previousScrollLeft = editor.scrollDOM.scrollLeft;
    let restoreFrame = 0;
    const restoreScrollPosition = () => {
      editor.scrollDOM.scrollTop = previousScrollTop;
      editor.scrollDOM.scrollLeft = previousScrollLeft;
    };
    syncingValueRef.current = true;
    try {
      editor.dispatch({ changes: change });
      if (revealPosition !== null) {
        editor.dispatch({ selection: { anchor: revealPosition }, scrollIntoView: true });
        if (scrollContent) {
          restoreFrame = requestAnimationFrame(() => {
            const targetScrollTop = scrollContent.scrollTop;
            if (targetScrollTop !== previousContentScrollTop) {
              scrollContent.scrollTop = previousContentScrollTop;
              scrollContent.scrollTo({ top: targetScrollTop, behavior: "smooth" });
            }
          });
        }
      } else {
        restoreScrollPosition();
        restoreFrame = requestAnimationFrame(restoreScrollPosition);
      }
    } finally {
      syncingValueRef.current = false;
    }
    return () => cancelAnimationFrame(restoreFrame);
  }, [value]);

  return (
    <div className="apple-editor-shell" style={{ minHeight: editorMinHeight }}>
      <div className="apple-editor-surface">
        <div ref={hostRef} />
      </div>
    </div>
  );
});

export default ConfigTextEditor;
