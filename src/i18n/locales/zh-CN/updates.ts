/** 应用更新相关文案（状态栏提示、升级流程）。 */
export default {
  notice: {
    title: "有可用更新",
    available: "有可用更新",
    updateNow: "立即重启更新",
    later: "稍后",
    installing: "下载安装中…",
    noNotes: "本次更新暂无日志",
  },
  toast: {
    updated: "已更新到 v{{version}}",
  },
  error: {
    proxyHint: "无法连接 GitHub，请检查网络代理后重试",
    checkFailed: "检查更新失败",
  },
  webMock: {
    notes: `## 亮点

- 冷启动更快，窗口恢复更流畅
- 供应商切换器重做，内联展示健康状态

## 新功能

- **MCP 健康探测**：MCP 页按服务器展示延迟与错误率
- **Skills 同步**：一键同步 \`.codex/skills\`，支持冲突预览
- 全部管理页补齐 \`zh-CN\` / \`en-US\` 文案

## 修复

- 快速切换供应商时认证状态不再闪烁
- 安装过程中更新弹窗不再抢占前台焦点
- 备份保留策略现遵守配置的 10 份上限

## 说明

- 仅 Web 调试模拟：此环境不会下载、安装或重启应用
- 完整更新日志见[发布页面](https://example.com/releases)`,
  },
} as const;
