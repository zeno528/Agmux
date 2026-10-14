/** App update strings (status-bar notice, upgrade flow). */
export default {
  notice: {
    title: "Update available",
    available: "Update",
    updateNow: "Restart to update",
    later: "Later",
    installing: "Downloading…",
    noNotes: "No release notes for this version",
  },
  toast: {
    updated: "Updated to v{{version}}",
  },
  error: {
    proxyHint: "Cannot reach GitHub. Check your system proxy and try again",
    checkFailed: "Update check failed",
  },
  webMock: {
    notes: `## Highlights

- Faster cold start and smoother window restore
- Redesigned provider switcher with inline health status

## New

- **MCP health probes**: per-server latency and error rate in the MCP view
- **Skills sync**: one-click sync from \`.codex/skills\` with conflict preview
- Full \`zh-CN\` / \`en-US\` coverage for every management page

## Fixes

- Auth status no longer flickers when switching profiles quickly
- Update dialog stays in the background while installing
- Backup retention now respects the configured 10-file limit

## Notes

- Web mock only: this environment does not download, install, or restart the app
- See the [release page](https://example.com/releases) for the full changelog`,
  },
} as const;
