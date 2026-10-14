// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { SkillTargetLogo } from "../../components/SkillTargetLogo";
import { setupI18n } from "../../i18n";
import McpDiffPage, { mcpDiffLines } from "./McpDiffPage";
import { mcpEntryAction } from "./McpView";
import type { McpSyncDiffEntry } from "../../types";

const viewSource = readFileSync(new URL("./McpView.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");
const pageSource = readFileSync(new URL("./McpDiffPage.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

const entry = (overrides: Partial<McpSyncDiffEntry>): McpSyncDiffEntry => ({
  name: "fixture-server",
  kind: "changed",
  live_spec: null,
  db_spec: null,
  live_toml: null,
  db_toml: null,
  ...overrides,
});

describe("MCP 差异动词映射", () => {
  it("同步 = 采纳外部修改：live_only/changed 把 live 片段写入镜像，db_only 删除镜像条目", () => {
    expect(mcpEntryAction(entry({ kind: "live_only", live_toml: "[mcp_servers.s]" }), "adopt")).toEqual({ side: "mirror", name: "fixture-server", fragment: "[mcp_servers.s]" });
    expect(mcpEntryAction(entry({ kind: "changed", live_toml: "[mcp_servers.s]\nurl = \"l\"" }), "adopt")).toEqual({ side: "mirror", name: "fixture-server", fragment: "[mcp_servers.s]\nurl = \"l\"" });
    expect(mcpEntryAction(entry({ kind: "db_only", db_toml: "D" }), "adopt")).toEqual({ side: "mirror", name: "fixture-server", fragment: null });
  });

  it("撤销 = 回退外部修改：changed/db_only 把数据库片段写回 live，live_only 从 live 移除", () => {
    expect(mcpEntryAction(entry({ kind: "live_only", live_toml: "L" }), "revert")).toEqual({ side: "live", name: "fixture-server", fragment: null });
    expect(mcpEntryAction(entry({ kind: "changed", db_toml: "[mcp_servers.s]\nurl = \"d\"" }), "revert")).toEqual({ side: "live", name: "fixture-server", fragment: "[mcp_servers.s]\nurl = \"d\"" });
    expect(mcpEntryAction(entry({ kind: "db_only", db_toml: "D" }), "revert")).toEqual({ side: "live", name: "fixture-server", fragment: "D" });
  });

  it("差异数据缺失时返回 null，由调用方报错而不是抛异常", () => {
    expect(mcpEntryAction(entry({ kind: "live_only", live_toml: null }), "adopt")).toBeNull();
    expect(mcpEntryAction(entry({ kind: "changed", db_toml: null }), "revert")).toBeNull();
  });
});

describe("MCP 差异行级对比", () => {
  it("内容相同：全部为 same 行", () => {
    const toml = '[mcp_servers.a]\ncommand = "npx"';
    expect(mcpDiffLines(toml, toml)).toEqual([
      { type: "same", text: "[mcp_servers.a]" },
      { type: "same", text: 'command = "npx"' },
    ]);
  });

  it("单行修改：旧行 del、新行 add，未涉及行保持 same", () => {
    const lines = mcpDiffLines('command = "old"', 'command = "new"');
    expect(lines).toEqual([
      { type: "del", text: 'command = "old"' },
      { type: "add", text: 'command = "new"' },
    ]);
    const withContext = mcpDiffLines('[mcp_servers.a]\ncommand = "old"', '[mcp_servers.a]\ncommand = "new"');
    expect(withContext).toEqual([
      { type: "same", text: "[mcp_servers.a]" },
      { type: "del", text: 'command = "old"' },
      { type: "add", text: 'command = "new"' },
    ]);
  });

  it("新增行与删除行各自成块", () => {
    expect(mcpDiffLines("a", "a\nb")).toEqual([
      { type: "same", text: "a" },
      { type: "add", text: "b" },
    ]);
    expect(mcpDiffLines("a\nb", "a")).toEqual([
      { type: "same", text: "a" },
      { type: "del", text: "b" },
    ]);
  });

  it("忽略结尾空行差异，不产生伪变更", () => {
    expect(mcpDiffLines("a\n", "a")).toEqual([{ type: "same", text: "a" }]);
    expect(mcpDiffLines("a", "a\n\n")).toEqual([{ type: "same", text: "a" }]);
  });

  it("忽略开头空行差异，代码块顶部不出现空档", () => {
    expect(mcpDiffLines("\n[mcp_servers.a]", "[mcp_servers.a]")).toEqual([{ type: "same", text: "[mcp_servers.a]" }]);
    expect(mcpDiffLines("[mcp_servers.a]", "\n\n[mcp_servers.a]")).toEqual([{ type: "same", text: "[mcp_servers.a]" }]);
  });
});

describe("MCP 差异二级页", () => {
  it.each(["codex", "claude"] as const)("%s 差异标题复用对应客户端编辑页的标识", (target) => {
    setupI18n("zh-CN");
    const html = renderToStaticMarkup(createElement(McpDiffPage, {
      target, preview: null, previewError: "", resolving: false,
      onBack: () => undefined, onResolve: () => undefined, onResolveAll: () => undefined,
    }));
    const logo = renderToStaticMarkup(createElement(SkillTargetLogo, { target, variant: "title" }));
    expect(html).toContain(`${logo}<span class="apple-title">`);
  });

  it("有差异才显示列表提示，数字显示在客户端切换器", () => {
    // 入口条件与角标文本都走共享规则（见 managementDataCache），页面不再自己判一遍
    expect(viewSource).toContain("!header?.editing && mcpDiffBadgeText(targetBadge)");
    expect(viewSource).toContain('t("list.viewDiff")');
    expect(viewSource).not.toContain("{header?.badgeText ? (");
    expect(viewSource).toContain('className="apple-count-badge"');
    expect(viewSource).not.toContain("list.diffChip");
    expect(viewSource).not.toContain("McpSyncDialog");
  });

  it("窗口激活时刷新差异预览（差异只可能来自 Codex 侧先改），首次挂载不重复请求", () => {
    expect(viewSource).toContain('useEffect(() => { if (activationEpoch === 0) return; if (target === "claude") void refresh(true, []); else void loadPreview(); }, [activationEpoch]);');
  });

  it("差异始终展开，保留 MCP 卡片头部", () => {
    expect(pageSource).toContain('<McpDiffDetail entry={entry} />');
    expect(pageSource).not.toContain("apple-disclosure");
    expect(pageSource).not.toContain("collapsed");
    expect(pageSource).not.toContain("onToggle");
    expect(pageSource).toContain('className="apple-list-row mcp-expanded-card__header"');
    expect(pageSource).not.toContain("mcp-diff-detail");
  });

  it("内容被修改的差异按行级对比渲染（db_toml vs live_toml），不做字段值配对", () => {
    expect(pageSource).toContain("mcpDiffLines(entry.db_toml ?? \"\", entry.live_toml ?? \"\")");
    expect(pageSource).toContain('className="mcp-diff-block"');
    expect(pageSource).not.toContain("changed_fields.map");
  });

  it("动词按钮使用统一 action-button 样式并显示图标与文本", () => {
    expect(pageSource).toContain('onResolve(entry, "revert")');
    expect(pageSource).toContain('onResolve(entry, "adopt")');
    expect(pageSource).toContain('className="apple-action-button mcp-diff-action-button text-[var(--text-secondary)] enabled:hover:text-accent disabled:cursor-not-allowed disabled:opacity-40"');
    expect(pageSource).toContain('{t("diff.revert")}');
    expect(pageSource).toContain('{t("diff.adopt")}');
    expect(pageSource).not.toContain("apple-inline-btn");
  });

  it("卡片不再响应空白处点击，正文允许选中", () => {
    expect(pageSource).toContain('className="apple-group"');
    expect(pageSource).not.toContain("onClick={(event) => {");
  });

  it("标题栏提供悬停说明卡片，解释红绿语义与两个动词，关键词加粗", () => {
    expect(pageSource).toContain('t("diff.help.title")');
    expect(pageSource).toContain('<strong className="font-semibold text-(--danger)">{t("diff.help.red.keyword")}</strong>');
    expect(pageSource).toContain('<strong className="font-semibold text-(--success)">{t("diff.help.green.keyword")}</strong>');
    expect(pageSource).toContain('<strong className="font-semibold">{t("diff.help.adopt.keyword")}</strong>');
    expect(pageSource).toContain('<strong className="font-semibold">{t("diff.help.revert.keyword")}</strong>');
    expect(pageSource).toContain('<AppTooltip label={t("diff.help.title")}>');
  });

  it("解析失败时先讲后果与重建边界，报错原文默认摊开", () => {
    // 用户要知道的是"Codex 起不来"，不是解析器报错原文
    expect(pageSource).toContain('{t("diff.parseFailedTitle")}');
    expect(pageSource).toContain('{t(onRebuild ? "diff.parseFailedImpact" : "diff.claudeParseFailedImpact")}');
    expect(pageSource).toContain('{t("diff.rebuild")}');
    // 报错原文是唯一能定位到行的信息，这页就是为它存在的：摊开，不藏进折叠
    expect(pageSource).toContain('{t("diff.rawError")}');
    expect(pageSource).toContain("{previewError}</pre>");
    expect(pageSource).not.toContain("<details");
    expect(pageSource).not.toContain("diff.rebuildDescription");
    expect(pageSource).not.toContain("diff.rebuildScope");
  });

  it("图标锚在标题行右侧，不随内容高度拉伸、也不产生悬挂缩进", () => {
    // self-stretch 是账号卡那种多行文本块的处理；这里图标跟的是单行标题，拉伸会让它飘到卡片中间
    expect(pageSource).not.toContain("self-stretch");
    // 图标在标题之后：卡片里所有文本左边缘对齐
    expect(pageSource).toContain('<div className="setting-title">{t("diff.parseFailedTitle")}</div>\n              <CircleAlert className="h-[18px] w-[18px] shrink-0 text-[var(--danger)]" strokeWidth={2} aria-hidden="true" />');
  });

  it("页脚提供全部同步/全部撤销批量动作", () => {
    expect(pageSource).toContain('onResolveAll("revert")');
    expect(pageSource).toContain('onResolveAll("adopt")');
  });

  it("只重连配置真的被改过的那几台，不因为「数据变了」就无脑全量重探", () => {
    // 探测名单参数化：不传 = 全连（挂载/整段重建）
    expect(viewSource).toContain("const refresh = async (force = false, only?: string[]) => {");
    expect(viewSource).toContain("const targets = only ? next.filter((server) => only.includes(server.name)) : next;");
    // 删除：剩下的几台配置没动 → 一台都不连
    expect(viewSource).toContain("await refresh(true, []);");
    // 逐条处理：同步只写数据库镜像（不碰配置文件）→ 不连；撤回只动这一台 → 只连它
    expect(viewSource).toContain('await refresh(true, verb === "revert" ? [entry.name] : []);');
    // 批量：同上，名单换成被处理的那几台
    expect(viewSource).toContain('await refresh(true, verb === "revert" ? actions.map((action) => action.name) : []);');
    // 编辑保存：不是"跳过它、其余全连"（那等于全量重探），而是谁都不连，
    // 随后 probe 只验刚保存的那一台的连通性——保存的目的是"这台还能不能用"，
    // 不是工具清单（那是点扳手才有的事），所以这里不能传 includeTools
    const saveAnchor = viewSource.indexOf("onBack={(savedServer)");
    const savePath = viewSource.slice(saveAnchor, saveAnchor + 700);
    expect(savePath).toContain("refresh(true, [])");
    // 静默：保存已经弹过"已保存"，连通结果看那一行的状态灯，不再叠一条通知
    expect(savePath).toContain("probe(savedServer, { manual: false })");
    // 探测不等 refresh 的返回值（它可能为 null 把探测整个吞掉），顺序也不能反
    expect(savePath.indexOf("await refresh(true, [])")).toBeLessThan(savePath.indexOf("probe(savedServer"));
    expect(savePath).not.toContain("includeTools");
    expect(viewSource).not.toContain("skip");
  });

  it("批量动作整批一次提交，不再逐条调用（逐条会各备份一次并留下半完成状态）", () => {
    expect(viewSource).toContain("api.setMcpMirrorEntries(actions)");
    expect(viewSource).toContain("api.revertMcpLiveEntries(actions)");
    expect(viewSource).toContain("await applyDiffActions(actions, verb)");
    expect(viewSource).not.toContain("for (const entry of entries)");
  });
});
