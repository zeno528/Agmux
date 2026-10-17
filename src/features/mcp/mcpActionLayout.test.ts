// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { Children, createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { setupI18n } from "../../i18n";
import McpView, { compareMcpServers, McpServerRow, McpTargetSwitch } from "./McpView";
import { setMcpDiffBadge } from "../../app/managementDataCache";
import McpConnectionForm from "./McpConnectionForm";

const viewSource = readFileSync(new URL("./McpView.tsx", import.meta.url), "utf8");
const editSource = readFileSync(new URL("./McpEdit.tsx", import.meta.url), "utf8");
const claudeEditSource = readFileSync(new URL("./ClaudeMcpEdit.tsx", import.meta.url), "utf8");
const sharedFormSource = readFileSync(new URL("./McpConnectionForm.tsx", import.meta.url), "utf8");

describe("MCP 操作入口", () => {
  it("禁用时旧成功结果和进行中的检测不再点亮状态灯", () => {
    setupI18n("zh-CN");
    const noop = () => undefined;
    const html = renderToStaticMarkup(createElement(McpServerRow, {
      server: { name: "sample", enabled: false, command: "sample-mcp", args: [], env: {}, url: null,
        startup_timeout_sec: null, tool_timeout_sec: null, bearer_token_env_var: null, http_headers: {}, env_http_headers: {} },
      result: { ok: true, latency_ms: 1, status: null, protocol_version: null, server_info: null,
        tools: [], tools_truncated: false, error: null, tools_error: null },
      probing: true, detailsVisible: false, toolsBusy: false, toolsLoaded: false,
      onEdit: noop, onDelete: noop, onProbe: noop, onToggleTools: noop,
    }));
    const indicator = html.match(/<span\b[^>]*role="img"[^>]*>/)![0];
    expect(indicator).not.toContain("bg-success");
    expect(indicator).toContain('aria-label="已禁用"');
  });

  it("已有 MCP 名称仅展示，新建名称可填写，连接字段仍可编辑", () => {
    setupI18n("zh-CN");
    const noop = () => undefined;
    for (const nameReadOnly of [true, false]) {
      const html = renderToStaticMarkup(createElement(McpConnectionForm, {
        name: "sample", setName: noop, nameReadOnly,
        transport: "http", setTransport: noop,
        command: "", setCommand: noop, argsText: "", setArgsText: noop,
        url: "https://example.test/mcp", setUrl: noop,
        envPairs: [], setEnvPairs: noop, headerPairs: [], setHeaderPairs: noop,
        timeoutFields: null,
      }));
      const inputs = html.match(/<input\b[^>]*>/g)!;
      expect(inputs.some((input) => input.includes('value="sample"'))).toBe(!nameReadOnly);
      if (nameReadOnly) expect(html).toContain('<div class="app-input flex min-w-0 items-center gap-2"><span class="truncate font-medium text-(--text-secondary)">sample</span></div>');
      expect(inputs.find((input) => input.includes('value="https://example.test/mcp"'))).not.toMatch(/readOnly|disabled/);
    }
  });

  it("真实客户端切换只替换内容组件，页头与切换器保留相同类型、位置和 key", () => {
    setupI18n("zh-CN");
    const frames: { headerType: unknown; switchType: unknown; key: string | null; value: string; contentKey: string | null }[] = [];
    function Capture() {
      // 在 React 的真实服务端渲染中执行父组件，触发同一个 setTarget；不模拟组件状态。
      const tree = McpView({ activationEpoch: 0, popEpoch: 0 });
      const children = Children.toArray(tree.props.children) as ReactElement<{ children: ReactNode }>[];
      const header = children[0];
      const content = children[children.length - 1];
      const toolbar = Children.toArray(header!.props.children)[1] as ReactElement<{ children: ReactNode }>;
      const control = Children.toArray(toolbar.props.children).find((child) => isValidElement(child) && child.type === McpTargetSwitch) as ReactElement<Parameters<typeof McpTargetSwitch>[0]>;
      frames.push({ headerType: header!.type, switchType: control.type, key: control.key, value: control.props.value, contentKey: content!.key });
      if (control.props.value === "codex") control.props.onChange("claude");
      return null;
    }
    renderToStaticMarkup(createElement(Capture));
    expect(frames.map((frame) => frame.value)).toEqual(["codex", "claude"]);
    expect(frames[0]!.headerType).toBe("header");
    expect(frames[1]!.headerType).toBe(frames[0]!.headerType);
    expect(frames[1]!.switchType).toBe(frames[0]!.switchType);
    expect(frames.map((frame) => frame.key)).toEqual([".$target-switch", ".$target-switch"]);
    expect(frames.map((frame) => frame.contentKey)).toEqual([".$codex", ".$claude"]);
    expect(viewSource.match(/<McpTargetSwitch\b/g)).toHaveLength(1);
  });

  it("客户端页签复用共享滑块，保留图标、选中语义和页头尺寸", () => {
    setupI18n("zh-CN");
    setMcpDiffBadge({ count: 7, error: false });
    setMcpDiffBadge({ count: 8, error: false }, "claude");
    for (const [index, value] of (["codex", "claude"] as const).entries()) {
      const html = renderToStaticMarkup(createElement(McpTargetSwitch, { value, onChange: () => undefined }));
      expect(html).toContain('class="app-segmented-control mcp-target-switch h-[var(--toolbar-control-height)] shrink-0"');
      expect(html).toContain('role="tablist"');
      expect(html).toContain(`style="--segment-count:2;--segment-index:${index}"`);
      const tabs = [...html.matchAll(/<button[^>]*role="tab"[^>]*aria-selected="(true|false)"[^>]*>(.*?)<\/button>/g)];
      expect(tabs).toHaveLength(2);
      tabs.forEach((tab, tabIndex) => expect(tab[1]).toBe(tabIndex === index ? "true" : "false"));
      // 页签图标保持清晰：Codex 用正文色，Claude 用品牌色。
      expect(tabs[0]![2]).toContain("text-(--text-primary)");
      expect(tabs[1]![2]).toContain("text-(--brand-claude)");
      expect(tabs[0]![2]).toContain('class="apple-count-badge" aria-hidden="true">7</span>');
      expect(tabs[1]![2]).toContain('class="apple-count-badge" aria-hidden="true">8</span>');
      expect(tabs[0]![2]).not.toContain("opacity-");
      expect(tabs[1]![2]).not.toContain("opacity-");
      expect(html).not.toContain("app-button--primary");
    }
    setMcpDiffBadge({ count: 0, error: false });
    setMcpDiffBadge({ count: 0, error: false }, "claude");
    const styles = readFileSync(new URL("../../style.css", import.meta.url), "utf8");
    expect(styles).toContain(".mcp-target-switch .apple-count-badge {");
    expect(styles).toContain("position: static;");
  });

  it("两个客户端共用编辑标题、按钮、通知和卸载确认文案", () => {
    for (const key of [
      "edit.back", "edit.createTitle", "edit.editTitle",
      "edit.name", "edit.namePlaceholder", "edit.format", "edit.cancel", "edit.saving", "edit.save",
      "feedback.formatted", "feedback.formatFailed", "feedback.invalidName", "feedback.saved",
    ]) {
      expect(editSource + sharedFormSource).toContain(`t("${key}"`);
      expect(claudeEditSource + sharedFormSource).toContain(`t("${key}"`);
    }
    const claudeView = viewSource.slice(viewSource.indexOf("function McpClientView("));
    for (const key of ["confirm.deleteTitle", "confirm.deleteDescription", "confirm.delete", "feedback.deleted", "empty.description"]) {
      expect(viewSource).toContain(key);
      expect(claudeView).toContain(key);
    }
    expect(claudeEditSource + viewSource).not.toContain('"claude.');
  });

  it("列表把编辑、测试、工具和卸载收进三点菜单，开关仍在最右侧", () => {
    expect(viewSource).toContain("<MoreHorizontal");
    expect(viewSource).toContain('className="app-select-menu app-popover"');
    expect(viewSource).toContain('aria-label={t("list.moreTooltip")}');
    expect(viewSource).toContain("<Pencil");
    expect(viewSource).toContain("<Wrench");
    expect(viewSource).toContain('className="apple-icon-button');
    expect(viewSource).toContain("<AppSwitch");
    expect(viewSource).toContain("menuTriggerRef");
    expect(viewSource).toContain('onDelete={(target) => void removeServer(target)}');
    expect(viewSource).toContain('setMenuOpen(false); onDelete(server);');
    expect(viewSource).toContain('{t("edit.uninstall")}');
  });

  it("两个客户端编辑页都不再提供卸载入口", () => {
    for (const source of [editSource, claudeEditSource]) {
      expect(source).not.toContain("onDelete");
      expect(source).not.toContain('t("edit.uninstall")');
    }
  });

  it("源码编辑不触发保存，普通输入保留回车保存", () => {
    for (const source of [editSource, claudeEditSource]) {
      expect(source).toContain('event.key === "Enter" && !event.nativeEvent.isComposing && !(event.target instanceof Element && event.target.closest("textarea, .apple-editor-shell"))');
      expect(source).not.toContain('event.ctrlKey && event.key === "Enter"');
    }
  });

  it("两端复用连接表单，Claude 保留原生 JSON 保存入口和毫秒超时", () => {
    for (const source of [editSource, claudeEditSource]) expect(source).toContain("<McpConnectionForm");
    expect(claudeEditSource).toContain("patchClaudeMcpForm(jsonText, next, field)");
    expect(claudeEditSource).toContain("onChange={editJson}");
    expect(claudeEditSource).toContain("disabled={!initialized || !formValid || saving}");
    expect(claudeEditSource).toContain("api.claudeSaveMcpServer(server?.name ?? null, trimmedName, jsonText)");
    expect(claudeEditSource).toContain("min={1000} step={1000}");
    expect(claudeEditSource).not.toContain("startupTimeout");
  });

  it("共享表单高级区常显无折叠标题，环境变量使用添加变量，启动参数独占一行", () => {
    expect(sharedFormSource).not.toContain("AppDisclosure");
    expect(sharedFormSource).not.toContain('t("edit.advanced")');
    expect(sharedFormSource).toContain('className="mt-4 grid gap-4"');
    expect(sharedFormSource).toContain('t("edit.addVariable")');
    expect(sharedFormSource).toContain("ArgsEditor");
    expect(sharedFormSource).toContain('text ? text.split(/\\r?\\n/) : [""]');
    expect(sharedFormSource).toContain('t("edit.addArgument")');
    expect(sharedFormSource).not.toContain("<textarea");
    // 只剩名称/传输类型一处在用双列网格，启动参数不再与启动命令同排
    expect(sharedFormSource.match(/sm:grid-cols-2/g)).toHaveLength(1);
    // 环境变量/请求头为空时兜底一行占位空行
    expect(sharedFormSource).toContain("const rows = pairs.length ? pairs : [{ key: \"\", value: \"\" }];");
    // 行删除按钮只在仅剩一行且为空时禁用；多行时空行也按行删除
    expect(sharedFormSource).toContain("disabled={args.length === 1 && !arg.trim()}");
    expect(sharedFormSource).toContain("disabled={rows.length === 1 && !pair.key.trim() && !pair.value.trim()}");
    // 请求头不做掩码显示，没有查看小眼睛
    expect(sharedFormSource).not.toContain("maskValue");
    expect(sharedFormSource).not.toContain("EyeOff");
  });

  it("进入列表只做静默连通性探测，工具按钮才刷新工具", () => {
    expect(viewSource).not.toContain("MCP_STATUS_REFRESH_MS");
    expect(viewSource).toContain("probe(server, { manual: false })");
    // 工具只从扳手来（保存等其余路径一律只验连通性，不拉清单）
    expect([...viewSource.matchAll(/probe\(server, \{ includeTools: true \}\)/g)]).toHaveLength(1);
    expect(viewSource).toContain("api.probeMcpServer(name, includeTools, manual, scope)");
  });

  it("Claude MCP 复用共享列表和探测缓存，进页不重新点亮状态灯", () => {
    expect(viewSource).toContain("getCachedClaudeMcpServers()");
    expect(viewSource).toContain('useMcpProbes(target,');
    expect(viewSource).toContain("applyCache(next)");
    expect(viewSource).not.toContain("setProbeResults({});");
  });

  it("Claude MCP 进页和激活强刷列表，单条保存、删除和开关不全量重探", () => {
    const claude = viewSource.slice(viewSource.indexOf("function McpClientView("));
    expect(claude).toContain("void refresh(true);");
    expect(claude).toContain('if (target === "claude") void refresh(true, []);');
    expect(claude).toContain("await refresh(true, []);");
    expect(claude).toContain("void refresh(true, [name]);");
    expect(claude).toContain("refresh(true, enabled ? [target.name] : [])");
    expect(claude).toContain("only ? next.filter((server) => only.includes(server.name)) : next");
  });

  it("MCP 卡片空白处可折叠且不抢占操作控件", () => {
    expect(viewSource).toContain('event.target.closest(\'button, input, code, [role="switch"]\')');
    expect(viewSource).toContain("if (!detailsVisible");
    expect(viewSource).toContain("onToggleTools(server);");
  });

  it("工具加载不占用工具按钮，完成后不强制重新展开", () => {
    expect(viewSource).toContain("{toolsBusy ? <span className=\"muted shrink-0\"><LoadingSpinner /></span> : null}");
    expect(viewSource).not.toContain("{toolsBusy ? <MetaChip><LoadingSpinner /></MetaChip> : null}");
    expect(viewSource).toContain("<McpToolsPanel result={result} pending={toolsBusy || !toolsLoaded} />");
    // 工具还在取的时候不下"服务端没有返回工具"的结论：那一刻的 result 是连通探测留下的空壳
    expect(viewSource).toContain("!result.tools.length && !pending");
    expect(viewSource).not.toContain("{loading ? <div className=\"grid place-items-center py-2\"><LoadingSpinner /></div>");
    expect(viewSource).not.toContain("disabled={toolsBusy}");
    expect(viewSource).toContain("if (toolsLoading[name])");
  });

  it("服务器禁用或测试中时，三点菜单中的测试和工具动作仍禁用，编辑可用", () => {
    expect(viewSource).toContain("disabled={probing || server.enabled === false}");
    expect(viewSource).toContain("disabled={server.enabled === false}");
    expect(viewSource).toContain("onEdit(server)");
  });

  it("列表按类型分组（stdio → http → unknown）优先、组内按名称", () => {
    expect(viewSource).toContain("orderedServers.map((server) => (");
    const fixture = [
      { name: "zeta", url: "https://x" },
      { name: "alpha", command: "npx" },
      { name: "mystery" },
      { name: "beta", command: "uvx" },
    ] as unknown as Parameters<typeof compareMcpServers>[0][];
    expect([...fixture].sort(compareMcpServers).map((item) => item.name)).toEqual(["alpha", "beta", "zeta", "mystery"]);
  });
});
