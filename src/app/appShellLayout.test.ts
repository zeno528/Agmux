// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./AppShell.tsx", import.meta.url), "utf8");
const hooksSource = readFileSync(new URL("./appShellHooks.ts", import.meta.url), "utf8");
const profileEditSource = readFileSync(new URL("../features/codex/CodexProfileEdit.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../style.css", import.meta.url), "utf8");

describe("AppShell 布局", () => {
  it("保持侧栏导航紧贴品牌区，并按 Codex/Claude 分组数据驱动渲染，通用导航不显示标题", () => {
    expect(source).toContain("apple-sidebar relative flex h-full shrink-0 flex-col");
    expect(source).toContain('className="mx-1.5 mt-3 flex flex-1 flex-col gap-3"');
    expect(source).toContain("sidebarGroups.map");
    expect(source).toContain('className={navClass} data-active={view === item.view ? "true" : undefined}');
    expect(source).not.toContain("apple-sidebar-group-label");
    expect(source).toContain('src="/codex.svg"');
    expect(source).toContain('src="/claude-code.svg"');
    expect(source).toContain('key: "clients"');
    expect(source.indexOf('labelKey: "nav.groupCodex"')).toBeLessThan(source.indexOf('labelKey: "nav.groupClaude"'));
    expect(source).toContain('view: "claudeProfiles"');
    expect(source).toContain('view: "codexProfiles"');
    expect(source).not.toContain('labelKey: "nav.providers"');
    expect(source).not.toContain('labelKey: "nav.claudeProviders"');
    expect(styles).toContain(".apple-sidebar-group--content");
    expect(source.indexOf('key: "clients"')).toBeLessThan(source.indexOf('key: "common"'));
    expect(source.indexOf('labelKey: "nav.plugins"')).toBeLessThan(source.indexOf('labelKey: "nav.mcp"'));
    expect(source).toContain('key: "footer"');
    expect(source.indexOf('labelKey: "nav.accounts"')).toBeLessThan(source.indexOf('labelKey: "nav.settings"'));
    expect(source).toContain('group.key === "footer" ? <UpdateNotice');
    // 客户端按钮与下方功能入口复用全局侧栏文字动画，不能用 display:none 瞬间切换。
    expect(styles).not.toContain(".apple-sidebar-group-label");
    expect(styles).toContain(".apple-sidebar-nav-button img");
    // 每个导航项必须渲染可见文案 + 收缩态悬浮提示（曾因修复闭合标签丢失过，钉死）
    expect(source).toContain('<span className="apple-sidebar-label" aria-hidden={sidebar.sidebarCollapsed}>{t(item.labelKey)}</span>');
    expect(source).toContain('sidebar.sidebarCollapsed && sidebar.sidebarFlyoutArmed ? <span className="apple-sidebar-flyout"');
    // 底部绝对定位的手写按钮容器已由分组数据驱动渲染取代
    expect(source).not.toContain("absolute inset-x-1.5 bottom-4");
  });

  it("切页记忆：进过的页面 Activity 保活，reset 键已拆除", () => {
    // 渲染以 Record<AppView, ReactNode> 为唯一清单：新增 AppView 漏页是编译错误
    expect(source).toContain("const pages: Record<AppView, ReactNode>");
    // 只保活进过的页面：未访问页连渲染都不发生，冷启动零新增
    expect(source).toContain("visitedViews.has(pageView)");
    expect(source).toContain('<Activity key={pageView} mode={view === pageView ? "visible" : "hidden"}>');
    // reset 键是"切回必回列表"的根源：每次进入 bump 强制重挂载，禁止复活
    expect(source).not.toContain("profilesReset");
    expect(source).not.toContain("mcpReset");
    // 外层 key 不得含 view：否则每次切页整树重挂载，保活失效
    expect(source).toContain('key={state ? "app" : "loading"}');
    expect(source).not.toContain('key={state ? view : "loading"}');
    expect(source).toContain('<SettingsView state={state} onPreviewTheme={previewTheme}');
    // 首进页面必须在渲染期进挂载清单：useEffect 晚一帧才提交，
    // 「旧页已隐藏、新页未挂载」的空主区域帧正是首进闪屏的来源
    expect(source).toContain('if (!visitedViews.has(view)) {\n    setVisitedViews(new Set(visitedViews).add(view));\n  }');
    expect(source).not.toContain("setVisitedViews((prev) => (prev.has(view)");
  });

  it("重复点击当前视图的侧栏按钮弹回该视图一级，切其他视图不弹", () => {
    // 只有重复点击当前视图才 bump popRequest；目标视图收到递增的 epoch 自行清子页状态
    expect(source).toContain("if (target !== view) setView(target);");
    expect(source).toContain("setPopRequest((current) => ({ view: target, epoch: (current?.epoch ?? 0) + 1 }))");
    // 托盘与编辑页的程序化跳转不参与弹回
    expect(source).toContain('openSettings={() => setView("settings")}');
    expect(source).toContain('onManageChatgptAccounts={() => setView("accounts")}');
    // 有整页子级的五个视图都接了 popEpoch；effect 体逐字锁定：只清子页状态、带零反应守卫、无刷新调用
    const popEffects: [string, string][] = [
      ["features/codex/CodexProfilesView", "if (!popEpoch) return;\n    setEditingProfile(null);\n    setEditDetail(null);\n    setCreatingProfile(false);\n  }, [popEpoch]);"],
      ["features/claude/ClaudeProfilesView", "if (!popEpoch) return;\n    setEditingProfile(null);\n    setEditDetail(null);\n    setCreatingProfile(false);\n  }, [popEpoch]);"],
      ["features/mcp/McpView", "if (!popEpoch) return;\n    setEditingServer(null);\n    setCreatingServer(false);\n    setDiffOpen(false);\n  }, [popEpoch]);"],
      ["features/plugins/CodexPluginsView", "if (!popEpoch) return;\n    setSelectedPlugin(null);\n    setShowAdd(false);\n    setShowSources(false);\n  }, [popEpoch]);"],
      ["features/skills/SkillsView", "if (!popEpoch) return;\n    setImporting(false);\n  }, [popEpoch]);"],
    ];
    for (const [file, effect] of popEffects) {
      const viewSource = readFileSync(new URL(`../${file}.tsx`, import.meta.url), "utf8");
      expect(viewSource, file).toContain(effect);
    }
  });

  it("一级切页不补播动画，同页内部挂载不重复播", () => {
    // view 改变时重新标记已有内容，只观察后续页内切换。
    expect(source).toContain("}, [mainRef, view]);");
    // 隐藏保活页仍在 DOM（display:none），必须按可见性跳过
    expect(source).toContain("offsetParent === null");
    // played 随 effect 重建：同一次停留内同一元素不重播
    expect(source).toContain("const played = new WeakSet<Element>(main.querySelectorAll(PAGE_ENTER_TARGET));");
  });

  it("侧栏 MCP 角标首屏只读缓存，差异查询延迟到首屏之后再执行", () => {
    // 首屏：useSyncExternalStore 从 localStorage 缓存直出，不发起任何查询
    expect(source).toContain("useSyncExternalStore(subscribeMcpDiffBadge, getMcpDiffBadge)");
    expect(source).toContain('badgeDot: Boolean(mcpBadge)');
    expect(source).not.toContain("item.badgeText");
    // 查询：必须被 startupReady 门控 + 固定延迟，禁止直接进首屏/冷启动关键路径
    expect(source).toContain("if (!startupReady) return;");
    expect(source).toContain("window.setTimeout(checkMcpDiff, 1500);");
    // 不重复查：非静默启动时 onActive 已经查过，定时器只补静默启动那条路
    expect(source).toContain("if (activationEpoch > 0) return;");
  });

  it("客户端更新缓存驱动展开/收缩侧栏的状态圆点", () => {
    expect(source).toContain("subscribeCliUpdate");
    expect(source).toContain("badgeDot: Boolean(codexCliUpdate?.available)");
    expect(source).toContain("badgeDot: Boolean(claudeCliUpdate?.available)");
    expect(source).toContain('className="apple-sidebar-update-dot"');
    expect(styles).toContain(".apple-sidebar-update-dot {");
  });

  it("MCP 页查到差异后写回共享缓存，侧栏与页面同源", () => {
    const mcpViewSource = readFileSync(new URL("../features/mcp/McpView.tsx", import.meta.url), "utf8");
    expect(mcpViewSource).toContain("setMcpDiffBadge({ count: preview.entries.length, error: false }, target)");
  });

  it("config.toml 解析失败时侧栏角标同步亮起，不点进 MCP 页也能看见", () => {
    const mcpViewSource = readFileSync(new URL("../features/mcp/McpView.tsx", import.meta.url), "utf8");
    // MCP 页查失败写回 error 态
    expect(mcpViewSource).toContain("setMcpDiffBadge({ count: 0, error: true }, target)");
    // 启动后的静默刷新同理：失败不能被吞掉
    expect(source).toContain("setMcpDiffBadge({ count: 0, error: true }, target)");
    // 侧栏圆点、客户端数字和更新入口使用同一份差异状态。
    expect(source).toContain("const mcpBadge = mcpDiffBadgeText(mcpDiffBadge)");
    expect(mcpViewSource).toContain("mcpDiffBadgeText(targetBadge)");
  });

  it("窗口激活时顺带查一次 MCP 差异，与启动后那次共用同一条规则", () => {
    // 激活那几件套：刷新 state / 账号状态 / MCP 差异 / 恢复轮询
    const onActive = source.slice(source.indexOf("const onActive = ()"), source.indexOf("const onInactive = ()"));
    expect(onActive).toContain("checkMcpDiff()");
    expect(onActive).toContain("void refresh()");
    // 整个 AppShell 里只发这一个请求：复制一份出来就等于两处规则会分家
    expect(source.split("api.codexMcpSyncPreview()").length - 1).toBe(1);
  });

  it("首屏稳定后统一预热管理页数据，失败无感", () => {
    // 冷启动条款：预热必须延迟（首屏之后）+ 异步 fire-and-forget + 失败无感
    expect(source).toContain("if (!startupReady) return;");
    expect(source).toContain("window.setTimeout(() => {");
    for (const loader of ["loadMcpServers()", "loadSkills()", "loadPlugins()", "loadPluginMarketplaces()"]) {
      expect(source).toContain(`void ${loader}.catch(() => undefined);`);
    }
  });

  it("只在窗口从非激活状态恢复时刷新", () => {
    expect(hooksSource).toContain("const activeRef = useRef(!document.hidden);");
    expect(hooksSource).toContain("if (activeRef.current) return false;");
    expect(hooksSource).toContain("if (!activeRef.current) return false;");
    expect(source).toContain("if (!activate()) return;");
    expect(source).toContain("if (!deactivate()) return;");
    expect(source).toContain("if (isTauri && appWindow) {");
    expect(source).toContain("appWindow.onFocusChanged");
    expect(source).toContain("appWindow.isFocused()");
  });

  it("账号入口固定在设置入口上方（分组数组顺序），并通过全局视图打开", () => {
    expect(source).toContain('view: "accounts"');
    expect(source).toContain('view: "settings"');
    expect(source.indexOf('view: "accounts"')).toBeLessThan(source.indexOf('view: "settings"'));
    expect(source).toContain('onManageChatgptAccounts={() => setView("accounts")}');
  });

  it("托盘账号项恢复窗口后打开账号页", () => {
    expect(source).toContain('listen("tray-open-accounts", () => latest.current.openAccounts())');
    expect(source).toContain('openAccounts={() => setView("accounts")}');
  });

  it("冷启动窗口一路透传到供应商卡：余额刷新只在进程启动期间延后", () => {
    const profilesViewSource = readFileSync(new URL("../features/codex/CodexProfilesView.tsx", import.meta.url), "utf8");
    const codexProfileCardSource = readFileSync(new URL("../features/codex/CodexProfileCard.tsx", import.meta.url), "utf8");
    expect(source).toContain("coldStart={!startupReady}");
    expect(profilesViewSource).toContain("coldStart={coldStart}");
    expect(codexProfileCardSource).toContain("coldStart: boolean;");
  });

  it("首屏完成后才启动自动更新检查", () => {
    const startup = source.slice(source.indexOf("let delayedAuth: number | undefined;"));
    expect(source).toContain("const [startupReady, setStartupReady] = useState(false);");
    expect(source).toContain("setStartupReady(true);");
    const updater = source.match(/<AppUpdateProvider\b[^>]*>/)?.[0] ?? "";
    expect(updater).toContain("enabled={Boolean(state?.settings.auto_check_update) && startupReady}");
    expect(updater).toContain("ready={startupReady}");
    expect(updater).toContain("proxyMode={state?.settings.proxy_mode}");
    expect(updater).toContain("proxyUrl={state?.settings.proxy_url}");
    expect(startup.indexOf("setStartupReady(true);")).toBeGreaterThan(startup.indexOf("await appWindow?.show();"));
  });

  it("启动里程碑上报：state_ready 在数据就绪时、pre_show 带留痕、window_shown 在出窗后", () => {
    const startup = source.slice(source.indexOf("let delayedAuth: number | undefined;"));
    // 没有这些埋点，冷启动 650ms 拆不出 JS 段耗时，性能回归就只能靠猜
    expect(source).toContain('api.reportStartupMark("state_ready"');
    expect(source).toContain('reportStartupMark("window_pre_show"');
    expect(source).toContain('let rafPath = "fallback";');
    // rAF 留痕必须在 show() 之前——探针在窗口可见后就杀进程，show 之后再报会被吃掉
    expect(startup.indexOf("reportStartupMark(\"window_pre_show\"")).toBeLessThan(startup.indexOf("await appWindow?.show();"));
    expect(startup.indexOf("reportStartupMark(\"window_shown\"")).toBeGreaterThan(startup.indexOf("await appWindow?.show();"));
  });

  it("认证快照由全局状态完成后再交给配置编辑页", () => {
    expect(hooksSource).toContain("const [authStatusReady, setAuthStatusReady] = useState(false);");
    expect(hooksSource).toContain("setAuthStatusReady(true);");
    expect(source).toContain("authStatusReady={authStatusReady}");
    expect(source).toContain("onAuthStatusChange={updateAuthStatus}");
  });

  it("移除侧栏激活装饰条，并固定悬浮卡片为普通字重", () => {
    expect(source).not.toContain("apple-sidebar-indicator");
    expect(source).toContain('const navClass = "apple-sidebar-nav-button app-selection-state";');
    expect(source).toContain('data-active={view === item.view ? "true" : undefined}');
    expect(source).not.toContain('active ? "bg-(--tile-bg) text-accent" :');
    expect(source).not.toContain('active ? "bg-(--selection-bg) text-accent" :');
    expect(source).not.toContain('active ? "bg-(--selection-bg) font-semibold text-accent" :');
    // 侧栏标签的颜色两主题都靠继承 body（浅 #1c1c1e / 深 #ffffff），激活项不单独变色。
    // 禁止再给标签写直接 color 规则，保持激活态与普通项一致。
    expect(styles).not.toContain(".apple-sidebar-nav-button .apple-sidebar-label");
    expect(styles).not.toContain(".apple-sidebar-indicator");
    expect(styles).toMatch(/\.apple-sidebar-flyout \{[\s\S]*font-weight: 400;/);
    expect(styles).toContain("color: var(--text-primary);");
  });

  it("让设置项说明在浅色和深色主题都保持弱层级", () => {
    const descriptionStyles = styles.match(/\.setting-description \{[\s\S]*?\n\}/)?.[0];
    expect(descriptionStyles).toContain("color: var(--text-secondary);");
    expect(descriptionStyles).toContain("font-weight: 400;");
    expect(styles).toContain(":root.dark .setting-description {\n  color: var(--text-secondary);\n}");
  });

  it("按 4K 150% 缩放换算侧栏与窗口标题栏尺寸，并让窗口控制区底边与主卡片顶边重合", () => {
    expect(styles).toContain("--sidebar-width-collapsed: 3.125rem;");
    expect(styles).toContain("--window-chrome-height: 2.2083333333rem;");
  });

  it("浅色主题的侧栏与窗口标题栏使用统一背景色", () => {
    expect(styles).toContain("--sidebar-bg: var(--panel-bg);");
    expect(styles).toMatch(/\.apple-window-chrome \{[\s\S]*?background: var\(--sidebar-bg\);/);
    expect(styles).toMatch(/\.apple-sidebar \{[\s\S]*?background: var\(--sidebar-bg\);/);
  });

  it("让全局卡片浅色使用微暖白、深色保持原卡片底色", () => {
    expect(styles).toContain("--panel-bg: #f5f5f5;");
    expect(styles).toContain("--panel-bg: #292b30;");
    expect(styles).toMatch(/\.apple-group \{[\s\S]*?background: var\(--panel-bg\);/);
    expect(styles).toMatch(/\.panel \{[\s\S]*?background: var\(--panel-bg\);/);
  });

  it("普通悬停块复用侧边栏原有的深浅色高亮", () => {
    expect(styles).toMatch(/:root \{[^}]*--hover-bg: rgb\(0 0 0 \/ 0\.05\);/);
    expect(styles).toMatch(/:root\.dark \{[^}]*--hover-bg: rgb\(255 255 255 \/ 0\.08\);/);
    expect(styles.match(/--hover-bg:/g)).toHaveLength(2);
    for (const selector of ['[data-window-controls-hover="true"] .window-control-button:hover', ".apple-back-button:hover", ".editor-ghost-group:hover", ".app-dialog-close:hover", ".app-toast__close:hover"]) {
      expect(styles).toContain(`${selector} {\n  background: var(--hover-bg);`);
    }
    expect(styles).toContain(":where(.apple-icon-button:hover:not(:disabled), .apple-icon-button:focus-visible:not(:disabled)) {\n  background: var(--hover-bg);");
    // :where 归零守卫优先级是硬契约：否则通用悬停压过组内透明覆盖，ghost 组悬停退化成左右分块高亮
    expect(styles).toContain('.editor-ghost:where(:not(:disabled):not([aria-disabled="true"])):hover {\n  background: var(--hover-bg);');
    expect(styles).toContain(".editor-ghost-group > .editor-ghost:hover {\n  background: transparent;\n}");
  });

  it("让主内容表面浅色使用白色、深色使用 #1e1e1e", () => {
    expect(styles).toContain("--main-surface-bg: #ffffff;");
    expect(styles).toContain("--main-surface-bg: #1e1e1e;");
    expect(styles).toContain("--input-bg: var(--main-surface-bg);");
    expect(styles).toMatch(/\.apple-main-card \{[\s\S]*?background: var\(--main-surface-bg\);/);
    expect(styles).toMatch(/\.apple-page-bar \{[\s\S]*?background: var\(--main-surface-bg\);/);
    expect(styles).toMatch(/\.apple-edit-toolbar--footer \{[\s\S]*?background: var\(--main-surface-bg\);/);
  });

  it("让捕获配置图标按钮保持圆形", () => {
    expect(styles).toContain(".apple-toolbar-group > .apple-icon-button {\n  border-radius: 999px;");
  });

  it("让 Codex 状态操作组使用主题表面,仅圆点承载运行色并让运行圆点呼吸", () => {
    const capsuleStyles = styles.match(/\.codex-status-control \{[\s\S]*?\n\}/)?.[0] ?? "";
    const signalStyles = styles.match(/\.codex-status__signal \{[\s\S]*?\n\}/)?.[0] ?? "";
    expect(capsuleStyles).toContain("border: 1px solid var(--primary-button-hover-bg);");
    expect(capsuleStyles).toContain("background: var(--primary-button-bg);");
    expect(styles).not.toContain("color-mix(in srgb, var(--success) 18%, var(--panel-ring))");
    expect(signalStyles).toContain("color: color-mix(in srgb, var(--primary-button-text) 65%, transparent);");
    expect(signalStyles).toContain("background: transparent;");
    expect(styles).not.toContain("color-mix(in srgb, var(--text-secondary) 8%, transparent)");
    expect(styles).toContain(".codex-status--running .codex-status__signal {\n  color: var(--success);\n}");
    expect(styles).toContain(".codex-status__signal-dot {\n  position: relative;\n  isolation: isolate;\n  width: 0.375rem;\n  height: 0.375rem;");
    expect(styles).toContain(".codex-status--running .codex-status__signal-dot {\n  animation: codex-status-breathe 2.8s ease-in-out infinite;");
    expect(styles).toContain("0%, 100% { transform: scale(1); }\n  50% { transform: scale(1.3); }");
    expect(styles).toContain(".codex-status__signal-dot::after {");
    expect(styles).toContain("background: currentColor;");
    expect(styles).toContain("filter: blur(2px);");
    expect(styles).toContain(".codex-status--running .codex-status__signal-dot::after {\n  animation: codex-status-breathe-halo 2.8s ease-in-out infinite;");
    expect(styles).toContain("@keyframes codex-status-breathe-halo {");
    expect(styles).toContain("0%, 100% { opacity: 0; transform: scale(1); }\n  50% { opacity: 0.18; transform: scale(1.35); }");
    expect(styles).not.toContain("codex-status-halo");
    expect(styles).not.toContain("codex-status-sheen");
    expect(styles).not.toContain(".codex-status--running::after");
    expect(styles).toContain(".codex-status__action {");
    expect(styles).toContain("border-radius: 50%;");
    expect(styles).toContain("width: calc(var(--control-height) - 0.125rem);");
    expect(styles).toContain("height: calc(var(--control-height) - 0.125rem);");
    expect(styles).toContain("margin: 0.125rem;");
    expect(styles).toContain("border: 0;");
    expect(styles).toContain("background: color-mix(in srgb, currentColor 12%, transparent);");
  });

  it("让标题栏药丸共用统一高度", () => {
    expect(styles).toContain("--toolbar-control-height: calc(var(--control-height) + 2px);");
    expect(styles).toContain(".codex-status-control {\n  display: inline-flex;\n  height: var(--toolbar-control-height);");
    expect(styles).toContain(".apple-action-button {\n  align-items: center;");
    expect(styles).toContain("height: var(--toolbar-control-height);");
    expect(styles).toContain(".app-input--pill {\n  height: var(--toolbar-control-height);");
    expect(styles).toContain(".provider-page-brand {\n  display: inline-flex;\n  height: var(--toolbar-control-height);");
  });

  it("让工具栏保持药丸圆角，分段控件使用圆角矩形", () => {
    expect(styles).toContain(".apple-toolbar-group {\n  border-radius: 999px;");
    expect(styles).toMatch(/\.app-segmented-control \{[^}]*border-radius: 0\.75rem;/);
  });

  it("让共享面板的分割线与内容左右内边距对齐", () => {
    expect(styles).toContain(".apple-panel-section + .apple-panel-section {\n  position: relative;\n  border-top: 0;");
    expect(styles).toContain(".apple-panel-section + .apple-panel-section::before {\n  content: \"\";\n  position: absolute;\n  top: 0;\n  right: var(--gap-card-inline);\n  left: var(--gap-card-inline);\n  border-top: 1px solid var(--panel-divider);");
  });

  it("所有编辑正文复用共享容器，不依赖 apple-group 的嵌套层数", () => {
    const files = ["codex/CodexProfileEdit", "profiles/ProfileIconEdit", "claude/ClaudeProfileEdit", "mcp/McpEdit", "mcp/ClaudeMcpEdit", "plugins/PluginDetailView", "plugins/AddPluginView"];
    // 「必须使用 apple-edit-surface」由 check-style-leaks 的结构检查兜底；这里守另一半：不得再挂 apple-group
    for (const file of files) {
      const page = readFileSync(new URL(`../features/${file}.tsx`, import.meta.url), "utf8");
      expect(page).not.toContain('className="apple-group');
    }
    expect(styles).not.toContain(".apple-edit-page > .apple-edit-content > .apple-group:not(.apple-list-card)");
  });

  it("编辑正文占满原卡片宽度，首分区不叠加上内边距", () => {
    expect(styles).toContain(".apple-edit-surface {\n  --gap-card-inline: 0px;");
    const rule = styles.match(/\.apple-edit-surface\.apple-panel-section,\n\.apple-edit-surface > \.apple-panel-section:first-child \{([^}]+)\}/)?.[1];
    expect(rule).toContain("padding-top: 0;");
    expect(styles).toContain(".apple-panel-section {\n  padding: var(--gap-card) var(--gap-card-inline);");
    expect(styles).toContain("padding-top: var(--gap-page);");
  });

  it("编辑正文末尾只保留滚动容器的共享留白", () => {
    const contentRule = styles.match(/\.apple-edit-content \{([^}]+)\}/)?.[1];
    expect(styles).toContain("--gap-section: 1rem;");
    expect(contentRule).toContain("padding-bottom: var(--gap-section);");
    const lastSectionRule = styles.match(/\.apple-edit-surface\.apple-panel-section,\n\.apple-edit-surface > \.apple-panel-section:last-child \{([^}]+)\}/)?.[1];
    expect(lastSectionRule).toContain("padding-bottom: 0;");
  });

  it("让配置卡片与独立列表卡片复用全局描边", () => {
    expect(styles).toContain("--card-edge-shadow: 0 0 0 0.5px var(--panel-border);");
    expect(styles).toContain(".panel,\n.apple-group,\n.app-segmented-control,\n.apple-list-row,\n.apple-editor-surface {\n  box-shadow: var(--card-edge-shadow);");
  });

  it("让技能预览器的 Markdown 分割线使用全局分割线", () => {
    expect(styles).toContain(".skill-markdown-preview hr {\n  border: 0;\n  border-top: 1px solid var(--panel-divider);");
  });

  it("将通知条与页面顶部操作按钮对齐", () => {
    expect(styles).toContain(".app-toast-viewport {\n  position: fixed;\n  top: 2.5rem;");
  });

  it("拖拽时不改变供应商标题颜色", () => {
    expect(styles).not.toContain(".profile-drag-preview.is-drag-hover h3");
  });

  it("编辑页滚动条收在主卡片边界内", () => {
    expect(profileEditSource).toContain('className="apple-edit-content"');
    expect(profileEditSource).toContain('className="apple-edit-surface"');
    expect(styles).not.toContain(".apple-edit-card-frame");
  });

  it("编辑页在详情完成后再一次性揭示，保留页面进入动画", () => {
    // 动画改由 AppShell 的 WAAPI 步进驱动（整设备像素，避免缓动尾段亚像素发虚），CSS 只留布局类
    expect(styles).toContain(".apple-page-enter {\n  display: flex;");
    expect(styles).not.toContain("@keyframes apple-page-enter");
    expect(source).toContain("function animatePageEnter");
    expect(source).toContain('matchMedia("(prefers-reduced-motion: reduce)")');
    expect(profileEditSource).toContain("if (((!create && !detail) || authStatusPending) && !loadError) return null;");
  });

  it("主区域统一使用 52px 物理像素左右留白，并从右侧内边距扣除滚动条槽", () => {
    expect(styles).toContain("--gap-main: 2.1666666667rem;");
    expect(styles).toContain("padding-right: calc(var(--gap-main) - 10px);");
  });

  it("让编辑页的表单大卡片与主视图使用相同圆角", () => {
    expect(styles).toContain(".apple-edit-content > .apple-group {\n  margin-top: 0;\n  border-radius: var(--radius-card);");
  });

  it("将 Skill 更新徽标锚定在导入按钮右上角，中心落在药丸边缘（一半压按钮一半露出）", () => {
    // 药丸高 38px → 半径 19px；偏移 = 尺寸/2 - 19×(1-cos45°) = 9 - 5.6 = 3.4px
    expect(styles).toContain(".apple-count-badge {\n  position: absolute;\n  left: auto;\n  right: -3.4px;\n  top: -3.4px;");
  });

  it("配置编辑器恢复原生横向滚动，不再自绘同步滚动条", () => {
    expect(styles).toContain("max-height: var(--editor-max-height);");
    expect(styles).toContain(".cm-editor .cm-scroller { overflow-x: auto; overflow-y: auto; overscroll-behavior: contain; }");
    expect(styles).not.toContain(".cm-horizontal-scrollbar");
  });

  it("通用模板复用 Skill 预览的视口高度，保留自己的宽度及其他编辑器限高", () => {
    const skillsSource = readFileSync(new URL("../features/skills/SkillsView.tsx", import.meta.url), "utf8");
    const templateRule = styles.match(/\.claude-template-dialog \{([^}]+)\}/)?.[1];
    expect(styles.includes("--dialog-preview-height: 60vh;")).toBe(true);
    expect(skillsSource.match(/h-\[var\(--dialog-preview-height\)\]/g)).toHaveLength(4);
    expect(templateRule).toContain("width: min(clamp(720px, 70vw, 900px), calc(100vw - 2rem));");
    expect(templateRule).toContain("--editor-max-height: var(--dialog-preview-height);");
    expect(styles.includes("--editor-max-height: min(34rem, 60vh);")).toBe(true);
  });

  it("编辑器滚动条在鼠标移出后隐藏，不受拖动留下的焦点影响", () => {
    expect(styles.includes(".apple-editor-shell:not(:hover) .cm-scroller::-webkit-scrollbar-thumb {\n  background: transparent;")).toBe(true);
    expect(styles.includes(".apple-editor-shell:not(:hover):not(:focus-within)")).toBe(false);
  });

  it("取消编辑器外围的焦点发光描边", () => {
    expect(styles).toContain(".cm-editor.cm-focused { outline: none; }");
    expect(styles).not.toContain(".apple-editor-shell:focus-within");
  });

  it("让有溢出的滚动条常驻显示缩略图", () => {
    expect(styles).toContain("*::-webkit-scrollbar-thumb {\n  background: var(--scrollbar-thumb);");
    expect(styles).toContain("scrollbar-color: var(--scrollbar-thumb) transparent;");
    expect(styles).not.toContain("*:hover::-webkit-scrollbar-thumb {\n  background: var(--scrollbar-thumb);");
  });

  it("避免侧栏宽度动画期间重测主区域滚动条占位", () => {
    expect(source).not.toContain("new ResizeObserver(updateScrollbarSize)");
    expect(source).toContain("updateScrollbarSize();");
  });

  it("让供应商卡片使用略圆的圆角", () => {
    expect(styles).toContain(".profile-list > .apple-group {\n  border-radius: var(--radius-card);");
  });

  it("新增带官网预设时显示官网地址输入框", () => {
    expect(profileEditSource).toContain("(!create || Boolean(selectedPreset?.admin_url))");
    expect(profileEditSource).not.toContain("(!create || selectedPreset?.base_url)");
  });

  it("官网地址按钮复用行内按钮样式并显示文案", () => {
    expect(profileEditSource).toContain('className="apple-inline-btn apple-inline-btn--quiet !h-5 shrink-0"');
    expect(profileEditSource).toContain('{t("card.openWebsite")}');
  });
});
