//! 插件市场服务：GitHub 插件的发现、安装与本地生命周期。
//!
//! 安装模型（以实测的 Codex CLI 0.149 为准）：
//! - 安装 = `codex plugin marketplace add <git 源>` + `codex plugin add <插件@市场>`；
//!   卸载 = `codex plugin remove <插件@市场>`——官方路径，状态由 Codex 自己维护；
//! - 预览走 Agmux 自己的 GitHub 拉取（清单、文件列表、内容类型，不落盘）；
//! - 安装清单与操作能力来自 `plugin/installed`，不将缓存目录当作已安装状态；
//! - origin 只区分官方与第三方来源，操作能力由 CLI 的 source 与 installPolicy 决定。

use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::AppContext;

/// 前端展示用的已安装插件摘要。
#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginSummary {
    pub name: String,
    pub version: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub desktop_visible: bool,
    pub can_toggle: bool,
    pub can_uninstall: bool,
    /// 来源标签，不代表操作权限。
    pub origin: String,
    /// Codex 返回的市场名（卸载选择器要用）。
    pub marketplace: Option<String>,
    pub store_path: String,
    pub source_url: Option<String>,
    /// 插件目录内的本地图标文件（assets/icon.png 约定）；无则前端回落首字母。
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexAppSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub icon_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginSkill {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginMarketplace {
    pub name: String,
    pub root: String,
    pub kind: String,
    pub source_url: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MarketplacePlugin {
    pub plugin_id: String,
    pub name: String,
    pub version: Option<String>,
    pub installed: bool,
    pub auth_policy: String,
    pub source: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// 市场物化目录内的本地图标（未安装插件同样有本地 checkout 文件）。
    pub icon_path: Option<String>,
}

/// 外部市场快照更新后，已安装插件的可升级项。
#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginUpdate {
    pub name: String,
    pub marketplace: String,
    pub version: String,
}

/// Skill 启停的目标工具：同一份仓库正本分发到各工具自己的 skills 目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillTool {
    Codex,
    Claude,
}

impl SkillTool {
    /// 目标工具的 skills 分发根（相对用户家目录）；两边同为 `<name>/SKILL.md` 布局。
    pub fn skills_dir(self, home: &Path) -> std::path::PathBuf {
        match self {
            SkillTool::Codex => home.join(".codex/skills"),
            SkillTool::Claude => home.join(".claude/skills"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SkillSummary {
    pub name: String,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub store_path: String,
    pub source_path: Option<String>,
    pub update_available: bool,
    pub enabled: bool,
    /// Claude Code 侧的分发状态（`~/.claude/skills/<name>` 是否存在）。
    pub claude_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SkillCandidate {
    pub name: String,
    pub description: Option<String>,
    pub store_path: String,
    pub source: String,
    pub has_content_conflict: bool,
    pub is_update: bool,
    pub modified_at: u64,
}

/// plugin.json 的解析子集（官方字段很多，这里只取列表与详情需要的）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginManifest {
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    logo: Option<String>,
    #[serde(default)]
    interface: Option<PluginInterface>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginInterface {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    short_description: Option<String>,
    #[serde(default)]
    long_description: Option<String>,
    /// 图标的权威相对路径（实测 github → ./assets/github.png）；logoDark 为深色版备选。
    #[serde(default)]
    logo: Option<String>,
    #[serde(default)]
    logo_dark: Option<String>,
}

pub(super) const MANIFEST_RELATIVE_PATH: &str = ".codex-plugin/plugin.json";
/// Claude 布局的清单回退路径（多 Agent 插件两种都有，如 ponytail）。
pub(super) const CLAUDE_MANIFEST_RELATIVE_PATH: &str = ".claude-plugin/plugin.json";
pub(super) const SKILL_SOURCE_FILE: &str = ".sources.json";
pub(super) const SKILL_BACKUP_DIRECTORY: &str = ".backups";
/// Codex marketplace 插件的物化缓存（仅用于补充文件元数据）。
pub(super) const PLUGIN_CACHE_RELATIVE_PATH: &str = "plugins/cache";

pub(super) mod appserver;
mod catalog;
mod claude;
mod cli;
mod ops;
mod skills;
mod store;

pub use claude::{
    ClaudeAvailablePlugin, ClaudeMarketplace, ClaudePluginEntry, ClaudePluginInventory,
    ClaudePluginUpdate, ClaudePluginUpdateResult,
};
pub(super) use cli::find_claude_cli;
pub(crate) use cli::{wait_child_with_timeout, ChildOutput};

#[cfg(test)]
pub(super) fn test_context() -> (tempfile::TempDir, AppContext) {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    let context = AppContext::new(paths).unwrap();
    (home, context)
}
