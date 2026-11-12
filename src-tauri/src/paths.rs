use std::path::{Path, PathBuf};

use crate::error::{app_err, AppResult};

/// 正式版的数据目录名。改名发布时旧数据由用户从设置页「导出备份 / 恢复」自行搬过来，
/// 应用不做目录迁移。
const ROOT_DIR: &str = ".agmux";
/// DEV（debug 构建）的数据目录名：与安装的正式版完全隔离。
/// 同一个库被两个版本轮流迁移会互相锁死：迁移只增不减，DEV 把库迁到 v26 后，
/// 迁移更少的正式版就再也开不了这个库（启动即崩）。DEV 不碰正式版的数据。
const DEV_ROOT_DIR: &str = ".agmux-dev";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub settings: PathBuf,
    /// 应用内更新「已更新到 vX」一次性标记：安装成功前写入，新版本启动时消费。
    /// 存文件而不是前端 localStorage——Windows 安装器会立即结束进程，WebView 的
    /// localStorage 异步落盘可能来不及提交导致标记丢失、通知不弹。
    pub update_marker: PathBuf,
    pub config_backup: PathBuf,
    pub database_backup: PathBuf,
    pub codex_files_backup: PathBuf,
    /// 运行日志目录：tauri-plugin-log 按大小轮转写入，保留最近几个归档
    pub logs: PathBuf,
    pub codex_home: PathBuf,
    /// Claude Code 的家目录（~/.claude）：供应商配置写入其下 settings.json。
    pub claude_home: PathBuf,
    /// Claude Desktop 3P 配置根候选：macOS 为 `~/Library/Application Support`，
    /// Windows 为 `%LOCALAPPDATA%`（MSIX 虚拟化候选在 claude_desktop 模块内解析）。
    pub claude_desktop_base: PathBuf,
    /// 供应商终端的 per-session settings 覆盖文件目录：只放 `claude --settings`
    /// 的临时覆盖，不进 ~/.claude，不参与 apply 与回写。
    pub terminal: PathBuf,
    /// 插件图标瘦身缓存：超重位图解码缩放后的重编码结果落这里，键含源文件 mtime。
    pub plugin_icons: PathBuf,
}

impl AppPaths {
    pub fn codex_config(&self) -> PathBuf {
        self.codex_home.join("config.toml")
    }

    /// Claude Code 用户范围 MCP 配置（官方格式：用户目录下的 ~/.claude.json）。
    pub fn claude_mcp_config(&self) -> PathBuf {
        self.claude_home
            .parent()
            .unwrap_or(&self.claude_home)
            .join(".claude.json")
    }

    /// 某个 Claude 供应商的终端覆盖文件：固定命名、每次覆写，不随进程删除。
    pub fn terminal_settings(&self, profile_id: &str) -> PathBuf {
        self.terminal.join(format!("claude_{profile_id}.json"))
    }

    pub fn ensure(&self) -> AppResult<()> {
        for dir in [
            &self.root,
            &self.config_backup,
            &self.database_backup,
            &self.codex_files_backup,
            &self.logs,
            &self.terminal,
            &self.plugin_icons,
        ] {
            std::fs::create_dir_all(dir)
                .map_err(|error| app_err!("无法创建目录 {}: {error}", dir.display()))?;
        }
        Ok(())
    }
}

pub fn app_paths() -> AppResult<AppPaths> {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut paths = from_home(&home_dir())?;
    // 重定向过的 %LOCALAPPDATA% 只在真实环境读取；from_home 保持纯 home 派生，
    // 测试注入的临时 home 才能把 Claude Desktop 3P 路径一起隔离进 tempdir。
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.claude_desktop_base = PathBuf::from(local);
    }
    Ok(paths)
}

/// 数据根目录名：正式版 `.agmux`，DEV（debug 构建）`.agmux-dev`。
/// 按构建类型分开，是为了让 DEV 的数据库迁移永远碰不到正式版的库：
/// 迁移只追加不重写，DEV 迁过的库对迁移更少的正式版来说就是「版本太高」，
/// 正式版会直接起不来。DEV 与正式版除了 ~/.codex、~/.claude 这些被管的
/// 客户端配置外，不共享任何状态。
fn data_root_dir() -> &'static str {
    if cfg!(debug_assertions) {
        DEV_ROOT_DIR
    } else {
        ROOT_DIR
    }
}

pub fn from_home(home: &Path) -> AppResult<AppPaths> {
    let root = home.join(data_root_dir());
    Ok(AppPaths {
        database: root.join("agmux.db"),
        settings: root.join("settings.json"),
        update_marker: root.join("update-marker"),
        config_backup: root.join("backups").join("config"),
        database_backup: root.join("backups").join("database"),
        codex_files_backup: root.join("backups").join("codex-files"),
        logs: root.join("logs"),
        terminal: root.join("terminal"),
        plugin_icons: root.join("plugin-icons"),
        codex_home: home.join(".codex"),
        claude_home: home.join(".claude"),
        claude_desktop_base: claude_desktop_base(home),
        root,
    })
}

/// macOS：`~/Library/Application Support`。Windows：默认 `home\AppData\Local`
///（真实环境的 %LOCALAPPDATA% 覆盖在 app_paths；MSIX 候选在 claude_desktop 模块内按目录存在性选择）。
/// 其他平台按 XDG 惯例给 `.config`（产品不支持，仅让路径可解析）。
fn claude_desktop_base(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library").join("Application Support")
    }
    #[cfg(windows)]
    {
        home.join("AppData").join("Local")
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        home.join(".config")
    }
}

pub fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|base| base.home_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

/// Unix 秒（i64 语义由调用方按需转换）：备份文件名与 token 时间戳共用同一个"当前时刻"定义。
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_use_agmux_and_codex_directories() {
        let home = Path::new("/home/user");
        let paths = from_home(home).unwrap();
        // 数据根目录按构建类型分开：DEV 用 .agmux-dev，正式版用 .agmux
        let root = home.join(data_root_dir());
        assert_eq!(
            paths.root,
            home.join(if cfg!(debug_assertions) {
                ".agmux-dev"
            } else {
                ".agmux"
            })
        );
        assert_eq!(paths.root, root);
        assert_eq!(paths.database, root.join("agmux.db"));
        assert_eq!(paths.logs, root.join("logs"));
        assert_eq!(
            paths.codex_config(),
            home.join(".codex").join("config.toml")
        );
        assert_eq!(paths.claude_home, home.join(".claude"));
        assert_eq!(paths.claude_mcp_config(), home.join(".claude.json"));
        #[cfg(target_os = "macos")]
        assert_eq!(
            paths.claude_desktop_base,
            home.join("Library").join("Application Support")
        );
        // from_home 不读 %LOCALAPPDATA%：测试注入的 home 必须连桌面端路径一起隔离
        #[cfg(windows)]
        assert_eq!(
            paths.claude_desktop_base,
            home.join("AppData").join("Local")
        );
        assert_eq!(paths.terminal, root.join("terminal"));
        assert_eq!(
            paths.terminal_settings("p-1"),
            root.join("terminal").join("claude_p-1.json")
        );
    }
}
