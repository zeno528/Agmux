//! 插件市场服务：GitHub 插件的发现、安装与本地生命周期。
//!
//! 安装模型（以实测的 Codex CLI 0.149 为准）：
//! - 安装 = `codex plugin marketplace add <git 源>` + `codex plugin add <插件@市场>`；
//!   卸载 = `codex plugin remove <插件@市场>`——官方路径，状态由 Codex 自己维护；
//! - 预览走 Agmux 自己的 GitHub 拉取（清单、文件列表、内容类型，不落盘）；
//! - 列表以 `codex plugin list` 为主源（覆盖官方运行时/捆绑/外部市场，含启停状态），
//!   CLI 不在时回退扫 `~/.codex/plugins/cache/`；Skill 注册表 `~/.agents/.skill-lock.json`
//!   与家目录四套 marketplace 布局的 local 条目也在列；
//! - origin 语义：agmux=本应用经 CLI 安装；codex=用户自装的外部市场插件（可卸载）；
//!   official=openai 运行时/捆绑市场（只读）；skill=Skill 注册表（只读）；
//!   personal/claude/cursor=家目录 local 条目（可禁用/移除，条目暂存可恢复）。

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
    pub category: Option<String>,
    pub capabilities: Vec<String>,
    pub contains: Vec<String>,
    pub enabled: bool,
    /// official=OpenAI 官方市场（只读）；codex=Codex 管理的外部市场。
    pub origin: String,
    /// 来自 `codex plugin list` 的市场名（卸载选择器要用）。
    pub marketplace: Option<String>,
    pub store_path: String,
    pub source_url: Option<String>,
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
    pub category: Option<String>,
    pub capabilities: Vec<String>,
    pub contains: Vec<String>,
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

/// 预览阶段的候选插件（一个仓库可能包含多个插件根目录）。
#[derive(Debug, Clone, Serialize)]
pub struct PluginCandidate {
    pub sub_path: String,
    pub name: String,
    pub version: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub capabilities: Vec<String>,
    pub contains: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPreview {
    pub repo: String,
    pub reference: String,
    pub default_branch: String,
    pub candidates: Vec<PluginCandidate>,
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
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

pub(super) const MANIFEST_RELATIVE_PATH: &str = ".codex-plugin/plugin.json";
/// Claude 布局的清单回退路径（多 Agent 插件两种都有，如 ponytail）。
pub(super) const CLAUDE_MANIFEST_RELATIVE_PATH: &str = ".claude-plugin/plugin.json";
pub(super) const SKILL_SOURCE_FILE: &str = ".sources.json";
pub(super) const SKILL_BACKUP_DIRECTORY: &str = ".backups";
/// Codex marketplace 插件的物化缓存（相对 codex home，CLI 缺席时的回退数据源）。
pub(super) const PLUGIN_CACHE_RELATIVE_PATH: &str = "plugins/cache";

use catalog::manifest_description;

impl PluginSummary {
    fn from_parts(
        name: &str,
        manifest: Option<&PluginManifest>,
        contains: Vec<String>,
        origin: &str,
        store_path: &Path,
    ) -> Self {
        let interface = manifest.and_then(|item| item.interface.as_ref());
        Self {
            name: manifest
                .map(|item| item.name.clone())
                .unwrap_or_else(|| name.to_string()),
            version: manifest.and_then(|item| item.version.clone()),
            display_name: interface.and_then(|item| item.display_name.clone()),
            description: manifest.and_then(manifest_description),
            category: interface.and_then(|item| item.category.clone()),
            capabilities: interface
                .map(|item| item.capabilities.clone())
                .unwrap_or_default(),
            contains,
            enabled: false,
            origin: origin.to_string(),
            marketplace: None,
            store_path: store_path.display().to_string(),
            source_url: None,
        }
    }
}

mod catalog;
mod cli;
mod ops;
mod skills;
mod store;

pub(crate) use cli::{wait_child_with_timeout, ChildOutput};

#[cfg(test)]
pub(super) fn test_context() -> (tempfile::TempDir, AppContext) {
    let home = tempfile::tempdir().unwrap();
    let paths = crate::paths::from_home(home.path()).unwrap();
    paths.ensure().unwrap();
    let context = AppContext::new(paths).unwrap();
    (home, context)
}
